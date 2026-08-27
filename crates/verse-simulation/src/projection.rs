// SPDX-License-Identifier: AGPL-3.0-or-later

//! Actor-aware public/private network projections.
//!
//! Canonical snapshots remain simulation and persistence artifacts. These
//! projections are the only shapes intended to cross an untrusted connection.
//! They preserve the canonical sequence, tick, and hash, so observers can
//! still correlate traffic timing and hash changes with hidden activity.

use std::collections::BTreeMap;

use thiserror::Error;
use verse_protocol::{
    ActorPrivateSnapshot, BlockSnapshot, GridMotionSnapshot, GridSnapshot, InventorySnapshot,
    OwnedGridMassSnapshot, PROJECTION_SCHEMA_VERSION, PlayerLifeState, PlayerMotionSnapshot,
    PlayerSnapshot, ProjectedMotionSnapshot, ProjectedWorldSnapshot, PublicBlockSnapshot,
    PublicGridMotionSnapshot, PublicGridSnapshot, PublicPlayerLifeState,
    PublicPlayerMotionSnapshot, PublicPlayerSnapshot,
};

use crate::model::WorldState;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectionError {
    #[error("canonical authority graph is invalid: {0}")]
    InvalidAuthority(String),
    #[error("actor {0} is not bound to a canonical player")]
    UnboundActor(String),
    #[error("canonical projection invariant failed: {0}")]
    InvalidCanonicalSnapshot(String),
}

impl WorldState {
    /// Builds a stable public projection and, for an authenticated player, an
    /// exact private projection containing only actor-owned authority records.
    pub fn project_world_snapshot(
        &self,
        actor_player_id: Option<&str>,
    ) -> Result<ProjectedWorldSnapshot, ProjectionError> {
        let inventory_owners = self.projection_inventory_owners(actor_player_id)?;
        let canonical = self.snapshot();

        let mut players = canonical
            .players
            .iter()
            .map(public_player)
            .collect::<Vec<_>>();
        players.sort_by(|left, right| left.player_id.cmp(&right.player_id));

        let mut grids = canonical.grids.iter().map(public_grid).collect::<Vec<_>>();
        grids.sort_by(|left, right| left.grid_id.cmp(&right.grid_id));
        for grid in &mut grids {
            grid.blocks
                .sort_by(|left, right| left.block_id.cmp(&right.block_id));
        }

        let actor_private = actor_player_id
            .map(|actor| {
                let player = canonical
                    .players
                    .iter()
                    .find(|player| player.player_id == actor)
                    .cloned()
                    .ok_or_else(|| ProjectionError::UnboundActor(actor.to_owned()))?;

                let mut inventories = canonical
                    .inventories
                    .iter()
                    .filter(|inventory| {
                        inventory_owners
                            .get(&inventory.inventory_id)
                            .is_some_and(|owner| owner == actor)
                    })
                    .cloned()
                    .collect::<Vec<InventorySnapshot>>();
                inventories.sort_by(|left, right| left.inventory_id.cmp(&right.inventory_id));

                let mut death_drops = canonical
                    .death_drops
                    .iter()
                    .filter(|drop| drop.owner_player_id == actor)
                    .cloned()
                    .collect::<Vec<_>>();
                death_drops.sort_by(|left, right| left.drop_id.cmp(&right.drop_id));

                let mut owned_grid_masses = canonical
                    .grids
                    .iter()
                    .filter(|grid| grid.owner_player_id == actor)
                    .map(|grid| OwnedGridMassSnapshot {
                        grid_id: grid.grid_id.clone(),
                        mass_kg: grid.mass_kg,
                    })
                    .collect::<Vec<_>>();
                owned_grid_masses.sort_by(|left, right| left.grid_id.cmp(&right.grid_id));

                Ok(ActorPrivateSnapshot {
                    player,
                    inventories,
                    death_drops,
                    owned_grid_masses,
                })
            })
            .transpose()?;

        let mut voxels = canonical.voxels;
        voxels.sort_by_key(|voxel| voxel.coordinate);

        Ok(ProjectedWorldSnapshot {
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            schema_version: canonical.schema_version,
            content_manifest_version: canonical.content_manifest_version,
            universe_id: canonical.universe_id,
            cell_id: canonical.cell_id,
            event_sequence: canonical.event_sequence,
            simulation_tick: canonical.simulation_tick,
            fencing_token: canonical.fencing_token,
            world_hash: canonical.world_hash,
            players,
            environment: canonical.environment,
            voxels,
            grids,
            conservation_valid: canonical.conservation.valid,
            actor_private,
        })
    }

