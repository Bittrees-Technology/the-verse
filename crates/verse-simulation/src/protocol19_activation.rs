// SPDX-License-Identifier: AGPL-3.0-or-later

//! Signed, universe-wide protocol-19 activation and verified boot.
//!
//! The root active head is the sole commit point. Before it exists, files in
//! the activation namespace are staging debris. After it exists, recovery is
//! verification-only and never falls back to protocol 18.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, VerifyingKey};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use verse_protocol::protocol_v19::Protocol19CompatibilityTuple;

#[cfg(test)]
use crate::cell_directory::CellAssignmentRecord;
use crate::cell_directory::CellDirectoryError;
use crate::grid_handoff_v2::migration_transform::ValidatedProtocol19MigrationTransform;
use crate::persistence::{SystemTrustedClock, TrustedClock};
use crate::protocol19_install::{
    OpenedProtocol19PreparedInstall, PreparedProtocol19World, Protocol19InstallError,
    Protocol19PreparedInstallSummary,
};
use crate::protocol19_source::ValidatedFrozenProtocol18Source;

pub const ACTIVE_PROTOCOL_HEAD_FILE: &str = "active-protocol-head-v1.json";
const ACTIVATION_DIRECTORY: &str = "protocol-19-activation-v1";
const ACTIVATION_LOCK_FILE: &str = "writer.lock";
const UNIVERSE_LIFECYCLE_HEAD_FILE: &str = "universe-lifecycle-v1.head.json";
const POLICY_SCHEMA_VERSION: u32 = 1;
const AUTHORIZATION_SCHEMA_VERSION: u32 = 1;
const ACTIVE_HEAD_SCHEMA_VERSION: u32 = 1;
const SIGNATURE_SCHEME: &str = "ed25519-strict-v1";
const AUTHORIZATION_KIND: &str = "activate_prepared_protocol19_world";
const ACTIVE_MODE: &str = "active";
const POLICY_HASH_DOMAIN: &[u8] = b"the-verse/protocol-19-activation-policy/v1\0";
const SIGNER_ID_DOMAIN: &[u8] = b"the-verse/protocol-19-activation-signer/v1\0";
const AUTHORIZATION_SIGNING_DOMAIN: &[u8] =
    b"the-verse/protocol-19-world-activation-authorization/v1\0";
const AUTHORIZATION_HASH_DOMAIN: &[u8] =
    b"the-verse/protocol-19-signed-activation-authorization/v1\0";
const ACTIVE_HEAD_HASH_DOMAIN: &[u8] = b"the-verse/protocol-19-active-head/v1\0";
const MAX_POLICY_BYTES: usize = 64 * 1_024;
const MAX_AUTHORIZATION_BYTES: usize = 256 * 1_024;
const MAX_ACTIVE_HEAD_BYTES: usize = 256 * 1_024;
const MAX_AUTHORIZATION_VALIDITY_MILLIS: u64 = 24 * 60 * 60 * 1_000;
const REQUIRED_SIGNER_COUNT: usize = 3;
const REQUIRED_THRESHOLD: u16 = 2;

