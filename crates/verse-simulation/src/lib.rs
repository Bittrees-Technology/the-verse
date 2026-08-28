// SPDX-License-Identifier: AGPL-3.0-or-later

//! Server-authoritative simulation kernel for The Verse P0 playable proof.

mod celestial;
mod content;
mod engine;
mod event;
mod model;
mod persistence;
mod projection;
mod targeting;

pub use celestial::{CelestialError, registry_snapshot, universe_manifest};
pub use content::ContentManifest;
pub use engine::{AdvanceImpact, AdvanceOutcome, IntentError, Runtime, RuntimeError};
pub use event::{
    CanonicalEvent, EVENT_SCHEMA_VERSION, EventPayload,
    PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION, ProductionMachineOutcome,
    ProductionMachineOutcomeKind, ProductionScheduleOccurrence,
};
pub use model::{
    Block, Grid, InventoryRecord, Ledger, Player, ProductionClock, VoxelField,
    WORLD_SCHEMA_VERSION, WorldState,
};
pub use persistence::{PersistenceError, Store};
pub use projection::{
    InterestEntityIdentity, InterestProjectionState, ProjectedInterestFrame, ProjectionError,
    ProjectionSource,
};
