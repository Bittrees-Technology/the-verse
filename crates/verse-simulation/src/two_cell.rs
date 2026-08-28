// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bounded local two-cell coordinator for the P1.7 correctness proof.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;
use uuid::Uuid;
use verse_protocol::{CellKeyV1, ClientMessage, IntentReceipt};

use crate::cell_directory::{
    AggregatePlacementState, CellAssignmentState, CellDirectoryError, LocalCellDirectory,
    MobileAggregateKind, TransferPhase,
};
use crate::engine::{AdvanceImpact, Runtime, RuntimeError};
use crate::handoff::{
    HandoffArtifactError, HandoffError, LocalHandoffArtifactStore, PlayerTransferContext,
    PlayerTransferPackage, prepare_eva_player_transfer, stage_eva_player_quarantine,
};
use crate::model::{TransferConservationWitness, TransferWitnessDirection, WorldState};
use crate::{CellLifecycleStatus, EVENT_SCHEMA_VERSION, WORLD_SCHEMA_VERSION, celestial};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedPlayerHandoff {
    pub player_id: String,
    pub transfer_id: String,
    pub source_cell_key: CellKeyV1,
    pub destination_cell_key: CellKeyV1,
    pub placement_generation: u64,
    pub destination_movement_epoch: u64,
}

/// A session-scoped compare-and-swap permit for one resident player route.
///
/// The gateway retains this value until the player acknowledges a transfer-
/// linked destination baseline. A route cannot silently follow the directory
/// across a handoff and expose or mutate the destination ahead of that ACK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidentPlayerRoute {
    pub cell_key: CellKeyV1,
    pub placement_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoCellAdvanceOutcome {
    pub impact: AdvanceImpact,
    pub handoffs: Vec<CompletedPlayerHandoff>,
}

impl TwoCellAdvanceOutcome {
    #[must_use]
    pub const fn changed(&self) -> bool {
        !matches!(self.impact, AdvanceImpact::None)
    }
}