#[derive(Debug, Error)]
pub enum Protocol19ActivationError {
    #[error("protocol-19 activation is invalid: {0}")]
    Invalid(String),
    #[error("protocol-19 activation JSON is invalid: {0}")]
    Json(String),
    #[error("protocol-19 activation file is too large: {0}")]
    TooLarge(PathBuf),
    #[error("protocol-19 activation I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("another protocol-19 activation or runtime process is active")]
    WriterConflict,
    #[error("protocol-19 activation injected failure: {0:?}")]
    Injected(Protocol19ActivationFailpoint),
    #[error("protocol-19 prepared installation failed: {0}")]
    Install(String),
    #[error(transparent)]
    Directory(#[from] CellDirectoryError),
}

impl From<Protocol19InstallError> for Protocol19ActivationError {
    fn from(source: Protocol19InstallError) -> Self {
        Self::Install(source.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol19ActivationFailpoint {
    NamespaceSynced,
    AuthorizationSynced,
    HistoryHeadSynced,
    SelectorTempSyncedBeforeRename,
    SelectorRenamedBeforeDirectorySync,
    SelectorDirectorySyncedBeforeMemory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivationSignerV1 {
    signer_id: String,
    public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivationTrustPolicyDocumentV1 {
    schema_version: u32,
    signature_scheme: String,
    universe_id: String,
    policy_generation: u64,
    threshold: u16,
    maximum_authorization_validity_millis: u64,
    signers: Vec<ActivationSignerV1>,
    policy_hash: String,
}

#[derive(Debug, Clone)]
pub struct Protocol19ActivationTrustPolicy {
    document: ActivationTrustPolicyDocumentV1,
    verifying_keys: Vec<(String, VerifyingKey)>,
    bytes: Vec<u8>,
}

impl Protocol19ActivationTrustPolicy {
    #[cfg(test)]
    pub fn from_public_keys(
        universe_id: impl Into<String>,
        policy_generation: u64,
        maximum_authorization_validity_millis: u64,
        public_keys: [[u8; 32]; REQUIRED_SIGNER_COUNT],
    ) -> Result<Self, Protocol19ActivationError> {
        let mut signers = public_keys
            .into_iter()
            .map(|public_key| ActivationSignerV1 {
                signer_id: protocol19_activation_signer_id(&public_key),
                public_key: encode_hex(&public_key),
            })
            .collect::<Vec<_>>();
        signers.sort_by(|left, right| left.signer_id.cmp(&right.signer_id));
        let mut document = ActivationTrustPolicyDocumentV1 {
            schema_version: POLICY_SCHEMA_VERSION,
            signature_scheme: SIGNATURE_SCHEME.into(),
            universe_id: universe_id.into(),
            policy_generation,
            threshold: REQUIRED_THRESHOLD,
            maximum_authorization_validity_millis,
            signers,
            policy_hash: String::new(),
        };
        document.policy_hash = calculate_policy_hash(&document)?;
        let bytes = serde_json::to_vec(&document)
            .map_err(|source| Protocol19ActivationError::Json(source.to_string()))?;
        Self::from_canonical_bytes(&bytes, &document.policy_hash)
    }

    pub fn from_canonical_bytes(
        bytes: &[u8],
        expected_policy_hash: &str,
    ) -> Result<Self, Protocol19ActivationError> {
        if bytes.is_empty() || bytes.len() > MAX_POLICY_BYTES {
            return Err(Protocol19ActivationError::TooLarge(PathBuf::from(
                "activation-policy-v1.json",
            )));
        }
        let document = serde_json::from_slice::<ActivationTrustPolicyDocumentV1>(bytes)
            .map_err(|source| Protocol19ActivationError::Json(source.to_string()))?;
        let canonical = serde_json::to_vec(&document)
            .map_err(|source| Protocol19ActivationError::Json(source.to_string()))?;
        if canonical != bytes {
            return Err(Protocol19ActivationError::Invalid(
                "activation policy bytes are not canonical".into(),
            ));
        }
        validate_policy_document(&document, expected_policy_hash)?;
        let mut verifying_keys = Vec::with_capacity(document.signers.len());
        for signer in &document.signers {
            let public_key = decode_hex_array::<32>(&signer.public_key, "signer public key")?;
            let key = VerifyingKey::from_bytes(&public_key).map_err(|_| {
                Protocol19ActivationError::Invalid("signer public key is invalid".into())
            })?;
            verifying_keys.push((signer.signer_id.clone(), key));
        }
        Ok(Self {
            document,
            verifying_keys,
            bytes: bytes.to_vec(),
        })
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn policy_hash(&self) -> &str {
        &self.document.policy_hash
    }

    pub const fn policy_generation(&self) -> u64 {
        self.document.policy_generation
    }

    pub fn universe_id(&self) -> &str {
        &self.document.universe_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Protocol19ActivationAuthorizationV1 {
    pub schema_version: u32,
    pub authorization_kind: String,
    pub signature_scheme: String,
    pub compatibility: Protocol19CompatibilityTuple,
    pub universe_id: String,
    pub world_seed: String,
    pub prepared_install_head_hash: String,
    pub migration_receipt_hash: String,
    pub migration_anchor_hash: String,
    pub target_manifest_hash: String,
    pub target_directory_document_hash: String,
    pub target_assignment_root: String,
    pub target_placement_root: String,
    pub cell_set_root: String,
    pub global_conservation_root: String,
    pub normalized_gameplay_root: String,
    pub identity_map_root: String,
    pub production_origin_root: String,
    pub cell_count: u64,
    pub signer_policy_hash: String,
    pub signer_policy_generation: u64,
    pub activation_generation: u64,
    pub authorization_nonce: String,
    pub authorized_activation_unix_ms: u64,
    pub not_before_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub previous_activation_head_hash: String,
}

impl Protocol19ActivationAuthorizationV1 {
    pub fn for_prepared_world(
        prepared: &Protocol19PreparedActivationSummary,
        policy: &Protocol19ActivationTrustPolicy,
        authorization_nonce: impl Into<String>,
        authorized_activation_unix_ms: u64,
        not_before_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<Self, Protocol19ActivationError> {
        let authorization = Self {
            schema_version: AUTHORIZATION_SCHEMA_VERSION,
            authorization_kind: AUTHORIZATION_KIND.into(),
            signature_scheme: SIGNATURE_SCHEME.into(),
            compatibility: prepared.compatibility.clone(),
            universe_id: prepared.universe_id.clone(),
            world_seed: prepared.world_seed.to_string(),
            prepared_install_head_hash: prepared.prepared_install_head_hash.clone(),
            migration_receipt_hash: prepared.migration_receipt_hash.clone(),
            migration_anchor_hash: prepared.migration_anchor_hash.clone(),
            target_manifest_hash: prepared.target_manifest_hash.clone(),
            target_directory_document_hash: prepared.target_directory_document_hash.clone(),
            target_assignment_root: prepared.target_assignment_root.clone(),
            target_placement_root: prepared.target_placement_root.clone(),
            cell_set_root: prepared.cell_set_root.clone(),
            global_conservation_root: prepared.global_conservation_root.clone(),
            normalized_gameplay_root: prepared.normalized_gameplay_root.clone(),
            identity_map_root: prepared.identity_map_root.clone(),
            production_origin_root: prepared.production_origin_root.clone(),
            cell_count: prepared.cell_count,
            signer_policy_hash: policy.policy_hash().to_owned(),
            signer_policy_generation: policy.policy_generation(),
            activation_generation: 1,
            authorization_nonce: authorization_nonce.into(),
            authorized_activation_unix_ms,
            not_before_unix_ms,
            expires_at_unix_ms,
            previous_activation_head_hash: String::new(),
        };
        validate_authorization_shape(&authorization, policy)?;
        Ok(authorization)
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, Protocol19ActivationError> {
        let canonical = serde_json::to_vec(self)
            .map_err(|source| Protocol19ActivationError::Json(source.to_string()))?;
        let mut message = Vec::with_capacity(AUTHORIZATION_SIGNING_DOMAIN.len() + canonical.len());
        message.extend_from_slice(AUTHORIZATION_SIGNING_DOMAIN);
        message.extend_from_slice(&canonical);
        Ok(message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Protocol19ActivationSignatureV1 {
    pub signer_id: String,
    pub signature: String,
}

impl Protocol19ActivationSignatureV1 {
    pub fn new(signer_id: impl Into<String>, signature: [u8; 64]) -> Self {
        Self {
            signer_id: signer_id.into(),
            signature: encode_hex(&signature),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedProtocol19ActivationAuthorizationV1 {
    pub authorization: Protocol19ActivationAuthorizationV1,
    pub signatures: Vec<Protocol19ActivationSignatureV1>,
}

impl SignedProtocol19ActivationAuthorizationV1 {
    pub fn new(
        authorization: Protocol19ActivationAuthorizationV1,
        mut signatures: Vec<Protocol19ActivationSignatureV1>,
    ) -> Result<Self, Protocol19ActivationError> {
        signatures.sort_by(|left, right| left.signer_id.cmp(&right.signer_id));
        let signed = Self {
            authorization,
            signatures,
        };
        validate_signature_order(&signed.signatures)?;
        Ok(signed)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, Protocol19ActivationError> {
        validate_signature_order(&self.signatures)?;
        let bytes = serde_json::to_vec(self)
            .map_err(|source| Protocol19ActivationError::Json(source.to_string()))?;
        if bytes.is_empty() || bytes.len() > MAX_AUTHORIZATION_BYTES {
            return Err(Protocol19ActivationError::TooLarge(PathBuf::from(
                "activation-authorization-v1.json",
            )));
        }
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Protocol19PreparedActivationSummary {
    pub compatibility: Protocol19CompatibilityTuple,
    pub universe_id: String,
    pub world_seed: u64,
    pub target_manifest_hash: String,
    pub migration_anchor_hash: String,
    pub migration_receipt_hash: String,
    pub target_directory_document_hash: String,
    pub target_assignment_root: String,
    pub target_placement_root: String,
    pub identity_map_root: String,
    pub production_origin_root: String,
    pub global_conservation_root: String,
    pub normalized_gameplay_root: String,
    pub cell_count: u64,
    pub cell_set_root: String,
    pub prepared_install_head_hash: String,
}

impl From<Protocol19PreparedInstallSummary> for Protocol19PreparedActivationSummary {
    fn from(value: Protocol19PreparedInstallSummary) -> Self {
        Self {
            compatibility: value.compatibility,
            universe_id: value.universe_id,
            world_seed: value.world_seed,
            target_manifest_hash: value.target_manifest_hash,
            migration_anchor_hash: value.migration_anchor_hash,
            migration_receipt_hash: value.migration_receipt_hash,
            target_directory_document_hash: value.target_directory_document_hash,
            target_assignment_root: value.target_assignment_root,
            target_placement_root: value.target_placement_root,
            identity_map_root: value.identity_map_root,
            production_origin_root: value.production_origin_root,
            global_conservation_root: value.global_conservation_root,
            normalized_gameplay_root: value.normalized_gameplay_root,
            cell_count: value.cell_count,
            cell_set_root: value.cell_set_root,
            prepared_install_head_hash: value.prepared_install_head_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Protocol19ActivatedWorldSummary {
    pub compatibility: Protocol19CompatibilityTuple,
    pub universe_id: String,
    pub world_seed: u64,
    pub prepared_install_head_hash: String,
    pub migration_receipt_hash: String,
    pub active_head_hash: String,
    pub authorization_hash: String,
    pub signer_policy_hash: String,
    pub activation_generation: u64,
    pub authorized_activation_unix_ms: u64,
    pub cell_count: u64,
}

#[derive(Debug)]
pub struct ActivatedProtocol19World {
    summary: Protocol19ActivatedWorldSummary,
    #[allow(dead_code)] // consumed by the next lifecycle-v2 integration slice
    prepared: OpenedProtocol19PreparedInstall,
    _activation_lock: File,
}

#[allow(dead_code)] // authority mutation stays crate-private until lifecycle-v2 coordinates it
impl ActivatedProtocol19World {
    pub fn summary(&self) -> &Protocol19ActivatedWorldSummary {
        &self.summary
    }

    /// Returns the exact directory-v3 assignment held under this activated
    /// universe capability. The active signed head remains the immutable root
    /// of the directory history.
    #[cfg(test)]
    pub(crate) fn cell_assignment(
        &self,
        cell_key: &verse_protocol::CellKeyV1,
    ) -> Result<&CellAssignmentRecord, Protocol19ActivationError> {
        self.prepared.cell_assignment(cell_key).map_err(Into::into)
    }

    /// Claims one sleeping cell. The directory derives the successor
    /// generation and fencing token; the caller supplies neither value.
    #[cfg(test)]
    pub(crate) fn claim_cell_authority(
        &mut self,
        cell_key: &verse_protocol::CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
    ) -> Result<CellAssignmentRecord, Protocol19ActivationError> {
        self.prepared
            .claim_cell_authority(cell_key, expected_generation, holder_id)
            .map_err(Into::into)
    }

    /// Replaces the holder of an assigned cell after this process has acquired
    /// the activated universe's exclusive writer set.
    #[cfg(test)]
    pub(crate) fn recover_cell_authority(
        &mut self,
        cell_key: &verse_protocol::CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
    ) -> Result<CellAssignmentRecord, Protocol19ActivationError> {
        self.prepared
            .recover_cell_authority(cell_key, expected_generation, holder_id)
            .map_err(Into::into)
    }

    /// Releases an assigned cell only when the exact generation and holder
    /// still match and no nonterminal transfer pins the cell.
    #[cfg(test)]
    pub(crate) fn release_cell_authority(
        &mut self,
        cell_key: &verse_protocol::CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
    ) -> Result<CellAssignmentRecord, Protocol19ActivationError> {
        self.prepared
            .release_cell_authority(cell_key, expected_generation, holder_id)
            .map_err(Into::into)
    }

    /// Runs one bounded production-only lifecycle dispatch. Gameplay
    /// admission remains closed until ordinary event-17 integration.
    pub(crate) fn dispatch_background_production_with_clock(
        &mut self,
        cell_key: &verse_protocol::CellKeyV1,
        holder_id: &str,
        clock: &dyn TrustedClock,
    ) -> Result<
        crate::protocol19_install::Protocol19BackgroundDispatchOutcome,
        Protocol19ActivationError,
    > {
        self.prepared
            .dispatch_background_production(cell_key, holder_id, clock)
            .map_err(Into::into)
    }

    #[cfg(test)]
    fn set_lifecycle_failpoint_for_test(
        &mut self,
        cell_key: &verse_protocol::CellKeyV1,
        failpoint: crate::grid_handoff_v2::lifecycle_v2::LifecycleAppendFailpointV2,
    ) -> Result<(), Protocol19ActivationError> {
        self.prepared
            .set_lifecycle_failpoint_for_test(cell_key, failpoint)
            .map_err(Into::into)
    }

    #[cfg(test)]
    fn set_lifecycle_coordinator_failpoint_for_test(
        &mut self,
        failpoint: crate::protocol19_install::Protocol19LifecycleCoordinatorFailpoint,
    ) {
        self.prepared
            .set_lifecycle_coordinator_failpoint_for_test(failpoint);
    }

    #[cfg(test)]
    fn reseal_pending_lifecycle_outside_state_machine_for_test(
        &mut self,
    ) -> Result<(), Protocol19ActivationError> {
        self.prepared
            .reseal_pending_lifecycle_outside_state_machine_for_test()
            .map_err(Into::into)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveProtocol19HeadV1 {
    schema_version: u32,
    compatibility: Protocol19CompatibilityTuple,
    mode: String,
    universe_id: String,
    world_seed: String,
    prepared_install_head_hash: String,
    migration_receipt_hash: String,
    migration_anchor_hash: String,
    target_manifest_hash: String,
    target_directory_document_hash: String,
    cell_set_root: String,
    global_conservation_root: String,
    normalized_gameplay_root: String,
    identity_map_root: String,
    production_origin_root: String,
    cell_count: u64,
    authorization_hash: String,
    signer_policy_hash: String,
    signer_policy_generation: u64,
    activation_generation: u64,
    authorized_activation_unix_ms: u64,
    previous_activation_head_hash: String,
    head_hash: String,
}

impl ActiveProtocol19HeadV1 {
    fn new(
        prepared: &Protocol19PreparedInstallSummary,
        authorization_hash: String,
        authorization: &Protocol19ActivationAuthorizationV1,
    ) -> Result<Self, Protocol19ActivationError> {
        let mut head = Self {
            schema_version: ACTIVE_HEAD_SCHEMA_VERSION,
            compatibility: prepared.compatibility.clone(),
            mode: ACTIVE_MODE.into(),
            universe_id: prepared.universe_id.clone(),
            world_seed: prepared.world_seed.to_string(),
            prepared_install_head_hash: prepared.prepared_install_head_hash.clone(),
            migration_receipt_hash: prepared.migration_receipt_hash.clone(),
            migration_anchor_hash: prepared.migration_anchor_hash.clone(),
            target_manifest_hash: prepared.target_manifest_hash.clone(),
            target_directory_document_hash: prepared.target_directory_document_hash.clone(),
            cell_set_root: prepared.cell_set_root.clone(),
            global_conservation_root: prepared.global_conservation_root.clone(),
            normalized_gameplay_root: prepared.normalized_gameplay_root.clone(),
            identity_map_root: prepared.identity_map_root.clone(),
            production_origin_root: prepared.production_origin_root.clone(),
            cell_count: prepared.cell_count,
            authorization_hash,
            signer_policy_hash: authorization.signer_policy_hash.clone(),
            signer_policy_generation: authorization.signer_policy_generation,
            activation_generation: authorization.activation_generation,
            authorized_activation_unix_ms: authorization.authorized_activation_unix_ms,
            previous_activation_head_hash: authorization.previous_activation_head_hash.clone(),
            head_hash: String::new(),
        };
        head.head_hash = head.calculate_hash()?;
        head.validate()?;
        Ok(head)
    }

    fn calculate_hash(&self) -> Result<String, Protocol19ActivationError> {
        let mut material = self.clone();
        material.head_hash.clear();
        hash_json(ACTIVE_HEAD_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), Protocol19ActivationError> {
        let seed = self.world_seed.parse::<u64>().map_err(|_| {
            Protocol19ActivationError::Invalid("active-head seed is not canonical".into())
        })?;
        if self.schema_version != ACTIVE_HEAD_SCHEMA_VERSION
            || self.compatibility != Protocol19CompatibilityTuple::canonical()
            || self.mode != ACTIVE_MODE
            || seed.to_string() != self.world_seed
            || self.activation_generation != 1
            || !self.previous_activation_head_hash.is_empty()
            || self.authorized_activation_unix_ms == 0
            || self.cell_count == 0
            || !valid_hashes([
                &self.prepared_install_head_hash,
                &self.migration_receipt_hash,
                &self.migration_anchor_hash,
                &self.target_manifest_hash,
                &self.target_directory_document_hash,
                &self.cell_set_root,
                &self.global_conservation_root,
                &self.normalized_gameplay_root,
                &self.identity_map_root,
                &self.production_origin_root,
                &self.authorization_hash,
                &self.signer_policy_hash,
                &self.head_hash,
            ])
            || self.head_hash != self.calculate_hash()?
        {
            return Err(Protocol19ActivationError::Invalid(
                "active protocol-19 head is not canonical".into(),
            ));
        }
        Ok(())
    }

    fn encode_canonical(&self) -> Result<Vec<u8>, Protocol19ActivationError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|source| Protocol19ActivationError::Json(source.to_string()))?;
        if bytes.is_empty() || bytes.len() > MAX_ACTIVE_HEAD_BYTES {
            return Err(Protocol19ActivationError::TooLarge(PathBuf::from(
                ACTIVE_PROTOCOL_HEAD_FILE,
            )));
        }
        Ok(bytes)
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, Protocol19ActivationError> {
        if bytes.is_empty() || bytes.len() > MAX_ACTIVE_HEAD_BYTES {
            return Err(Protocol19ActivationError::TooLarge(PathBuf::from(
                ACTIVE_PROTOCOL_HEAD_FILE,
            )));
        }
        let head = serde_json::from_slice::<Self>(bytes)
            .map_err(|source| Protocol19ActivationError::Json(source.to_string()))?;
        head.validate()?;
        if head.encode_canonical()? != bytes {
            return Err(Protocol19ActivationError::Invalid(
                "active-head bytes are not canonical".into(),
            ));
        }
        Ok(head)
    }
}

pub fn protocol19_activation_signer_id(public_key: &[u8; 32]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(SIGNER_ID_DOMAIN);
    hasher.update(public_key);
    hasher.finalize().to_hex().to_string()
}

pub fn prepare_protocol19_for_activation(
    universe_root: impl AsRef<Path>,
    world_seed: u64,
) -> Result<Protocol19PreparedActivationSummary, Protocol19ActivationError> {
    ensure_no_active_selector(universe_root.as_ref())?;
    let source = ValidatedFrozenProtocol18Source::acquire_existing(&universe_root, world_seed)
        .map_err(|source| Protocol19ActivationError::Invalid(source.to_string()))?;
    let manifest = crate::manifest_v5::build_validated_manifest_v5(world_seed)
        .map_err(|source| Protocol19ActivationError::Invalid(source.to_string()))?;
    let transform = ValidatedProtocol19MigrationTransform::derive(&source, &manifest)
        .map_err(|source| Protocol19ActivationError::Invalid(source.to_string()))?;
    let prepared = crate::protocol19_install::prepare_or_recover(&transform, &manifest)?;
    prepared.summary().map(Into::into).map_err(Into::into)
}

pub fn activate_protocol19_world(
    universe_root: impl AsRef<Path>,
    world_seed: u64,
    policy: &Protocol19ActivationTrustPolicy,
    signed_authorization_bytes: &[u8],
) -> Result<Protocol19ActivatedWorldSummary, Protocol19ActivationError> {
    activate_protocol19_world_with_failpoint(
        universe_root.as_ref(),
        world_seed,
        policy,
        signed_authorization_bytes,
        &SystemTrustedClock,
        None,
    )
}

#[cfg(test)]
pub(crate) fn activate_protocol19_world_with_clock(
    universe_root: impl AsRef<Path>,
    world_seed: u64,
    policy: &Protocol19ActivationTrustPolicy,
    signed_authorization_bytes: &[u8],
    clock: &dyn TrustedClock,
) -> Result<Protocol19ActivatedWorldSummary, Protocol19ActivationError> {
    activate_protocol19_world_with_failpoint(
        universe_root.as_ref(),
        world_seed,
        policy,
        signed_authorization_bytes,
        clock,
        None,
    )
}

fn activate_protocol19_world_with_failpoint(
    universe_root: &Path,
    world_seed: u64,
    policy: &Protocol19ActivationTrustPolicy,
    signed_authorization_bytes: &[u8],
    clock: &dyn TrustedClock,
    failpoint: Option<Protocol19ActivationFailpoint>,
) -> Result<Protocol19ActivatedWorldSummary, Protocol19ActivationError> {
    if protocol19_is_activated(universe_root)? {
        let supplied = decode_signed_authorization(signed_authorization_bytes)?;
        let supplied_hash = hash_json(AUTHORIZATION_HASH_DOMAIN, &supplied)?;
        let committed = open_activated_protocol19_world(universe_root, world_seed, policy)
            .map(|world| world.summary().clone())?;
        if committed.authorization_hash != supplied_hash {
            return Err(Protocol19ActivationError::Invalid(
                "activation retry supplied a different authorization".into(),
            ));
        }
        return Ok(committed);
    }
    let source = ValidatedFrozenProtocol18Source::acquire_existing(universe_root, world_seed)
        .map_err(|source| Protocol19ActivationError::Invalid(source.to_string()))?;
    let manifest = crate::manifest_v5::build_validated_manifest_v5(world_seed)
        .map_err(|source| Protocol19ActivationError::Invalid(source.to_string()))?;
    let transform = ValidatedProtocol19MigrationTransform::derive(&source, &manifest)
        .map_err(|source| Protocol19ActivationError::Invalid(source.to_string()))?;
    let prepared = crate::protocol19_install::prepare_or_recover(&transform, &manifest)?;
    commit_activation(
        &prepared,
        policy,
        signed_authorization_bytes,
        clock,
        failpoint,
    )
}

pub fn open_activated_protocol19_world(
    universe_root: impl AsRef<Path>,
    expected_world_seed: u64,
    policy: &Protocol19ActivationTrustPolicy,
) -> Result<ActivatedProtocol19World, Protocol19ActivationError> {
    open_activated_protocol19_world_inner(universe_root.as_ref(), expected_world_seed, policy, None)
}

#[cfg(test)]
fn open_activated_protocol19_world_with_lifecycle_initialization_failpoint(
    universe_root: impl AsRef<Path>,
    expected_world_seed: u64,
    policy: &Protocol19ActivationTrustPolicy,
    failpoint: crate::grid_handoff_v2::lifecycle_v2::LifecycleInitializationFailpointV2,
) -> Result<ActivatedProtocol19World, Protocol19ActivationError> {
    open_activated_protocol19_world_inner(
        universe_root.as_ref(),
        expected_world_seed,
        policy,
        Some(failpoint),
    )
}

fn open_activated_protocol19_world_inner(
    universe_root: &Path,
    expected_world_seed: u64,
    policy: &Protocol19ActivationTrustPolicy,
    initialization_failpoint: Option<
        crate::grid_handoff_v2::lifecycle_v2::LifecycleInitializationFailpointV2,
    >,
) -> Result<ActivatedProtocol19World, Protocol19ActivationError> {
    if !protocol19_is_activated(universe_root)? {
        return Err(Protocol19ActivationError::Invalid(
            "universe has no active protocol-19 global head".into(),
        ));
    }
    let selector_path = universe_root.join(ACTIVE_PROTOCOL_HEAD_FILE);
    let first_bytes = read_bounded(&selector_path, MAX_ACTIVE_HEAD_BYTES)?;
    let head = ActiveProtocol19HeadV1::decode_canonical(&first_bytes)?;
    if head.world_seed != expected_world_seed.to_string()
        || head.universe_id != policy.universe_id()
    {
        return Err(Protocol19ActivationError::Invalid(
            "active head differs from the configured universe or seed".into(),
        ));
    }
    let validated_prepared = crate::protocol19_install::validate_from_active_head(
        universe_root,
        &head.prepared_install_head_hash,
    )?;
    let activation_root = universe_root.join(ACTIVATION_DIRECTORY);
    let activation_lock = acquire_activation_lock(&activation_root, false)?;
    let second_bytes = read_bounded(&selector_path, MAX_ACTIVE_HEAD_BYTES)?;
    if second_bytes != first_bytes {
        return Err(Protocol19ActivationError::Invalid(
            "active protocol selector changed during verified boot".into(),
        ));
    }
    let signed_bytes = read_bounded(
        &activation_root.join(authorization_file_name(&head.authorization_hash)),
        MAX_AUTHORIZATION_BYTES,
    )?;
    let signed = decode_signed_authorization(&signed_bytes)?;
    if hash_json(AUTHORIZATION_HASH_DOMAIN, &signed)? != head.authorization_hash {
        return Err(Protocol19ActivationError::Invalid(
            "signed authorization differs from the active head".into(),
        ));
    }
    verify_authorization(
        &signed,
        validated_prepared.summary(),
        policy,
        head.authorized_activation_unix_ms,
    )?;
    if validated_prepared.receipt().trusted_cutoff_unix_ms > head.authorized_activation_unix_ms {
        return Err(Protocol19ActivationError::Invalid(
            "active head predates the trusted migration cut-off".into(),
        ));
    }
    validate_head_bindings(&head, validated_prepared.summary(), &signed.authorization)?;
    let history_bytes = read_bounded(
        &activation_root.join(active_head_file_name(&head.head_hash)),
        MAX_ACTIVE_HEAD_BYTES,
    )?;
    if history_bytes != first_bytes {
        return Err(Protocol19ActivationError::Invalid(
            "active-head history differs from the global selector".into(),
        ));
    }
    validate_activation_file_set(&activation_root, &head)?;
    let prepared = validated_prepared
        .open_with_lifecycle_initialization_failpoint(&head.head_hash, initialization_failpoint)?;
    let summary = active_summary(&head, prepared.summary())?;
    Ok(ActivatedProtocol19World {
        summary,
        prepared,
        _activation_lock: activation_lock,
    })
}

pub fn protocol19_is_activated(
    universe_root: impl AsRef<Path>,
) -> Result<bool, Protocol19ActivationError> {
    match fs::symlink_metadata(universe_root.as_ref().join(ACTIVE_PROTOCOL_HEAD_FILE)) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(Protocol19ActivationError::Invalid(
            "active protocol selector is not a real file".into(),
        )),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error(
            universe_root.as_ref().join(ACTIVE_PROTOCOL_HEAD_FILE),
            source,
        )),
    }
}

pub(crate) fn ensure_legacy_protocol_not_activated(
    possible_universe_or_cell_root: &Path,
) -> Result<(), String> {
    let mut candidates = vec![possible_universe_or_cell_root.to_path_buf()];
    if possible_universe_or_cell_root
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "cells")
        && let Some(universe_root) = possible_universe_or_cell_root
            .parent()
            .and_then(Path::parent)
    {
        candidates.push(universe_root.to_path_buf());
    }
    for root in candidates {
        let selector = root.join(ACTIVE_PROTOCOL_HEAD_FILE);
        match fs::symlink_metadata(&selector) {
            Ok(_) => {
                return Err(format!(
                    "protocol 19 is active at {}; protocol-18 authority is fenced",
                    root.display()
                ));
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(format!(
                    "cannot determine active protocol at {}: {source}",
                    selector.display()
                ));
            }
        }
    }
    Ok(())
}

fn commit_activation(
    prepared: &PreparedProtocol19World<'_, '_>,
    policy: &Protocol19ActivationTrustPolicy,
    signed_authorization_bytes: &[u8],
    clock: &dyn TrustedClock,
    mut failpoint: Option<Protocol19ActivationFailpoint>,
) -> Result<Protocol19ActivatedWorldSummary, Protocol19ActivationError> {
    let universe_root = prepared.universe_root();
    let activation_root = universe_root.join(ACTIVATION_DIRECTORY);
    create_real_directory(universe_root, &activation_root)?;
    let _activation_lock = acquire_activation_lock(&activation_root, true)?;
    inject(
        &mut failpoint,
        Protocol19ActivationFailpoint::NamespaceSynced,
    )?;
    if protocol19_is_activated(universe_root)? {
        return Err(Protocol19ActivationError::Invalid(
            "active protocol selector appeared while the frozen source was locked".into(),
        ));
    }
    reset_uncommitted_selector_temps(universe_root)?;
    reset_uncommitted_activation(&activation_root)?;
    let signed = decode_signed_authorization(signed_authorization_bytes)?;
    let prepared_summary = prepared.summary()?;
    let now_unix_ms = clock
        .now_unix_ms()
        .map_err(|source| Protocol19ActivationError::Invalid(source.to_string()))?;
    verify_authorization(&signed, &prepared_summary, policy, now_unix_ms)?;
    let receipt_bytes = read_bounded(
        &universe_root
            .join("protocol-19-prepared-install-v1")
            .join("migration-receipt-v1.json"),
        16 * 1_024 * 1_024,
    )?;
    let receipt = crate::protocol19_migration::recover_canonical_migration_receipt(&receipt_bytes)
        .map_err(|source| Protocol19ActivationError::Invalid(source.to_string()))?;
    if receipt.trusted_cutoff_unix_ms > now_unix_ms
        || receipt.trusted_cutoff_unix_ms > signed.authorization.authorized_activation_unix_ms
        || receipt.migration_receipt_hash != prepared_summary.migration_receipt_hash
        || receipt.migration_anchor_hash != prepared_summary.migration_anchor_hash
        || receipt.target_manifest_hash != prepared_summary.target_manifest_hash
        || receipt.universe_id != prepared_summary.universe_id
        || receipt.world_seed != prepared_summary.world_seed
    {
        return Err(Protocol19ActivationError::Invalid(
            "activation receipt differs from its prepared world or trusted cut-off".into(),
        ));
    }
    let authorization_hash = hash_json(AUTHORIZATION_HASH_DOMAIN, &signed)?;
    let head = ActiveProtocol19HeadV1::new(
        &prepared_summary,
        authorization_hash.clone(),
        &signed.authorization,
    )?;
    let canonical_signed = signed.canonical_bytes()?;
    let head_bytes = head.encode_canonical()?;
    atomic_write(
        &activation_root.join(authorization_file_name(&authorization_hash)),
        &canonical_signed,
    )?;
    inject(
        &mut failpoint,
        Protocol19ActivationFailpoint::AuthorizationSynced,
    )?;
    atomic_write(
        &activation_root.join(active_head_file_name(&head.head_hash)),
        &head_bytes,
    )?;
    inject(
        &mut failpoint,
        Protocol19ActivationFailpoint::HistoryHeadSynced,
    )?;
    persist_selector(universe_root, &head_bytes, &mut failpoint)?;
    validate_activation_file_set(&activation_root, &head)?;
    active_summary(&head, &prepared_summary)
}

fn verify_authorization(
    signed: &SignedProtocol19ActivationAuthorizationV1,
    prepared: &Protocol19PreparedInstallSummary,
    policy: &Protocol19ActivationTrustPolicy,
    trusted_time_unix_ms: u64,
) -> Result<(), Protocol19ActivationError> {
    validate_authorization_shape(&signed.authorization, policy)?;
    validate_authorization_bindings(&signed.authorization, prepared)?;
    if trusted_time_unix_ms < signed.authorization.not_before_unix_ms
        || trusted_time_unix_ms >= signed.authorization.expires_at_unix_ms
    {
        return Err(Protocol19ActivationError::Invalid(
            "activation authorization is not valid at the trusted commit time".into(),
        ));
    }
    validate_signature_order(&signed.signatures)?;
    if signed.signatures.len() != usize::from(policy.document.threshold) {
        return Err(Protocol19ActivationError::Invalid(
            "activation authorization must contain exactly the policy threshold signatures".into(),
        ));
    }
    let message = signed.authorization.signing_bytes()?;
    for detached in &signed.signatures {
        let (_, key) = policy
            .verifying_keys
            .iter()
            .find(|(signer_id, _)| signer_id == &detached.signer_id)
            .ok_or_else(|| {
                Protocol19ActivationError::Invalid(
                    "activation authorization contains an unknown signer".into(),
                )
            })?;
        let signature_bytes = decode_hex_array::<64>(&detached.signature, "activation signature")?;
        let signature = Signature::from_bytes(&signature_bytes);
        key.verify_strict(&message, &signature).map_err(|_| {
            Protocol19ActivationError::Invalid("activation signature is invalid".into())
        })?;
    }
    Ok(())
}

fn validate_policy_document(
    policy: &ActivationTrustPolicyDocumentV1,
    expected_policy_hash: &str,
) -> Result<(), Protocol19ActivationError> {
    let ordered = policy
        .signers
        .windows(2)
        .all(|pair| pair[0].signer_id < pair[1].signer_id);
    let signers_valid = policy.signers.iter().all(|signer| {
        decode_hex_array::<32>(&signer.public_key, "signer public key")
            .is_ok_and(|key| signer.signer_id == protocol19_activation_signer_id(&key))
    });
    if policy.schema_version != POLICY_SCHEMA_VERSION
        || policy.signature_scheme != SIGNATURE_SCHEME
        || policy.universe_id.is_empty()
        || policy.policy_generation == 0
        || policy.threshold != REQUIRED_THRESHOLD
        || policy.signers.len() != REQUIRED_SIGNER_COUNT
        || !ordered
        || !signers_valid
        || policy.maximum_authorization_validity_millis == 0
        || policy.maximum_authorization_validity_millis > MAX_AUTHORIZATION_VALIDITY_MILLIS
        || !valid_hash(&policy.policy_hash)
        || policy.policy_hash != calculate_policy_hash(policy)?
        || policy.policy_hash != expected_policy_hash
    {
        return Err(Protocol19ActivationError::Invalid(
            "activation trust policy is not the externally anchored canonical 2-of-3 policy".into(),
        ));
    }
    Ok(())
}

fn validate_authorization_shape(
    authorization: &Protocol19ActivationAuthorizationV1,
    policy: &Protocol19ActivationTrustPolicy,
) -> Result<(), Protocol19ActivationError> {
    let validity = authorization
        .expires_at_unix_ms
        .checked_sub(authorization.not_before_unix_ms);
    if authorization.schema_version != AUTHORIZATION_SCHEMA_VERSION
        || authorization.authorization_kind != AUTHORIZATION_KIND
        || authorization.signature_scheme != SIGNATURE_SCHEME
        || authorization.compatibility != Protocol19CompatibilityTuple::canonical()
        || authorization.universe_id != policy.document.universe_id
        || authorization
            .world_seed
            .parse::<u64>()
            .map_or(true, |seed| seed.to_string() != authorization.world_seed)
        || authorization.signer_policy_hash != policy.document.policy_hash
        || authorization.signer_policy_generation != policy.document.policy_generation
        || authorization.activation_generation != 1
        || !authorization.previous_activation_head_hash.is_empty()
        || !valid_nonce(&authorization.authorization_nonce)
        || authorization.authorized_activation_unix_ms < authorization.not_before_unix_ms
        || authorization.authorized_activation_unix_ms >= authorization.expires_at_unix_ms
        || validity.is_none_or(|duration| {
            duration == 0 || duration > policy.document.maximum_authorization_validity_millis
        })
        || authorization.cell_count == 0
        || !valid_hashes([
            &authorization.prepared_install_head_hash,
            &authorization.migration_receipt_hash,
            &authorization.migration_anchor_hash,
            &authorization.target_manifest_hash,
            &authorization.target_directory_document_hash,
            &authorization.target_assignment_root,
            &authorization.target_placement_root,
            &authorization.cell_set_root,
            &authorization.global_conservation_root,
            &authorization.normalized_gameplay_root,
            &authorization.identity_map_root,
            &authorization.production_origin_root,
            &authorization.signer_policy_hash,
        ])
    {
        return Err(Protocol19ActivationError::Invalid(
            "activation authorization shape or validity is invalid".into(),
        ));
    }
    Ok(())
}

fn validate_authorization_bindings(
    authorization: &Protocol19ActivationAuthorizationV1,
    prepared: &Protocol19PreparedInstallSummary,
) -> Result<(), Protocol19ActivationError> {
    if authorization.compatibility != prepared.compatibility
        || authorization.universe_id != prepared.universe_id
        || authorization.world_seed != prepared.world_seed.to_string()
        || authorization.prepared_install_head_hash != prepared.prepared_install_head_hash
        || authorization.migration_receipt_hash != prepared.migration_receipt_hash
        || authorization.migration_anchor_hash != prepared.migration_anchor_hash
        || authorization.target_manifest_hash != prepared.target_manifest_hash
        || authorization.target_directory_document_hash != prepared.target_directory_document_hash
        || authorization.target_assignment_root != prepared.target_assignment_root
        || authorization.target_placement_root != prepared.target_placement_root
        || authorization.cell_set_root != prepared.cell_set_root
        || authorization.global_conservation_root != prepared.global_conservation_root
        || authorization.normalized_gameplay_root != prepared.normalized_gameplay_root
        || authorization.identity_map_root != prepared.identity_map_root
        || authorization.production_origin_root != prepared.production_origin_root
        || authorization.cell_count != prepared.cell_count
    {
        return Err(Protocol19ActivationError::Invalid(
            "signed authorization differs from the exact prepared universe".into(),
        ));
    }
    Ok(())
}

fn validate_head_bindings(
    head: &ActiveProtocol19HeadV1,
    prepared: &Protocol19PreparedInstallSummary,
    authorization: &Protocol19ActivationAuthorizationV1,
) -> Result<(), Protocol19ActivationError> {
    validate_authorization_bindings(authorization, prepared)?;
    if head.compatibility != prepared.compatibility
        || head.universe_id != prepared.universe_id
        || head.world_seed != prepared.world_seed.to_string()
        || head.prepared_install_head_hash != prepared.prepared_install_head_hash
        || head.migration_receipt_hash != prepared.migration_receipt_hash
        || head.migration_anchor_hash != prepared.migration_anchor_hash
        || head.target_manifest_hash != prepared.target_manifest_hash
        || head.target_directory_document_hash != prepared.target_directory_document_hash
        || head.cell_set_root != prepared.cell_set_root
        || head.global_conservation_root != prepared.global_conservation_root
        || head.normalized_gameplay_root != prepared.normalized_gameplay_root
        || head.identity_map_root != prepared.identity_map_root
        || head.production_origin_root != prepared.production_origin_root
        || head.cell_count != prepared.cell_count
        || head.signer_policy_hash != authorization.signer_policy_hash
        || head.signer_policy_generation != authorization.signer_policy_generation
        || head.activation_generation != authorization.activation_generation
        || head.authorized_activation_unix_ms != authorization.authorized_activation_unix_ms
        || head.previous_activation_head_hash != authorization.previous_activation_head_hash
    {
        return Err(Protocol19ActivationError::Invalid(
            "active head differs from its signed authorization or prepared world".into(),
        ));
    }
    Ok(())
}

fn active_summary(
    head: &ActiveProtocol19HeadV1,
    prepared: &Protocol19PreparedInstallSummary,
) -> Result<Protocol19ActivatedWorldSummary, Protocol19ActivationError> {
    validate_head_bindings_shape(head, prepared)?;
    Ok(Protocol19ActivatedWorldSummary {
        compatibility: head.compatibility.clone(),
        universe_id: head.universe_id.clone(),
        world_seed: prepared.world_seed,
        prepared_install_head_hash: head.prepared_install_head_hash.clone(),
        migration_receipt_hash: head.migration_receipt_hash.clone(),
        active_head_hash: head.head_hash.clone(),
        authorization_hash: head.authorization_hash.clone(),
        signer_policy_hash: head.signer_policy_hash.clone(),
        activation_generation: head.activation_generation,
        authorized_activation_unix_ms: head.authorized_activation_unix_ms,
        cell_count: head.cell_count,
    })
}

fn validate_head_bindings_shape(
    head: &ActiveProtocol19HeadV1,
    prepared: &Protocol19PreparedInstallSummary,
) -> Result<(), Protocol19ActivationError> {
    if head.prepared_install_head_hash != prepared.prepared_install_head_hash
        || head.migration_receipt_hash != prepared.migration_receipt_hash
        || head.cell_count != prepared.cell_count
    {
        return Err(Protocol19ActivationError::Invalid(
            "active summary differs from prepared authority".into(),
        ));
    }
    Ok(())
}

fn decode_signed_authorization(
    bytes: &[u8],
) -> Result<SignedProtocol19ActivationAuthorizationV1, Protocol19ActivationError> {
    if bytes.is_empty() || bytes.len() > MAX_AUTHORIZATION_BYTES {
        return Err(Protocol19ActivationError::TooLarge(PathBuf::from(
            "activation-authorization-v1.json",
        )));
    }
    let signed = serde_json::from_slice::<SignedProtocol19ActivationAuthorizationV1>(bytes)
        .map_err(|source| Protocol19ActivationError::Json(source.to_string()))?;
    if signed.canonical_bytes()? != bytes {
        return Err(Protocol19ActivationError::Invalid(
            "signed activation authorization bytes are not canonical".into(),
        ));
    }
    Ok(signed)
}

fn validate_signature_order(
    signatures: &[Protocol19ActivationSignatureV1],
) -> Result<(), Protocol19ActivationError> {
    if signatures.is_empty()
        || signatures.len() > REQUIRED_SIGNER_COUNT
        || !signatures
            .windows(2)
            .all(|pair| pair[0].signer_id < pair[1].signer_id)
        || signatures.iter().any(|signature| {
            !valid_hash(&signature.signer_id)
                || decode_hex_array::<64>(&signature.signature, "activation signature").is_err()
        })
    {
        return Err(Protocol19ActivationError::Invalid(
            "activation signatures are empty, duplicated, unordered, or malformed".into(),
        ));
    }
    Ok(())
}

fn calculate_policy_hash(
    policy: &ActivationTrustPolicyDocumentV1,
) -> Result<String, Protocol19ActivationError> {
    let mut material = policy.clone();
    material.policy_hash.clear();
    hash_json(POLICY_HASH_DOMAIN, &material)
}

fn hash_json<T: Serialize>(domain: &[u8], value: &T) -> Result<String, Protocol19ActivationError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|source| Protocol19ActivationError::Json(source.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_hashes<'a>(values: impl IntoIterator<Item = &'a String>) -> bool {
    values.into_iter().all(|value| valid_hash(value))
}

fn valid_nonce(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn decode_hex_array<const N: usize>(
    value: &str,
    label: &str,
) -> Result<[u8; N], Protocol19ActivationError> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(Protocol19ActivationError::Invalid(format!(
            "{label} is not fixed-width lowercase hexadecimal"
        )));
    }
    let mut output = [0_u8; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(output)
}

fn hex_nibble(byte: u8) -> Result<u8, Protocol19ActivationError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(Protocol19ActivationError::Invalid(
            "hexadecimal text is invalid".into(),
        )),
    }
}

fn ensure_no_active_selector(root: &Path) -> Result<(), Protocol19ActivationError> {
    if protocol19_is_activated(root)? {
        return Err(Protocol19ActivationError::Invalid(
            "universe is already activated under protocol 19".into(),
        ));
    }
    Ok(())
}

fn authorization_file_name(hash: &str) -> String {
    format!("authorization-{hash}.json")
}

fn active_head_file_name(hash: &str) -> String {
    format!("active-head-{hash}.json")
}

fn create_real_directory(parent: &Path, path: &Path) -> Result<(), Protocol19ActivationError> {
    fs::create_dir_all(path).map_err(|source| io_error(path, source))?;
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Protocol19ActivationError::Invalid(
            "activation namespace is not a real directory".into(),
        ));
    }
    sync_directory(parent)
}

fn acquire_activation_lock(root: &Path, create: bool) -> Result<File, Protocol19ActivationError> {
    let path = root.join(ACTIVATION_LOCK_FILE);
    let file = OpenOptions::new()
        .create(create)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|source| io_error(&path, source))?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(file),
        Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
            Err(Protocol19ActivationError::WriterConflict)
        }
        Err(source) => Err(io_error(path, source)),
    }
}

fn reset_uncommitted_activation(root: &Path) -> Result<(), Protocol19ActivationError> {
    for entry in fs::read_dir(root).map_err(|source| io_error(root, source))? {
        let entry = entry.map_err(|source| io_error(root, source))?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            Protocol19ActivationError::Invalid(
                "activation namespace contains non-UTF-8 debris".into(),
            )
        })?;
        let recognized = name == ACTIVATION_LOCK_FILE || is_activation_staging_name(name);
        if !entry
            .file_type()
            .map_err(|source| io_error(entry.path(), source))?
            .is_file()
            || !recognized
        {
            return Err(Protocol19ActivationError::Invalid(
                "activation namespace contains unknown debris".into(),
            ));
        }
        if name != ACTIVATION_LOCK_FILE {
            fs::remove_file(entry.path()).map_err(|source| io_error(entry.path(), source))?;
        }
    }
    sync_directory(root)
}

fn reset_uncommitted_selector_temps(root: &Path) -> Result<(), Protocol19ActivationError> {
    let prefix = format!(".{ACTIVE_PROTOCOL_HEAD_FILE}.tmp-");
    for entry in fs::read_dir(root).map_err(|source| io_error(root, source))? {
        let entry = entry.map_err(|source| io_error(root, source))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(uuid) = name.strip_prefix(&prefix) else {
            continue;
        };
        if Uuid::parse_str(uuid).is_err()
            || !entry
                .file_type()
                .map_err(|source| io_error(entry.path(), source))?
                .is_file()
        {
            return Err(Protocol19ActivationError::Invalid(
                "active-selector staging debris is malformed".into(),
            ));
        }
        fs::remove_file(entry.path()).map_err(|source| io_error(entry.path(), source))?;
    }
    sync_directory(root)
}

fn is_activation_staging_name(name: &str) -> bool {
    fn content_addressed(name: &str, prefix: &str) -> bool {
        name.strip_prefix(prefix)
            .and_then(|tail| tail.strip_suffix(".json"))
            .is_some_and(valid_hash)
    }

    fn temporary(name: &str, prefix: &str) -> bool {
        let Some(tail) = name.strip_prefix(prefix) else {
            return false;
        };
        let Some((hash, uuid)) = tail.split_once(".json.tmp-") else {
            return false;
        };
        valid_hash(hash) && Uuid::parse_str(uuid).is_ok()
    }

    content_addressed(name, "authorization-")
        || content_addressed(name, "active-head-")
        || temporary(name, ".authorization-")
        || temporary(name, ".active-head-")
}

fn validate_activation_file_set(
    root: &Path,
    head: &ActiveProtocol19HeadV1,
) -> Result<(), Protocol19ActivationError> {
    let mut expected = vec![
        ACTIVATION_LOCK_FILE.to_owned(),
        authorization_file_name(&head.authorization_hash),
        active_head_file_name(&head.head_hash),
    ];
    let lifecycle_head = root.join(UNIVERSE_LIFECYCLE_HEAD_FILE);
    if lifecycle_head
        .try_exists()
        .map_err(|source| io_error(&lifecycle_head, source))?
    {
        expected.push(UNIVERSE_LIFECYCLE_HEAD_FILE.to_owned());
    }
    let mut observed = Vec::new();
    let mut lifecycle_temp_count = 0usize;
    for entry in fs::read_dir(root).map_err(|source| io_error(root, source))? {
        let entry = entry.map_err(|source| io_error(root, source))?;
        if !entry
            .file_type()
            .map_err(|source| io_error(entry.path(), source))?
            .is_file()
        {
            return Err(Protocol19ActivationError::Invalid(
                "activation namespace contains a non-file artifact".into(),
            ));
        }
        let name = entry.file_name().into_string().map_err(|_| {
            Protocol19ActivationError::Invalid(
                "activation namespace contains a non-UTF-8 artifact".into(),
            )
        })?;
        if let Some(uuid) = name.strip_prefix(&format!(".{UNIVERSE_LIFECYCLE_HEAD_FILE}.tmp-")) {
            if Uuid::parse_str(uuid).is_err() {
                return Err(Protocol19ActivationError::Invalid(
                    "activation namespace contains malformed lifecycle staging debris".into(),
                ));
            }
            lifecycle_temp_count += 1;
            if lifecycle_temp_count > 64 {
                return Err(Protocol19ActivationError::Invalid(
                    "activation namespace contains too much lifecycle staging debris".into(),
                ));
            }
            continue;
        }
        observed.push(name);
    }
    expected.sort_unstable();
    observed.sort_unstable();
    if observed != expected {
        return Err(Protocol19ActivationError::Invalid(
            "activation namespace is not the exact committed artifact set".into(),
        ));
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Protocol19ActivationError> {
    let parent = path.parent().ok_or_else(|| {
        Protocol19ActivationError::Invalid("activation artifact has no parent".into())
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            Protocol19ActivationError::Invalid("activation artifact name is not UTF-8".into())
        })?;
    let temporary = parent.join(format!(".{name}.tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| io_error(&temporary, source))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| io_error(&temporary, source))?;
        fs::rename(&temporary, path).map_err(|source| io_error(path, source))?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn persist_selector(
    universe_root: &Path,
    bytes: &[u8],
    failpoint: &mut Option<Protocol19ActivationFailpoint>,
) -> Result<(), Protocol19ActivationError> {
    let path = universe_root.join(ACTIVE_PROTOCOL_HEAD_FILE);
    let temporary = universe_root.join(format!(
        ".{ACTIVE_PROTOCOL_HEAD_FILE}.tmp-{}",
        Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| io_error(&temporary, source))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| io_error(&temporary, source))?;
        inject(
            failpoint,
            Protocol19ActivationFailpoint::SelectorTempSyncedBeforeRename,
        )?;
        fs::rename(&temporary, &path).map_err(|source| io_error(&path, source))?;
        inject(
            failpoint,
            Protocol19ActivationFailpoint::SelectorRenamedBeforeDirectorySync,
        )?;
        sync_directory(universe_root)?;
        inject(
            failpoint,
            Protocol19ActivationFailpoint::SelectorDirectorySyncedBeforeMemory,
        )
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, Protocol19ActivationError> {
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    let length = file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .len();
    if length > u64::try_from(maximum).unwrap_or(u64::MAX) {
        return Err(Protocol19ActivationError::TooLarge(path.to_owned()));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    file.take(u64::try_from(maximum).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    if bytes.len() > maximum {
        return Err(Protocol19ActivationError::TooLarge(path.to_owned()));
    }
    Ok(bytes)
}

fn sync_directory(path: &Path) -> Result<(), Protocol19ActivationError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(path, source))
}

fn inject(
    selected: &mut Option<Protocol19ActivationFailpoint>,
    current: Protocol19ActivationFailpoint,
) -> Result<(), Protocol19ActivationError> {
    if *selected == Some(current) {
        #[cfg(test)]
        if std::env::var_os("VERSE_PROTOCOL19_ACTIVATION_HARD_EXIT").is_some() {
            std::process::exit(98);
        }
        *selected = None;
        Err(Protocol19ActivationError::Injected(current))
    } else {
        Ok(())
    }
}

fn io_error(path: impl AsRef<Path>, source: std::io::Error) -> Protocol19ActivationError {
    Protocol19ActivationError::Io {
        path: path.as_ref().to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::Arc;
    use std::{path::PathBuf, process::Command};

    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::{TempDir, tempdir};

    use super::*;
    use crate::persistence::PersistenceError;

    const TEST_SEED: u64 = 8_119;
    const TRUSTED_NOW: u64 = 2_000_000_000_000;
    const SUBPROCESS_MODE_ENV: &str = "VERSE_PROTOCOL19_ACTIVATION_SUBPROCESS_MODE";
    const SUBPROCESS_ROOT_ENV: &str = "VERSE_PROTOCOL19_ACTIVATION_SUBPROCESS_ROOT";

    #[derive(Debug)]
    struct ManualClock(u64);

    impl TrustedClock for ManualClock {
        fn now_unix_ms(&self) -> Result<u64, PersistenceError> {
            Ok(self.0)
        }
    }

    fn fixture() -> (
        TempDir,
        Protocol19PreparedActivationSummary,
        Protocol19ActivationTrustPolicy,
        [SigningKey; 3],
    ) {
        let root = tempdir().expect("temporary universe root");
        crate::protocol19_install::initialize_frozen_protocol18_fixture_for_test(root.path(), true);
        let prepared = prepare_protocol19_for_activation(root.path(), TEST_SEED)
            .expect("protocol-19 world prepares");
        let keys = [
            SigningKey::from_bytes(&[1; 32]),
            SigningKey::from_bytes(&[2; 32]),
            SigningKey::from_bytes(&[3; 32]),
        ];
        let policy = Protocol19ActivationTrustPolicy::from_public_keys(
            prepared.universe_id.clone(),
            1,
            60_000,
            keys.each_ref().map(|key| key.verifying_key().to_bytes()),
        )
        .expect("test policy validates");
        (root, prepared, policy, keys)
    }

    fn signed_authorization(
        prepared: &Protocol19PreparedActivationSummary,
        policy: &Protocol19ActivationTrustPolicy,
        keys: &[SigningKey; 3],
        signer_indexes: &[usize],
        not_before: u64,
        expires_at: u64,
    ) -> Vec<u8> {
        let authorization = Protocol19ActivationAuthorizationV1::for_prepared_world(
            prepared,
            policy,
            "00112233445566778899aabbccddeeff",
            not_before,
            not_before,
            expires_at,
        )
        .expect("authorization builds");
        sign_authorization(authorization, keys, signer_indexes)
    }

    fn sign_authorization(
        authorization: Protocol19ActivationAuthorizationV1,
        keys: &[SigningKey; 3],
        signer_indexes: &[usize],
    ) -> Vec<u8> {
        let message = authorization.signing_bytes().expect("signing bytes encode");
        let signatures = signer_indexes
            .iter()
            .map(|index| {
                let key = &keys[*index];
                Protocol19ActivationSignatureV1::new(
                    protocol19_activation_signer_id(&key.verifying_key().to_bytes()),
                    key.sign(&message).to_bytes(),
                )
            })
            .collect();
        SignedProtocol19ActivationAuthorizationV1::new(authorization, signatures)
            .expect("signed authorization orders")
            .canonical_bytes()
            .expect("signed authorization encodes")
    }

    fn test_keys() -> [SigningKey; 3] {
        [
            SigningKey::from_bytes(&[1; 32]),
            SigningKey::from_bytes(&[2; 32]),
            SigningKey::from_bytes(&[3; 32]),
        ]
    }

    fn remove_legacy_runtime_material(root: &Path) {
        let retained_root_names = [
            ACTIVE_PROTOCOL_HEAD_FILE,
            ACTIVATION_DIRECTORY,
            "protocol-19-prepared-install-v1",
            "protocol-19-directory-v3",
            "cells",
        ];
        for entry in fs::read_dir(root).expect("universe root reads") {
            let entry = entry.expect("universe entry reads");
            let name = entry.file_name();
            let name = name.to_str().expect("test path is UTF-8");
            if retained_root_names.contains(&name) {
                continue;
            }
            if entry.file_type().expect("entry type reads").is_dir() {
                fs::remove_dir_all(entry.path()).expect("legacy root directory removes");
            } else {
                fs::remove_file(entry.path()).expect("legacy root file removes");
            }
        }
        for entry in fs::read_dir(root.join("cells")).expect("cell routes read") {
            let cell_root = entry.expect("cell route reads").path();
            for artifact in fs::read_dir(&cell_root).expect("cell root reads") {
                let artifact = artifact.expect("cell artifact reads");
                if artifact.file_name() == "protocol-19-world-v21" {
                    continue;
                }
                if artifact
                    .file_type()
                    .expect("cell artifact type reads")
                    .is_dir()
                {
                    fs::remove_dir_all(artifact.path()).expect("legacy cell directory removes");
                } else {
                    fs::remove_file(artifact.path()).expect("legacy cell file removes");
                }
            }
        }
    }

    fn failpoint_label(failpoint: Protocol19ActivationFailpoint) -> &'static str {
        match failpoint {
            Protocol19ActivationFailpoint::NamespaceSynced => "namespace_synced",
            Protocol19ActivationFailpoint::AuthorizationSynced => "authorization_synced",
            Protocol19ActivationFailpoint::HistoryHeadSynced => "history_head_synced",
            Protocol19ActivationFailpoint::SelectorTempSyncedBeforeRename => {
                "selector_temp_synced_before_rename"
            }
            Protocol19ActivationFailpoint::SelectorRenamedBeforeDirectorySync => {
                "selector_renamed_before_directory_sync"
            }
            Protocol19ActivationFailpoint::SelectorDirectorySyncedBeforeMemory => {
                "selector_directory_synced_before_memory"
            }
        }
    }

    fn parse_failpoint_label(label: &str) -> Protocol19ActivationFailpoint {
        match label {
            "namespace_synced" => Protocol19ActivationFailpoint::NamespaceSynced,
            "authorization_synced" => Protocol19ActivationFailpoint::AuthorizationSynced,
            "history_head_synced" => Protocol19ActivationFailpoint::HistoryHeadSynced,
            "selector_temp_synced_before_rename" => {
                Protocol19ActivationFailpoint::SelectorTempSyncedBeforeRename
            }
            "selector_renamed_before_directory_sync" => {
                Protocol19ActivationFailpoint::SelectorRenamedBeforeDirectorySync
            }
            "selector_directory_synced_before_memory" => {
                Protocol19ActivationFailpoint::SelectorDirectorySyncedBeforeMemory
            }
            _ => panic!("unsupported activation failpoint label {label}"),
        }
    }

    #[test]
    fn protocol19_activation_subprocess_driver() {
        let Some(mode) = std::env::var_os(SUBPROCESS_MODE_ENV) else {
            return;
        };
        let root = PathBuf::from(
            std::env::var_os(SUBPROCESS_ROOT_ENV).expect("subprocess root is configured"),
        );
        let prepared = prepare_protocol19_for_activation(&root, TEST_SEED)
            .expect("subprocess prepared world reopens");
        let keys = test_keys();
        let policy = Protocol19ActivationTrustPolicy::from_public_keys(
            prepared.universe_id.clone(),
            1,
            60_000,
            keys.each_ref().map(|key| key.verifying_key().to_bytes()),
        )
        .expect("subprocess policy builds");
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        let failpoint = parse_failpoint_label(&mode.to_string_lossy());
        let result = activate_protocol19_world_with_failpoint(
            &root,
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
            Some(failpoint),
        );
        panic!("hard-exit activation failpoint returned instead of terminating: {result:?}");
    }

    #[test]
    fn canonical_two_of_three_activation_restarts_from_only_the_global_head() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 2],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        let activated = activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        assert_eq!(activated.cell_count, 2);
        assert_eq!(
            activated.prepared_install_head_hash,
            prepared.prepared_install_head_hash
        );
        assert!(root.path().join(ACTIVE_PROTOCOL_HEAD_FILE).is_file());
        let mut replay = Protocol19ActivationAuthorizationV1::for_prepared_world(
            &prepared,
            &policy,
            "ffeeddccbbaa99887766554433221100",
            TRUSTED_NOW,
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        )
        .expect("second authorization builds");
        replay.not_before_unix_ms = TRUSTED_NOW - 500;
        let replay = sign_authorization(replay, &keys, &[0, 2]);
        assert!(
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &replay,
                &ManualClock(TRUSTED_NOW),
            )
            .is_err()
        );

        let reopened = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("expired authorization does not deactivate committed head");
        assert_eq!(reopened.summary(), &activated);
        assert!(matches!(
            crate::Store::open_for_cell(
                root.path().join("cells").join(
                    crate::celestial::cell_id(&crate::cell_origin_key())
                        .expect("origin ID derives")
                ),
                TEST_SEED,
                crate::cell_origin_key(),
            ),
            Err(PersistenceError::InvalidRuntimeUniverseManifest(_))
        ));
        drop(reopened);

        let manifest = crate::celestial::universe_manifest(
            TEST_SEED,
            crate::WORLD_SCHEMA_VERSION,
            crate::EVENT_SCHEMA_VERSION,
        )
        .expect("legacy manifest builds");
        assert!(matches!(
            crate::LocalCellDirectory::open(
                root.path(),
                &manifest,
                crate::proof_cell_keys().expect("proof cells derive")
            ),
            Err(crate::CellDirectoryError::InvalidDirectory(_))
        ));
        remove_legacy_runtime_material(root.path());
        let standalone = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("active world boots without legacy runtime material");
        assert_eq!(standalone.summary(), &activated);
    }

    #[test]
    fn authorization_requires_threshold_and_exact_time_window() {
        let (root, prepared, policy, keys) = fixture();
        let one_signature = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        assert!(
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &one_signature,
                &ManualClock(TRUSTED_NOW),
            )
            .is_err()
        );
        assert!(!root.path().join(ACTIVE_PROTOCOL_HEAD_FILE).exists());

        let expired = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 10_000,
            TRUSTED_NOW,
        );
        assert!(
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &expired,
                &ManualClock(TRUSTED_NOW),
            )
            .is_err()
        );
        assert!(!root.path().join(ACTIVE_PROTOCOL_HEAD_FILE).exists());

        let future = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW + 1,
            TRUSTED_NOW + 10_000,
        );
        assert!(
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &future,
                &ManualClock(TRUSTED_NOW),
            )
            .is_err()
        );

        let before_cutoff = signed_authorization(&prepared, &policy, &keys, &[0, 1], 1, 100);
        assert!(
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &before_cutoff,
                &ManualClock(1),
            )
            .is_err()
        );

        assert!(
            Protocol19ActivationAuthorizationV1::for_prepared_world(
                &prepared,
                &policy,
                "00112233445566778899aabbccddeeff",
                TRUSTED_NOW,
                TRUSTED_NOW,
                TRUSTED_NOW + 60_001,
            )
            .is_err()
        );
    }

    #[test]
    fn first_activation_rejects_noninitial_generation_or_history() {
        let (root, prepared, policy, keys) = fixture();
        let mutations: [fn(&mut Protocol19ActivationAuthorizationV1); 3] = [
            |value: &mut Protocol19ActivationAuthorizationV1| {
                value.activation_generation = 2;
            },
            |value: &mut Protocol19ActivationAuthorizationV1| {
                value.previous_activation_head_hash = "a".repeat(64);
            },
            |value: &mut Protocol19ActivationAuthorizationV1| {
                value.signer_policy_generation = 2;
            },
        ];
        for mutate in mutations {
            let mut authorization = Protocol19ActivationAuthorizationV1::for_prepared_world(
                &prepared,
                &policy,
                "00112233445566778899aabbccddeeff",
                TRUSTED_NOW,
                TRUSTED_NOW - 1_000,
                TRUSTED_NOW + 10_000,
            )
            .expect("authorization builds");
            mutate(&mut authorization);
            let signed = sign_authorization(authorization, &keys, &[0, 1]);
            assert!(
                activate_protocol19_world_with_clock(
                    root.path(),
                    TEST_SEED,
                    &policy,
                    &signed,
                    &ManualClock(TRUSTED_NOW),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn valid_signatures_cannot_authorize_changed_prepared_bindings() {
        type Mutator = fn(&mut Protocol19ActivationAuthorizationV1);
        let mutations: [Mutator; 16] = [
            |value| value.compatibility.protocol_version += 1,
            |value| value.universe_id.push_str("-other"),
            |value| value.world_seed = "8120".into(),
            |value| value.prepared_install_head_hash = "f".repeat(64),
            |value| value.migration_receipt_hash = "f".repeat(64),
            |value| value.migration_anchor_hash = "f".repeat(64),
            |value| value.target_manifest_hash = "f".repeat(64),
            |value| value.target_directory_document_hash = "f".repeat(64),
            |value| value.target_assignment_root = "f".repeat(64),
            |value| value.target_placement_root = "f".repeat(64),
            |value| value.cell_set_root = "f".repeat(64),
            |value| value.global_conservation_root = "f".repeat(64),
            |value| value.normalized_gameplay_root = "f".repeat(64),
            |value| value.identity_map_root = "f".repeat(64),
            |value| value.production_origin_root = "f".repeat(64),
            |value| value.cell_count += 1,
        ];
        let (root, prepared, policy, keys) = fixture();
        for mutate in mutations {
            let mut authorization = Protocol19ActivationAuthorizationV1::for_prepared_world(
                &prepared,
                &policy,
                "00112233445566778899aabbccddeeff",
                TRUSTED_NOW,
                TRUSTED_NOW - 1_000,
                TRUSTED_NOW + 10_000,
            )
            .expect("authorization builds");
            mutate(&mut authorization);
            let signed = sign_authorization(authorization, &keys, &[0, 1]);
            assert!(
                activate_protocol19_world_with_clock(
                    root.path(),
                    TEST_SEED,
                    &policy,
                    &signed,
                    &ManualClock(TRUSTED_NOW),
                )
                .is_err()
            );
            assert!(!root.path().join(ACTIVE_PROTOCOL_HEAD_FILE).exists());
        }
    }

    #[test]
    fn duplicate_unknown_and_altered_signatures_fail_closed() {
        let (root, prepared, policy, keys) = fixture();
        let authorization = Protocol19ActivationAuthorizationV1::for_prepared_world(
            &prepared,
            &policy,
            "00112233445566778899aabbccddeeff",
            TRUSTED_NOW,
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        )
        .expect("authorization builds");
        let message = authorization.signing_bytes().expect("signing bytes encode");
        let surplus = sign_authorization(authorization.clone(), &keys, &[0, 1, 2]);
        assert!(
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &surplus,
                &ManualClock(TRUSTED_NOW),
            )
            .is_err()
        );
        assert!(!root.path().join(ACTIVE_PROTOCOL_HEAD_FILE).exists());

        let signer_id = protocol19_activation_signer_id(&keys[0].verifying_key().to_bytes());
        let signature = keys[0].sign(&message).to_bytes();
        let duplicate = SignedProtocol19ActivationAuthorizationV1 {
            authorization: authorization.clone(),
            signatures: vec![
                Protocol19ActivationSignatureV1::new(signer_id.clone(), signature),
                Protocol19ActivationSignatureV1::new(signer_id, signature),
            ],
        };
        assert!(duplicate.canonical_bytes().is_err());

        let mut unknown = serde_json::from_slice::<SignedProtocol19ActivationAuthorizationV1>(
            &sign_authorization(authorization.clone(), &keys, &[0, 1]),
        )
        .expect("signed envelope decodes");
        unknown.signatures[1].signer_id = "f".repeat(64);
        unknown
            .signatures
            .sort_by(|left, right| left.signer_id.cmp(&right.signer_id));
        let unknown = serde_json::to_vec(&unknown).expect("unknown envelope encodes");
        assert!(
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &unknown,
                &ManualClock(TRUSTED_NOW),
            )
            .is_err()
        );

        let mut altered = serde_json::from_slice::<SignedProtocol19ActivationAuthorizationV1>(
            &sign_authorization(authorization, &keys, &[0, 1]),
        )
        .expect("signed envelope decodes");
        let replacement = if altered.signatures[0].signature.starts_with("00") {
            "ff"
        } else {
            "00"
        };
        altered.signatures[0]
            .signature
            .replace_range(0..2, replacement);
        let altered = serde_json::to_vec(&altered).expect("altered envelope encodes");
        assert!(
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &altered,
                &ManualClock(TRUSTED_NOW),
            )
            .is_err()
        );
    }

    #[test]
    fn every_activation_crash_boundary_is_retryable_or_forward_recoverable() {
        let failpoints = [
            Protocol19ActivationFailpoint::NamespaceSynced,
            Protocol19ActivationFailpoint::AuthorizationSynced,
            Protocol19ActivationFailpoint::HistoryHeadSynced,
            Protocol19ActivationFailpoint::SelectorTempSyncedBeforeRename,
            Protocol19ActivationFailpoint::SelectorRenamedBeforeDirectorySync,
            Protocol19ActivationFailpoint::SelectorDirectorySyncedBeforeMemory,
        ];
        for failpoint in failpoints {
            let (root, prepared, policy, keys) = fixture();
            let signed = signed_authorization(
                &prepared,
                &policy,
                &keys,
                &[0, 1],
                TRUSTED_NOW - 1_000,
                TRUSTED_NOW + 10_000,
            );
            assert!(matches!(
                activate_protocol19_world_with_failpoint(
                    root.path(),
                    TEST_SEED,
                    &policy,
                    &signed,
                    &ManualClock(TRUSTED_NOW),
                    Some(failpoint),
                ),
                Err(Protocol19ActivationError::Injected(observed)) if observed == failpoint
            ));
            let recovered = activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &signed,
                &ManualClock(TRUSTED_NOW),
            )
            .expect("activation retry recovers");
            assert_eq!(
                recovered.prepared_install_head_hash,
                prepared.prepared_install_head_hash
            );
        }
    }

    #[test]
    fn process_crash_at_every_activation_boundary_recovers_exactly() {
        let failpoints = [
            Protocol19ActivationFailpoint::NamespaceSynced,
            Protocol19ActivationFailpoint::AuthorizationSynced,
            Protocol19ActivationFailpoint::HistoryHeadSynced,
            Protocol19ActivationFailpoint::SelectorTempSyncedBeforeRename,
            Protocol19ActivationFailpoint::SelectorRenamedBeforeDirectorySync,
            Protocol19ActivationFailpoint::SelectorDirectorySyncedBeforeMemory,
        ];
        for failpoint in failpoints {
            let (root, prepared, policy, keys) = fixture();
            let status = Command::new(std::env::current_exe().expect("test executable resolves"))
                .arg("--exact")
                .arg("protocol19_activation::tests::protocol19_activation_subprocess_driver")
                .arg("--nocapture")
                .env(SUBPROCESS_MODE_ENV, failpoint_label(failpoint))
                .env(SUBPROCESS_ROOT_ENV, root.path())
                .env("VERSE_PROTOCOL19_ACTIVATION_HARD_EXIT", "1")
                .status()
                .expect("activation crash subprocess runs");
            assert_eq!(status.code(), Some(98));
            let signed = signed_authorization(
                &prepared,
                &policy,
                &keys,
                &[0, 1],
                TRUSTED_NOW - 1_000,
                TRUSTED_NOW + 10_000,
            );
            let recovered = activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &signed,
                &ManualClock(TRUSTED_NOW),
            )
            .expect("hard-crashed activation recovers");
            assert_eq!(recovered.active_head_hash.len(), 64);
            assert_eq!(
                recovered.prepared_install_head_hash,
                prepared.prepared_install_head_hash
            );
        }
    }

    #[test]
    fn authorization_tamper_blocks_directory_recovery_without_repair() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[1, 2],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        let activated = activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let authorization_path = root
            .path()
            .join(ACTIVATION_DIRECTORY)
            .join(authorization_file_name(&activated.authorization_hash));
        let mut changed = fs::read(&authorization_path).expect("authorization reads");
        changed.push(b'\n');
        fs::write(&authorization_path, &changed).expect("test tamper writes");
        let directory_root = root.path().join("protocol-19-directory-v3");
        let history_path = directory_root.join("history-v3.ndjson");
        let head_path = directory_root.join("head-v3.json");
        let mut changed_history = fs::read(&history_path).expect("directory history reads");
        changed_history.extend_from_slice(b"{\"torn\":");
        fs::write(&history_path, &changed_history).expect("torn history suffix writes");
        let unchanged_head = fs::read(&head_path).expect("directory head reads");

        assert!(open_activated_protocol19_world(root.path(), TEST_SEED, &policy).is_err());
        assert_eq!(
            fs::read(&authorization_path).expect("tampered authorization rereads"),
            changed
        );
        assert_eq!(
            fs::read(&history_path).expect("rejected directory history rereads"),
            changed_history
        );
        assert_eq!(
            fs::read(&head_path).expect("rejected directory head rereads"),
            unchanged_head
        );
    }

    #[test]
    fn active_directory_torn_suffix_recovers_after_signed_prefix_validation() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let history_path = root
            .path()
            .join("protocol-19-directory-v3")
            .join("history-v3.ndjson");
        let expected = fs::read(&history_path).expect("directory history reads");
        let mut changed = expected.clone();
        changed.extend_from_slice(b"{\"torn\":");
        fs::write(&history_path, &changed).expect("test tamper writes");

        let world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("a torn suffix outside the signed and headed prefix is recoverable");
        drop(world);
        assert_eq!(
            fs::read(&history_path).expect("recovered directory history rereads"),
            expected
        );
    }

    #[test]
    fn policy_hash_is_external_and_canonical() {
        let (_root, _prepared, policy, _keys) = fixture();
        assert!(
            Protocol19ActivationTrustPolicy::from_canonical_bytes(
                policy.canonical_bytes(),
                &"0".repeat(64),
            )
            .is_err()
        );
        let mut noncanonical = policy.canonical_bytes().to_vec();
        noncanonical.push(b'\n');
        assert!(
            Protocol19ActivationTrustPolicy::from_canonical_bytes(
                &noncanonical,
                policy.policy_hash(),
            )
            .is_err()
        );
    }

    #[test]
    fn protocol19_activation_golden_vector_v1() {
        let keys = test_keys();
        let prepared = Protocol19PreparedActivationSummary {
            compatibility: Protocol19CompatibilityTuple::canonical(),
            universe_id: "golden-universe-v1".into(),
            world_seed: TEST_SEED,
            target_manifest_hash: "1".repeat(64),
            migration_anchor_hash: "2".repeat(64),
            migration_receipt_hash: "3".repeat(64),
            target_directory_document_hash: "4".repeat(64),
            target_assignment_root: "5".repeat(64),
            target_placement_root: "6".repeat(64),
            identity_map_root: "7".repeat(64),
            production_origin_root: "8".repeat(64),
            global_conservation_root: "9".repeat(64),
            normalized_gameplay_root: "a".repeat(64),
            cell_count: 2,
            cell_set_root: "b".repeat(64),
            prepared_install_head_hash: "c".repeat(64),
        };
        let policy = Protocol19ActivationTrustPolicy::from_public_keys(
            prepared.universe_id.clone(),
            1,
            60_000,
            keys.each_ref().map(|key| key.verifying_key().to_bytes()),
        )
        .expect("golden policy builds");
        let authorization = Protocol19ActivationAuthorizationV1::for_prepared_world(
            &prepared,
            &policy,
            "00112233445566778899aabbccddeeff",
            TRUSTED_NOW,
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        )
        .expect("golden authorization builds");
        let signing_bytes = authorization.signing_bytes().expect("signing bytes encode");
        let signed_bytes = sign_authorization(authorization.clone(), &keys, &[0, 2]);
        let signed = decode_signed_authorization(&signed_bytes).expect("envelope decodes");
        let authorization_hash =
            hash_json(AUTHORIZATION_HASH_DOMAIN, &signed).expect("authorization hashes");
        let install = Protocol19PreparedInstallSummary {
            compatibility: prepared.compatibility.clone(),
            universe_id: prepared.universe_id.clone(),
            world_seed: prepared.world_seed,
            target_manifest_hash: prepared.target_manifest_hash.clone(),
            migration_anchor_hash: prepared.migration_anchor_hash.clone(),
            migration_receipt_hash: prepared.migration_receipt_hash.clone(),
            target_directory_document_hash: prepared.target_directory_document_hash.clone(),
            target_assignment_root: prepared.target_assignment_root.clone(),
            target_placement_root: prepared.target_placement_root.clone(),
            identity_map_root: prepared.identity_map_root.clone(),
            production_origin_root: prepared.production_origin_root.clone(),
            global_conservation_root: prepared.global_conservation_root.clone(),
            normalized_gameplay_root: prepared.normalized_gameplay_root.clone(),
            cell_count: prepared.cell_count,
            cell_set_root: prepared.cell_set_root.clone(),
            prepared_install_head_hash: prepared.prepared_install_head_hash.clone(),
        };
        let head =
            ActiveProtocol19HeadV1::new(&install, authorization_hash.clone(), &authorization)
                .expect("active head builds");
        let expected_policy = concat!(
            r#"{"schema_version":1,"signature_scheme":"ed25519-strict-v1","universe_id":"golden-universe-v1","policy_generation":1,"threshold":2,"maximum_authorization_validity_millis":60000,"signers":["#,
            r#"{"signer_id":"07fdc37c75962c36d371245d9e554dcb8797c82df76cbe57500889e68ec72c8a","public_key":"8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394"},"#,
            r#"{"signer_id":"15b97447851ed679e73dc2f178904a012dc8c5987025c516c6b02ceb069b0edb","public_key":"8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c"},"#,
            r#"{"signer_id":"9f4ad621eea25bd0dc2ba93fe72fa7d86f953e629d5673ffa314e772a869719a","public_key":"ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1"}],"policy_hash":"1ca229a5361d27e9776c3e03355712c80e3e950af497e0c1fadec133121a6a58"}"#,
        );
        let expected_authorization = r#"{"schema_version":1,"authorization_kind":"activate_prepared_protocol19_world","signature_scheme":"ed25519-strict-v1","compatibility":{"protocol_version":19,"projection_schema_version":5,"world_schema_version":21,"event_schema_version":17,"content_schema_version":11,"content_manifest_version":"p1.5.0","celestial_registry_schema_version":1,"universe_manifest_schema_version":5,"interest_schema_version":3,"operation_fingerprint_schema_version":2,"lifecycle_control_schema_version":2,"production_occurrence_schema_version":1,"cell_key_schema_version":1,"directory_schema_version":3,"transfer_package_schema_version":2},"universe_id":"golden-universe-v1","world_seed":"8119","prepared_install_head_hash":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","migration_receipt_hash":"3333333333333333333333333333333333333333333333333333333333333333","migration_anchor_hash":"2222222222222222222222222222222222222222222222222222222222222222","target_manifest_hash":"1111111111111111111111111111111111111111111111111111111111111111","target_directory_document_hash":"4444444444444444444444444444444444444444444444444444444444444444","target_assignment_root":"5555555555555555555555555555555555555555555555555555555555555555","target_placement_root":"6666666666666666666666666666666666666666666666666666666666666666","cell_set_root":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","global_conservation_root":"9999999999999999999999999999999999999999999999999999999999999999","normalized_gameplay_root":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","identity_map_root":"7777777777777777777777777777777777777777777777777777777777777777","production_origin_root":"8888888888888888888888888888888888888888888888888888888888888888","cell_count":2,"signer_policy_hash":"1ca229a5361d27e9776c3e03355712c80e3e950af497e0c1fadec133121a6a58","signer_policy_generation":1,"activation_generation":1,"authorization_nonce":"00112233445566778899aabbccddeeff","authorized_activation_unix_ms":2000000000000,"not_before_unix_ms":1999999999000,"expires_at_unix_ms":2000000010000,"previous_activation_head_hash":""}"#;
        let expected_signature_0 = "e46b28476f48f267ac439b4fae25ee581b94becb7d77204663e491338e3c97d95ac742c165f32a3a826c1a61a9b5253e7b7beec90aec9fdc8e63a1dac7b53d05";
        let expected_signature_1 = "41f8eb8f096a592d9c1e49faa8429a10050b3168078df44603ad15e7f5f69a25ec7c3bb6b4db2230a51a742fc3d270af20b2a5304a6edd9d2f877b075c9bee0e";
        let expected_active_head = r#"{"schema_version":1,"compatibility":{"protocol_version":19,"projection_schema_version":5,"world_schema_version":21,"event_schema_version":17,"content_schema_version":11,"content_manifest_version":"p1.5.0","celestial_registry_schema_version":1,"universe_manifest_schema_version":5,"interest_schema_version":3,"operation_fingerprint_schema_version":2,"lifecycle_control_schema_version":2,"production_occurrence_schema_version":1,"cell_key_schema_version":1,"directory_schema_version":3,"transfer_package_schema_version":2},"mode":"active","universe_id":"golden-universe-v1","world_seed":"8119","prepared_install_head_hash":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","migration_receipt_hash":"3333333333333333333333333333333333333333333333333333333333333333","migration_anchor_hash":"2222222222222222222222222222222222222222222222222222222222222222","target_manifest_hash":"1111111111111111111111111111111111111111111111111111111111111111","target_directory_document_hash":"4444444444444444444444444444444444444444444444444444444444444444","cell_set_root":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","global_conservation_root":"9999999999999999999999999999999999999999999999999999999999999999","normalized_gameplay_root":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","identity_map_root":"7777777777777777777777777777777777777777777777777777777777777777","production_origin_root":"8888888888888888888888888888888888888888888888888888888888888888","cell_count":2,"authorization_hash":"caa083bb028e7a3b88d672befd83c6cdcd72877a46510f4cb88afa890604a1ec","signer_policy_hash":"1ca229a5361d27e9776c3e03355712c80e3e950af497e0c1fadec133121a6a58","signer_policy_generation":1,"activation_generation":1,"authorized_activation_unix_ms":2000000000000,"previous_activation_head_hash":"","head_hash":"907d4f29af82422a004e8215e2162b781f9e7495a923c8c82f4699de7bd2aff7"}"#;
        let expected_signer_ids = [
            "07fdc37c75962c36d371245d9e554dcb8797c82df76cbe57500889e68ec72c8a",
            "15b97447851ed679e73dc2f178904a012dc8c5987025c516c6b02ceb069b0edb",
            "9f4ad621eea25bd0dc2ba93fe72fa7d86f953e629d5673ffa314e772a869719a",
        ];
        assert_eq!(policy.canonical_bytes(), expected_policy.as_bytes());
        assert_eq!(
            policy.policy_hash(),
            "1ca229a5361d27e9776c3e03355712c80e3e950af497e0c1fadec133121a6a58"
        );
        assert_eq!(
            policy
                .document
                .signers
                .iter()
                .map(|signer| signer.signer_id.as_str())
                .collect::<Vec<_>>(),
            expected_signer_ids
        );
        let mut expected_signing = AUTHORIZATION_SIGNING_DOMAIN.to_vec();
        expected_signing.extend_from_slice(expected_authorization.as_bytes());
        assert_eq!(signing_bytes, expected_signing);
        assert_eq!(signed.signatures[0].signer_id, expected_signer_ids[1]);
        assert_eq!(signed.signatures[0].signature, expected_signature_0);
        assert_eq!(signed.signatures[1].signer_id, expected_signer_ids[2]);
        assert_eq!(signed.signatures[1].signature, expected_signature_1);
        let expected_envelope = format!(
            r#"{{"authorization":{},"signatures":[{{"signer_id":"{}","signature":"{}"}},{{"signer_id":"{}","signature":"{}"}}]}}"#,
            expected_authorization,
            expected_signer_ids[1],
            expected_signature_0,
            expected_signer_ids[2],
            expected_signature_1,
        );
        assert_eq!(signed_bytes, expected_envelope.as_bytes());
        assert_eq!(
            authorization_hash,
            "caa083bb028e7a3b88d672befd83c6cdcd72877a46510f4cb88afa890604a1ec"
        );
        assert_eq!(
            head.encode_canonical().expect("head encodes"),
            expected_active_head.as_bytes()
        );
        assert_eq!(
            head.head_hash,
            "907d4f29af82422a004e8215e2162b781f9e7495a923c8c82f4699de7bd2aff7"
        );
    }

    #[test]
    fn activated_capability_holds_all_writers() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let held = Arc::new(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
                .expect("activated world opens"),
        );
        assert!(matches!(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy),
            Err(Protocol19ActivationError::Install(_) | Protocol19ActivationError::WriterConflict)
        ));
        drop(held);
        open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("released activated world reopens");
    }

    #[test]
    fn uncoordinated_directory_authority_changes_cannot_bypass_lifecycle() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");

        let cell_key = crate::cell_origin_key();
        let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("activated world opens");
        let genesis = world
            .cell_assignment(&cell_key)
            .expect("genesis assignment resolves")
            .clone();
        assert_eq!(genesis.state, crate::CellAssignmentState::Sleeping);
        assert!(genesis.holder_id.is_none());

        let claimed = world
            .claim_cell_authority(&cell_key, genesis.assignment_generation, "worker-v19-a")
            .expect("sleeping cell claims");
        assert_eq!(
            claimed.assignment_generation,
            genesis.assignment_generation + 1
        );
        assert_eq!(
            claimed.authority_fencing_token,
            genesis.authority_fencing_token + 1
        );
        assert_eq!(claimed.state, crate::CellAssignmentState::Assigned);
        assert_eq!(claimed.holder_id.as_deref(), Some("worker-v19-a"));
        assert!(
            world
                .recover_cell_authority(&cell_key, genesis.assignment_generation, "worker-v19-a",)
                .is_err(),
            "a claim cannot alias a recovery retry"
        );
        assert_eq!(
            world
                .claim_cell_authority(&cell_key, genesis.assignment_generation, "worker-v19-a")
                .expect("uncertain claim redelivery is exact"),
            claimed
        );
        assert!(
            world
                .claim_cell_authority(&cell_key, genesis.assignment_generation, "worker-v19-b")
                .is_err(),
            "another holder cannot alias the committed successor"
        );

        let recovered = world
            .recover_cell_authority(&cell_key, claimed.assignment_generation, "worker-v19-b")
            .expect("successor holder recovers assigned cell");
        assert_eq!(
            recovered.assignment_generation,
            claimed.assignment_generation + 1
        );
        assert_eq!(
            recovered.authority_fencing_token,
            claimed.authority_fencing_token + 1
        );
        assert_eq!(recovered.holder_id.as_deref(), Some("worker-v19-b"));
        assert!(
            world
                .claim_cell_authority(&cell_key, claimed.assignment_generation, "worker-v19-b",)
                .is_err(),
            "a recovery cannot alias a claim retry"
        );
        assert!(
            world
                .release_cell_authority(&cell_key, claimed.assignment_generation, "worker-v19-a",)
                .is_err(),
            "the fenced predecessor cannot release its successor"
        );

        let released = world
            .release_cell_authority(&cell_key, recovered.assignment_generation, "worker-v19-b")
            .expect("current holder releases to sleeping");
        assert_eq!(released.state, crate::CellAssignmentState::Sleeping);
        assert!(released.holder_id.is_none());
        assert!(
            world
                .release_cell_authority(
                    &cell_key,
                    recovered.assignment_generation,
                    "worker-v19-impostor",
                )
                .is_err(),
            "another holder cannot alias the release retry"
        );
        assert_eq!(
            world
                .release_cell_authority(&cell_key, recovered.assignment_generation, "worker-v19-b",)
                .expect("release redelivery is exact"),
            released
        );
        drop(world);

        assert!(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy).is_err(),
            "test-only raw directory history cannot reopen as a healthy lifecycle"
        );
    }

    #[test]
    fn lifecycle_v2_no_work_dispatch_claims_and_releases_without_polling() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");

        let cell_key = crate::cell_origin_key();
        let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("activated lifecycle opens");
        let before = world
            .cell_assignment(&cell_key)
            .expect("sleeping assignment resolves")
            .clone();
        let outcome = world
            .dispatch_background_production_with_clock(
                &cell_key,
                "lifecycle-worker-a",
                &ManualClock(TRUSTED_NOW + 1_000),
            )
            .expect("empty cell dispatches");
        assert_eq!(outcome.mode, "sleeping");
        assert_eq!(outcome.committed_quanta, 0);
        assert_eq!(outcome.acknowledged_production_sequence, 0);
        assert_eq!(outcome.next_scheduled_for_unix_ms, None);
        let after = world
            .cell_assignment(&cell_key)
            .expect("released assignment resolves")
            .clone();
        assert_eq!(after.state, crate::CellAssignmentState::Sleeping);
        assert_eq!(
            after.assignment_generation,
            before.assignment_generation + 1
        );
        assert_eq!(
            after.authority_fencing_token,
            before.authority_fencing_token + 1
        );
        drop(world);

        let cell_root = root
            .path()
            .join("cells")
            .join(&after.cell_id)
            .join("protocol-19-world-v21");
        assert!(cell_root.join("lifecycle-v2.genesis.json").is_file());
        assert!(cell_root.join("lifecycle-v2.ndjson").is_file());
        assert!(cell_root.join("lifecycle-v2.head.json").is_file());

        let mut reopened = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("completed lifecycle history reopens");
        let second = reopened
            .dispatch_background_production_with_clock(
                &cell_key,
                "lifecycle-worker-b",
                &ManualClock(TRUSTED_NOW + 2_000),
            )
            .expect("restarted empty cell dispatches");
        assert_eq!(second.mode, "sleeping");
        assert_eq!(second.committed_quanta, 0);
        assert_eq!(
            reopened
                .cell_assignment(&cell_key)
                .expect("second release resolves")
                .assignment_generation,
            after.assignment_generation + 1
        );
    }

    #[test]
    fn lifecycle_v2_rejects_clock_rollback_and_incomplete_runtime_artifacts() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let cell_key = crate::cell_origin_key();
        let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("activated lifecycle opens");
        world
            .dispatch_background_production_with_clock(
                &cell_key,
                "lifecycle-worker-clock",
                &ManualClock(TRUSTED_NOW + 1_000),
            )
            .expect("first dispatch establishes trusted time");
        assert!(
            world
                .dispatch_background_production_with_clock(
                    &cell_key,
                    "lifecycle-worker-clock-next",
                    &ManualClock(TRUSTED_NOW),
                )
                .is_err(),
            "trusted time may not move backwards"
        );
        let cell_id = world
            .cell_assignment(&cell_key)
            .expect("cell resolves")
            .cell_id
            .clone();
        drop(world);

        let lifecycle_head = root
            .path()
            .join("cells")
            .join(cell_id)
            .join("protocol-19-world-v21")
            .join("lifecycle-v2.head.json");
        fs::remove_file(&lifecycle_head).expect("test removes one runtime artifact");
        assert!(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy).is_err(),
            "an incomplete lifecycle runtime set fails closed"
        );
    }