    /// Builds stable high-rate public motion and binds the exact canonical
    /// motion record only to the matching authenticated player.
    pub fn project_motion_snapshot(
        &self,
        actor_player_id: Option<&str>,
    ) -> Result<ProjectedMotionSnapshot, ProjectionError> {
        self.projection_inventory_owners(actor_player_id)?;
        let canonical = self.motion_snapshot();

        let mut players = canonical
            .players
            .iter()
            .map(public_player_motion)
            .collect::<Vec<_>>();
        players.sort_by(|left, right| left.player_id.cmp(&right.player_id));

        let mut grids = canonical
            .grids
            .iter()
            .map(public_grid_motion)
            .collect::<Vec<_>>();
        grids.sort_by(|left, right| left.grid_id.cmp(&right.grid_id));

        let actor_private = actor_player_id
            .map(|actor| {
                canonical
                    .players
                    .iter()
                    .find(|player| player.player_id == actor)
                    .cloned()
                    .ok_or_else(|| ProjectionError::UnboundActor(actor.to_owned()))
            })
            .transpose()?;

        Ok(ProjectedMotionSnapshot {
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            event_sequence: canonical.event_sequence,
            simulation_tick: canonical.simulation_tick,
            world_hash: canonical.world_hash,
            players,
            grids,
            actor_private,
        })
    }

    fn projection_inventory_owners(
        &self,
        actor_player_id: Option<&str>,
    ) -> Result<BTreeMap<String, String>, ProjectionError> {
        self.validate_player_roster()
            .map_err(ProjectionError::InvalidAuthority)?;
        if let Some(actor) = actor_player_id
            && self.player.get(actor).is_none()
        {
            return Err(ProjectionError::UnboundActor(actor.to_owned()));
        }

        self.inventories
            .keys()
            .map(|inventory_id| {
                self.inventory_owner_player_id(inventory_id)
                    .map(|owner| (inventory_id.clone(), owner.to_owned()))
                    .map_err(ProjectionError::InvalidAuthority)
            })
            .collect()
    }
}

fn public_life_state(life_state: &PlayerLifeState) -> PublicPlayerLifeState {
    match life_state {
        PlayerLifeState::Alive => PublicPlayerLifeState::Alive,
        PlayerLifeState::Incapacitated { .. } => PublicPlayerLifeState::Incapacitated,
    }
}

fn public_player(player: &PlayerSnapshot) -> PublicPlayerSnapshot {
    PublicPlayerSnapshot {
        player_id: player.player_id.clone(),
        position: player.position,
        orientation: player.orientation,
        linear_velocity: player.linear_velocity,
        angular_velocity: player.angular_velocity,
        surface_contact: player.surface_contact,
        locomotion_kind: player.locomotion.kind,
        life_state: public_life_state(&player.life_state),
        helmet_closed: player.helmet_closed,
        jetpack_enabled: player.jetpack_enabled,
    }
}

fn public_block(block: &BlockSnapshot) -> PublicBlockSnapshot {
    PublicBlockSnapshot {
        block_id: block.block_id.clone(),
        coordinate: block.coordinate,
        kind: block.kind,
        orientation: block.orientation,
        health: block.health,
        max_health: block.max_health,
        construction_complete: block.construction_complete,
    }
}

