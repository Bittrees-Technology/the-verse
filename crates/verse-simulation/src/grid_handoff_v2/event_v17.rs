// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant canonical event-17 envelope for package-v2 grid handoff.
//!
//! This module is private and cannot be decoded by the active event-16 Store.
//! It establishes the replay seam before a mutually exclusive world-21 Store
//! is allowed to persist protocol-19 state.

use serde::{Deserialize, Serialize};

use super::state::{
    DraftGridDirectoryAuthorityV2, DraftGridTransferAbortSideV2, DraftGridTransferCellStateV2,
};
use super::{
    DRAFT_GRID_TRANSFER_PACKAGE_SCHEMA_VERSION, DraftGridClosureError, DraftGridClosurePackageV2,
    hash_json, valid_blake3_hex, valid_stable_id,
};
use crate::event::ProductionScheduleOccurrence;

pub(super) const DRAFT_GRID_EVENT_SCHEMA_VERSION: u32 = 17;
const DRAFT_GRID_EVENT_SCHEMA_NAME: &str = "verse.world_event";
const DRAFT_GRID_PROTOCOL_VERSION: u32 = 19;
const DRAFT_GRID_WORLD_SCHEMA_VERSION: u32 = 21;
const DRAFT_GRID_UNIVERSE_MANIFEST_SCHEMA_VERSION: u32 = 5;
const DRAFT_GRID_DIRECTORY_SCHEMA_VERSION: u32 = 3;
const DRAFT_GRID_EVENT_PAYLOAD_HASH_DOMAIN: &[u8] = b"the-verse/grid-event-payload/v17\0";
const DRAFT_GRID_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/world-event/v17\0";
const MAX_DRAFT_GRID_EVENT_BYTES: usize = 20 * 1_024 * 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridCompatibilityTupleV17 {
    protocol_version: u32,
    projection_schema_version: u32,
    world_schema_version: u32,
    event_schema_version: u32,
    content_schema_version: u32,
    celestial_registry_schema_version: u32,
    universe_manifest_schema_version: u32,
    interest_schema_version: u32,
    operation_fingerprint_schema_version: u32,
    lifecycle_control_schema_version: u32,
    production_occurrence_schema_version: u32,
    cell_key_schema_version: u32,
    directory_schema_version: u32,
    transfer_package_schema_version: u32,
}

impl DraftGridCompatibilityTupleV17 {
    const fn canonical() -> Self {
        Self {
            protocol_version: DRAFT_GRID_PROTOCOL_VERSION,
            projection_schema_version: 5,
            world_schema_version: DRAFT_GRID_WORLD_SCHEMA_VERSION,
            event_schema_version: DRAFT_GRID_EVENT_SCHEMA_VERSION,
            content_schema_version: 11,
            celestial_registry_schema_version: 1,
            universe_manifest_schema_version: DRAFT_GRID_UNIVERSE_MANIFEST_SCHEMA_VERSION,
            interest_schema_version: 3,
            operation_fingerprint_schema_version: 2,
            lifecycle_control_schema_version: 2,
            production_occurrence_schema_version: 1,
            cell_key_schema_version: 1,
            directory_schema_version: DRAFT_GRID_DIRECTORY_SCHEMA_VERSION,
            transfer_package_schema_version: DRAFT_GRID_TRANSFER_PACKAGE_SCHEMA_VERSION,
        }
    }
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
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftCanonicalGridEventV17 {
    schema_name: String,
    schema_version: u32,
    compatibility: DraftGridCompatibilityTupleV17,
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
            Self::ProductionQuantumCommitted { occurrence, .. } => (&occurrence.cell_id, 0),
        }
    }
}

impl DraftCanonicalGridEventV17 {
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
            compatibility: DraftGridCompatibilityTupleV17::canonical(),
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

    fn calculate_payload_hash(&self) -> Result<String, DraftGridClosureError> {
        hash_json(DRAFT_GRID_EVENT_PAYLOAD_HASH_DOMAIN, &self.payload)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.event_hash.clear();
        hash_json(DRAFT_GRID_EVENT_HASH_DOMAIN, &material)
    }

    pub(super) fn validate_for_state(
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
        let effective_payload_fence = if payload_fence == 0 {
            self.authority_fencing_token
        } else {
            payload_fence
        };
        if self.schema_name != DRAFT_GRID_EVENT_SCHEMA_NAME
            || self.schema_version != DRAFT_GRID_EVENT_SCHEMA_VERSION
            || self.compatibility != DraftGridCompatibilityTupleV17::canonical()
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
            || self.authority_fencing_token != effective_payload_fence
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

    fn encode_canonical(&self) -> Result<Vec<u8>, DraftGridClosureError> {
        let bytes = serde_json::to_vec(self).map_err(|source| {
            DraftGridClosureError::Invalid(format!("draft event-17 cannot encode: {source}"))
        })?;
        if bytes.len() > MAX_DRAFT_GRID_EVENT_BYTES {
            return Err(DraftGridClosureError::TooLarge);
        }
        Ok(bytes)
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, DraftGridClosureError> {
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
            || event.compatibility != DraftGridCompatibilityTupleV17::canonical()
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
    use super::super::tests::package_fixture;
    use super::*;
    use crate::cell_directory::TransferPhase;
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
    fn canonical_round_trip_rehydrates_grid_and_rider_poses() {
        let (state, package, authority) = prepared_fixture();
        assert_ne!(package.grid.position, Default::default());
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
        assert!(stage_prepared_grid_event_v17(&state, &package, &authority, &context).is_err());
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
            stage_prepared_grid_event_v17(&state, &package, &authority, &context)
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