    #[test]
    fn lifecycle_v2_universe_commitment_rejects_deleted_cell_history() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let cell_key = crate::cell_origin_key();
        let world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("universe lifecycle bootstraps");
        let cell_id = world
            .cell_assignment(&cell_key)
            .expect("cell resolves")
            .cell_id
            .clone();
        drop(world);

        let lifecycle_root = root
            .path()
            .join("cells")
            .join(cell_id)
            .join("protocol-19-world-v21");
        fs::remove_file(lifecycle_root.join("lifecycle-v2.ndjson"))
            .expect("test removes lifecycle history");
        fs::remove_file(lifecycle_root.join("lifecycle-v2.head.json"))
            .expect("test removes lifecycle head");
        assert!(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy).is_err(),
            "a universe-committed lifecycle cannot be recreated after deletion"
        );
    }

    #[test]
    fn lifecycle_v2_rejects_children_without_the_universe_head() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        drop(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
                .expect("universe lifecycle bootstraps"),
        );
        fs::remove_file(
            root.path()
                .join("protocol-19-activation-v1")
                .join("universe-lifecycle-v1.head.json"),
        )
        .expect("test removes universe lifecycle head");
        assert!(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy).is_err(),
            "existing child histories cannot be blessed by a replacement universe head"
        );
    }

    #[test]
    fn lifecycle_v2_universe_commitment_rejects_child_head_rollback() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let cell_key = crate::cell_origin_key();
        let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("universe lifecycle bootstraps");
        let cell_id = world
            .cell_assignment(&cell_key)
            .expect("cell resolves")
            .cell_id
            .clone();
        let lifecycle_root = root
            .path()
            .join("cells")
            .join(cell_id)
            .join("protocol-19-world-v21");
        let initial_history = fs::read(lifecycle_root.join("lifecycle-v2.ndjson"))
            .expect("initial lifecycle history reads");
        let initial_head = fs::read(lifecycle_root.join("lifecycle-v2.head.json"))
            .expect("initial lifecycle head reads");
        world
            .dispatch_background_production_with_clock(
                &cell_key,
                "lifecycle-rollback-worker",
                &ManualClock(TRUSTED_NOW + 1_000),
            )
            .expect("lifecycle advances");
        drop(world);

        fs::write(lifecycle_root.join("lifecycle-v2.ndjson"), initial_history)
            .expect("test rolls child history back");
        fs::write(lifecycle_root.join("lifecycle-v2.head.json"), initial_head)
            .expect("test rolls child head back");
        assert!(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy).is_err(),
            "a child valid-prefix rollback cannot satisfy the universe commitment"
        );
    }

    #[test]
    fn lifecycle_v2_append_failures_recover_one_exact_authority_transaction() {
        use crate::grid_handoff_v2::lifecycle_v2::LifecycleAppendFailpointV2;

        for (index, failpoint) in [
            LifecycleAppendFailpointV2::JournalSyncedBeforeHead,
            LifecycleAppendFailpointV2::HeadRenamedBeforeMemory,
        ]
        .into_iter()
        .enumerate()
        {
            let (root, prepared, policy, keys) = fixture();
            let signed = signed_authorization(
                &prepared,
                &policy,
                &keys,
                &[0, 1],
                TRUSTED_NOW - 1_000,
                TRUSTED_NOW + 10_000,
            );
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &signed,
                &ManualClock(TRUSTED_NOW),
            )
            .expect("world activates");
            let cell_key = crate::cell_origin_key();
            let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
                .expect("activated lifecycle opens");
            let before = world
                .cell_assignment(&cell_key)
                .expect("sleeping assignment resolves")
                .clone();
            world
                .set_lifecycle_failpoint_for_test(&cell_key, failpoint)
                .expect("lifecycle failpoint installs");
            let holder = format!("lifecycle-retry-{index}");
            assert!(
                world
                    .dispatch_background_production_with_clock(
                        &cell_key,
                        &holder,
                        &ManualClock(TRUSTED_NOW + 1_000),
                    )
                    .is_err(),
                "injected lifecycle append must surface"
            );
            assert_eq!(
                world
                    .cell_assignment(&cell_key)
                    .expect("directory remains at a defined frontier"),
                &before,
                "the first request append fails before directory mutation"
            );
            drop(world);

            let mut recovered = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
                .expect("lifecycle journal recovers its exact successor");
            let outcome = recovered
                .dispatch_background_production_with_clock(
                    &cell_key,
                    &holder,
                    &ManualClock(TRUSTED_NOW + 1_000),
                )
                .expect("exact retry completes and releases");
            assert_eq!(outcome.mode, "sleeping");
            let after = recovered
                .cell_assignment(&cell_key)
                .expect("recovered release resolves");
            assert_eq!(
                after.assignment_generation,
                before.assignment_generation + 1
            );
            assert_eq!(
                after.authority_fencing_token,
                before.authority_fencing_token + 1
            );
        }
    }

    #[test]
    fn lifecycle_v2_recovers_write_ahead_commit_before_child_materialization() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let cell_key = crate::cell_origin_key();
        let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("universe lifecycle bootstraps");
        let before = world
            .cell_assignment(&cell_key)
            .expect("sleeping assignment resolves")
            .clone();
        world.set_lifecycle_coordinator_failpoint_for_test(
            crate::protocol19_install::Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleWriteAheadCommitted,
        );
        assert!(
            world
                .dispatch_background_production_with_clock(
                    &cell_key,
                    "lifecycle-write-ahead-recovery",
                    &ManualClock(TRUSTED_NOW + 1_000),
                )
                .is_err(),
            "injected gap after write-ahead commit must surface"
        );
        assert_eq!(
            world
                .cell_assignment(&cell_key)
                .expect("directory remains unchanged"),
            &before
        );
        drop(world);

        let mut recovered = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("authorized child append materializes on restart");
        let outcome = recovered
            .dispatch_background_production_with_clock(
                &cell_key,
                "lifecycle-write-ahead-recovery",
                &ManualClock(TRUSTED_NOW + 1_000),
            )
            .expect("exact retry claims and releases once");
        assert_eq!(outcome.mode, "sleeping");
        let after = recovered
            .cell_assignment(&cell_key)
            .expect("released assignment resolves");
        assert_eq!(
            after.assignment_generation,
            before.assignment_generation + 1
        );
        assert_eq!(
            after.authority_fencing_token,
            before.authority_fencing_token + 1
        );
    }

    #[test]
    fn lifecycle_v2_first_bootstrap_crash_boundaries_recover_exact_genesis() {
        use crate::grid_handoff_v2::lifecycle_v2::LifecycleInitializationFailpointV2;

        for failpoint in [
            LifecycleInitializationFailpointV2::EmptyHistoryCreated,
            LifecycleInitializationFailpointV2::EmptyHeadCommitted,
            LifecycleInitializationFailpointV2::InitialJournalSyncedBeforeHead,
            LifecycleInitializationFailpointV2::InitialHeadRenamedBeforeMemory,
        ] {
            let (root, prepared, policy, keys) = fixture();
            let signed = signed_authorization(
                &prepared,
                &policy,
                &keys,
                &[0, 1],
                TRUSTED_NOW - 1_000,
                TRUSTED_NOW + 10_000,
            );
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &signed,
                &ManualClock(TRUSTED_NOW),
            )
            .expect("world activates");
            assert!(
                open_activated_protocol19_world_with_lifecycle_initialization_failpoint(
                    root.path(),
                    TEST_SEED,
                    &policy,
                    failpoint,
                )
                .is_err(),
                "injected {failpoint:?} bootstrap boundary must surface"
            );
            if failpoint == LifecycleInitializationFailpointV2::EmptyHeadCommitted {
                let interrupted_root = fs::read_dir(root.path().join("cells"))
                    .expect("cell roots read")
                    .map(|entry| {
                        entry
                            .expect("cell root reads")
                            .path()
                            .join("protocol-19-world-v21")
                    })
                    .find(|cell_root| cell_root.join("lifecycle-v2.head.json").exists())
                    .expect("interrupted bootstrap cell exists");
                OpenOptions::new()
                    .append(true)
                    .open(interrupted_root.join("lifecycle-v2.ndjson"))
                    .and_then(|mut file| file.write_all(b"{\"unterminated\":"))
                    .expect("test appends an unterminated initial record fragment");
            }

            let mut recovered = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
                .unwrap_or_else(|error| panic!("{failpoint:?} must recover: {error}"));
            let cell_key = crate::cell_origin_key();
            let outcome = recovered
                .dispatch_background_production_with_clock(
                    &cell_key,
                    "initial-lifecycle-recovery",
                    &ManualClock(TRUSTED_NOW + 1_000),
                )
                .expect("recovered immutable genesis claims and releases once");
            assert_eq!(outcome.mode, "sleeping");
            assert_eq!(outcome.committed_quanta, 0);
        }
    }

    #[test]
    fn lifecycle_v2_rejects_resealed_pending_successor_before_child_mutation() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let cell_key = crate::cell_origin_key();
        let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("universe lifecycle bootstraps");
        let cell_id = world
            .cell_assignment(&cell_key)
            .expect("cell resolves")
            .cell_id
            .clone();
        world.set_lifecycle_coordinator_failpoint_for_test(
            crate::protocol19_install::Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleWriteAheadCommitted,
        );
        assert!(
            world
                .dispatch_background_production_with_clock(
                    &cell_key,
                    "resealed-pending-rejection",
                    &ManualClock(TRUSTED_NOW + 1_000),
                )
                .is_err()
        );
        let lifecycle_root = root
            .path()
            .join("cells")
            .join(cell_id)
            .join("protocol-19-world-v21");
        let history_path = lifecycle_root.join("lifecycle-v2.ndjson");
        let head_path = lifecycle_root.join("lifecycle-v2.head.json");
        let history_before = fs::read(&history_path).expect("child history reads");
        let head_before = fs::read(&head_path).expect("child head reads");
        world
            .reseal_pending_lifecycle_outside_state_machine_for_test()
            .expect("test reseals a structurally valid invalid successor");
        drop(world);

        assert!(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy).is_err(),
            "a resealed pending record outside the state machine must fail closed"
        );
        assert_eq!(
            fs::read(history_path).expect("rejected child history reads"),
            history_before
        );
        assert_eq!(
            fs::read(head_path).expect("rejected child head reads"),
            head_before
        );
    }

    #[test]
    fn lifecycle_v2_universe_head_atomic_boundaries_recover_exactly() {
        use crate::protocol19_install::Protocol19LifecycleCoordinatorFailpoint;

        for failpoint in [
            Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleBeginSyncedBeforeRename,
            Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleBeginRenamedBeforeMemory,
            Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleFinishSyncedBeforeRename,
            Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleFinishRenamedBeforeMemory,
        ] {
            let (root, prepared, policy, keys) = fixture();
            let signed = signed_authorization(
                &prepared,
                &policy,
                &keys,
                &[0, 1],
                TRUSTED_NOW - 1_000,
                TRUSTED_NOW + 10_000,
            );
            activate_protocol19_world_with_clock(
                root.path(),
                TEST_SEED,
                &policy,
                &signed,
                &ManualClock(TRUSTED_NOW),
            )
            .expect("world activates");
            let cell_key = crate::cell_origin_key();
            let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
                .expect("universe lifecycle bootstraps");
            let before = world
                .cell_assignment(&cell_key)
                .expect("sleeping assignment resolves")
                .clone();
            world.set_lifecycle_coordinator_failpoint_for_test(failpoint);
            assert!(
                world
                    .dispatch_background_production_with_clock(
                        &cell_key,
                        "universe-head-atomic-recovery",
                        &ManualClock(TRUSTED_NOW + 1_000),
                    )
                    .is_err(),
                "injected {failpoint:?} boundary must surface"
            );
            drop(world);

            let mut recovered = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
                .unwrap_or_else(|error| panic!("{failpoint:?} must recover: {error}"));
            let outcome = recovered
                .dispatch_background_production_with_clock(
                    &cell_key,
                    "universe-head-atomic-recovery",
                    &ManualClock(TRUSTED_NOW + 1_000),
                )
                .expect("recovered universe head completes exactly once");
            assert_eq!(outcome.mode, "sleeping");
            assert_eq!(outcome.committed_quanta, 0);
            let after = recovered
                .cell_assignment(&cell_key)
                .expect("recovered release resolves");
            assert_eq!(
                after.assignment_generation,
                before.assignment_generation + 1
            );
            assert_eq!(
                after.authority_fencing_token,
                before.authority_fencing_token + 1
            );
        }
    }

    #[test]
    fn lifecycle_v2_recovery_truncates_partial_tail_and_removes_bounded_stale_temp() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let cell_key = crate::cell_origin_key();
        let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("activated lifecycle opens");
        world
            .dispatch_background_production_with_clock(
                &cell_key,
                "lifecycle-tail-recovery",
                &ManualClock(TRUSTED_NOW + 1_000),
            )
            .expect("empty cell dispatch establishes a complete lifecycle history");
        let cell_id = world
            .cell_assignment(&cell_key)
            .expect("cell resolves")
            .cell_id
            .clone();
        drop(world);

        let lifecycle_root = root
            .path()
            .join("cells")
            .join(cell_id)
            .join("protocol-19-world-v21");
        let history = lifecycle_root.join("lifecycle-v2.ndjson");
        let committed_len = fs::metadata(&history)
            .expect("history metadata reads")
            .len();
        OpenOptions::new()
            .append(true)
            .open(&history)
            .and_then(|mut file| file.write_all(b"{\"partial\":"))
            .expect("test appends an unterminated lifecycle fragment");
        let stale_temp =
            lifecycle_root.join(".lifecycle-v2.head.json.00000000-0000-4000-8000-000000000000.tmp");
        fs::write(&stale_temp, b"stale").expect("test creates a bounded stale head temporary");

        drop(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
                .expect("bounded recovery restores the committed lifecycle frontier"),
        );
        assert_eq!(
            fs::metadata(history)
                .expect("recovered history metadata reads")
                .len(),
            committed_len
        );
        assert!(!stale_temp.exists());
    }

    #[test]
    fn lifecycle_v2_recovers_directory_commit_before_lifecycle_finalization() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let cell_key = crate::cell_origin_key();
        let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("activated lifecycle opens");
        let before = world
            .cell_assignment(&cell_key)
            .expect("sleeping assignment resolves")
            .clone();
        world.set_lifecycle_coordinator_failpoint_for_test(
            crate::protocol19_install::Protocol19LifecycleCoordinatorFailpoint::DirectoryAuthorityCommitted,
        );
        assert!(
            world
                .dispatch_background_production_with_clock(
                    &cell_key,
                    "lifecycle-directory-gap",
                    &ManualClock(TRUSTED_NOW + 1_000),
                )
                .is_err()
        );
        let assigned = world
            .cell_assignment(&cell_key)
            .expect("directory successor committed")
            .clone();
        assert_eq!(assigned.state, crate::CellAssignmentState::Assigned);
        assert_eq!(
            assigned.assignment_generation,
            before.assignment_generation + 1
        );
        drop(world);

        let mut recovered = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("split authority transaction reopens");
        let outcome = recovered
            .dispatch_background_production_with_clock(
                &cell_key,
                "lifecycle-directory-gap",
                &ManualClock(TRUSTED_NOW + 1_000),
            )
            .expect("exact pending claim finalizes without another generation");
        assert_eq!(outcome.mode, "sleeping");
        let released = recovered
            .cell_assignment(&cell_key)
            .expect("cell releases after recovery");
        assert_eq!(
            released.assignment_generation,
            assigned.assignment_generation
        );
        assert_eq!(
            released.authority_fencing_token,
            assigned.authority_fencing_token
        );
    }

    #[test]
    fn lifecycle_v2_recovers_release_commit_before_sleeping_finalization() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let cell_key = crate::cell_origin_key();
        let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("activated lifecycle opens");
        let before = world
            .cell_assignment(&cell_key)
            .expect("sleeping assignment resolves")
            .clone();
        world.set_lifecycle_coordinator_failpoint_for_test(
            crate::protocol19_install::Protocol19LifecycleCoordinatorFailpoint::DirectoryReleaseCommitted,
        );
        assert!(
            world
                .dispatch_background_production_with_clock(
                    &cell_key,
                    "lifecycle-release-gap",
                    &ManualClock(TRUSTED_NOW + 1_000),
                )
                .is_err()
        );
        let released = world
            .cell_assignment(&cell_key)
            .expect("directory release committed")
            .clone();
        assert_eq!(released.state, crate::CellAssignmentState::Sleeping);
        assert_eq!(
            released.assignment_generation,
            before.assignment_generation + 1
        );
        drop(world);

        let mut recovered = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("split release transaction reopens");
        let outcome = recovered
            .dispatch_background_production_with_clock(
                &cell_key,
                "lifecycle-release-gap",
                &ManualClock(TRUSTED_NOW + 1_000),
            )
            .expect("exact pending release finalizes without reacquiring the cell");
        assert_eq!(outcome.mode, "sleeping");
        assert_eq!(
            recovered
                .cell_assignment(&cell_key)
                .expect("release remains the directory tip"),
            &released
        );
    }

    #[test]
    fn lifecycle_v2_expired_same_holder_recovery_advances_fence_once() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let cell_key = crate::cell_origin_key();
        let mut world = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("activated lifecycle opens");
        world.set_lifecycle_coordinator_failpoint_for_test(
            crate::protocol19_install::Protocol19LifecycleCoordinatorFailpoint::AuthorityFinalized,
        );
        assert!(
            world
                .dispatch_background_production_with_clock(
                    &cell_key,
                    "lifecycle-restarted-holder",
                    &ManualClock(TRUSTED_NOW + 1_000),
                )
                .is_err()
        );
        let assigned = world
            .cell_assignment(&cell_key)
            .expect("initial authority committed")
            .clone();
        assert_eq!(assigned.state, crate::CellAssignmentState::Assigned);
        drop(world);

        let mut recovered = open_activated_protocol19_world(root.path(), TEST_SEED, &policy)
            .expect("assigned lifecycle reopens under the exclusive writer lock");
        let outcome = recovered
            .dispatch_background_production_with_clock(
                &cell_key,
                "lifecycle-restarted-holder",
                &ManualClock(TRUSTED_NOW + 16_001),
            )
            .expect("expired logical authority is recovered even with the same stable holder id");
        assert_eq!(outcome.mode, "sleeping");
        let released = recovered
            .cell_assignment(&cell_key)
            .expect("recovered empty cell releases");
        assert_eq!(
            released.assignment_generation,
            assigned.assignment_generation + 1
        );
        assert_eq!(
            released.authority_fencing_token,
            assigned.authority_fencing_token + 1
        );
    }

    #[test]
    fn lifecycle_v2_all_cell_preflight_precedes_any_runtime_write() {
        let (root, prepared, policy, keys) = fixture();
        let signed = signed_authorization(
            &prepared,
            &policy,
            &keys,
            &[0, 1],
            TRUSTED_NOW - 1_000,
            TRUSTED_NOW + 10_000,
        );
        activate_protocol19_world_with_clock(
            root.path(),
            TEST_SEED,
            &policy,
            &signed,
            &ManualClock(TRUSTED_NOW),
        )
        .expect("world activates");
        let cell_roots = crate::proof_cell_keys()
            .expect("proof cells derive")
            .iter()
            .map(|cell_key| {
                root.path()
                    .join("cells")
                    .join(crate::cell_id(cell_key).expect("cell identity derives"))
                    .join("protocol-19-world-v21")
            })
            .collect::<Vec<_>>();
        let first_before = fs::read_dir(&cell_roots[0])
            .expect("first cell reads")
            .map(|entry| {
                let path = entry.expect("first cell entry reads").path();
                let name = path
                    .file_name()
                    .expect("first cell file has a name")
                    .to_owned();
                (name, fs::read(path).expect("first cell file reads"))
            })
            .collect::<BTreeMap<_, _>>();
        fs::write(cell_roots[1].join("snapshot-v21.json"), b"{}")
            .expect("test corrupts a canonical-named second-cell artifact");

        assert!(
            open_activated_protocol19_world(root.path(), TEST_SEED, &policy).is_err(),
            "a later invalid cell blocks activated recovery"
        );
        let first_after = fs::read_dir(&cell_roots[0])
            .expect("first cell rereads")
            .map(|entry| {
                let path = entry.expect("first cell entry rereads").path();
                let name = path
                    .file_name()
                    .expect("first cell file has a name")
                    .to_owned();
                (name, fs::read(path).expect("first cell file rereads"))
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(first_after, first_before);
        assert!(!cell_roots[0].join("lifecycle-v2.ndjson").exists());
        assert!(!cell_roots[0].join("lifecycle-v2.head.json").exists());
    }
}
