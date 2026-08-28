// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant canonical event-17 envelope for package-v2 grid handoff.
//!
//! This module is private and cannot be decoded by the active event-16 Store.
//! It establishes the replay seam before a mutually exclusive world-21 Store
//! is allowed to persist protocol-19 state.

use serde::{Deserialize, Serialize};

use super::state::{
    DraftGridDirectoryAuthorityV2, DraftGridTransferAbortSideV2, DraftGridTransferCellStateV2,
    ValidatedDraftGridTransferCellStateV21,
};
use super::{
    DraftGridClosureError, DraftGridClosurePackageV2, DraftGridCompatibilityTupleV19, hash_json,
    valid_blake3_hex, valid_stable_id,
};
use crate::cell_directory_v3::{
    ValidatedCellAuthorityV3, ValidatedCurrentCellAuthorityV3, ValidatedCurrentGridAuthorityV3,
    ValidatedGridTransferAuthorityV3, ValidatedManifestBoundCellAuthorityV3,
    ValidatedManifestBoundGridAuthorityV3,
};
use crate::event::ProductionScheduleOccurrence;

pub(super) const DRAFT_GRID_EVENT_SCHEMA_VERSION: u32 = 17;
const DRAFT_GRID_EVENT_SCHEMA_NAME: &str = "verse.world_event";
const DRAFT_GRID_EVENT_PAYLOAD_HASH_DOMAIN: &[u8] = b"the-verse/grid-event-payload/v17\0";
const DRAFT_GRID_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/world-event/v17\0";
pub(super) const MAX_DRAFT_GRID_EVENT_BYTES: usize = 20 * 1_024 * 1_024;

/// Serialized production authority is an event commitment, never a capability.
/// Replay must resolve and compare the exact non-Serde directory capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftProductionAuthorityClaimV17 {
    directory_revision: u64,
    directory_document_hash: String,
    assignment_generation: u64,
    fencing_token: u64,
}

impl DraftProductionAuthorityClaimV17 {
    pub(super) fn from_validated(authority: &ValidatedCellAuthorityV3) -> Self {
        Self {
            directory_revision: authority.directory_revision(),
            directory_document_hash: authority.directory_document_hash().to_owned(),
            assignment_generation: authority.assignment_generation(),
            fencing_token: authority.fencing_token(),
        }
    }

    #[cfg(test)]
    pub(super) fn for_test(state: &DraftGridTransferCellStateV2) -> Self {
        Self {
            directory_revision: 1,
            directory_document_hash: blake3::hash(state.base().cell_id.as_bytes())
                .to_hex()
                .to_string(),
            assignment_generation: 1,
            fencing_token: state.base().fencing_token,
        }
    }

    fn validate(&self) -> bool {
        self.directory_revision > 0
            && valid_blake3_hex(&self.directory_document_hash)
            && self.assignment_generation > 0
            && self.fencing_token > 0
    }

    pub(super) fn directory_revision(&self) -> u64 {
        self.directory_revision
    }

    pub(super) fn directory_document_hash(&self) -> &str {
        &self.directory_document_hash
    }

    pub(super) fn assignment_generation(&self) -> u64 {
        self.assignment_generation
    }

    pub(super) fn fencing_token(&self) -> u64 {
        self.fencing_token
    }

