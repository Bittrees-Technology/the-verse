// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bounded local two-cell coordinator for the P1.7 correctness proof.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;
use uuid::Uuid;
use verse_protocol::CellKeyV1;

use crate::cell_directory::{
    AggregatePlacementState, CellAssignmentState, CellDirectoryError, LocalCellDirectory,
    MobileAggregateKind, TransferPhase,
};
use crate::engine::{Runtime, RuntimeError};
use crate::handoff::{
    HandoffArtifactError, HandoffError, LocalHandoffArtifactStore, PlayerTransferContext,
    PlayerTransferPackage, prepare_eva_player_transfer, stage_eva_player_quarantine,
};
use crate::model::{TransferConservationWitness, TransferWitnessDirection, WorldState};
use crate::{EVENT_SCHEMA_VERSION, WORLD_SCHEMA_VERSION, celestial};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedPlayerHandoff {
    pub transfer_id: String,
    pub source_cell_key: CellKeyV1,
    pub destination_cell_key: CellKeyV1,
    pub placement_generation: u64,
    pub destination_movement_epoch: u64,
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
            let assignment = match current.state {
                CellAssignmentState::Sleeping => {
                    directory.claim(&key, current.assignment_generation, &cell_holder)?
                }
                CellAssignmentState::Assigned
                    if current.holder_id.as_deref() == Some(cell_holder.as_str()) =>
                {
                    current
                }
                _ => {
                    return Err(TwoCellRuntimeError::Invalid(format!(
                        "proof cell {} is already assigned to another holder",
                        current.cell_id
                    )));
                }
            };
            if assignment.assignment_generation == 0 {
                return Err(TwoCellRuntimeError::Invalid(
                    "claimed cell has no assignment generation".into(),
                ));
            }
            let cell_root = directory.cell_store_root(&key)?;
            let runtime =
                Runtime::open_for_cell(cell_root, world_seed, key.clone(), snapshot_every)?;
            cells.push(CellSlot { key, runtime });
        }
        let artifacts = LocalHandoffArtifactStore::open(root.join("handoff-artifacts"))?;
        let mut coordinator = Self {
            root,
            directory,
            artifacts,
            cells,
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
        let destination_index = (0..self.cells.len())
            .find(|index| *index != source_index)
            .ok_or_else(|| TwoCellRuntimeError::Invalid("destination cell is not hosted".into()))?;
        let source_key = self.cells[source_index].key.clone();
        let destination_key = self.cells[destination_index].key.clone();
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
            transfer_id,
            source_cell_key: source_key,
            destination_cell_key: destination_key,
            placement_generation: finalized.resulting_placement_generation,
            destination_movement_epoch: destination_player.movement_epoch,
        })
    }

    fn reconcile_transfers(&mut self) -> Result<(), TwoCellRuntimeError> {
        let transfer_ids = self
            .directory
            .transfer_records()
            .into_iter()
            .map(|transfer| transfer.transfer_id)
            .collect::<Vec<_>>();
        for transfer_id in transfer_ids {
            self.reconcile_transfer(&transfer_id)?;
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

        if transfer.phase == TransferPhase::Aborted {
            if self.cells[source_index]
                .runtime
                .state()
                .player_transfer_locks
                .contains_key(&package.aggregate_id)
            {
                self.cells[source_index]
                    .runtime
                    .commit_player_transfer_aborted(&package, &transfer)?;
            }
            if self.cells[destination_index]
                .runtime
                .state()
                .player_transfer_reservations
                .contains_key(transfer_id)
            {
                self.cells[destination_index]
                    .runtime
                    .commit_player_transfer_aborted(&package, &transfer)?;
            }
            return Ok(transfer);
        }

        if matches!(
            transfer.phase,
            TransferPhase::Prepared | TransferPhase::Quarantined
        ) {
            self.cells[source_index]
                .runtime
                .commit_player_transfer_prepared(&package, &transfer)?;
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
            self.cells[destination_index]
                .runtime
                .commit_player_transfer_quarantined(&package, &receipt)?;
            self.artifacts.persist_quarantine_receipt(&receipt)?;
            transfer = self.directory.record_quarantine(
                transfer_id,
                &package.package_hash,
                &receipt.receipt_hash,
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

        if matches!(
            transfer.phase,
            TransferPhase::Committed | TransferPhase::Imported | TransferPhase::Finalized
        ) && !has_exact_transfer_witness(
            self.cells[destination_index].runtime.state(),
            &package,
            TransferWitnessDirection::Import,
        )? {
            self.cells[destination_index]
                .runtime
                .commit_player_transfer_imported(&package, &receipt, &transfer)?;
        }
        if transfer.phase == TransferPhase::Committed {
            transfer = self.directory.record_imported(transfer_id)?;
        }
        if matches!(
            transfer.phase,
            TransferPhase::Imported | TransferPhase::Finalized
        ) && !has_exact_transfer_witness(
            self.cells[source_index].runtime.state(),
            &package,
            TransferWitnessDirection::Export,
        )? {
            self.cells[source_index]
                .runtime
                .commit_player_transfer_exported(&package, &transfer)?;
        }
        if transfer.phase == TransferPhase::Imported {
            transfer = self.directory.finalize_transfer(transfer_id)?;
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
        let mut residents = BTreeMap::<String, CellKeyV1>::new();
        for cell in &self.cells {
            for (player_id, _) in cell.runtime.state().player.iter() {
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

    fn prepare_test_transfer(
        coordinator: &mut LocalTwoCellRuntime,
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
        coordinator.cells[source_index]
            .runtime
            .commit_player_transfer_prepared(&package, &prepared)
            .expect("source locks");
        (transfer_id, source_index, destination_index, package)
    }

    #[test]
    fn local_coordinator_commits_and_recovers_one_complete_player_handoff() {
        let root = tempdir().expect("universe root");
        let seed = 8_031;
        let cells = initialize_boundary_universe(root.path(), seed);

        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator opens");
        let completed = coordinator
            .handoff_player("player-local")
            .expect("handoff completes");
        assert_eq!(completed.source_cell_key, cells[0]);
        assert_eq!(completed.destination_cell_key, cells[1]);
        assert_eq!(completed.placement_generation, 2);
        assert_eq!(completed.destination_movement_epoch, 2);
        assert_eq!(
            coordinator
                .directory()
                .transfer(&completed.transfer_id)
                .expect("transfer remains auditable")
                .phase,
            TransferPhase::Finalized
        );
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
    fn reopen_rolls_a_prepared_transfer_forward_exactly_once() {
        let root = tempdir().expect("universe root");
        let seed = 8_032;
        let cells = initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator opens");
        let (transfer_id, _, _, _) = prepare_test_transfer(&mut coordinator);
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
    fn reopen_completes_a_directory_committed_transfer() {
        let root = tempdir().expect("universe root");
        let seed = 8_033;
        initialize_boundary_universe(root.path(), seed);
        let mut coordinator = LocalTwoCellRuntime::open(root.path(), seed, 20, "test-host")
            .expect("coordinator opens");
        let (transfer_id, _, destination_index, package) = prepare_test_transfer(&mut coordinator);
        let (_, receipt) = stage_eva_player_quarantine(
            coordinator.cells[destination_index].runtime.state(),
            coordinator.cells[destination_index]
                .runtime
                .state()
                .fencing_token,
            &package,
        )
        .expect("receipt prepares");
        coordinator.cells[destination_index]
            .runtime
            .commit_player_transfer_quarantined(&package, &receipt)
            .expect("destination quarantines");
        coordinator
            .artifacts
            .persist_quarantine_receipt(&receipt)
            .expect("receipt persists");
        coordinator
            .directory
            .record_quarantine(&transfer_id, &package.package_hash, &receipt.receipt_hash)
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
            prepare_test_transfer(&mut coordinator);
        let (_, receipt) = stage_eva_player_quarantine(
            coordinator.cells[destination_index].runtime.state(),
            coordinator.cells[destination_index]
                .runtime
                .state()
                .fencing_token,
            &package,
        )
        .expect("receipt prepares");
        coordinator.cells[destination_index]
            .runtime
            .commit_player_transfer_quarantined(&package, &receipt)
            .expect("destination quarantines");
        coordinator
            .artifacts
            .persist_quarantine_receipt(&receipt)
            .expect("receipt persists");
        coordinator
            .directory
            .record_quarantine(&transfer_id, &package.package_hash, &receipt.receipt_hash)
            .expect("directory records quarantine");
        let aborted = coordinator
            .directory
            .abort_transfer(&transfer_id)
            .expect("directory aborts");
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