#[derive(Debug, Error)]
pub enum TwoCellRuntimeError {
    #[error(transparent)]
    Directory(#[from] CellDirectoryError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Handoff(#[from] HandoffError),
    #[error(transparent)]
    Artifact(#[from] HandoffArtifactError),
    #[error("player {player_id} route is stale")]
    StalePlayerRoute {
        player_id: String,
        requested: Box<ResidentPlayerRoute>,
        current: Box<ResidentPlayerRoute>,
    },
    #[error("two-cell coordinator invariant failed: {0}")]
    Invalid(String),
}

#[derive(Debug)]
struct CellSlot {
    key: CellKeyV1,
    runtime: Runtime,
}

/// One local coordinator, two independently fenced cell stores, and one
/// durable placement directory. This is deliberately bounded to the accepted
/// adjacent proof-cell contract and is not a general distributed scheduler.
#[derive(Debug)]
pub struct LocalTwoCellRuntime {
    root: PathBuf,
    directory: LocalCellDirectory,
    artifacts: LocalHandoffArtifactStore,
    cells: Vec<CellSlot>,
    halted: bool,
}

impl LocalTwoCellRuntime {
    pub fn open(
        root: impl AsRef<Path>,
        world_seed: u64,
        snapshot_every: u64,
        holder_id: &str,
    ) -> Result<Self, TwoCellRuntimeError> {
        if holder_id.is_empty() || holder_id.len() > 96 {
            return Err(TwoCellRuntimeError::Invalid(
                "holder ID must contain 1..=96 bytes".into(),
            ));
        }
        let root = root.as_ref().to_path_buf();
        let manifest =
            celestial::universe_manifest(world_seed, WORLD_SCHEMA_VERSION, EVENT_SCHEMA_VERSION)
                .map_err(|source| TwoCellRuntimeError::Invalid(source.to_string()))?;
        let proof_cells = crate::proof_cell_keys()?;
        let mut directory = LocalCellDirectory::open(&root, &manifest, proof_cells.clone())?;
        let mut cells = Vec::with_capacity(2);
        for key in proof_cells {
            let current = directory.assignment(&key)?.clone();
            let cell_holder = format!("{holder_id}:{}", &current.cell_id[..16]);
            let cell_root = directory.cell_store_root(&key)?;
            // Acquire the independently fenced cell writer before advancing the
            // directory assignment. A failed Store acquisition therefore cannot
            // strand a successor generation while the old writer remains live.
            let runtime = Runtime::open_directory_managed_for_cell(
                cell_root,
                world_seed,
                key.clone(),
                snapshot_every,
            )?;
            let authority_fencing_token = runtime.state().fencing_token;
            let assignment = match current.state {
                CellAssignmentState::Sleeping => directory.claim(
                    &key,
                    current.assignment_generation,
                    &cell_holder,
                    authority_fencing_token,
                )?,
                CellAssignmentState::Assigned => directory.recover_assignment(
                    &key,
                    current.assignment_generation,
                    &cell_holder,
                    authority_fencing_token,
                )?,
                _ => {
                    return Err(TwoCellRuntimeError::Invalid(format!(
                        "proof cell {} retained an incomplete assignment transition",
                        current.cell_id
                    )));
                }
            };
            if assignment.assignment_generation == 0 {
                return Err(TwoCellRuntimeError::Invalid(
                    "claimed cell has no assignment generation".into(),
                ));
            }
            cells.push(CellSlot { key, runtime });
        }
        let artifacts = LocalHandoffArtifactStore::open(root.join("handoff-artifacts"))?;
        let mut coordinator = Self {
            root,
            directory,
            artifacts,
            cells,
            halted: false,
        };
        coordinator.reconcile_transfers()?;
        coordinator.register_resident_players()?;
        Ok(coordinator)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn directory(&self) -> &LocalCellDirectory {
        &self.directory
    }

    pub fn runtime_for_cell(&self, key: &CellKeyV1) -> Option<&Runtime> {
        self.cells
            .iter()
            .find(|cell| &cell.key == key)
            .map(|cell| &cell.runtime)
    }

    pub fn public_origin_runtime(&self) -> &Runtime {
        self.runtime_for_cell(&crate::cell_origin_key())
            .expect("the accepted proof topology always hosts the origin cell")
    }

    pub fn runtime_for_player(&self, player_id: &str) -> Result<&Runtime, TwoCellRuntimeError> {
        let route = self.resident_player_route(player_id)?;
        self.runtime_for_player_at(player_id, &route)
    }

    pub fn resident_player_route(
        &self,
        player_id: &str,
    ) -> Result<ResidentPlayerRoute, TwoCellRuntimeError> {
        let placement = self.directory.placement(player_id)?;
        if placement.aggregate_kind != MobileAggregateKind::Player
            || placement.state != AggregatePlacementState::Resident
            || placement.active_transfer_id.is_some()
        {
            return Err(TwoCellRuntimeError::Invalid(
                "player does not have one resident directory placement".into(),
            ));
        }
        let route = ResidentPlayerRoute {
            cell_key: placement.cell_key.clone(),
            placement_generation: placement.placement_generation,
        };
        let runtime = self
            .runtime_for_cell(&placement.cell_key)
            .ok_or_else(|| TwoCellRuntimeError::Invalid("resident cell is not hosted".into()))?;
        if runtime.state().player.get(player_id).is_none() {
            return Err(TwoCellRuntimeError::Invalid(
                "resident directory placement has no live player".into(),
            ));
        }
        Ok(route)
    }

    pub fn runtime_for_player_at(
        &self,
        player_id: &str,
        requested: &ResidentPlayerRoute,
    ) -> Result<&Runtime, TwoCellRuntimeError> {
        let current = self.resident_player_route(player_id)?;
        if &current != requested {
            return Err(TwoCellRuntimeError::StalePlayerRoute {
                player_id: player_id.to_owned(),
                requested: Box::new(requested.clone()),
                current: Box::new(current),
            });
        }
        self.runtime_for_cell(&requested.cell_key)
            .ok_or_else(|| TwoCellRuntimeError::Invalid("resident cell is not hosted".into()))
    }

    pub fn lifecycle_status(&self) -> CellLifecycleStatus {
        self.public_origin_runtime().lifecycle_status()
    }

    pub fn persist_snapshots(&mut self) -> Result<(), TwoCellRuntimeError> {
        let result = self
            .cells
            .iter_mut()
            .try_for_each(|cell| cell.runtime.persist_snapshot().map_err(Into::into));
        if result.is_err() {
            self.halted = true;
        }
        result
    }

    pub fn renew_leases(&mut self) -> Result<(), TwoCellRuntimeError> {
        let result = self
            .cells
            .iter_mut()
            .try_for_each(|cell| cell.runtime.renew_lease().map_err(Into::into));
        if result.is_err() {
            self.halted = true;
        }
        result
    }

    #[must_use]
    pub fn is_halted(&self) -> bool {
        self.halted || self.cells.iter().any(|cell| cell.runtime.is_halted())
    }

    pub fn execute_as_at(
        &mut self,
        player_id: &str,
        requested: &ResidentPlayerRoute,
        message: &ClientMessage,
    ) -> Result<IntentReceipt, TwoCellRuntimeError> {
        let result = self.execute_as_at_inner(player_id, requested, message);
        if result.as_ref().is_err_and(two_cell_error_is_fatal) {
            self.halted = true;
        }
        result
    }

    fn execute_as_at_inner(
        &mut self,
        player_id: &str,
        requested: &ResidentPlayerRoute,
        message: &ClientMessage,
    ) -> Result<IntentReceipt, TwoCellRuntimeError> {
        let current = self.resident_player_route(player_id)?;
        if &current != requested {
            return Err(TwoCellRuntimeError::StalePlayerRoute {
                player_id: player_id.to_owned(),
                requested: Box::new(requested.clone()),
                current: Box::new(current),
            });
        }
        let index = self
            .cell_index(&requested.cell_key)
            .ok_or_else(|| TwoCellRuntimeError::Invalid("resident cell is not hosted".into()))?;
        Ok(self.cells[index].runtime.execute_as(player_id, message)?)
    }

    pub fn advance_with_outcome(
        &mut self,
        delta_millis: u16,
    ) -> Result<TwoCellAdvanceOutcome, TwoCellRuntimeError> {
        let result = self.advance_with_outcome_inner(delta_millis);
        if result.is_err() {
            self.halted = true;
        }
        result
    }

    fn advance_with_outcome_inner(
        &mut self,
        delta_millis: u16,
    ) -> Result<TwoCellAdvanceOutcome, TwoCellRuntimeError> {
        let hosted_cells = self
            .cells
            .iter()
            .map(|cell| cell.key.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let mut impact = AdvanceImpact::None;
        for cell in &mut self.cells {
            impact = impact.combine(
                cell.runtime
                    .advance_with_outcome_in_cells(delta_millis, &hosted_cells)?
                    .impact,
            );
        }

        let mut crossings = Vec::new();
        for cell in &self.cells {
            for (player_id, player) in cell.runtime.state().player.iter() {
                let placement = self.directory.placement(player_id)?;
                let addressed_cell = celestial::cell_key_from_address(&player.address)
                    .map_err(|source| TwoCellRuntimeError::Invalid(source.to_string()))?;
                if placement.state == AggregatePlacementState::Resident
                    && placement.cell_key == cell.key
                    && addressed_cell != placement.cell_key
                {
                    crossings.push(player_id.clone());
                }
            }
        }
        crossings.sort();
        crossings.dedup();
        let mut handoffs = Vec::with_capacity(crossings.len());
        for player_id in crossings {
            handoffs.push(self.handoff_player(&player_id)?);
            impact = AdvanceImpact::Structural;
        }
        Ok(TwoCellAdvanceOutcome { impact, handoffs })
    }

    pub fn handoff_player(
        &mut self,
        player_id: &str,
    ) -> Result<CompletedPlayerHandoff, TwoCellRuntimeError> {
        let placement = self.directory.placement(player_id)?.clone();
        if placement.aggregate_kind != MobileAggregateKind::Player
            || placement.state != AggregatePlacementState::Resident
            || placement.active_transfer_id.is_some()
        {
            return Err(TwoCellRuntimeError::Invalid(
                "player is not resident at a transferable placement".into(),
            ));
        }
        let source_index = self
            .cell_index(&placement.cell_key)
            .ok_or_else(|| TwoCellRuntimeError::Invalid("source cell is not hosted".into()))?;
        let source_player = self.cells[source_index]
            .runtime
            .state()
            .player
            .get(player_id)
            .ok_or_else(|| TwoCellRuntimeError::Invalid("source player is absent".into()))?;
        let destination_key = celestial::cell_key_from_address(&source_player.address)
            .map_err(|source| TwoCellRuntimeError::Invalid(source.to_string()))?;
        if destination_key == placement.cell_key {
            return Err(TwoCellRuntimeError::Invalid(
                "player has not crossed a canonical cell boundary".into(),
            ));
        }
        let destination_index = self
            .cell_index(&destination_key)
            .ok_or_else(|| TwoCellRuntimeError::Invalid("destination cell is not hosted".into()))?;
        let source_key = self.cells[source_index].key.clone();
        let source_assignment = self.directory.assignment(&source_key)?.clone();
        let destination_assignment = self.directory.assignment(&destination_key)?.clone();
        if source_assignment.state != CellAssignmentState::Assigned
            || destination_assignment.state != CellAssignmentState::Assigned
        {
            return Err(TwoCellRuntimeError::Invalid(
                "both proof cells must be assigned before handoff".into(),
            ));
        }
        let transfer_id = format!("player-transfer-{}", Uuid::new_v4());
        let context = PlayerTransferContext {
            transfer_id: transfer_id.clone(),
            source_cell_key: source_key.clone(),
            destination_cell_key: destination_key.clone(),
            source_assignment_generation: source_assignment.assignment_generation,
            destination_assignment_generation: destination_assignment.assignment_generation,
            source_fencing_token: self.cells[source_index].runtime.state().fencing_token,
            prior_placement_generation: placement.placement_generation,
            resulting_placement_generation: placement
                .placement_generation
                .checked_add(1)
                .ok_or_else(|| {
                    TwoCellRuntimeError::Invalid("placement generation is exhausted".into())
                })?,
        };
        let package = prepare_eva_player_transfer(
            self.cells[source_index].runtime.state(),
            player_id,
            &context,
        )?;
        self.artifacts.persist_package(&package)?;
        self.directory.prepare_transfer(
            player_id,
            placement.placement_generation,
            &transfer_id,
            &destination_key,
            &package.package_hash,
        )?;
        let finalized = self.reconcile_transfer(&transfer_id)?;

        let destination_player = self.cells[destination_index]
            .runtime
            .state()
            .player
            .get(player_id)
            .ok_or_else(|| {
                TwoCellRuntimeError::Invalid("destination player is absent after import".into())
            })?;
        if self.cells[source_index]
            .runtime
            .state()
            .player
            .get(player_id)
            .is_some()
        {
            return Err(TwoCellRuntimeError::Invalid(
                "source player remains live after finalization".into(),
            ));
        }
        Ok(CompletedPlayerHandoff {
            player_id: player_id.to_owned(),
            transfer_id,
            source_cell_key: source_key,
            destination_cell_key: destination_key,
            placement_generation: finalized.resulting_placement_generation,
            destination_movement_epoch: destination_player.movement_epoch,
        })
    }

    pub fn abort_transfer(
        &mut self,
        transfer_id: &str,
    ) -> Result<crate::cell_directory::CellTransferRecord, TwoCellRuntimeError> {
        self.directory.request_abort(transfer_id)?;
        self.reconcile_transfer(transfer_id)
    }

    fn reconcile_transfers(&mut self) -> Result<(), TwoCellRuntimeError> {
        for transfer in self.directory.transfer_records() {
            if matches!(
                transfer.phase,
                TransferPhase::Finalized | TransferPhase::Aborted
            ) {
                let source_index = self.cell_index(&transfer.source_cell_key).ok_or_else(|| {
                    TwoCellRuntimeError::Invalid("terminal transfer source is not hosted".into())
                })?;
                let destination_index = self
                    .cell_index(&transfer.destination_cell_key)
                    .ok_or_else(|| {
                        TwoCellRuntimeError::Invalid(
                            "terminal transfer destination is not hosted".into(),
                        )
                    })?;
                self.cells[source_index]
                    .runtime
                    .verify_transfer_record_proofs(&transfer)?;
                self.cells[destination_index]
                    .runtime
                    .verify_transfer_record_proofs(&transfer)?;
                continue;
            }
            self.reconcile_transfer(&transfer.transfer_id)?;
        }
        Ok(())
    }

    fn reconcile_transfer(
        &mut self,
        transfer_id: &str,
    ) -> Result<crate::cell_directory::CellTransferRecord, TwoCellRuntimeError> {
        let mut transfer = self.directory.transfer(transfer_id)?.clone();
        let package = self.artifacts.load_package(transfer_id)?;
        let source_index = self.cell_index(&transfer.source_cell_key).ok_or_else(|| {
            TwoCellRuntimeError::Invalid("transfer source cell is not hosted".into())
        })?;
        let destination_index =
            self.cell_index(&transfer.destination_cell_key)
                .ok_or_else(|| {
                    TwoCellRuntimeError::Invalid("transfer destination cell is not hosted".into())
                })?;
        if source_index == destination_index {
            return Err(TwoCellRuntimeError::Invalid(
                "transfer source and destination resolve to the same cell".into(),
            ));
        }

        // Directory phase flags are never authority by themselves. Every
        // persisted proof must resolve to the exact replay-derived cell event
        // before reconciliation may use it to move or finish placement.
        self.cells[source_index]
            .runtime
            .verify_transfer_record_proofs(&transfer)?;
        self.cells[destination_index]
            .runtime
            .verify_transfer_record_proofs(&transfer)?;

        if transfer.phase == TransferPhase::Aborted {
            return Ok(transfer);
        }

        if transfer.phase == TransferPhase::Aborting {
            if transfer.source_abort_proof.is_none() {
                let proof = self.cells[source_index]
                    .runtime
                    .commit_player_transfer_aborted(&package, &transfer)?;
                self.directory.record_abort_cleanup(transfer_id, &proof)?;
            }
            if transfer.destination_abort_proof.is_none() {
                let proof = self.cells[destination_index]
                    .runtime
                    .commit_player_transfer_aborted(&package, &transfer)?;
                self.directory.record_abort_cleanup(transfer_id, &proof)?;
            }
            transfer = self.directory.finalize_abort(transfer_id)?;
            return Ok(transfer);
        }

        if matches!(
            transfer.phase,
            TransferPhase::Prepared | TransferPhase::Quarantined
        ) {
            let prepare_proof = self.cells[source_index]
                .runtime
                .commit_player_transfer_prepared(&package, &transfer)?;
            transfer = self
                .directory
                .record_source_prepared(transfer_id, &prepare_proof)?;
            let (_, receipt) = stage_eva_player_quarantine(
                self.cells[destination_index].runtime.state(),
                self.cells[destination_index].runtime.state().fencing_token,
                &package,
            )?;
            if transfer
                .quarantine_receipt_hash
                .as_deref()
                .is_some_and(|hash| hash != receipt.receipt_hash)
            {
                return Err(TwoCellRuntimeError::Invalid(
                    "durable quarantine receipt disagrees with the destination reservation".into(),
                ));
            }
            let quarantine_proof = self.cells[destination_index]
                .runtime
                .commit_player_transfer_quarantined(&package, &receipt)?;
            self.artifacts.persist_quarantine_receipt(&receipt)?;
            transfer = self.directory.record_quarantine(
                transfer_id,
                &package.package_hash,
                &receipt.receipt_hash,
                &quarantine_proof,
            )?;
        }

        if transfer.phase == TransferPhase::Quarantined {
            transfer = self
                .directory
                .commit_transfer(transfer_id, transfer.prior_placement_generation)?;
        }

        let receipt = self.artifacts.load_quarantine_receipt(transfer_id)?;
        if transfer.quarantine_receipt_hash.as_deref() != Some(&receipt.receipt_hash) {
            return Err(TwoCellRuntimeError::Invalid(
                "transfer directory does not bind the persisted quarantine receipt".into(),
            ));
        }

        if transfer.phase == TransferPhase::Committed {
            let proof = self.cells[destination_index]
                .runtime
                .commit_player_transfer_imported(&package, &receipt, &transfer)?;
            transfer = self.directory.record_imported(transfer_id, &proof)?;
        } else if matches!(
            transfer.phase,
            TransferPhase::Imported | TransferPhase::Finalized
        ) && !has_exact_transfer_witness(
            self.cells[destination_index].runtime.state(),
            &package,
            TransferWitnessDirection::Import,
        )? {
            self.cells[destination_index]
                .runtime
                .commit_player_transfer_imported(&package, &receipt, &transfer)?;
        }
        if transfer.phase == TransferPhase::Imported {
            let proof = self.cells[source_index]
                .runtime
                .commit_player_transfer_exported(&package, &transfer)?;
            transfer = self.directory.finalize_transfer(transfer_id, &proof)?;
        } else if transfer.phase == TransferPhase::Finalized
            && !has_exact_transfer_witness(
                self.cells[source_index].runtime.state(),
                &package,
                TransferWitnessDirection::Export,
            )?
        {
            self.cells[source_index]
                .runtime
                .commit_player_transfer_exported(&package, &transfer)?;
        }
        if transfer.phase != TransferPhase::Finalized {
            return Err(TwoCellRuntimeError::Invalid(
                "transfer reconciliation did not reach a terminal phase".into(),
            ));
        }
        Ok(transfer)
    }

    fn cell_index(&self, key: &CellKeyV1) -> Option<usize> {
        self.cells.iter().position(|cell| &cell.key == key)
    }

    fn register_resident_players(&mut self) -> Result<(), TwoCellRuntimeError> {
        let hosted_cells = self
            .cells
            .iter()
            .map(|cell| cell.key.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let mut residents = BTreeMap::<String, CellKeyV1>::new();
        for cell in &self.cells {
            for (player_id, player) in cell.runtime.state().player.iter() {
                let addressed_cell = celestial::cell_key_from_address(&player.address)
                    .map_err(|source| TwoCellRuntimeError::Invalid(source.to_string()))?;
                if addressed_cell != cell.key && !hosted_cells.contains(&addressed_cell) {
                    return Err(TwoCellRuntimeError::Invalid(format!(
                        "player {player_id} is outside the hosted proof topology"
                    )));
                }
                if residents
                    .insert(player_id.clone(), cell.key.clone())
                    .is_some()
                {
                    return Err(TwoCellRuntimeError::Invalid(format!(
                        "player {player_id} is live in both proof cells"
                    )));
                }
            }
        }
        for (player_id, key) in residents {
            match self.directory.placement(&player_id) {
                Ok(existing)
                    if existing.aggregate_kind == MobileAggregateKind::Player
                        && existing.cell_key == key
                        && existing.state == AggregatePlacementState::Resident => {}
                Ok(_) => {
                    return Err(TwoCellRuntimeError::Invalid(format!(
                        "player {player_id} disagrees with the durable directory"
                    )));
                }
                Err(CellDirectoryError::UnknownAggregate(_)) => {
                    self.directory.register_placement(
                        &player_id,
                        MobileAggregateKind::Player,
                        &key,
                    )?;
                }
                Err(source) => return Err(source.into()),
            }
        }
        Ok(())
    }
}

fn two_cell_error_is_fatal(source: &TwoCellRuntimeError) -> bool {
    !matches!(
        source,
        TwoCellRuntimeError::StalePlayerRoute { .. }
            | TwoCellRuntimeError::Runtime(RuntimeError::Intent(_))
    )
}

fn has_exact_transfer_witness(
    world: &WorldState,
    package: &PlayerTransferPackage,
    direction: TransferWitnessDirection,
) -> Result<bool, TwoCellRuntimeError> {
    let expected = TransferConservationWitness {
        transfer_id: package.transfer_id.clone(),
        package_hash: package.package_hash.clone(),
        counterparty_cell_id: match direction {
            TransferWitnessDirection::Import => package.source_cell_id.clone(),
            TransferWitnessDirection::Export => package.destination_cell_id.clone(),
        },
        direction,
        contents: package.conservation.inventory_contents.clone(),
    };
    match world.transfer_witnesses.get(&package.transfer_id) {
        Some(existing) if existing == &expected => Ok(true),
        Some(_) => Err(TwoCellRuntimeError::Invalid(
            "cell transfer witness conflicts with the immutable package".into(),
        )),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write as _;
    use std::path::Path;

    use tempfile::tempdir;
    use verse_protocol::{LocomotionKind, Vec3};

    use super::*;
    use crate::{Store, TransferPhase};

    fn initialize_boundary_universe(root: &Path, seed: u64) -> [CellKeyV1; 2] {
        let manifest =
            celestial::universe_manifest(seed, WORLD_SCHEMA_VERSION, EVENT_SCHEMA_VERSION)
                .expect("manifest builds");
        let cells = crate::proof_cell_keys().expect("proof cells build");
        let directory = LocalCellDirectory::open(root, &manifest, cells.clone())
            .expect("directory initializes");
        let source_root = directory
            .cell_store_root(&cells[0])
            .expect("source root derives");
        drop(directory);

        let mut store =
            Store::open_for_cell(&source_root, seed, cells[0].clone()).expect("source store opens");
        let mut world = store.load_world().expect("source world loads");
        world.fencing_token = store.fencing_token();
        let boundary_address = celestial::address_from_origin_offset_um(
            &world.cell_address,
            [i128::from(celestial::CELL_EDGE_UM / 2), 0, 0],
        )
        .expect("boundary address canonicalizes");
        let boundary_position =
            celestial::local_position_from_address(&world.cell_address, &boundary_address)
                .expect("boundary position hydrates");
        let player = world.player.get_mut("player-local").expect("source player");
        player.address = boundary_address;
        player.position = boundary_position;
        player.linear_velocity = Vec3::ZERO;
        player.locomotion.kind = LocomotionKind::Eva;
        player.locomotion.support = None;
        player.locomotion.magnetic_boots_enabled = false;
        store
            .save_snapshot(&world)
            .expect("boundary state persists");
        drop(store);
        cells
    }

    fn initialize_near_unhosted_west_boundary(root: &Path, seed: u64) -> [CellKeyV1; 2] {
        let manifest =
            celestial::universe_manifest(seed, WORLD_SCHEMA_VERSION, EVENT_SCHEMA_VERSION)
                .expect("manifest builds");
        let cells = crate::proof_cell_keys().expect("proof cells build");
        let directory = LocalCellDirectory::open(root, &manifest, cells.clone())
            .expect("directory initializes");
        let source_root = directory
            .cell_store_root(&cells[0])
            .expect("source root derives");
        drop(directory);

        let mut store =
            Store::open_for_cell(&source_root, seed, cells[0].clone()).expect("source store opens");
        let mut world = store.load_world().expect("source world loads");
        world.fencing_token = store.fencing_token();
        let address = celestial::address_from_origin_offset_um(
            &world.cell_address,
            [-i128::from(celestial::CELL_EDGE_UM / 2) + 1_000, 0, 0],
        )
        .expect("near-west-boundary address canonicalizes inside origin");
        let position = celestial::local_position_from_address(&world.cell_address, &address)
            .expect("near-west-boundary position hydrates");
        let player = world.player.get_mut("player-local").expect("source player");
        player.address = address;
        player.position = position;
        player.linear_velocity = Vec3::new(-10.0, 0.0, 0.0);
        player.locomotion.kind = LocomotionKind::Eva;
        player.locomotion.support = None;
        player.locomotion.magnetic_boots_enabled = false;
        store
            .save_snapshot(&world)
            .expect("near-boundary state persists");
        cells
    }

    fn prepare_test_transfer(
        coordinator: &mut LocalTwoCellRuntime,
        record_source_proof: bool,
    ) -> (String, usize, usize, PlayerTransferPackage) {
        let player_id = "player-local";
        let placement = coordinator
            .directory
            .placement(player_id)
            .expect("player placement")
            .clone();
        let source_index = coordinator
            .cell_index(&placement.cell_key)
            .expect("source hosted");
        let destination_index = (0..coordinator.cells.len())
            .find(|index| *index != source_index)
            .expect("destination hosted");
        let source_key = coordinator.cells[source_index].key.clone();
        let destination_key = coordinator.cells[destination_index].key.clone();
        let source_assignment = coordinator
            .directory
            .assignment(&source_key)
            .expect("source assignment")
            .clone();
        let destination_assignment = coordinator
            .directory
            .assignment(&destination_key)
            .expect("destination assignment")
            .clone();
        let transfer_id = format!("player-transfer-{}", Uuid::new_v4());
        let context = PlayerTransferContext {
            transfer_id: transfer_id.clone(),
            source_cell_key: source_key,
            destination_cell_key: destination_key.clone(),
            source_assignment_generation: source_assignment.assignment_generation,
            destination_assignment_generation: destination_assignment.assignment_generation,
            source_fencing_token: coordinator.cells[source_index]
                .runtime
                .state()
                .fencing_token,
            prior_placement_generation: placement.placement_generation,
            resulting_placement_generation: placement.placement_generation + 1,
        };
        let package = prepare_eva_player_transfer(
            coordinator.cells[source_index].runtime.state(),
            player_id,
            &context,
        )
        .expect("package prepares");
        coordinator
            .artifacts
            .persist_package(&package)
            .expect("package persists");
        let prepared = coordinator
            .directory
            .prepare_transfer(
                player_id,
                placement.placement_generation,
                &transfer_id,
                &destination_key,
                &package.package_hash,
            )
            .expect("directory prepares");
        let prepare_proof = coordinator.cells[source_index]
            .runtime
            .commit_player_transfer_prepared(&package, &prepared)
            .expect("source locks");
        if record_source_proof {
            coordinator
                .directory
                .record_source_prepared(&transfer_id, &prepare_proof)
                .expect("directory binds source prepare proof");
        }
        (transfer_id, source_index, destination_index, package)
    }

    #[test]
    fn local_coordinator_commits_and_recovers_one_complete_player_handoff() {
        let root = tempdir().expect("universe root");
        let seed = 8_031;
        let cells = initialize_boundary_universe(root.path(), seed);

        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator opens");
        let source_route = coordinator
            .resident_player_route("player-local")
            .expect("source route resolves");
        let completed = coordinator
            .handoff_player("player-local")
            .expect("handoff completes");
        assert_eq!(completed.source_cell_key, cells[0]);
        assert_eq!(completed.destination_cell_key, cells[1]);
        assert_eq!(completed.placement_generation, 2);
        assert_eq!(completed.destination_movement_epoch, 2);
        assert!(matches!(
            coordinator.runtime_for_player_at("player-local", &source_route),
            Err(TwoCellRuntimeError::StalePlayerRoute { .. })
        ));
        let destination_route = coordinator
            .resident_player_route("player-local")
            .expect("destination route resolves");
        assert_eq!(destination_route.cell_key, cells[1]);
        assert_eq!(destination_route.placement_generation, 2);
        assert!(
            coordinator
                .runtime_for_player_at("player-local", &destination_route)
                .is_ok()
        );
        let transfer = coordinator
            .directory()
            .transfer(&completed.transfer_id)
            .expect("transfer remains auditable");
        assert_eq!(transfer.phase, TransferPhase::Finalized);
        assert!(transfer.source_prepare_proof.is_some());
        assert!(transfer.destination_quarantine_proof.is_some());
        assert!(transfer.import_proof.is_some());
        assert!(transfer.finalization_proof.is_some());
        assert!(
            coordinator
                .runtime_for_cell(&cells[0])
                .expect("source runtime")
                .state()
                .player
                .get("player-local")
                .is_none()
        );
        let destination = coordinator
            .runtime_for_cell(&cells[1])
            .expect("destination runtime")
            .state();
        assert!(destination.player.get("player-local").is_some());
        assert!(destination.conservation().valid);
        drop(coordinator);

        let destination_root = root
            .path()
            .join("cells")
            .join(celestial::cell_id(&cells[1]).expect("destination cell ID"));
        let mut store = Store::open_for_cell(&destination_root, seed, cells[1].clone())
            .expect("destination store reopens");
        let mut moved_world = store.load_world().expect("destination world loads");
        moved_world.fencing_token = store.fencing_token();
        let moved_address = celestial::address_from_origin_offset_um(
            &moved_world.cell_address,
            [-(i128::from(celestial::CELL_EDGE_UM) / 2) + 1_000_000, 0, 0],
        )
        .expect("movement address canonicalizes");
        let moved_position =
            celestial::local_position_from_address(&moved_world.cell_address, &moved_address)
                .expect("movement position hydrates");
        let moved_player = moved_world
            .player
            .get_mut("player-local")
            .expect("destination player");
        moved_player.address = moved_address;
        moved_player.position = moved_position;
        store
            .save_snapshot(&moved_world)
            .expect("post-handoff movement persists");
        drop(store);

        let recovered = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator recovers");
        assert!(
            recovered
                .runtime_for_cell(&cells[0])
                .expect("source recovers")
                .state()
                .player
                .get("player-local")
                .is_none()
        );
        assert!(
            recovered
                .runtime_for_cell(&cells[1])
                .expect("destination recovers")
                .state()
                .player
                .get("player-local")
                .is_some()
        );
    }

    #[test]
    fn coordinator_rejects_unhosted_face_before_physics_becomes_durable() {
        let root = tempdir().expect("universe root");
        let seed = 8_032;
        let cells = initialize_near_unhosted_west_boundary(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "west-boundary")
            .expect("coordinator opens");
        let before = coordinator
            .runtime_for_cell(&cells[0])
            .expect("origin runtime exists")
            .state()
            .clone();

        let outcome = coordinator
            .advance_with_outcome(17)
            .expect("unsupported final pose is contained without halting");
        assert!(!outcome.changed());
        assert!(outcome.handoffs.is_empty());
        let after = coordinator
            .runtime_for_cell(&cells[0])
            .expect("origin runtime exists")
            .state();
        assert_eq!(after.event_sequence, before.event_sequence);
        assert_eq!(after.state_hash(), before.state_hash());
        assert_eq!(
            celestial::cell_key_from_address(
                &after
                    .player
                    .get("player-local")
                    .expect("player remains live")
                    .address
            )
            .expect("player address remains canonical"),
            cells[0]
        );
        assert!(!coordinator.is_halted());

        drop(coordinator);
        let reopened = LocalTwoCellRuntime::open(root.path(), seed, 20, "west-boundary-reopen")
            .expect("contained universe reopens");
        let recovered = reopened
            .runtime_for_cell(&cells[0])
            .expect("reopened origin runtime exists")
            .state();
        assert_eq!(recovered.event_sequence, before.event_sequence);
        assert_eq!(
            recovered
                .player
                .get("player-local")
                .expect("recovered player remains live")
                .address,
            before
                .player
                .get("player-local")
                .expect("prior player remains live")
                .address
        );
    }

    #[test]
    fn standalone_runtime_cannot_bypass_directory_managed_admission() {
        let root = tempdir().expect("universe root");
        let seed = 8_040;
        let cells = initialize_boundary_universe(root.path(), seed);
        let source_root = root
            .path()
            .join("cells")
            .join(celestial::cell_id(&cells[0]).expect("source cell ID"));

        assert!(matches!(
            Runtime::open_for_cell(&source_root, seed, cells[0].clone(), 20),
            Err(RuntimeError::DirectoryManagedCellRequiresCoordinator)
        ));

        LocalTwoCellRuntime::open(root.path(), seed, 20, "coordinator-host")
            .expect("directory coordinator retains the authority capability");
    }

    #[test]
    fn historic_transfer_world_root_is_bound_by_the_lifecycle_hash_chain() {
        let root = tempdir().expect("universe root");
        let seed = 8_037;
        let cells = initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 1, "test-host")
            .expect("coordinator opens");
        coordinator
            .handoff_player("player-local")
            .expect("handoff completes and snapshots each event");
        drop(coordinator);

        let source_root = root
            .path()
            .join("cells")
            .join(celestial::cell_id(&cells[0]).expect("source cell ID"));
        let boundary_path = source_root.join("transfer-boundaries.ndjson");
        let text = fs::read_to_string(&boundary_path).expect("boundary journal reads");
        let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
        let mut first: serde_json::Value =
            serde_json::from_str(&lines[0]).expect("boundary parses");
        first["resulting_world_hash"] = serde_json::Value::String("0".repeat(64));
        lines[0] = serde_json::to_string(&first).expect("tampered boundary serializes");
        fs::write(&boundary_path, format!("{}\n", lines.join("\n")))
            .expect("tampered boundary writes");

        assert!(matches!(
            LocalTwoCellRuntime::open(root.path(), seed, 1, "replacement-host"),
            Err(TwoCellRuntimeError::Runtime(RuntimeError::Persistence(
                crate::PersistenceError::InvalidTransferBoundary(_)
            )))
        ));
    }

    #[test]
    fn partial_transfer_boundary_tail_is_truncated_before_recovery() {
        let root = tempdir().expect("universe root");
        let seed = 8_038;
        let cells = initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator opens");
        coordinator
            .handoff_player("player-local")
            .expect("handoff completes");
        drop(coordinator);

        let destination_root = root
            .path()
            .join("cells")
            .join(celestial::cell_id(&cells[1]).expect("destination cell ID"));
        let boundary_path = destination_root.join("transfer-boundaries.ndjson");
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&boundary_path)
            .expect("boundary journal opens");
        file.write_all(b"{\"partial\":true")
            .and_then(|()| file.sync_data())
            .expect("partial boundary tail writes");
        drop(file);

        LocalTwoCellRuntime::open(root.path(), seed, 20, "replacement-host")
            .expect("partial boundary tail is discarded safely");
        assert_eq!(
            fs::read(&boundary_path)
                .expect("boundary journal reads")
                .last(),
            Some(&b'\n')
        );
    }

    #[test]
    fn multiple_unanchored_transfer_boundaries_are_rejected() {
        let root = tempdir().expect("universe root");
        let seed = 8_039;
        let cells = initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator opens");
        coordinator
            .handoff_player("player-local")
            .expect("handoff completes");
        drop(coordinator);

        let source_root = root
            .path()
            .join("cells")
            .join(celestial::cell_id(&cells[0]).expect("source cell ID"));
        let boundaries = fs::read_to_string(source_root.join("transfer-boundaries.ndjson"))
            .expect("boundary journal reads")
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("boundary parses"))
            .collect::<Vec<_>>();
        assert_eq!(boundaries.len(), 2, "source proof has prepare and export");
        let last = boundaries.last().expect("last boundary exists");

        let lifecycle_path = source_root.join("cell-lifecycle.json");
        let mut lifecycle: serde_json::Value =
            serde_json::from_slice(&fs::read(&lifecycle_path).expect("lifecycle reads"))
                .expect("lifecycle parses");
        lifecycle["transfer_boundary_head_hash"] = serde_json::Value::String(String::new());
        lifecycle["last_world_event_sequence"] = serde_json::json!(0);
        lifecycle["last_world_event_hash"] = serde_json::Value::String(String::new());
        lifecycle["last_world_state_hash"] = serde_json::Value::String(String::new());
        lifecycle["pending_world_commit"] = serde_json::json!({
            "event_sequence": last["event_sequence"],
            "event_hash": last["event_hash"],
            "occurred_at_unix_ms": 1,
            "prior_next_occurrence": null,
            "resulting_next_occurrence": null
        });
        fs::write(
            &lifecycle_path,
            serde_json::to_vec(&lifecycle).expect("lifecycle serializes"),
        )
        .expect("lifecycle writes");

        assert!(matches!(
            LocalTwoCellRuntime::open(root.path(), seed, 20, "replacement-host"),
            Err(TwoCellRuntimeError::Runtime(RuntimeError::Persistence(
                crate::PersistenceError::InvalidTransferBoundary(_)
            )))
        ));
    }

    #[test]
    fn reopen_rolls_a_prepared_transfer_forward_exactly_once() {
        let root = tempdir().expect("universe root");
        let seed = 8_032;
        let cells = initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator opens");
        let (transfer_id, _, _, _) = prepare_test_transfer(&mut coordinator, true);
        assert_eq!(
            coordinator
                .directory
                .transfer(&transfer_id)
                .expect("transfer exists")
                .phase,
            TransferPhase::Prepared
        );
        drop(coordinator);

        let recovered = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("prepared transfer reconciles");
        assert_eq!(
            recovered
                .directory
                .transfer(&transfer_id)
                .expect("transfer remains auditable")
                .phase,
            TransferPhase::Finalized
        );
        assert!(
            recovered
                .runtime_for_cell(&cells[0])
                .expect("source runtime")
                .state()
                .player
                .get("player-local")
                .is_none()
        );
        assert!(
            recovered
                .runtime_for_cell(&cells[1])
                .expect("destination runtime")
                .state()
                .player
                .get("player-local")
                .is_some()
        );
    }

    #[test]
    fn terminal_transfer_reopens_after_ephemeral_artifacts_are_removed() {
        let root = tempdir().expect("universe root");
        let seed = 8_037;
        initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator opens");
        let completed = coordinator
            .handoff_player("player-local")
            .expect("handoff completes");
        drop(coordinator);

        let artifact_root = root
            .path()
            .join("handoff-artifacts")
            .join(&completed.transfer_id);
        fs::remove_file(artifact_root.join("package.json")).expect("package artifact removes");
        fs::remove_file(artifact_root.join("quarantine-receipt.json"))
            .expect("receipt artifact removes");

        let recovered = LocalTwoCellRuntime::open(root.path(), seed, 20, "successor-host")
            .expect("terminal proof reopens without package artifacts");
        assert_eq!(
            recovered
                .directory
                .transfer(&completed.transfer_id)
                .expect("terminal transfer remains auditable")
                .phase,
            TransferPhase::Finalized
        );
    }

    #[test]
    fn successor_recovers_prepare_event_before_directory_proof_cas() {
        let root = tempdir().expect("universe root");
        let seed = 8_036;
        initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "first-host")
            .expect("first coordinator opens");
        let (transfer_id, _, _, _) = prepare_test_transfer(&mut coordinator, false);
        let original_generation = coordinator
            .directory
            .transfer(&transfer_id)
            .expect("prepared transfer exists")
            .source_assignment_generation;
        assert!(
            coordinator
                .directory
                .transfer(&transfer_id)
                .expect("prepared transfer exists")
                .source_prepare_proof
                .is_none()
        );
        drop(coordinator);

        let recovered = LocalTwoCellRuntime::open(root.path(), seed, 20, "successor-host")
            .expect("successor reconstructs the proof from the cell boundary");
        let transfer = recovered
            .directory
            .transfer(&transfer_id)
            .expect("transfer remains auditable");
        assert_eq!(transfer.phase, TransferPhase::Finalized);
        assert_eq!(
            transfer
                .source_prepare_proof
                .as_ref()
                .expect("source proof recovers")
                .source_assignment_generation,
            original_generation
        );
        assert!(
            recovered
                .directory
                .assignment(&transfer.source_cell_key)
                .expect("successor source assignment")
                .assignment_generation
                > original_generation
        );
    }

    #[test]
    fn successor_holder_recovers_pinned_assignments_and_rolls_forward() {
        let root = tempdir().expect("universe root");
        let seed = 8_035;
        let cells = initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "first-host")
            .expect("first coordinator opens");
        let (transfer_id, _, _, _) = prepare_test_transfer(&mut coordinator, true);
        let source_generation = coordinator
            .directory
            .assignment(&cells[0])
            .expect("source assignment")
            .assignment_generation;
        drop(coordinator);

        let recovered = LocalTwoCellRuntime::open(root.path(), seed, 20, "successor-host")
            .expect("successor coordinator takes over and reconciles");
        assert!(
            recovered
                .directory
                .assignment(&cells[0])
                .expect("recovered source assignment")
                .assignment_generation
                > source_generation
        );
        let transfer = recovered
            .directory
            .transfer(&transfer_id)
            .expect("transfer remains auditable");
        assert_eq!(transfer.phase, TransferPhase::Finalized);
        assert!(transfer.import_proof.is_some());
        assert!(transfer.finalization_proof.is_some());
    }

    #[test]
    fn reopen_completes_a_directory_committed_transfer() {
        let root = tempdir().expect("universe root");
        let seed = 8_033;
        initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator opens");
        let (transfer_id, _, destination_index, package) =
            prepare_test_transfer(&mut coordinator, true);
        let (_, receipt) = stage_eva_player_quarantine(
            coordinator.cells[destination_index].runtime.state(),
            coordinator.cells[destination_index]
                .runtime
                .state()
                .fencing_token,
            &package,
        )
        .expect("receipt prepares");
        let quarantine_proof = coordinator.cells[destination_index]
            .runtime
            .commit_player_transfer_quarantined(&package, &receipt)
            .expect("destination quarantines");
        coordinator
            .artifacts
            .persist_quarantine_receipt(&receipt)
            .expect("receipt persists");
        coordinator
            .directory
            .record_quarantine(
                &transfer_id,
                &package.package_hash,
                &receipt.receipt_hash,
                &quarantine_proof,
            )
            .expect("directory records quarantine");
        coordinator
            .directory
            .commit_transfer(&transfer_id, package.prior_placement_generation)
            .expect("directory commits");
        drop(coordinator);

        let recovered = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("committed transfer reconciles");
        assert_eq!(
            recovered
                .directory
                .transfer(&transfer_id)
                .expect("transfer remains auditable")
                .phase,
            TransferPhase::Finalized
        );
    }

    #[test]
    fn reopen_cleans_both_cells_after_a_quarantined_transfer_aborts() {
        let root = tempdir().expect("universe root");
        let seed = 8_034;
        let cells = initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator opens");
        let (transfer_id, source_index, destination_index, package) =
            prepare_test_transfer(&mut coordinator, true);
        let (_, receipt) = stage_eva_player_quarantine(
            coordinator.cells[destination_index].runtime.state(),
            coordinator.cells[destination_index]
                .runtime
                .state()
                .fencing_token,
            &package,
        )
        .expect("receipt prepares");
        let quarantine_proof = coordinator.cells[destination_index]
            .runtime
            .commit_player_transfer_quarantined(&package, &receipt)
            .expect("destination quarantines");
        coordinator
            .artifacts
            .persist_quarantine_receipt(&receipt)
            .expect("receipt persists");
        coordinator
            .directory
            .record_quarantine(
                &transfer_id,
                &package.package_hash,
                &receipt.receipt_hash,
                &quarantine_proof,
            )
            .expect("directory records quarantine");
        let aborted = coordinator
            .directory
            .request_abort(&transfer_id)
            .expect("directory begins abort");
        coordinator.cells[source_index]
            .runtime
            .commit_player_transfer_aborted(&package, &aborted)
            .expect("source unlocks before crash");
        assert!(
            coordinator.cells[destination_index]
                .runtime
                .state()
                .player_transfer_reservations
                .contains_key(&transfer_id)
        );
        drop(coordinator);

        let recovered = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("aborted transfer reconciles");
        assert_eq!(
            recovered
                .directory
                .transfer(&transfer_id)
                .expect("transfer remains auditable")
                .phase,
            TransferPhase::Aborted
        );
        assert!(
            recovered
                .runtime_for_cell(&cells[0])
                .expect("source runtime")
                .state()
                .player_transfer_locks
                .is_empty()
        );
        assert!(
            recovered
                .runtime_for_cell(&cells[1])
                .expect("destination runtime")
                .state()
                .player_transfer_reservations
                .is_empty()
        );
    }
}
