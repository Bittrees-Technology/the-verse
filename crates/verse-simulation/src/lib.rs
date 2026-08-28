// SPDX-License-Identifier: AGPL-3.0-or-later

//! Server-authoritative simulation kernel for The Verse P0 playable proof.

mod content;
mod engine;
mod event;
mod model;
mod persistence;

pub use content::ContentManifest;
pub use engine::{IntentError, Runtime, RuntimeError};
pub use event::{CanonicalEvent, EventPayload};
pub use model::{Block, Grid, InventoryRecord, Ledger, Player, VoxelField, WorldState};
pub use persistence::{PersistenceError, Store};
