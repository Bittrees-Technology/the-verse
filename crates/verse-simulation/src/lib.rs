// SPDX-License-Identifier: AGPL-3.0-or-later

//! Server-authoritative simulation kernel for The Verse P0 playable proof.

mod celestial;
mod cell_directory;
#[allow(dead_code)]
mod cell_directory_v3;
mod content;
mod engine;
mod event;
#[allow(dead_code)]
mod grid_handoff_v2;
mod handoff;
mod identity;
#[allow(dead_code)]
mod manifest_v5;
mod model;
mod persistence;
mod projection;
#[allow(dead_code)]
mod protocol19_install;
#[allow(dead_code)]
mod protocol19_migration;
#[allow(dead_code)]
mod protocol19_source;
mod targeting;
mod two_cell;

pub use celestial::{
    CelestialError, address_from_origin_offset_um, cell_address_from_key, cell_id,
    cell_key_from_address, cell_origin_key, local_position_from_address, neighbor_cell_key,
    registry_snapshot, universe_manifest, validate_cell_key,
};
pub use cell_directory::{
    AggregatePlacementRecord, AggregatePlacementState, CELL_DIRECTORY_SCHEMA_VERSION,
    CellAssignmentRecord, CellAssignmentState, CellDirectoryError, CellTransferFinalizationProof,
    CellTransferImportProof, CellTransferPrepareProof, CellTransferQuarantineProof,
    CellTransferRecord, LocalCellDirectory, MobileAggregateKind, TransferPhase, proof_cell_keys,
};
pub use content::ContentManifest;
pub use engine::{
    AdvanceImpact, AdvanceOutcome, IntentError, ProductionDispatchOutcome, Runtime, RuntimeError,
    RuntimeOpenConfig,
};
pub use event::{
    CanonicalEvent, EVENT_SCHEMA_VERSION, EventPayload,
    PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION, ProductionMachineOutcome,
    ProductionMachineOutcomeKind, ProductionScheduleOccurrence,
};
pub use handoff::{
    HandoffArtifactError, HandoffError, LocalHandoffArtifactStore, MAX_TRANSFER_ARTIFACT_BYTES,
    PlayerTransferConservation, PlayerTransferContext, PlayerTransferPackage,
    PlayerTransferQuarantineReceipt, TRANSFER_PACKAGE_SCHEMA_VERSION, prepare_eva_player_transfer,
    quarantine_eva_player_transfer, stage_aborted_eva_unlock, stage_committed_eva_export,
    stage_committed_eva_import, stage_eva_player_quarantine, stage_prepared_eva_lock,
};
pub use identity::{SUBJECT_ID_SCHEMA_VERSION, SubjectIdError, canonical_subject_id};
pub use model::{
    ActorOperationHistory, Block, Grid, InventoryRecord, Ledger, Player, PlayerTransferLock,
    PlayerTransferReservation, ProductionClock, TransferConservationWitness,
    TransferWitnessDirection, VoxelField, WORLD_SCHEMA_VERSION, WorldState,
};
pub use persistence::{CellLifecycleStatus, LifecycleMode, PersistenceError, Store, TrustedClock};
pub use projection::{
    InterestEntityIdentity, InterestObserver, InterestProjectionState, ProjectedInterestFrame,
    ProjectionError, ProjectionSource,
};
pub use two_cell::{
    CompletedPlayerHandoff, LocalTwoCellRuntime, ResidentPlayerRoute, TwoCellAdvanceOutcome,
    TwoCellRuntimeError,
};