fn public_grid(grid: &GridSnapshot) -> PublicGridSnapshot {
    PublicGridSnapshot {
        grid_id: grid.grid_id.clone(),
        owner_player_id: grid.owner_player_id.clone(),
        position: grid.position,
        orientation: grid.orientation,
        linear_velocity: grid.linear_velocity,
        angular_velocity: grid.angular_velocity,
        anchored: grid.anchored,
        power: grid.power,
        blocks: grid.blocks.iter().map(public_block).collect(),
    }
}

fn public_player_motion(player: &PlayerMotionSnapshot) -> PublicPlayerMotionSnapshot {
    PublicPlayerMotionSnapshot {
        player_id: player.player_id.clone(),
        position: player.position,
        orientation: player.orientation,
        linear_velocity: player.linear_velocity,
        angular_velocity: player.angular_velocity,
        surface_contact: player.surface_contact,
        locomotion_kind: player.locomotion.kind,
        life_state: public_life_state(&player.life_state),
        jetpack_enabled: player.jetpack_enabled,
    }
}

fn public_grid_motion(grid: &GridMotionSnapshot) -> PublicGridMotionSnapshot {
    PublicGridMotionSnapshot {
        grid_id: grid.grid_id.clone(),
        position: grid.position,
        orientation: grid.orientation,
        linear_velocity: grid.linear_velocity,
        angular_velocity: grid.angular_velocity,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;
    use verse_protocol::{
        BlockKind, IVec3, InventoryContents, InventoryDomain, PlayerDeathCause, Quat, Vec3,
    };

    use super::*;
    use crate::model::{Block, DeathDrop, Grid, InventoryRecord};

    fn two_actor_world() -> WorldState {
        let mut world = WorldState::genesis(41);
        world.event_sequence = 12;
        world.simulation_tick = 81;

        let mut remote = world.player.primary().clone();
        remote.player_id = "player-remote".into();
        remote.position = Vec3::new(30.0, 9.0, -12.0);
        remote.inventory_id = "inventory-player-remote".into();
        remote.experience = 275;
        remote.suit_oxygen_milli = 412;
        remote.control_linear_input = Vec3::new(0.25, -0.5, 1.0);
        remote.control_angular_input = Vec3::new(-0.2, 0.1, 0.7);
        remote.last_received_input_sequence = 91;
        remote.last_processed_input_sequence = 89;
        remote.control_expires_at_simulation_tick = 99;
        remote.helmet_closed = false;
        remote.jetpack_enabled = false;
        world.player.by_id.insert(remote.player_id.clone(), remote);
        world.inventories.insert(
            "inventory-player-remote".into(),
            InventoryRecord {
                inventory_id: "inventory-player-remote".into(),
                domain: InventoryDomain::Player {
                    player_id: "player-remote".into(),
                },
                contents: InventoryContents {
                    ore: 0,
                    refined_material: 0,
                    components: 7,
                },
                capacity_liters: 1_200,
            },
        );
        world.ledger.genesis_components += 7;

        let remote_block = Block::new(
            "block-remote-structural",
            IVec3::ZERO,
            BlockKind::Structural,
        );
        world.ledger.genesis_installed_components += remote_block.component_cost;
        world.grids.insert(
            "grid-remote".into(),
            Grid {
                grid_id: "grid-remote".into(),
                owner_player_id: "player-remote".into(),
                anchor_reward_eligible: true,
                position: Vec3::new(40.0, 3.0, -20.0),
                orientation: Quat::IDENTITY,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                control_linear_input: Vec3::ZERO,
                control_angular_input: Vec3::ZERO,
                dampeners: true,
                anchored: false,
                blocks: BTreeMap::from([(remote_block.block_id.clone(), remote_block)]),
            },
        );

        for (actor, suffix, contents, position) in [
            (
                "player-local",
                "local",
                InventoryContents {
                    ore: 2,
                    refined_material: 0,
                    components: 0,
                },
                Vec3::new(2.0, 3.0, 4.0),
            ),
            (
                "player-remote",
                "remote",
                InventoryContents {
                    ore: 0,
                    refined_material: 3,
                    components: 0,
                },
                Vec3::new(22.0, 6.0, -4.0),
            ),
        ] {
            let inventory_id = format!("inventory-drop-{suffix}");
            let drop_id = format!("drop-{suffix}");
            let death_id = format!("death-{suffix}");
            world.inventories.insert(
                inventory_id.clone(),
                InventoryRecord {
                    inventory_id: inventory_id.clone(),
                    domain: InventoryDomain::Dropped {
                        reason: "player_death".into(),
                        owner_player_id: actor.into(),
                    },
                    contents,
                    capacity_liters: 8_000,
                },
            );
            world.death_drops.insert(
                drop_id.clone(),
                DeathDrop {
                    drop_id,
                    death_id,
                    inventory_id,
                    owner_player_id: actor.into(),
                    position,
                    created_event_sequence: 10,
                    cause: PlayerDeathCause::OxygenDepleted,
                },
            );
        }
        world.ledger.genesis_ore += 2;
        world.ledger.genesis_refined += 3;

        assert!(world.validate_player_roster().is_ok());
        assert!(world.conservation().valid);
        world
    }

    fn assert_no_keys(value: &Value, forbidden: &[&str]) {
        match value {
            Value::Object(map) => {
                for (key, nested) in map {
                    assert!(!forbidden.contains(&key.as_str()), "leaked key {key}");
                    assert_no_keys(nested, forbidden);
                }
            }
            Value::Array(values) => {
                for nested in values {
                    assert_no_keys(nested, forbidden);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn two_actor_world_projections_are_disjoint_stable_and_non_mutating() {
        let world = two_actor_world();
        let before = world.clone();
        let canonical = world.snapshot();
        let local = world
            .project_world_snapshot(Some("player-local"))
            .expect("local projection");
        let remote = world
            .project_world_snapshot(Some("player-remote"))
            .expect("remote projection");

        assert_eq!(world, before, "projection must not mutate canonical state");
        assert_eq!(local.event_sequence, canonical.event_sequence);
        assert_eq!(remote.event_sequence, canonical.event_sequence);
        assert_eq!(local.world_hash, canonical.world_hash);
        assert_eq!(remote.world_hash, canonical.world_hash);
        assert_eq!(local.players, remote.players);
        assert_eq!(local.grids, remote.grids);
        assert_eq!(local.conservation_valid, canonical.conservation.valid);

        let local_private = local.actor_private.expect("local private projection");
        let remote_private = remote.actor_private.expect("remote private projection");
        assert_eq!(local_private.player.player_id, "player-local");
        assert_eq!(remote_private.player.player_id, "player-remote");
        assert_eq!(remote_private.player.experience, 275);
        assert_eq!(remote_private.player.suit_oxygen_milli, 412);

        let local_inventory_ids = local_private
            .inventories
            .iter()
            .map(|inventory| inventory.inventory_id.as_str())
            .collect::<Vec<_>>();
        let remote_inventory_ids = remote_private
            .inventories
            .iter()
            .map(|inventory| inventory.inventory_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            local_inventory_ids,
            [
                "inventory-cargo-starter",
                "inventory-drop-local",
                "inventory-player-local"
            ]
        );
        assert_eq!(
            remote_inventory_ids,
            ["inventory-drop-remote", "inventory-player-remote"]
        );
        assert_eq!(local_private.death_drops[0].drop_id, "drop-local");
        assert_eq!(remote_private.death_drops[0].drop_id, "drop-remote");
        assert_eq!(local_private.owned_grid_masses[0].grid_id, "grid-starter");
        assert_eq!(remote_private.owned_grid_masses[0].grid_id, "grid-remote");
    }

    #[test]
    fn spectator_world_json_contains_no_private_authority_or_economy_values() {
        let world = two_actor_world();
        let canonical = world.snapshot();
        let spectator = world
            .project_world_snapshot(None)
            .expect("spectator projection");
        assert_eq!(spectator.event_sequence, canonical.event_sequence);
        assert_eq!(spectator.simulation_tick, canonical.simulation_tick);
        assert_eq!(spectator.world_hash, canonical.world_hash);
        assert!(spectator.actor_private.is_none());

        let value = serde_json::to_value(&spectator).expect("projection serializes");
        assert!(value.get("actor_private").is_none());
        assert_no_keys(
            &value,
            &[
                "inventory_id",
                "inventories",
                "contents",
                "capacity_liters",
                "used_liters",
                "mass_grams",
                "mass_kg",
                "death_drops",
                "ore_sources",
                "ore_live",
                "ore_consumed",
                "refined_sources",
                "refined_live",
                "refined_consumed",
                "component_sources",
                "components_live",
                "components_installed_or_destroyed",
                "movement_epoch",
                "last_received_input_sequence",
                "last_processed_input_sequence",
                "control_linear_input",
                "control_angular_input",
                "control_expires_at_simulation_tick",
                "experience",
                "career",
                "suit_oxygen_milli",
                "critical_oxygen_milli",
            ],
        );
        assert_eq!(value["conservation_valid"], true);
    }

    #[test]
    fn motion_projection_hides_controls_and_binds_exact_actor_motion() {
        let world = two_actor_world();
        let canonical = world.motion_snapshot();
        let spectator = world
            .project_motion_snapshot(None)
            .expect("spectator motion projection");
        let remote = world
            .project_motion_snapshot(Some("player-remote"))
            .expect("remote motion projection");

        assert_eq!(spectator.event_sequence, canonical.event_sequence);
        assert_eq!(spectator.world_hash, canonical.world_hash);
        assert_eq!(spectator.players, remote.players);
        assert!(spectator.actor_private.is_none());
        let spectator_json = serde_json::to_value(&spectator).expect("motion serializes");
        assert!(spectator_json.get("actor_private").is_none());
        assert_no_keys(
            &spectator_json,
            &[
                "movement_epoch",
                "last_received_input_sequence",
                "last_processed_input_sequence",
                "control_linear_input",
                "control_angular_input",
                "boost",
                "dampeners",
                "jump",
                "control_expires_at_simulation_tick",
                "environment",
            ],
        );

        let private = remote.actor_private.expect("actor-private motion");
        assert_eq!(private.player_id, "player-remote");
        assert_eq!(private.last_received_input_sequence, 91);
        assert_eq!(private.last_processed_input_sequence, 89);
        assert_eq!(private.control_linear_input, Vec3::new(0.25, -0.5, 1.0));
        assert_eq!(private.control_angular_input, Vec3::new(-0.2, 0.1, 0.7));
        assert_eq!(
            remote
                .players
                .iter()
                .find(|player| player.player_id == "player-remote")
                .expect("remote public motion")
                .jetpack_enabled,
            private.jetpack_enabled
        );
    }

    #[test]
    fn projection_fails_closed_for_unknown_actor_or_malformed_authority() {
        let world = two_actor_world();
        assert!(matches!(
            world.project_world_snapshot(Some("player-unknown")),
            Err(ProjectionError::UnboundActor(_))
        ));

        let mut malformed = world;
        malformed
            .grids
            .get_mut("grid-starter")
            .expect("starter grid")
            .blocks
            .get_mut("block-cargo")
            .expect("cargo block")
            .inventory_id = None;
        assert!(matches!(
            malformed.project_world_snapshot(None),
            Err(ProjectionError::InvalidAuthority(_))
        ));
        assert!(matches!(
            malformed.project_motion_snapshot(Some("player-local")),
            Err(ProjectionError::InvalidAuthority(_))
        ));
    }
}
