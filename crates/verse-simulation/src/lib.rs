// SPDX-License-Identifier: AGPL-3.0-or-later

//! Server-authoritative simulation kernel for The Verse P0 playable proof.

mod celestial;
mod cell_directory;
mod content;
mod engine;
mod event;
mod handoff;
mod model;
mod persistence;
mod projection;
mod targeting;

pub use celestial::{
    CelestialError, cell_address_from_key, cell_id, cell_key_from_address, cell_origin_key,
    neighbor_cell_key, registry_snapshot, universe_manifest, validate_cell_key,
};
pub use cell_directory::{
    AggregatePlacementRecord, AggregatePlacementState, CELL_DIRECTORY_SCHEMA_VERSION,
    CellAssignmentRecord, CellAssignmentState, CellDirectoryError, CellTransferRecord,
    LocalCellDirectory, MobileAggregateKind, TransferPhase, proof_cell_keys,
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
pub use model::{
    ActorOperationHistory, Block, Grid, InventoryRecord, Ledger, Player, PlayerTransferLock,
    PlayerTransferReservation, ProductionClock, TransferConservationWitness,
    TransferWitnessDirection, VoxelField, WORLD_SCHEMA_VERSION, WorldState,
};
pub use persistence::{CellLifecycleStatus, LifecycleMode, PersistenceError, Store, TrustedClock};
pub use projection::{
    InterestEntityIdentity, InterestProjectionState, ProjectedInterestFrame, ProjectionError,
    ProjectionSource,
};