    #[cfg(test)]
    pub(super) fn advance_test_assignment_generation(&mut self) {
        self.assignment_generation += 1;
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ValidatedDraftGridEventAuthorityV17<'a> {
    Grid(&'a ValidatedGridTransferAuthorityV3),
    Production(&'a ValidatedCellAuthorityV3),
}

/// Live authority can only be minted from the locked directory's current
/// head. The world-21 Store consumes this type; replay uses the separate
/// historical capability above.
#[derive(Debug)]
pub(super) enum ValidatedCurrentGridEventAuthorityV17<'authority, 'store> {
    Grid(&'authority ValidatedCurrentGridAuthorityV3<'store>),
    Production(&'authority ValidatedCurrentCellAuthorityV3<'store>),
}

/// Historical or current directory authority after the same manifest-5
/// capability has bound it. Replay and append use this instead of raw claims.
#[derive(Debug, Clone, Copy)]
pub(super) enum ValidatedManifestBoundGridEventAuthorityV17<'capability, 'authority, 'manifest> {
    Grid(&'capability ValidatedManifestBoundGridAuthorityV3<'authority, 'manifest>),
    Production(&'capability ValidatedManifestBoundCellAuthorityV3<'authority, 'manifest>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DraftGridEventAuthorityLookupV17<'a> {
    Grid {
        directory_revision: u64,
        directory_document_hash: &'a str,
        transfer_id: &'a str,
    },
    Production {
        directory_revision: u64,
        directory_document_hash: &'a str,
        cell_id: &'a str,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum DraftGridEventPayloadV17 {
    GridTransferPrepared {
        package: DraftGridClosurePackageV2,
        authority: DraftGridDirectoryAuthorityV2,
    },
    GridTransferQuarantined {
        package: DraftGridClosurePackageV2,
        authority: DraftGridDirectoryAuthorityV2,
    },
    GridTransferExported {
        package: DraftGridClosurePackageV2,
        authority: DraftGridDirectoryAuthorityV2,
    },
    GridTransferImported {
        package: DraftGridClosurePackageV2,
        authority: DraftGridDirectoryAuthorityV2,
    },
    GridTransferActivated {
        package: DraftGridClosurePackageV2,
        authority: DraftGridDirectoryAuthorityV2,
    },
    GridTransferFinalized {
        package: DraftGridClosurePackageV2,
        authority: DraftGridDirectoryAuthorityV2,
    },
    GridTransferAborted {
        package: DraftGridClosurePackageV2,
        authority: DraftGridDirectoryAuthorityV2,
        side: DraftGridTransferAbortSideV2,
    },
    ProductionQuantumCommitted {
        occurrence: ProductionScheduleOccurrence,
        accepted_trusted_at_unix_ms: u64,
        authority: DraftProductionAuthorityClaimV17,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftCanonicalGridEventV17 {
    schema_name: String,
    schema_version: u32,
    compatibility: DraftGridCompatibilityTupleV19,
    content_manifest_version: String,
    universe_manifest_hash: String,
    celestial_registry_hash: String,
    event_id: String,
    event_sequence: u64,
    occurred_at_unix_ms: u64,
    universe_id: String,
    cell_id: String,
    authority_fencing_token: u64,
    actor_player_id: Option<String>,
    actor_type: String,
    operation_id: Option<String>,
    operation_sequence: Option<u64>,
    intent_fingerprint: Option<String>,
    previous_event_hash: String,
    event_payload_hash: String,
    payload: DraftGridEventPayloadV17,
    event_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ValidatedDraftGridEventContextV17 {
    pub(super) event_sequence: u64,
    pub(super) previous_event_hash: String,
    pub(super) event_hash: String,
    pub(super) event_payload_hash: String,
    pub(super) occurred_at_unix_ms: u64,
    pub(super) authority_fencing_token: u64,
    payload: DraftGridEventPayloadV17,
}

/// Non-Serde capability proving one event-17 record advances one validated
/// world-21 state and shares its complete manifest-5 identity.
#[derive(Debug)]
pub(super) struct ValidatedManifestBoundGridEventV17<'event, 'state, 'manifest, 'authority> {
    event: &'event DraftCanonicalGridEventV17,
    state: &'state DraftGridTransferCellStateV2,
    manifest: &'manifest crate::manifest_v5::ValidatedUniverseManifestV5,
    authority: ValidatedDraftGridEventAuthorityV17<'authority>,
    context: ValidatedDraftGridEventContextV17,
}

impl ValidatedManifestBoundGridEventV17<'_, '_, '_, '_> {
    pub(super) fn event(&self) -> &DraftCanonicalGridEventV17 {
        self.event
    }

    pub(super) fn state(&self) -> &DraftGridTransferCellStateV2 {
        self.state
    }

    pub(super) fn manifest_hash(&self) -> &str {
        self.manifest.manifest_hash()
    }

    pub(super) fn manifest(&self) -> &crate::manifest_v5::ValidatedUniverseManifestV5 {
        self.manifest
    }

    pub(super) fn authority(&self) -> ValidatedDraftGridEventAuthorityV17<'_> {
        self.authority
    }

    pub(super) fn context(&self) -> &ValidatedDraftGridEventContextV17 {
        &self.context
    }
}

impl ValidatedDraftGridEventContextV17 {
    pub(super) fn require_payload(
        &self,
        expected: &DraftGridEventPayloadV17,
    ) -> Result<(), DraftGridClosureError> {
        if &self.payload != expected {
            return Err(DraftGridClosureError::Invalid(
                "validated event-17 context has the wrong operation payload".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn production_authority_claim(&self) -> Option<&DraftProductionAuthorityClaimV17> {
        match &self.payload {
            DraftGridEventPayloadV17::ProductionQuantumCommitted { authority, .. } => {
                Some(authority)
            }
            _ => None,
        }
    }
}

impl DraftGridEventPayloadV17 {
    fn hydrate_spatial_poses(&mut self) -> Result<(), DraftGridClosureError> {
        match self {
            Self::GridTransferPrepared { package, .. }
            | Self::GridTransferQuarantined { package, .. }
            | Self::GridTransferExported { package, .. }
            | Self::GridTransferImported { package, .. }
            | Self::GridTransferActivated { package, .. }
            | Self::GridTransferFinalized { package, .. }
            | Self::GridTransferAborted { package, .. } => package.hydrate_spatial_poses(),
            Self::ProductionQuantumCommitted { .. } => Ok(()),
        }
    }

    fn package_authority(
        &self,
    ) -> Option<(&DraftGridClosurePackageV2, &DraftGridDirectoryAuthorityV2)> {
        match self {
            Self::GridTransferPrepared { package, authority }
            | Self::GridTransferQuarantined { package, authority }
            | Self::GridTransferExported { package, authority }
            | Self::GridTransferImported { package, authority }
            | Self::GridTransferActivated { package, authority }
            | Self::GridTransferFinalized { package, authority }
            | Self::GridTransferAborted {
                package, authority, ..
            } => Some((package, authority)),
            Self::ProductionQuantumCommitted { .. } => None,
        }
    }

    fn expected_cell_and_fence(&self) -> (&str, u64) {
        match self {
            Self::GridTransferPrepared { package, authority }
            | Self::GridTransferExported { package, authority }
            | Self::GridTransferFinalized { package, authority } => {
                (&package.source_cell_id, authority.source_fencing_token())
            }
            Self::GridTransferQuarantined { package, authority }
            | Self::GridTransferImported { package, authority }
            | Self::GridTransferActivated { package, authority } => (
                &package.destination_cell_id,
                authority.destination_fencing_token(),
            ),
            Self::GridTransferAborted {
                package,
                authority,
                side,
            } => match side {
                DraftGridTransferAbortSideV2::Source => {
                    (&package.source_cell_id, authority.source_fencing_token())
                }
                DraftGridTransferAbortSideV2::Destination => (
                    &package.destination_cell_id,
                    authority.destination_fencing_token(),
                ),
            },
            Self::ProductionQuantumCommitted {
                occurrence,
                authority,
                ..
            } => (&occurrence.cell_id, authority.fencing_token),
        }
    }

    fn resolved_cell_authority(
        &self,
        authority: ValidatedDraftGridEventAuthorityV17<'_>,
    ) -> Result<ValidatedCellAuthorityV3, DraftGridClosureError> {
        match (self, authority) {
            (
                DraftGridEventPayloadV17::ProductionQuantumCommitted { .. },
                ValidatedDraftGridEventAuthorityV17::Production(authority),
            ) => Ok(authority.clone()),
            (
                DraftGridEventPayloadV17::GridTransferPrepared { .. }
                | DraftGridEventPayloadV17::GridTransferQuarantined { .. }
                | DraftGridEventPayloadV17::GridTransferExported { .. }
                | DraftGridEventPayloadV17::GridTransferImported { .. }
                | DraftGridEventPayloadV17::GridTransferActivated { .. }
                | DraftGridEventPayloadV17::GridTransferFinalized { .. }
                | DraftGridEventPayloadV17::GridTransferAborted { .. },
                ValidatedDraftGridEventAuthorityV17::Grid(authority),
            ) => {
                let (expected_cell, _) = self.expected_cell_and_fence();
                if expected_cell == authority.source_cell_id() {
                    authority.source_cell_authority().map_err(|source| {
                        DraftGridClosureError::Invalid(format!(
                            "event-17 source cell authority is not live: {source}"
                        ))
                    })
                } else if expected_cell == authority.destination_cell_id() {
                    authority.destination_cell_authority().map_err(|source| {
                        DraftGridClosureError::Invalid(format!(
                            "event-17 destination cell authority is not live: {source}"
                        ))
                    })
                } else {
                    Err(DraftGridClosureError::Invalid(
                        "event-17 grid capability belongs to different cells".into(),
                    ))
                }
            }
            _ => Err(DraftGridClosureError::Invalid(
                "event-17 operation received the wrong authority capability kind".into(),
            )),
        }
    }
}

impl DraftCanonicalGridEventV17 {
    #[cfg(test)]
    pub(super) fn new_system(
        state: &DraftGridTransferCellStateV2,
        event_id: impl Into<String>,
        occurred_at_unix_ms: u64,
        payload: DraftGridEventPayloadV17,
    ) -> Result<Self, DraftGridClosureError> {
        let base = state.base();
        let event_sequence = base.event_sequence.checked_add(1).ok_or_else(|| {
            DraftGridClosureError::Unsupported("draft event-17 sequence exhausted".into())
        })?;
        let mut event = Self {
            schema_name: DRAFT_GRID_EVENT_SCHEMA_NAME.into(),
            schema_version: DRAFT_GRID_EVENT_SCHEMA_VERSION,
            compatibility: DraftGridCompatibilityTupleV19::canonical(),
            content_manifest_version: base.content_manifest_version.clone(),
            universe_manifest_hash: base.universe_manifest_hash.clone(),
            celestial_registry_hash: base.celestial_registry_hash.clone(),
            event_id: event_id.into(),
            event_sequence,
            occurred_at_unix_ms,
            universe_id: base.universe_id.clone(),
            cell_id: base.cell_id.clone(),
            authority_fencing_token: base.fencing_token,
            actor_player_id: None,
            actor_type: "system".into(),
            operation_id: None,
            operation_sequence: None,
            intent_fingerprint: None,
            previous_event_hash: base.last_event_hash.clone(),
            event_payload_hash: String::new(),
            payload,
            event_hash: String::new(),
        };
        event.event_payload_hash = event.calculate_payload_hash()?;
        event.event_hash = event.calculate_hash()?;
        event.validate_for_state(state)?;
        Ok(event)
    }

    #[cfg(test)]
    pub(super) fn new_proven_system(
        state: &DraftGridTransferCellStateV2,
        event_id: impl Into<String>,
        occurred_at_unix_ms: u64,
        payload: DraftGridEventPayloadV17,
        authority: ValidatedDraftGridEventAuthorityV17<'_>,
    ) -> Result<Self, DraftGridClosureError> {
        Self::new_proven_system_with_manifest(
            state,
            None,
            event_id,
            occurred_at_unix_ms,
            payload,
            authority,
        )
    }

    #[cfg(test)]
    fn new_proven_system_with_manifest(
        state: &DraftGridTransferCellStateV2,
        world_v21_manifest: Option<&crate::manifest_v5::ValidatedUniverseManifestV5>,
        event_id: impl Into<String>,
        occurred_at_unix_ms: u64,
        payload: DraftGridEventPayloadV17,
        authority: ValidatedDraftGridEventAuthorityV17<'_>,
    ) -> Result<Self, DraftGridClosureError> {
        let cell_authority = payload.resolved_cell_authority(authority)?;
        let rebound = match world_v21_manifest {
            Some(manifest) => {
                state.rebind_world_v21_validated_cell_authority(manifest, &cell_authority)?
            }
            None => state.rebind_validated_cell_authority(&cell_authority)?,
        };
        let base = rebound.base();
        let event_sequence = base.event_sequence.checked_add(1).ok_or_else(|| {
            DraftGridClosureError::Unsupported("draft event-17 sequence exhausted".into())
        })?;
        let mut event = Self {
            schema_name: DRAFT_GRID_EVENT_SCHEMA_NAME.into(),
            schema_version: DRAFT_GRID_EVENT_SCHEMA_VERSION,
            compatibility: DraftGridCompatibilityTupleV19::canonical(),
            content_manifest_version: base.content_manifest_version.clone(),
            universe_manifest_hash: base.universe_manifest_hash.clone(),
            celestial_registry_hash: base.celestial_registry_hash.clone(),
            event_id: event_id.into(),
            event_sequence,
            occurred_at_unix_ms,
            universe_id: base.universe_id.clone(),
            cell_id: base.cell_id.clone(),
            authority_fencing_token: base.fencing_token,
            actor_player_id: None,
            actor_type: "system".into(),
            operation_id: None,
            operation_sequence: None,
            intent_fingerprint: None,
            previous_event_hash: base.last_event_hash.clone(),
            event_payload_hash: String::new(),
            payload,
            event_hash: String::new(),
        };
        event.event_payload_hash = event.calculate_payload_hash()?;
        event.event_hash = event.calculate_hash()?;
        event.bind_for_state(&rebound, authority)?;
        Ok(event)
    }

    #[cfg(test)]
    pub(super) fn new_live_system_for_store(
        state: &DraftGridTransferCellStateV2,
        event_id: impl Into<String>,
        occurred_at_unix_ms: u64,
        payload: DraftGridEventPayloadV17,
        authority: &ValidatedCurrentGridEventAuthorityV17<'_, '_>,
    ) -> Result<Self, DraftGridClosureError> {
        match authority {
            ValidatedCurrentGridEventAuthorityV17::Grid(authority) => Self::new_proven_system(
                state,
                event_id,
                occurred_at_unix_ms,
                payload,
                ValidatedDraftGridEventAuthorityV17::Grid((*authority).validated()),
            ),
            ValidatedCurrentGridEventAuthorityV17::Production(authority) => {
                Self::new_proven_system(
                    state,
                    event_id,
                    occurred_at_unix_ms,
                    payload,
                    ValidatedDraftGridEventAuthorityV17::Production((*authority).validated()),
                )
            }
        }
    }

    #[cfg(test)]
    pub(super) fn new_live_world_v21_system_for_store(
        state: &DraftGridTransferCellStateV2,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        event_id: impl Into<String>,
        occurred_at_unix_ms: u64,
        payload: DraftGridEventPayloadV17,
        authority: &ValidatedCurrentGridEventAuthorityV17<'_, '_>,
    ) -> Result<Self, DraftGridClosureError> {
        match authority {
            ValidatedCurrentGridEventAuthorityV17::Grid(authority) => {
                Self::new_proven_system_with_manifest(
                    state,
                    Some(manifest),
                    event_id,
                    occurred_at_unix_ms,
                    payload,
                    ValidatedDraftGridEventAuthorityV17::Grid((*authority).validated()),
                )
            }
            ValidatedCurrentGridEventAuthorityV17::Production(authority) => {
                Self::new_proven_system_with_manifest(
                    state,
                    Some(manifest),
                    event_id,
                    occurred_at_unix_ms,
                    payload,
                    ValidatedDraftGridEventAuthorityV17::Production((*authority).validated()),
                )
            }
        }
    }

    fn calculate_payload_hash(&self) -> Result<String, DraftGridClosureError> {
        hash_json(DRAFT_GRID_EVENT_PAYLOAD_HASH_DOMAIN, &self.payload)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.event_hash.clear();
        hash_json(DRAFT_GRID_EVENT_HASH_DOMAIN, &material)
    }

    fn validate_envelope_for_state(
        &self,
        state: &DraftGridTransferCellStateV2,
    ) -> Result<ValidatedDraftGridEventContextV17, DraftGridClosureError> {
        let base = state.base();
        let expected_sequence = base.event_sequence.checked_add(1).ok_or_else(|| {
            DraftGridClosureError::Unsupported("draft event-17 sequence exhausted".into())
        })?;
        let (expected_cell, payload_fence) = self.payload.expected_cell_and_fence();
        if let Some((package, authority)) = self.payload.package_authority() {
            package.validate_wire()?;
            authority.validate_package(package)?;
            if package.universe_id != self.universe_id
                || package.content_manifest_version != self.content_manifest_version
                || package.universe_manifest_hash != self.universe_manifest_hash
                || package.celestial_registry_hash != self.celestial_registry_hash
            {
                return Err(DraftGridClosureError::Invalid(
                    "event-17 package and universe bindings disagree".into(),
                ));
            }
        }
        if self.schema_name != DRAFT_GRID_EVENT_SCHEMA_NAME
            || self.schema_version != DRAFT_GRID_EVENT_SCHEMA_VERSION
            || self.compatibility != DraftGridCompatibilityTupleV19::canonical()
            || !valid_stable_id(&self.event_id)
            || self.event_sequence != expected_sequence
            || self.occurred_at_unix_ms == 0
            || self.universe_id != base.universe_id
            || self.content_manifest_version != base.content_manifest_version
            || self.universe_manifest_hash != base.universe_manifest_hash
            || self.celestial_registry_hash != base.celestial_registry_hash
            || self.cell_id != base.cell_id
            || self.cell_id != expected_cell
            || self.authority_fencing_token != base.fencing_token
            || self.authority_fencing_token != payload_fence
            || self.actor_player_id.is_some()
            || self.actor_type != "system"
            || self.operation_id.is_some()
            || self.operation_sequence.is_some()
            || self.intent_fingerprint.is_some()
            || self.previous_event_hash != base.last_event_hash
            || !valid_blake3_hex(&self.event_payload_hash)
            || self.event_payload_hash != self.calculate_payload_hash()?
            || !valid_blake3_hex(&self.event_hash)
            || self.event_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "canonical draft event-17 envelope is invalid for this cell frontier".into(),
            ));
        }
        if let DraftGridEventPayloadV17::ProductionQuantumCommitted {
            occurrence,
            accepted_trusted_at_unix_ms,
            authority,
        } = &self.payload
            && (occurrence.schema_version
                != crate::event::PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION
                || occurrence.universe_id != base.universe_id
                || occurrence.cell_id != base.cell_id
                || occurrence.lifecycle_generation != base.production_clock.lifecycle_generation
                || occurrence.production_quantum_sequence
                    != base
                        .production_clock
                        .last_committed_quantum_sequence
                        .checked_add(1)
                        .unwrap_or(0)
                || occurrence.scheduled_for_unix_ms == 0
                || occurrence.universe_manifest_hash != base.universe_manifest_hash
                || occurrence.celestial_registry_hash != base.celestial_registry_hash
                || !authority.validate()
                || *accepted_trusted_at_unix_ms < occurrence.scheduled_for_unix_ms
                || self.occurred_at_unix_ms != *accepted_trusted_at_unix_ms)
        {
            return Err(DraftGridClosureError::Invalid(
                "production event-17 occurrence is invalid for this cell frontier".into(),
            ));
        }
        Ok(ValidatedDraftGridEventContextV17 {
            event_sequence: self.event_sequence,
            previous_event_hash: self.previous_event_hash.clone(),
            event_hash: self.event_hash.clone(),
            event_payload_hash: self.event_payload_hash.clone(),
            occurred_at_unix_ms: self.occurred_at_unix_ms,
            authority_fencing_token: self.authority_fencing_token,
            payload: self.payload.clone(),
        })
    }

    pub(super) fn validate_world_v21<'event, 'state, 'manifest, 'authority>(
        &'event self,
        state: &ValidatedDraftGridTransferCellStateV21<'state, 'manifest>,
        authority: ValidatedManifestBoundGridEventAuthorityV17<'_, 'authority, 'manifest>,
    ) -> Result<
        ValidatedManifestBoundGridEventV17<'event, 'state, 'manifest, 'authority>,
        DraftGridClosureError,
    > {
        let manifest = state.manifest();
        let (context, validated_authority) = match authority {
            ValidatedManifestBoundGridEventAuthorityV17::Grid(authority) => {
                if authority.manifest_hash() != manifest.manifest_hash() {
                    return Err(DraftGridClosureError::Invalid(
                        "event-17 grid authority belongs to another manifest".into(),
                    ));
                }
                let validated = authority.authority();
                (
                    self.bind_for_state(
                        state.state(),
                        ValidatedDraftGridEventAuthorityV17::Grid(validated),
                    )?,
                    ValidatedDraftGridEventAuthorityV17::Grid(validated),
                )
            }
            ValidatedManifestBoundGridEventAuthorityV17::Production(authority) => {
                if authority.manifest_hash() != manifest.manifest_hash() {
                    return Err(DraftGridClosureError::Invalid(
                        "event-17 cell authority belongs to another manifest".into(),
                    ));
                }
                let validated = authority.authority();
                (
                    self.bind_for_state(
                        state.state(),
                        ValidatedDraftGridEventAuthorityV17::Production(validated),
                    )?,
                    ValidatedDraftGridEventAuthorityV17::Production(validated),
                )
            }
        };
        let document = manifest.document();
        if self.universe_id != manifest.universe_id()
            || self.universe_manifest_hash != manifest.manifest_hash()
            || self.celestial_registry_hash != document.celestial_registry_hash
            || self.content_manifest_version != document.compatibility.content_manifest_version
            || self.compatibility != document.compatibility
        {
            return Err(DraftGridClosureError::Invalid(
                "event-17 does not match the validated world-21 manifest identity".into(),
            ));
        }
        match &self.payload {
            DraftGridEventPayloadV17::GridTransferPrepared { package, .. }
            | DraftGridEventPayloadV17::GridTransferQuarantined { package, .. }
            | DraftGridEventPayloadV17::GridTransferExported { package, .. }
            | DraftGridEventPayloadV17::GridTransferImported { package, .. }
            | DraftGridEventPayloadV17::GridTransferActivated { package, .. }
            | DraftGridEventPayloadV17::GridTransferFinalized { package, .. }
            | DraftGridEventPayloadV17::GridTransferAborted { package, .. } => {
                package.validate_manifest_v5(manifest)?;
            }
            DraftGridEventPayloadV17::ProductionQuantumCommitted { occurrence, .. } => {
                if occurrence.universe_id != manifest.universe_id()
                    || occurrence.universe_manifest_hash != manifest.manifest_hash()
                    || occurrence.celestial_registry_hash != document.celestial_registry_hash
                {
                    return Err(DraftGridClosureError::Invalid(
                        "event-17 production occurrence does not match manifest 5".into(),
                    ));
                }
            }
        }
        Ok(ValidatedManifestBoundGridEventV17 {
            event: self,
            state: state.state(),
            manifest,
            authority: validated_authority,
            context,
        })
    }

    #[cfg(test)]
    pub(super) fn validate_for_state(
        &self,
        state: &DraftGridTransferCellStateV2,
    ) -> Result<ValidatedDraftGridEventContextV17, DraftGridClosureError> {
        self.validate_envelope_for_state(state)
    }

    pub(super) fn bind_for_state(
        &self,
        state: &DraftGridTransferCellStateV2,
        authority: ValidatedDraftGridEventAuthorityV17<'_>,
    ) -> Result<ValidatedDraftGridEventContextV17, DraftGridClosureError> {
        match (&self.payload, authority) {
            (
                payload @ (DraftGridEventPayloadV17::GridTransferPrepared {
                    authority: claimed,
                    ..
                }
                | DraftGridEventPayloadV17::GridTransferQuarantined {
                    authority: claimed,
                    ..
                }
                | DraftGridEventPayloadV17::GridTransferExported {
                    authority: claimed,
                    ..
                }
                | DraftGridEventPayloadV17::GridTransferImported {
                    authority: claimed,
                    ..
                }
                | DraftGridEventPayloadV17::GridTransferActivated {
                    authority: claimed,
                    ..
                }
                | DraftGridEventPayloadV17::GridTransferFinalized {
                    authority: claimed,
                    ..
                }
                | DraftGridEventPayloadV17::GridTransferAborted {
                    authority: claimed, ..
                }),
                ValidatedDraftGridEventAuthorityV17::Grid(validated),
            ) => {
                let trusted = DraftGridDirectoryAuthorityV2::from_validated_v3(validated);
                let (package, _) = payload
                    .package_authority()
                    .expect("grid payload has package");
                if claimed != &trusted
                    || validated.transfer_id() != package.transfer_id
                    || validated.package_hash() != package.package_hash
                    || validated.universe_id() != self.universe_id
                    || validated.universe_manifest_hash() != self.universe_manifest_hash
                {
                    return Err(DraftGridClosureError::Invalid(
                        "event-17 grid authority claim does not equal the exact historical directory capability"
                            .into(),
                    ));
                }
            }
            (
                DraftGridEventPayloadV17::ProductionQuantumCommitted {
                    occurrence,
                    authority: claimed,
                    ..
                },
                ValidatedDraftGridEventAuthorityV17::Production(validated),
            ) => {
                if claimed != &DraftProductionAuthorityClaimV17::from_validated(validated)
                    || validated.universe_id() != self.universe_id
                    || validated.universe_manifest_hash() != self.universe_manifest_hash
                    || validated.cell_id() != self.cell_id
                    || validated.cell_id() != occurrence.cell_id
                {
                    return Err(DraftGridClosureError::Invalid(
                        "event-17 production authority claim does not equal the exact historical directory capability"
                            .into(),
                    ));
                }
            }
            _ => {
                return Err(DraftGridClosureError::Invalid(
                    "event-17 operation received the wrong authority capability kind".into(),
                ));
            }
        }
        self.validate_envelope_for_state(state)
    }

    pub(super) fn rebind_for_state(
        &self,
        state: &DraftGridTransferCellStateV2,
        authority: ValidatedDraftGridEventAuthorityV17<'_>,
    ) -> Result<DraftGridTransferCellStateV2, DraftGridClosureError> {
        let cell_authority = self.payload.resolved_cell_authority(authority)?;
        state.rebind_validated_cell_authority(&cell_authority)
    }

    pub(super) fn rebind_world_v21_for_state(
        &self,
        state: &DraftGridTransferCellStateV2,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        authority: ValidatedDraftGridEventAuthorityV17<'_>,
    ) -> Result<DraftGridTransferCellStateV2, DraftGridClosureError> {
        let cell_authority = self.payload.resolved_cell_authority(authority)?;
        state.rebind_world_v21_validated_cell_authority(manifest, &cell_authority)
    }

    pub(super) fn payload(&self) -> &DraftGridEventPayloadV17 {
        &self.payload
    }

    pub(super) fn authority_lookup(&self) -> DraftGridEventAuthorityLookupV17<'_> {
        match &self.payload {
            DraftGridEventPayloadV17::GridTransferPrepared { authority, .. }
            | DraftGridEventPayloadV17::GridTransferQuarantined { authority, .. }
            | DraftGridEventPayloadV17::GridTransferExported { authority, .. }
            | DraftGridEventPayloadV17::GridTransferImported { authority, .. }
            | DraftGridEventPayloadV17::GridTransferActivated { authority, .. }
            | DraftGridEventPayloadV17::GridTransferFinalized { authority, .. }
            | DraftGridEventPayloadV17::GridTransferAborted { authority, .. } => {
                DraftGridEventAuthorityLookupV17::Grid {
                    directory_revision: authority.directory_revision(),
                    directory_document_hash: authority.directory_document_hash(),
                    transfer_id: authority.transfer_id(),
                }
            }
            DraftGridEventPayloadV17::ProductionQuantumCommitted {
                authority,
                occurrence,
                ..
            } => DraftGridEventAuthorityLookupV17::Production {
                directory_revision: authority.directory_revision(),
                directory_document_hash: authority.directory_document_hash(),
                cell_id: &occurrence.cell_id,
            },
        }
    }

    pub(super) fn event_sequence(&self) -> u64 {
        self.event_sequence
    }

    pub(super) fn event_hash(&self) -> &str {
        &self.event_hash
    }

    pub(super) fn previous_event_hash(&self) -> &str {
        &self.previous_event_hash
    }

    pub(super) fn cell_id(&self) -> &str {
        &self.cell_id
    }

    pub(super) fn event_payload_hash(&self) -> &str {
        &self.event_payload_hash
    }

    pub(super) fn encode_canonical(&self) -> Result<Vec<u8>, DraftGridClosureError> {
        let bytes = serde_json::to_vec(self).map_err(|source| {
            DraftGridClosureError::Invalid(format!("draft event-17 cannot encode: {source}"))
        })?;
        if bytes.len() > MAX_DRAFT_GRID_EVENT_BYTES {
            return Err(DraftGridClosureError::TooLarge);
        }
        Ok(bytes)
    }

    pub(super) fn decode_canonical(bytes: &[u8]) -> Result<Self, DraftGridClosureError> {
        if bytes.len() > MAX_DRAFT_GRID_EVENT_BYTES {
            return Err(DraftGridClosureError::TooLarge);
        }
        let mut event = serde_json::from_slice::<Self>(bytes).map_err(|source| {
            DraftGridClosureError::Invalid(format!("draft event-17 JSON is invalid: {source}"))
        })?;
        event.payload.hydrate_spatial_poses()?;
        let canonical = event.encode_canonical()?;
        if canonical != bytes {
            return Err(DraftGridClosureError::Invalid(
                "draft event-17 bytes are not canonical".into(),
            ));
        }
        if event.schema_name != DRAFT_GRID_EVENT_SCHEMA_NAME
            || event.schema_version != DRAFT_GRID_EVENT_SCHEMA_VERSION
            || event.compatibility != DraftGridCompatibilityTupleV19::canonical()
            || !valid_stable_id(&event.event_id)
            || event.event_sequence == 0
            || event.occurred_at_unix_ms == 0
            || event.universe_id.trim().is_empty()
            || !valid_blake3_hex(&event.cell_id)
            || event.authority_fencing_token == 0
            || event.actor_player_id.is_some()
            || event.actor_type != "system"
            || event.operation_id.is_some()
            || event.operation_sequence.is_some()
            || event.intent_fingerprint.is_some()
            || (event.event_sequence == 1 && !event.previous_event_hash.is_empty())
            || (event.event_sequence > 1 && !valid_blake3_hex(&event.previous_event_hash))
            || !valid_blake3_hex(&event.universe_manifest_hash)
            || !valid_blake3_hex(&event.celestial_registry_hash)
            || event.event_payload_hash != event.calculate_payload_hash()?
            || event.event_hash != event.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "draft event-17 envelope is not self-authenticating canonical material".into(),
            ));
        }
        if let Some((package, authority)) = event.payload.package_authority() {
            package.validate_wire()?;
            authority.validate_package(package)?;
        }
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::{
        DraftGridDirectoryAuthorityV2, DraftGridTransferCellStateV2, reconcile_prepared_grid_v2,
        stage_prepared_grid_event_v17,
    };
    use super::super::tests::{package_fixture, package_v3_directory_fixture};
    use super::*;
    use crate::cell_directory::TransferPhase;
    use crate::cell_directory_v3::{
        DraftDirectoryV3AuthorityHarness, DraftDirectoryV3AuthoritySeed,
    };
    use crate::event::{CanonicalEvent, EventPayload};

    fn prepared_fixture() -> (
        DraftGridTransferCellStateV2,
        DraftGridClosurePackageV2,
        DraftGridDirectoryAuthorityV2,
    ) {
        let (source, _, package) = package_fixture();
        let state = DraftGridTransferCellStateV2::new_with_production_origins(
            source,
            package.production_job_origins.clone(),
        )
        .expect("source draft state seals");
        let authority =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        (state, package, authority)
    }

    fn prepare_event(
        state: &DraftGridTransferCellStateV2,
        package: &DraftGridClosurePackageV2,
        authority: &DraftGridDirectoryAuthorityV2,
    ) -> DraftCanonicalGridEventV17 {
        DraftCanonicalGridEventV17::new_system(
            state,
            format!("prepare-{}", package.transfer_id),
            1_800_000_000_000,
            DraftGridEventPayloadV17::GridTransferPrepared {
                package: package.clone(),
                authority: authority.clone(),
            },
        )
        .expect("prepare event seals")
    }

    #[test]
    fn event17_requires_the_same_manifest5_state_and_package() {
        let (source, _, mut package) = package_v3_directory_fixture();
        let mut state = DraftGridTransferCellStateV2::new_with_production_origins(
            source,
            package.production_job_origins.clone(),
        )
        .expect("source draft state seals");
        let manifest = crate::manifest_v5::build_validated_manifest_v5(801)
            .expect("manifest-5 capability builds");
        state
            .rebind_test_manifest_v5(&manifest)
            .expect("world-21 state binds manifest 5");
        package.universe_manifest_hash = manifest.manifest_hash().to_owned();
        package.package_hash = package
            .calculate_package_hash()
            .expect("manifest-5 package rehashes");
        let mut directory = DraftDirectoryV3AuthorityHarness::new(DraftDirectoryV3AuthoritySeed {
            universe_id: package.universe_id.clone(),
            universe_manifest_hash: package.universe_manifest_hash.clone(),
            transfer_id: package.transfer_id.clone(),
            root_aggregate_id: package.root_aggregate_id.clone(),
            source_cell_key: package.source_cell_key.clone(),
            destination_cell_key: package.destination_cell_key.clone(),
            source_assignment_generation: package.source_assignment_generation,
            source_fencing_token: package.source_fencing_token,
            destination_assignment_generation: package.destination_assignment_generation,
            destination_fencing_token: package.destination_fencing_token,
            package_schema_version: package.schema_version,
            receipt_schema_version: package.receipt_schema_version,
            closure_root: package.closure_root.clone(),
            conservation_root: package.conservation_root.clone(),
            package_hash: package.package_hash.clone(),
            members: package.members.clone(),
            member_root: package.member_root.clone(),
        })
        .expect("manifest-5 directory harness builds");
        directory.prepare().expect("directory prepares");
        let grid_authority = directory.authority().expect("grid authority resolves");
        let claimed = DraftGridDirectoryAuthorityV2::from_validated_v3(&grid_authority);
        let event = prepare_event(&state, &package, &claimed);
        let state_capability = state
            .validate_world_v21(&manifest)
            .expect("state capability remints");
        let bound_authority = grid_authority
            .bind_manifest_v5(&manifest)
            .expect("directory authority binds manifest 5");
        let validated = event
            .validate_world_v21(
                &state_capability,
                ValidatedManifestBoundGridEventAuthorityV17::Grid(&bound_authority),
            )
            .expect("event, state, package, and manifest bind atomically");
        assert_eq!(validated.event(), &event);
        assert_eq!(validated.state(), &state);
        assert_eq!(validated.manifest_hash(), manifest.manifest_hash());
        assert_eq!(validated.context().event_hash, event.event_hash);

        let mut substituted_registry = event;
        substituted_registry.celestial_registry_hash = "ab".repeat(32);
        substituted_registry.event_hash = substituted_registry
            .calculate_hash()
            .expect("substituted event rehashes");
        assert!(
            substituted_registry
                .validate_world_v21(
                    &state_capability,
                    ValidatedManifestBoundGridEventAuthorityV17::Grid(&bound_authority),
                )
                .is_err()
        );
    }

    #[test]
    fn canonical_round_trip_rehydrates_grid_and_rider_poses() {
        let (state, package, authority) = prepared_fixture();
        assert_ne!(package.grid.position, verse_protocol::Vec3::default());
        let event = prepare_event(&state, &package, &authority);
        let bytes = event.encode_canonical().expect("event encodes");
        let decoded = DraftCanonicalGridEventV17::decode_canonical(&bytes)
            .expect("event decodes with derived poses restored");
        assert_eq!(decoded, event);
        decoded
            .validate_for_state(&state)
            .expect("decoded event validates for its exact predecessor");

        let mut whitespace = vec![b' '];
        whitespace.extend_from_slice(&bytes);
        assert!(DraftCanonicalGridEventV17::decode_canonical(&whitespace).is_err());

        let mut unknown = serde_json::from_slice::<serde_json::Value>(&bytes).expect("JSON parses");
        unknown
            .as_object_mut()
            .expect("event is object")
            .insert("unknown".into(), serde_json::json!(true));
        assert!(
            DraftCanonicalGridEventV17::decode_canonical(
                &serde_json::to_vec(&unknown).expect("tampered JSON encodes")
            )
            .is_err()
        );
    }

    #[test]
    fn compatibility_tuple_and_system_envelope_are_indivisible() {
        let (state, package, authority) = prepared_fixture();
        let event = prepare_event(&state, &package, &authority);

        macro_rules! reject_tuple_change {
            ($field:ident) => {{
                let mut changed = event.clone();
                changed.compatibility.$field += 1;
                changed.event_hash = changed.calculate_hash().expect("changed event rehashes");
                let bytes = changed.encode_canonical().expect("changed event encodes");
                assert!(DraftCanonicalGridEventV17::decode_canonical(&bytes).is_err());
            }};
        }
        reject_tuple_change!(protocol_version);
        reject_tuple_change!(projection_schema_version);
        reject_tuple_change!(world_schema_version);
        reject_tuple_change!(event_schema_version);
        reject_tuple_change!(content_schema_version);
        reject_tuple_change!(celestial_registry_schema_version);
        reject_tuple_change!(universe_manifest_schema_version);
        reject_tuple_change!(interest_schema_version);
        reject_tuple_change!(operation_fingerprint_schema_version);
        reject_tuple_change!(lifecycle_control_schema_version);
        reject_tuple_change!(production_occurrence_schema_version);
        reject_tuple_change!(cell_key_schema_version);
        reject_tuple_change!(directory_schema_version);
        reject_tuple_change!(transfer_package_schema_version);

        let mut changed_content = event.clone();
        changed_content
            .compatibility
            .content_manifest_version
            .push_str("-substituted");
        changed_content.event_hash = changed_content
            .calculate_hash()
            .expect("changed event rehashes");
        let changed_content_bytes = changed_content
            .encode_canonical()
            .expect("changed event encodes");
        assert!(DraftCanonicalGridEventV17::decode_canonical(&changed_content_bytes).is_err());

        let mut human = event.clone();
        human.actor_player_id = Some("player-local".into());
        human.actor_type = "human".into();
        human.event_hash = human.calculate_hash().expect("changed actor rehashes");
        assert!(human.validate_for_state(&state).is_err());

        let mut wrong_root = event;
        wrong_root.universe_manifest_hash = "ab".repeat(32);
        wrong_root.event_hash = wrong_root.calculate_hash().expect("changed root rehashes");
        assert!(wrong_root.validate_for_state(&state).is_err());
    }

    #[test]
    fn event_versions_and_operation_kinds_never_cross_decode_or_apply() {
        let (state, package, authority) = prepared_fixture();
        let event = prepare_event(&state, &package, &authority);
        let bytes = event.encode_canonical().expect("event encodes");
        assert!(serde_json::from_slice::<CanonicalEvent>(&bytes).is_err());

        let active = CanonicalEvent::new(
            1,
            state.base().content_manifest_version.clone(),
            state.base().universe_manifest_hash.clone(),
            state.base().celestial_registry_hash.clone(),
            state.base().universe_id.clone(),
            state.base().cell_id.clone(),
            state.base().fencing_token,
            None,
            "system",
            None,
            None,
            None,
            "",
            EventPayload::SuitModeChanged {
                helmet_closed: true,
                jetpack_enabled: false,
                magnetic_boots_enabled: true,
            },
        );
        assert!(
            DraftCanonicalGridEventV17::decode_canonical(
                &serde_json::to_vec(&active).expect("active event encodes")
            )
            .is_err()
        );

        let wrong_kind = DraftCanonicalGridEventV17::new_system(
            &state,
            format!("export-{}", package.transfer_id),
            1_800_000_000_000,
            DraftGridEventPayloadV17::GridTransferExported {
                package: package.clone(),
                authority: authority.clone(),
            },
        )
        .expect("same-cell wrong-kind event is canonical");
        let context = wrong_kind
            .validate_for_state(&state)
            .expect("wrong-kind envelope validates independently");
        let prior = state.clone();
        assert!(
            stage_prepared_grid_event_v17(&state, &package, &authority, &context, None).is_err()
        );
        assert_eq!(state, prior);
    }

    #[test]
    fn prepare_apply_and_successor_reconciliation_are_distinct() {
        let (state, package, authority) = prepared_fixture();
        let event = prepare_event(&state, &package, &authority);
        let context = event
            .validate_for_state(&state)
            .expect("prepare context validates");
        let (prepared, proof) =
            stage_prepared_grid_event_v17(&state, &package, &authority, &context, None)
                .expect("prepare applies once");
        assert_eq!(
            prepared.base().event_sequence,
            state.base().event_sequence + 1
        );
        assert_eq!(prepared.base().last_event_hash, event.event_hash);

        let (retry, retry_proof) = reconcile_prepared_grid_v2(&prepared, &package, &authority)
            .expect("committed prepare reconciles without a second event");
        assert_eq!(retry, prepared);
        assert_eq!(retry_proof, proof);

        let mut successor_state = prepared.clone();
        successor_state
            .advance_test_fence()
            .expect("successor reseals same gameplay");
        let mut successor_authority = authority;
        successor_authority.advance_test_source_authority();
        let (recovered, recovered_proof) =
            reconcile_prepared_grid_v2(&successor_state, &package, &successor_authority)
                .expect("successor reconciles the original proof");
        assert_eq!(recovered, successor_state);
        assert_eq!(recovered_proof, proof);
    }
}
