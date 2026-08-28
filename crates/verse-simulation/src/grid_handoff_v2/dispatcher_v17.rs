// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant trusted event-17 proof dispatcher.
//!
//! Canonical event bytes are only committed claims. This dispatcher accepts a
//! separately resolved, non-serializable directory-v3 capability, rebinds an
//! operational successor fence without changing gameplay, compares the claim
//! exactly, and applies one event to its exact predecessor. It never invokes a
//! reconciliation transaction during replay.

use super::DraftGridClosureError;
use super::event_v17::{
    DraftCanonicalGridEventV17, DraftGridEventAuthorityLookupV17, DraftGridEventPayloadV17,
    ValidatedDraftGridEventAuthorityV17,
};
use super::production::DraftImportedProductionOccurrenceControlsV2;
use super::state::{
    DraftGridAbortCleanupProofV2, DraftGridActivationProofV2, DraftGridDirectoryAuthorityV2,
    DraftGridExportProofV2, DraftGridFinalizationProofV2, DraftGridImportProofV2,
    DraftGridPrepareProofV2, DraftGridQuarantineProofV2, DraftGridTransferCellStateV2,
    DraftGridTransferQuarantineReceiptV2, DraftImportedProductionReleaseProofV2,
    stage_aborted_grid_cleanup_event_v17, stage_committed_grid_export_event_v17,
    stage_committed_grid_import_event_v17, stage_finalized_grid_source_event_v17,
    stage_grid_quarantine_event_v17, stage_imported_grid_activation_event_v17,
    stage_imported_production_occurrence_event_v17, stage_prepared_grid_event_v17,
};
use crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DraftGridEventApplicationV17 {
    pub(super) next_state: DraftGridTransferCellStateV2,
    pub(super) proof: DraftGridEventProofV17,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum DraftGridEventProofV17 {
    Prepared(DraftGridPrepareProofV2),
    Quarantined {
        receipt: DraftGridTransferQuarantineReceiptV2,
        proof: DraftGridQuarantineProofV2,
    },
    Exported(DraftGridExportProofV2),
    Imported(DraftGridImportProofV2),
    Activated(DraftGridActivationProofV2),
    Finalized(DraftGridFinalizationProofV2),
    Aborted(DraftGridAbortCleanupProofV2),
    Production {
        controls: DraftImportedProductionOccurrenceControlsV2,
        proof: DraftImportedProductionReleaseProofV2,
    },
}

pub(super) fn apply_proven_event_v17(
    state: &DraftGridTransferCellStateV2,
    event: &DraftCanonicalGridEventV17,
    authority: ValidatedDraftGridEventAuthorityV17<'_>,
) -> Result<DraftGridEventApplicationV17, DraftGridClosureError> {
    let rebound = event.rebind_for_state(state, authority)?;
    let context = event.bind_for_state(&rebound, authority)?;
    match (event.payload(), authority) {
        (
            DraftGridEventPayloadV17::GridTransferPrepared { package, .. },
            ValidatedDraftGridEventAuthorityV17::Grid(validated),
        ) => {
            let trusted = DraftGridDirectoryAuthorityV2::from_validated_v3(validated);
            let (next_state, proof) =
                stage_prepared_grid_event_v17(&rebound, package, &trusted, &context)?;
            Ok(DraftGridEventApplicationV17 {
                next_state,
                proof: DraftGridEventProofV17::Prepared(proof),
            })
        }
        (
            DraftGridEventPayloadV17::GridTransferQuarantined { package, .. },
            ValidatedDraftGridEventAuthorityV17::Grid(validated),
        ) => {
            let trusted = DraftGridDirectoryAuthorityV2::from_validated_v3(validated);
            let (next_state, receipt, proof) =
                stage_grid_quarantine_event_v17(&rebound, package, &trusted, &context)?;
            Ok(DraftGridEventApplicationV17 {
                next_state,
                proof: DraftGridEventProofV17::Quarantined { receipt, proof },
            })
        }
        (
            DraftGridEventPayloadV17::GridTransferExported { package, .. },
            ValidatedDraftGridEventAuthorityV17::Grid(validated),
        ) => {
            let trusted = DraftGridDirectoryAuthorityV2::from_validated_v3(validated);
            let (next_state, proof) =
                stage_committed_grid_export_event_v17(&rebound, package, &trusted, &context)?;
            Ok(DraftGridEventApplicationV17 {
                next_state,
                proof: DraftGridEventProofV17::Exported(proof),
            })
        }
        (
            DraftGridEventPayloadV17::GridTransferImported { package, .. },
            ValidatedDraftGridEventAuthorityV17::Grid(validated),
        ) => {
            let trusted = DraftGridDirectoryAuthorityV2::from_validated_v3(validated);
            let (next_state, proof) =
                stage_committed_grid_import_event_v17(&rebound, package, &trusted, &context)?;
            Ok(DraftGridEventApplicationV17 {
                next_state,
                proof: DraftGridEventProofV17::Imported(proof),
            })
        }
        (
            DraftGridEventPayloadV17::GridTransferActivated { package, .. },
            ValidatedDraftGridEventAuthorityV17::Grid(validated),
        ) => {
            let trusted = DraftGridDirectoryAuthorityV2::from_validated_v3(validated);
            let (next_state, proof) =
                stage_imported_grid_activation_event_v17(&rebound, package, &trusted, &context)?;
            Ok(DraftGridEventApplicationV17 {
                next_state,
                proof: DraftGridEventProofV17::Activated(proof),
            })
        }
        (
            DraftGridEventPayloadV17::GridTransferFinalized { package, .. },
            ValidatedDraftGridEventAuthorityV17::Grid(validated),
        ) => {
            let trusted = DraftGridDirectoryAuthorityV2::from_validated_v3(validated);
            let (next_state, proof) =
                stage_finalized_grid_source_event_v17(&rebound, package, &trusted, &context)?;
            Ok(DraftGridEventApplicationV17 {
                next_state,
                proof: DraftGridEventProofV17::Finalized(proof),
            })
        }
        (
            DraftGridEventPayloadV17::GridTransferAborted { package, .. },
            ValidatedDraftGridEventAuthorityV17::Grid(validated),
        ) => {
            let trusted = DraftGridDirectoryAuthorityV2::from_validated_v3(validated);
            let (next_state, proof) =
                stage_aborted_grid_cleanup_event_v17(&rebound, package, &trusted, &context)?;
            Ok(DraftGridEventApplicationV17 {
                next_state,
                proof: DraftGridEventProofV17::Aborted(proof),
            })
        }
        (
            DraftGridEventPayloadV17::ProductionQuantumCommitted { occurrence, .. },
            ValidatedDraftGridEventAuthorityV17::Production(_),
        ) => {
            let (next_state, controls, proof) = stage_imported_production_occurrence_event_v17(
                &rebound,
                occurrence.clone(),
                &context,
            )?;
            Ok(DraftGridEventApplicationV17 {
                next_state,
                proof: DraftGridEventProofV17::Production { controls, proof },
            })
        }
        _ => Err(DraftGridClosureError::Invalid(
            "event-17 dispatcher received the wrong authority capability kind".into(),
        )),
    }
}

pub(super) fn resolve_and_apply_proven_event_v17(
    state: &DraftGridTransferCellStateV2,
    event: &DraftCanonicalGridEventV17,
    directory_history: &DraftCellDirectoryHistoryStoreV3,
) -> Result<DraftGridEventApplicationV17, DraftGridClosureError> {
    match event.authority_lookup() {
        DraftGridEventAuthorityLookupV17::Grid {
            directory_revision,
            directory_document_hash,
            transfer_id,
        } => {
            let authority = directory_history
                .resolve_historical_grid_authority(
                    directory_revision,
                    directory_document_hash,
                    transfer_id,
                )
                .map_err(|source| {
                    DraftGridClosureError::Invalid(format!(
                        "event-17 historical grid authority cannot be resolved: {source}"
                    ))
                })?;
            apply_proven_event_v17(
                state,
                event,
                ValidatedDraftGridEventAuthorityV17::Grid(&authority),
            )
        }
        DraftGridEventAuthorityLookupV17::Production {
            directory_revision,
            directory_document_hash,
            cell_id,
        } => {
            let authority = directory_history
                .resolve_historical_cell_authority(
                    directory_revision,
                    directory_document_hash,
                    cell_id,
                )
                .map_err(|source| {
                    DraftGridClosureError::Invalid(format!(
                        "event-17 historical cell authority cannot be resolved: {source}"
                    ))
                })?;
            apply_proven_event_v17(
                state,
                event,
                ValidatedDraftGridEventAuthorityV17::Production(&authority),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::event_v17::DraftProductionAuthorityClaimV17;
    use super::super::tests::package_v3_directory_fixture;
    use super::*;
    use crate::cell_directory_v3::{
        DraftDirectoryV3AuthorityHarness, DraftDirectoryV3AuthoritySeed,
    };
    use crate::event::{
        PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION, ProductionScheduleOccurrence,
    };
    use crate::model::WorldState;
    use tempfile::tempdir;

    fn authority_harness(
        package: &super::super::DraftGridClosurePackageV2,
    ) -> DraftDirectoryV3AuthorityHarness {
        DraftDirectoryV3AuthorityHarness::new(DraftDirectoryV3AuthoritySeed {
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
        .expect("directory authority harness builds")
    }

    fn destination_state(
        package: &super::super::DraftGridClosurePackageV2,
    ) -> DraftGridTransferCellStateV2 {
        let mut destination = WorldState::genesis_for_cell(801, &package.destination_cell_key)
            .expect("destination world derives");
        destination.fencing_token = package.destination_fencing_token;
        DraftGridTransferCellStateV2::new_with_production_origins(destination, BTreeMap::new())
            .expect("destination state seals")
    }

    fn assert_event_proof(proof: &DraftGridEventProofV17, event: &DraftCanonicalGridEventV17) {
        let (sequence, hash, payload_hash) = match proof {
            DraftGridEventProofV17::Prepared(proof) => (
                proof.event_sequence,
                proof.event_hash.as_str(),
                proof.event_payload_hash.as_str(),
            ),
            DraftGridEventProofV17::Quarantined { proof, .. } => (
                proof.event_sequence,
                proof.event_hash.as_str(),
                proof.event_payload_hash.as_str(),
            ),
            DraftGridEventProofV17::Exported(proof) => (
                proof.event_sequence,
                proof.event_hash.as_str(),
                proof.event_payload_hash.as_str(),
            ),
            DraftGridEventProofV17::Imported(proof) => (
                proof.event_sequence,
                proof.event_hash.as_str(),
                proof.event_payload_hash.as_str(),
            ),
            DraftGridEventProofV17::Activated(proof) => (
                proof.event_sequence,
                proof.event_hash.as_str(),
                proof.event_payload_hash.as_str(),
            ),
            DraftGridEventProofV17::Finalized(proof) => (
                proof.event_sequence,
                proof.event_hash.as_str(),
                proof.event_payload_hash.as_str(),
            ),
            DraftGridEventProofV17::Aborted(proof) => (
                proof.event_sequence,
                proof.event_hash.as_str(),
                proof.event_payload_hash.as_str(),
            ),
            DraftGridEventProofV17::Production { proof, .. } => (
                proof.event_sequence,
                proof.event_hash.as_str(),
                proof.event_payload_hash.as_str(),
            ),
        };
        assert_eq!(sequence, event.event_sequence());
        assert_eq!(hash, event.event_hash());
        assert_eq!(payload_hash, event.event_payload_hash());
    }

    #[test]
    fn trusted_dispatcher_applies_all_event17_variants_once() {
        let (source_world, _, package) = package_v3_directory_fixture();
        let source = DraftGridTransferCellStateV2::new_with_production_origins(
            source_world,
            BTreeMap::new(),
        )
        .expect("source state seals");
        let destination = destination_state(&package);
        let mut directory = authority_harness(&package);
        directory.prepare().expect("directory prepares");

        let prepare_cap = directory.authority().expect("prepare authority resolves");
        let prepare_claim = DraftGridDirectoryAuthorityV2::from_validated_v3(&prepare_cap);
        let prepare_event = DraftCanonicalGridEventV17::new_proven_system(
            &source,
            "trusted-grid-prepare",
            1_800_000_000_000,
            DraftGridEventPayloadV17::GridTransferPrepared {
                package: package.clone(),
                authority: prepare_claim,
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&prepare_cap),
        )
        .expect("trusted prepare event seals");
        let prepare_event = DraftCanonicalGridEventV17::decode_canonical(
            &prepare_event
                .encode_canonical()
                .expect("trusted prepare event encodes"),
        )
        .expect("trusted prepare event reopens canonically");
        let prepared = apply_proven_event_v17(
            &source,
            &prepare_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&prepare_cap),
        )
        .expect("prepare applies");
        assert_event_proof(&prepared.proof, &prepare_event);
        assert!(
            apply_proven_event_v17(
                &prepared.next_state,
                &prepare_event,
                ValidatedDraftGridEventAuthorityV17::Grid(&prepare_cap),
            )
            .is_err(),
            "dispatcher never reconciles an already applied event"
        );
        let DraftGridEventProofV17::Prepared(prepare_proof) = &prepared.proof else {
            panic!("prepare returned the wrong proof kind");
        };
        directory
            .record_prepare(prepare_proof)
            .expect("directory records prepare proof");
        let historical_root = tempdir().expect("temporary history directory");
        let historical_directory = directory
            .persist_history(historical_root.path())
            .expect("complete historical directory persists");
        let replayed_prepare =
            resolve_and_apply_proven_event_v17(&source, &prepare_event, &historical_directory)
                .expect("old event resolves its exact authority after a later revision");
        assert_eq!(replayed_prepare, prepared);

        let quarantine_cap = directory
            .authority()
            .expect("quarantine authority resolves");
        let quarantine_claim = DraftGridDirectoryAuthorityV2::from_validated_v3(&quarantine_cap);
        let quarantine_event = DraftCanonicalGridEventV17::new_proven_system(
            &destination,
            "trusted-grid-quarantine",
            1_800_000_000_000,
            DraftGridEventPayloadV17::GridTransferQuarantined {
                package: package.clone(),
                authority: quarantine_claim,
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&quarantine_cap),
        )
        .expect("trusted quarantine event seals");
        let quarantined = apply_proven_event_v17(
            &destination,
            &quarantine_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&quarantine_cap),
        )
        .expect("quarantine applies");
        assert_event_proof(&quarantined.proof, &quarantine_event);
        let DraftGridEventProofV17::Quarantined {
            proof: quarantine_proof,
            ..
        } = &quarantined.proof
        else {
            panic!("quarantine returned the wrong proof kind");
        };
        directory
            .record_quarantine(quarantine_proof)
            .expect("directory records quarantine proof");
        directory
            .commit_placement()
            .expect("directory placement commits");

        let export_cap = directory.authority().expect("export authority resolves");
        let export_event = DraftCanonicalGridEventV17::new_proven_system(
            &prepared.next_state,
            "trusted-grid-export",
            1_800_000_001_000,
            DraftGridEventPayloadV17::GridTransferExported {
                package: package.clone(),
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(&export_cap),
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&export_cap),
        )
        .expect("trusted export event seals");
        let exported = apply_proven_event_v17(
            &prepared.next_state,
            &export_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&export_cap),
        )
        .expect("export applies");
        assert_event_proof(&exported.proof, &export_event);
        let DraftGridEventProofV17::Exported(export_proof) = &exported.proof else {
            panic!("export returned the wrong proof kind");
        };
        directory
            .record_export(export_proof)
            .expect("directory records export proof");

        let import_cap = directory.authority().expect("import authority resolves");
        let import_event = DraftCanonicalGridEventV17::new_proven_system(
            &quarantined.next_state,
            "trusted-grid-import",
            1_800_000_001_000,
            DraftGridEventPayloadV17::GridTransferImported {
                package: package.clone(),
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(&import_cap),
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&import_cap),
        )
        .expect("trusted import event seals");
        let imported = apply_proven_event_v17(
            &quarantined.next_state,
            &import_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&import_cap),
        )
        .expect("import applies");
        assert_event_proof(&imported.proof, &import_event);
        let DraftGridEventProofV17::Imported(import_proof) = &imported.proof else {
            panic!("import returned the wrong proof kind");
        };
        directory
            .record_import(import_proof)
            .expect("directory records import proof");

        let activation_cap = directory
            .authority()
            .expect("activation authority resolves");
        let activation_event = DraftCanonicalGridEventV17::new_proven_system(
            &imported.next_state,
            "trusted-grid-activation",
            1_800_000_002_000,
            DraftGridEventPayloadV17::GridTransferActivated {
                package: package.clone(),
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(&activation_cap),
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&activation_cap),
        )
        .expect("trusted activation event seals");
        let activated = apply_proven_event_v17(
            &imported.next_state,
            &activation_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&activation_cap),
        )
        .expect("activation applies");
        assert_event_proof(&activated.proof, &activation_event);
        let DraftGridEventProofV17::Activated(activation_proof) = &activated.proof else {
            panic!("activation returned the wrong proof kind");
        };
        directory
            .record_activation(activation_proof)
            .expect("directory records activation proof");

        let finalization_cap = directory
            .authority()
            .expect("finalization authority resolves");
        let finalization_event = DraftCanonicalGridEventV17::new_proven_system(
            &exported.next_state,
            "trusted-grid-finalization",
            1_800_000_003_000,
            DraftGridEventPayloadV17::GridTransferFinalized {
                package: package.clone(),
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(&finalization_cap),
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&finalization_cap),
        )
        .expect("trusted finalization event seals");
        let finalized = apply_proven_event_v17(
            &exported.next_state,
            &finalization_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&finalization_cap),
        )
        .expect("finalization applies");
        assert_event_proof(&finalized.proof, &finalization_event);
        let DraftGridEventProofV17::Finalized(finalization_proof) = &finalized.proof else {
            panic!("finalization returned the wrong proof kind");
        };
        directory
            .record_finalization(finalization_proof)
            .expect("directory records finalization proof");

        let destination_cap = directory
            .cell_authority(&package.destination_cell_id)
            .expect("destination cell authority resolves");
        let base = activated.next_state.base();
        let scheduled_for_unix_ms = base
            .production_clock
            .last_scheduled_for_unix_ms
            .checked_add(1_000)
            .unwrap_or(1_800_000_004_000)
            .max(1_800_000_004_000);
        let occurrence = ProductionScheduleOccurrence {
            schema_version: PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
            universe_id: base.universe_id.clone(),
            cell_id: base.cell_id.clone(),
            lifecycle_generation: base.production_clock.lifecycle_generation,
            production_quantum_sequence: base.production_clock.last_committed_quantum_sequence + 1,
            scheduled_for_unix_ms,
            universe_manifest_hash: base.universe_manifest_hash.clone(),
            celestial_registry_hash: base.celestial_registry_hash.clone(),
        };
        let production_event = DraftCanonicalGridEventV17::new_proven_system(
            &activated.next_state,
            "trusted-production",
            scheduled_for_unix_ms,
            DraftGridEventPayloadV17::ProductionQuantumCommitted {
                occurrence,
                accepted_trusted_at_unix_ms: scheduled_for_unix_ms,
                authority: DraftProductionAuthorityClaimV17::from_validated(&destination_cap),
            },
            ValidatedDraftGridEventAuthorityV17::Production(&destination_cap),
        )
        .expect("trusted production event seals");
        let production_history_root = tempdir().expect("temporary production history");
        let production_history = directory
            .persist_history(production_history_root.path())
            .expect("production authority history persists");
        let produced = resolve_and_apply_proven_event_v17(
            &activated.next_state,
            &production_event,
            &production_history,
        )
        .expect("production applies");
        assert_event_proof(&produced.proof, &production_event);
        let DraftGridEventProofV17::Production { proof, .. } = &produced.proof else {
            panic!("production returned the wrong proof kind");
        };
        assert_eq!(
            proof.authority_directory_revision,
            destination_cap.directory_revision()
        );
        assert_eq!(
            proof.authority_directory_document_hash,
            destination_cap.directory_document_hash()
        );
        assert_eq!(
            proof.authority_assignment_generation,
            destination_cap.assignment_generation()
        );
        assert_eq!(proof.live_fencing_token, destination_cap.fencing_token());

        let mut abort_directory = authority_harness(&package);
        abort_directory.prepare().expect("abort directory prepares");
        let abort_prepare_cap = abort_directory
            .authority()
            .expect("abort prepare authority resolves");
        let abort_prepare_event = DraftCanonicalGridEventV17::new_proven_system(
            &source,
            "trusted-abort-prepare",
            1_800_000_010_000,
            DraftGridEventPayloadV17::GridTransferPrepared {
                package: package.clone(),
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(&abort_prepare_cap),
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&abort_prepare_cap),
        )
        .expect("abort prepare event seals");
        let abort_prepared = apply_proven_event_v17(
            &source,
            &abort_prepare_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&abort_prepare_cap),
        )
        .expect("abort prepare applies");
        let DraftGridEventProofV17::Prepared(abort_prepare_proof) = &abort_prepared.proof else {
            panic!("abort prepare returned the wrong proof kind");
        };
        abort_directory
            .record_prepare(abort_prepare_proof)
            .expect("abort directory records prepare");
        abort_directory
            .request_abort()
            .expect("directory requests abort");
        let abort_cap = abort_directory
            .authority()
            .expect("abort authority resolves");
        let abort_event = DraftCanonicalGridEventV17::new_proven_system(
            &abort_prepared.next_state,
            "trusted-grid-abort",
            1_800_000_011_000,
            DraftGridEventPayloadV17::GridTransferAborted {
                package: package.clone(),
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(&abort_cap),
                side: super::super::state::DraftGridTransferAbortSideV2::Source,
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&abort_cap),
        )
        .expect("trusted abort event seals");
        let aborted = apply_proven_event_v17(
            &abort_prepared.next_state,
            &abort_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&abort_cap),
        )
        .expect("abort applies");
        assert_event_proof(&aborted.proof, &abort_event);

        let mut destination_abort_directory = authority_harness(&package);
        destination_abort_directory
            .prepare()
            .expect("destination-abort directory prepares");
        let destination_abort_prepare_cap = destination_abort_directory
            .authority()
            .expect("destination-abort prepare authority resolves");
        let destination_abort_prepare_event = DraftCanonicalGridEventV17::new_proven_system(
            &source,
            "trusted-destination-abort-prepare",
            1_800_000_012_000,
            DraftGridEventPayloadV17::GridTransferPrepared {
                package: package.clone(),
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(
                    &destination_abort_prepare_cap,
                ),
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&destination_abort_prepare_cap),
        )
        .expect("destination-abort prepare event seals");
        let destination_abort_prepared = apply_proven_event_v17(
            &source,
            &destination_abort_prepare_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&destination_abort_prepare_cap),
        )
        .expect("destination-abort prepare applies");
        let DraftGridEventProofV17::Prepared(destination_abort_prepare_proof) =
            &destination_abort_prepared.proof
        else {
            panic!("destination-abort prepare returned the wrong proof kind");
        };
        destination_abort_directory
            .record_prepare(destination_abort_prepare_proof)
            .expect("destination-abort directory records prepare");

        let destination_abort_quarantine_cap = destination_abort_directory
            .authority()
            .expect("destination-abort quarantine authority resolves");
        let destination_abort_quarantine_event = DraftCanonicalGridEventV17::new_proven_system(
            &destination,
            "trusted-destination-abort-quarantine",
            1_800_000_012_000,
            DraftGridEventPayloadV17::GridTransferQuarantined {
                package: package.clone(),
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(
                    &destination_abort_quarantine_cap,
                ),
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&destination_abort_quarantine_cap),
        )
        .expect("destination-abort quarantine event seals");
        let destination_abort_quarantined = apply_proven_event_v17(
            &destination,
            &destination_abort_quarantine_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&destination_abort_quarantine_cap),
        )
        .expect("destination-abort quarantine applies");
        let DraftGridEventProofV17::Quarantined {
            proof: destination_abort_quarantine_proof,
            ..
        } = &destination_abort_quarantined.proof
        else {
            panic!("destination-abort quarantine returned the wrong proof kind");
        };
        destination_abort_directory
            .record_quarantine(destination_abort_quarantine_proof)
            .expect("destination-abort directory records quarantine");
        destination_abort_directory
            .request_abort()
            .expect("destination-abort directory requests abort");
        let destination_abort_cap = destination_abort_directory
            .authority()
            .expect("destination-abort authority resolves");
        let destination_abort_event = DraftCanonicalGridEventV17::new_proven_system(
            &destination_abort_quarantined.next_state,
            "trusted-destination-grid-abort",
            1_800_000_013_000,
            DraftGridEventPayloadV17::GridTransferAborted {
                package,
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(&destination_abort_cap),
                side: super::super::state::DraftGridTransferAbortSideV2::Destination,
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&destination_abort_cap),
        )
        .expect("trusted destination abort event seals");
        let destination_aborted = apply_proven_event_v17(
            &destination_abort_quarantined.next_state,
            &destination_abort_event,
            ValidatedDraftGridEventAuthorityV17::Grid(&destination_abort_cap),
        )
        .expect("destination abort applies");
        assert_event_proof(&destination_aborted.proof, &destination_abort_event);
    }

    #[test]
    fn trusted_dispatcher_rejects_self_consistent_fabricated_grid_authority() {
        let (source_world, _, package) = package_v3_directory_fixture();
        let source = DraftGridTransferCellStateV2::new_with_production_origins(
            source_world,
            BTreeMap::new(),
        )
        .expect("source state seals");
        let mut directory = authority_harness(&package);
        directory.prepare().expect("directory prepares");
        let capability = directory.authority().expect("authority resolves");
        let mut fabricated = DraftGridDirectoryAuthorityV2::from_validated_v3(&capability);
        fabricated.advance_test_source_authority();
        let mut fabricated_state = source.clone();
        fabricated_state
            .advance_test_fence()
            .expect("fabricated state advances its operational fence");
        let event = DraftCanonicalGridEventV17::new_system(
            &fabricated_state,
            "fabricated-grid-authority",
            1_800_000_020_000,
            DraftGridEventPayloadV17::GridTransferPrepared {
                package,
                authority: fabricated,
            },
        )
        .expect("fabricated event remains self-consistent canonical material");
        let prior = fabricated_state.clone();
        assert!(
            apply_proven_event_v17(
                &fabricated_state,
                &event,
                ValidatedDraftGridEventAuthorityV17::Grid(&capability),
            )
            .is_err()
        );
        assert_eq!(fabricated_state, prior);
    }

    #[test]
    fn trusted_dispatcher_rebinds_successor_fence_without_changing_gameplay() {
        let (source_world, _, package) = package_v3_directory_fixture();
        let source = DraftGridTransferCellStateV2::new_with_production_origins(
            source_world,
            BTreeMap::new(),
        )
        .expect("source state seals");
        let prior_sequence = source.base().event_sequence;
        let prior_hash = source.base().last_event_hash.clone();
        let mut directory = authority_harness(&package);
        directory.prepare().expect("directory prepares");
        directory
            .advance_cell_authority(&package.source_cell_id)
            .expect("directory advances source authority");
        let capability = directory.authority().expect("successor authority resolves");
        assert_eq!(
            capability.live_source_fencing_token(),
            source.base().fencing_token + 1
        );
        let event = DraftCanonicalGridEventV17::new_proven_system(
            &source,
            "successor-fence-prepare",
            1_800_000_030_000,
            DraftGridEventPayloadV17::GridTransferPrepared {
                package,
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(&capability),
            },
            ValidatedDraftGridEventAuthorityV17::Grid(&capability),
        )
        .expect("successor-fenced event seals from the unchanged predecessor");
        let applied = apply_proven_event_v17(
            &source,
            &event,
            ValidatedDraftGridEventAuthorityV17::Grid(&capability),
        )
        .expect("successor-fenced event applies");
        let DraftGridEventProofV17::Prepared(proof) = applied.proof else {
            panic!("successor event returned the wrong proof kind");
        };
        assert_eq!(proof.prior_event_sequence, prior_sequence);
        assert_eq!(proof.prior_event_hash, prior_hash);
        assert_eq!(proof.fencing_token, capability.live_source_fencing_token());
        assert_eq!(applied.next_state.base().event_sequence, prior_sequence + 1);
    }

    #[test]
    fn live_event_sealing_requires_a_current_head_capability() {
        use super::super::event_v17::ValidatedCurrentGridEventAuthorityV17;

        let (source_world, _, package) = package_v3_directory_fixture();
        let source = DraftGridTransferCellStateV2::new_with_production_origins(
            source_world,
            BTreeMap::new(),
        )
        .expect("source state seals");
        let mut directory = authority_harness(&package);
        directory.prepare().expect("directory prepares");
        let root = tempdir().expect("temporary history directory");
        let directory = directory
            .persist_history(root.path())
            .expect("directory history persists");
        let capability = directory
            .current_grid_authority(&package.transfer_id)
            .expect("current grid authority resolves from the locked head");
        let event = DraftCanonicalGridEventV17::new_live_system_for_store(
            &source,
            "store-live-grid-prepare",
            1_800_000_030_500,
            DraftGridEventPayloadV17::GridTransferPrepared {
                package,
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(capability.validated()),
            },
            ValidatedCurrentGridEventAuthorityV17::Grid(&capability),
        )
        .expect("live event seals only through current-head authority");
        let applied = apply_proven_event_v17(
            &source,
            &event,
            ValidatedDraftGridEventAuthorityV17::Grid(capability.validated()),
        )
        .expect("the sealed live event applies exactly once");
        assert_event_proof(&applied.proof, &event);
    }

    #[test]
    fn trusted_dispatcher_rejects_a_predecessor_fence_never_issued_by_the_directory() {
        let (source_world, _, package) = package_v3_directory_fixture();
        let mut source = DraftGridTransferCellStateV2::new_with_production_origins(
            source_world,
            BTreeMap::new(),
        )
        .expect("source state seals");
        let mut directory = authority_harness(&package);
        directory.prepare().expect("directory prepares");
        directory
            .advance_cell_authority(&package.source_cell_id)
            .expect("directory advances source authority");
        let capability = directory.authority().expect("successor authority resolves");
        let unissued_fence = capability
            .source_fencing_history()
            .values()
            .next()
            .copied()
            .expect("fixture has an issued fence")
            .checked_sub(1)
            .expect("fixture first fence has a positive predecessor");
        source
            .replace_test_fence(unissued_fence)
            .expect("foreign-fenced state reseals for the negative test");
        let prior = source.clone();
        assert!(
            DraftCanonicalGridEventV17::new_proven_system(
                &source,
                "unissued-predecessor-fence",
                1_800_000_031_000,
                DraftGridEventPayloadV17::GridTransferPrepared {
                    package,
                    authority: DraftGridDirectoryAuthorityV2::from_validated_v3(&capability),
                },
                ValidatedDraftGridEventAuthorityV17::Grid(&capability),
            )
            .is_err()
        );
        assert_eq!(source, prior);
    }

    #[test]
    fn trusted_dispatcher_rejects_production_generation_cross_pair() {
        let (source_world, _, package) = package_v3_directory_fixture();
        let source = DraftGridTransferCellStateV2::new_with_production_origins(
            source_world,
            BTreeMap::new(),
        )
        .expect("source state seals");
        let mut directory = authority_harness(&package);
        directory.prepare().expect("directory prepares");
        let capability = directory
            .cell_authority(&package.source_cell_id)
            .expect("source cell authority resolves");
        let base = source.base();
        let occurrence = ProductionScheduleOccurrence {
            schema_version: PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
            universe_id: base.universe_id.clone(),
            cell_id: base.cell_id.clone(),
            lifecycle_generation: base.production_clock.lifecycle_generation,
            production_quantum_sequence: base.production_clock.last_committed_quantum_sequence + 1,
            scheduled_for_unix_ms: 1_800_000_040_000,
            universe_manifest_hash: base.universe_manifest_hash.clone(),
            celestial_registry_hash: base.celestial_registry_hash.clone(),
        };
        let mut fabricated = DraftProductionAuthorityClaimV17::from_validated(&capability);
        fabricated.advance_test_assignment_generation();
        let event = DraftCanonicalGridEventV17::new_system(
            &source,
            "fabricated-production-generation",
            occurrence.scheduled_for_unix_ms,
            DraftGridEventPayloadV17::ProductionQuantumCommitted {
                occurrence,
                accepted_trusted_at_unix_ms: 1_800_000_040_000,
                authority: fabricated,
            },
        )
        .expect("cross-paired production claim remains self-consistent event material");
        let prior = source.clone();
        assert!(
            apply_proven_event_v17(
                &source,
                &event,
                ValidatedDraftGridEventAuthorityV17::Production(&capability),
            )
            .is_err()
        );
        assert!(
            apply_proven_event_v17(
                &source,
                &event,
                ValidatedDraftGridEventAuthorityV17::Grid(
                    &directory.authority().expect("grid authority resolves")
                ),
            )
            .is_err(),
            "production cannot consume a grid-transfer capability kind"
        );
        assert_eq!(source, prior);
    }
}
