// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use verse_protocol::{
    BlockKind, BlockSnapshot, CareerSnapshot, ConservationSnapshot, GridSnapshot, IVec3,
    InventoryContents, InventoryDomain, InventorySnapshot, PlayerSnapshot, PowerSnapshot, Vec3,
    VoxelMaterial, VoxelSnapshot, WorldSnapshot,
};

use crate::content;

pub const WORLD_SCHEMA_VERSION: u32 = 2;
pub const PLAYER_INVENTORY_ID: &str = "inventory-player-local";
pub const STARTER_GRID_ID: &str = "grid-starter";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoxelField {
    pub occupied: BTreeSet<IVec3>,
    pub ferrite_ore: BTreeSet<IVec3>,
}

impl VoxelField {
    pub fn procedural_asteroid(seed: u64, radius: i32) -> Self {
        let mut occupied = BTreeSet::new();
        let mut ferrite_ore = BTreeSet::new();
        let radius_squared = i64::from(radius) * i64::from(radius);

        for x in -radius..=radius {
            for y in -radius..=radius {
                for z in -radius..=radius {
                    let coordinate = IVec3::new(x, y, z);
                    if coordinate.squared_distance(IVec3::ZERO) > radius_squared {
                        continue;
                    }
                    occupied.insert(coordinate);
                    if deterministic_material_hash(seed, coordinate).is_multiple_of(7) {
                        ferrite_ore.insert(coordinate);
                    }
                }
            }
        }

        Self {
            occupied,
            ferrite_ore,
        }
    }

    pub fn material(&self, coordinate: IVec3) -> Option<VoxelMaterial> {
        if !self.occupied.contains(&coordinate) {
            None
        } else if self.ferrite_ore.contains(&coordinate) {
            Some(VoxelMaterial::FerriteOre)
        } else {
            Some(VoxelMaterial::Rock)
        }
    }

    pub fn remove(&mut self, coordinate: IVec3) -> Option<VoxelMaterial> {
        let material = self.material(coordinate)?;
        self.occupied.remove(&coordinate);
        self.ferrite_ore.remove(&coordinate);
        Some(material)
    }

    pub fn snapshot(&self) -> Vec<VoxelSnapshot> {
        self.occupied
            .iter()
            .map(|coordinate| VoxelSnapshot {
                coordinate: *coordinate,
                material: if self.ferrite_ore.contains(coordinate) {
                    VoxelMaterial::FerriteOre
                } else {
                    VoxelMaterial::Rock
                },
            })
            .collect()
    }
}

fn deterministic_material_hash(seed: u64, coordinate: IVec3) -> u64 {
    let mut value = seed ^ 0x9E37_79B9_7F4A_7C15;
    for part in [coordinate.x, coordinate.y, coordinate.z] {
        value ^= i64::from(part)
            .cast_unsigned()
            .wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = value.rotate_left(27).wrapping_mul(0x94D0_49BB_1331_11EB);
    }
    value ^ (value >> 31)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Player {
    pub player_id: String,
    pub position: Vec3,
    pub inventory_id: String,
    pub experience: u64,
    pub career: CareerSnapshot,
}

impl Player {
    pub fn level(&self) -> u32 {
        let mut level = 1_u32;
        let mut threshold = 100_u64;
        while self.experience >= threshold && level < 100 {
            level += 1;
            threshold += u64::from(level) * 100;
        }
        level
    }

    pub fn next_level_experience(&self) -> u64 {
        (1..=self.level()).map(|level| u64::from(level) * 100).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryRecord {
    pub inventory_id: String,
    pub domain: InventoryDomain,
    pub contents: InventoryContents,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub block_id: String,
    pub coordinate: IVec3,
    pub kind: BlockKind,
    pub health: u16,
    pub component_cost: u64,
    pub inventory_id: Option<String>,
}

impl Block {
    pub fn new(block_id: impl Into<String>, coordinate: IVec3, kind: BlockKind) -> Self {
        let definition = content::block(kind);
        Self {
            block_id: block_id.into(),
            coordinate,
            kind,
            health: definition.max_health,
            component_cost: definition.component_cost,
            inventory_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub grid_id: String,
    pub position: Vec3,
    pub yaw_radians: f64,
    pub linear_velocity: Vec3,
    pub angular_velocity: f64,
    pub anchored: bool,
    pub blocks: BTreeMap<String, Block>,
}

impl Grid {
    pub fn block_at(&self, coordinate: IVec3) -> Option<&Block> {
        self.blocks
            .values()
            .find(|block| block.coordinate == coordinate)
    }

    pub fn power(&self) -> PowerSnapshot {
        let produced = self
            .blocks
            .values()
            .map(|block| content::block(block.kind).power_production)
            .sum::<f64>();
        let required = self
            .blocks
            .values()
            .map(|block| {
                if block.kind == BlockKind::Anchor && !self.anchored {
                    0.0
                } else {
                    content::block(block.kind).power_requirement
                }
            })
            .sum::<f64>();
        let stored = self
            .blocks
            .values()
            .map(|block| content::block(block.kind).stored_power)
            .sum::<f64>();

        PowerSnapshot {
            produced,
            required,
            stored,
            online: produced + stored > 0.0 && produced + stored >= required,
        }
    }

    pub fn world_coordinate(&self, local: IVec3) -> IVec3 {
        let cos = self.yaw_radians.cos();
        let sin = self.yaw_radians.sin();
        let x = f64::from(local.x).mul_add(cos, -f64::from(local.z) * sin);
        let z = f64::from(local.x).mul_add(sin, f64::from(local.z) * cos);
        IVec3::new(
            (self.position.x + x).round() as i32,
            (self.position.y + f64::from(local.y)).round() as i32,
            (self.position.z + z).round() as i32,
        )
    }

    pub fn anchor_touches(&self, voxels: &VoxelField) -> bool {
        self.blocks.values().any(|block| {
            if block.kind != BlockKind::Anchor {
                return false;
            }
            let world = self.world_coordinate(block.coordinate);
            voxels.occupied.contains(&world)
                || world
                    .neighbors()
                    .iter()
                    .any(|neighbor| voxels.occupied.contains(neighbor))
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub genesis_ore: u64,
    pub genesis_refined: u64,
    pub genesis_components: u64,
    pub genesis_installed_components: u64,
    pub mined_ore: u64,
    pub refine_batches: u64,
    pub crafted_components: u64,
    pub built_blocks: u64,
    pub destroyed_blocks: u64,
    pub destroyed_components: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldState {
    pub schema_version: u32,
    pub content_manifest_version: String,
    pub universe_id: String,
    pub cell_id: String,
    pub world_seed: u64,
    pub event_sequence: u64,
    pub simulation_tick: u64,
    pub fencing_token: u64,
    pub last_event_hash: String,
    pub player: Player,
    pub voxels: VoxelField,
    pub grids: BTreeMap<String, Grid>,
    pub inventories: BTreeMap<String, InventoryRecord>,
    pub ledger: Ledger,
    pub processed_operations: BTreeMap<String, verse_protocol::IntentReceipt>,
}

impl WorldState {
    pub fn genesis(seed: u64) -> Self {
        let player_inventory = InventoryRecord {
            inventory_id: PLAYER_INVENTORY_ID.into(),
            domain: InventoryDomain::Player {
                player_id: "player-local".into(),
            },
            contents: InventoryContents {
                ore: 0,
                refined_material: 0,
                components: 24,
            },
        };
        let cargo_inventory_id = "inventory-cargo-starter".to_owned();
        let cargo_inventory = InventoryRecord {
            inventory_id: cargo_inventory_id.clone(),
            domain: InventoryDomain::Cargo {
                block_id: "block-cargo".into(),
            },
            contents: InventoryContents::default(),
        };

        let mut blocks = BTreeMap::new();
        blocks.insert(
            "block-core".into(),
            Block::new("block-core", IVec3::ZERO, BlockKind::ControlCore),
        );
        blocks.insert(
            "block-power".into(),
            Block::new("block-power", IVec3::new(1, 0, 0), BlockKind::PowerSource),
        );
        let mut cargo_block = Block::new("block-cargo", IVec3::new(-1, 0, 0), BlockKind::Cargo);
        cargo_block.inventory_id = Some(cargo_inventory_id.clone());
        blocks.insert(cargo_block.block_id.clone(), cargo_block);
        for (block_id, coordinate) in [
            ("block-deck-a", IVec3::new(-1, 0, -1)),
            ("block-deck-b", IVec3::new(0, 0, -1)),
            ("block-deck-c", IVec3::new(1, 0, -1)),
            ("block-deck-d", IVec3::new(-1, 0, 1)),
            ("block-deck-e", IVec3::new(0, 0, 1)),
            ("block-deck-f", IVec3::new(1, 0, 1)),
            ("block-bow-a", IVec3::new(-2, 0, -2)),
            ("block-bow-b", IVec3::new(-1, 0, -2)),
            ("block-bow-c", IVec3::new(0, 0, -2)),
            ("block-bow-d", IVec3::new(1, 0, -2)),
            ("block-bow-e", IVec3::new(2, 0, -2)),
            ("block-stern-a", IVec3::new(-2, 0, 2)),
            ("block-stern-b", IVec3::new(-1, 0, 2)),
            ("block-stern-c", IVec3::new(0, 0, 2)),
            ("block-stern-d", IVec3::new(1, 0, 2)),
            ("block-stern-e", IVec3::new(2, 0, 2)),
            ("block-port-a", IVec3::new(-2, 0, -1)),
            ("block-port-b", IVec3::new(-2, 0, 1)),
            ("block-starboard-a", IVec3::new(2, 0, -1)),
            ("block-starboard-b", IVec3::new(2, 0, 1)),
        ] {
            blocks.insert(
                block_id.into(),
                Block::new(block_id, coordinate, BlockKind::Structural),
            );
        }
        blocks.insert(
            "block-battery".into(),
            Block::new("block-battery", IVec3::new(1, 1, 0), BlockKind::Battery),
        );
        blocks.insert(
            "block-drill".into(),
            Block::new("block-drill", IVec3::new(2, 0, 0), BlockKind::Drill),
        );

        let grid = Grid {
            grid_id: STARTER_GRID_ID.into(),
            position: Vec3::new(10.0, 0.0, 0.0),
            yaw_radians: 0.0,
            linear_velocity: Vec3::ZERO,
            angular_velocity: 0.0,
            anchored: false,
            blocks,
        };

        Self {
            schema_version: WORLD_SCHEMA_VERSION,
            content_manifest_version: content::manifest().manifest_version.clone(),
            universe_id: "the-verse-local".into(),
            cell_id: "cell-origin".into(),
            world_seed: seed,
            event_sequence: 0,
            simulation_tick: 0,
            fencing_token: 0,
            last_event_hash: String::new(),
            player: Player {
                player_id: "player-local".into(),
                position: Vec3::new(10.0, 3.0, 8.0),
                inventory_id: PLAYER_INVENTORY_ID.into(),
                experience: 0,
                career: CareerSnapshot::default(),
            },
            voxels: VoxelField::procedural_asteroid(seed, 8),
            grids: BTreeMap::from([(STARTER_GRID_ID.into(), grid)]),
            inventories: BTreeMap::from([
                (PLAYER_INVENTORY_ID.into(), player_inventory),
                (cargo_inventory_id, cargo_inventory),
            ]),
            ledger: Ledger {
                genesis_components: 24,
                genesis_installed_components: 25,
                ..Ledger::default()
            },
            processed_operations: BTreeMap::new(),
        }
    }

    pub fn state_hash(&self) -> String {
        // A replacement worker must acquire a newer fencing token, but that
        // operational lease change does not alter the canonical world. Keeping
        // it out of the aggregate hash lets recovery prove gameplay identity.
        let mut canonical = self.clone();
        canonical.fencing_token = 0;
        let bytes = serde_json::to_vec(&canonical).expect("WorldState serialization cannot fail");
        blake3::hash(&bytes).to_hex().to_string()
    }

    pub fn conservation(&self) -> ConservationSnapshot {
        let (ore_live, refined_live, components_live) = self.inventories.values().fold(
            (0_u64, 0_u64, 0_u64),
            |(ore, refined, components), inventory| {
                (
                    ore + inventory.contents.ore,
                    refined + inventory.contents.refined_material,
                    components + inventory.contents.components,
                )
            },
        );
        let live_blocks = self
            .grids
            .values()
            .flat_map(|grid| grid.blocks.values())
            .map(|block| block.component_cost)
            .sum::<u64>();
        let recipes = &content::manifest().recipes;
        let ore_sources = self.ledger.genesis_ore + self.ledger.mined_ore;
        let ore_consumed = self.ledger.refine_batches * recipes.refining.ore_input;
        let refined_sources = self.ledger.genesis_refined
            + self.ledger.refine_batches * recipes.refining.refined_output;
        let refined_consumed =
            self.ledger.crafted_components * recipes.component_crafting.refined_input;
        let component_sources = self.ledger.genesis_components
            + self.ledger.genesis_installed_components
            + self.ledger.crafted_components * recipes.component_crafting.component_output;
        let components_installed_or_destroyed = live_blocks + self.ledger.destroyed_components;

        ConservationSnapshot {
            ore_sources,
            ore_live,
            ore_consumed,
            refined_sources,
            refined_live,
            refined_consumed,
            component_sources,
            components_live,
            components_installed_or_destroyed,
            valid: ore_sources == ore_live + ore_consumed
                && refined_sources == refined_live + refined_consumed
                && component_sources == components_live + components_installed_or_destroyed,
        }
    }

    pub fn snapshot(&self) -> WorldSnapshot {
        let mut grids = self
            .grids
            .values()
            .map(|grid| GridSnapshot {
                grid_id: grid.grid_id.clone(),
                position: grid.position,
                yaw_radians: grid.yaw_radians,
                linear_velocity: grid.linear_velocity,
                angular_velocity: grid.angular_velocity,
                anchored: grid.anchored,
                power: grid.power(),
                blocks: grid
                    .blocks
                    .values()
                    .map(|block| BlockSnapshot {
                        block_id: block.block_id.clone(),
                        coordinate: block.coordinate,
                        kind: block.kind,
                        health: block.health,
                        inventory_id: block.inventory_id.clone(),
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        grids.sort_by(|left, right| left.grid_id.cmp(&right.grid_id));

        WorldSnapshot {
            schema_version: self.schema_version,
            content_manifest_version: self.content_manifest_version.clone(),
            universe_id: self.universe_id.clone(),
            cell_id: self.cell_id.clone(),
            event_sequence: self.event_sequence,
            simulation_tick: self.simulation_tick,
            fencing_token: self.fencing_token,
            world_hash: self.state_hash(),
            player: PlayerSnapshot {
                player_id: self.player.player_id.clone(),
                position: self.player.position,
                inventory_id: self.player.inventory_id.clone(),
                experience: self.player.experience,
                level: self.player.level(),
                next_level_experience: self.player.next_level_experience(),
                career: self.player.career.clone(),
            },
            voxels: self.voxels.snapshot(),
            grids,
            inventories: self
                .inventories
                .values()
                .map(|inventory| InventorySnapshot {
                    inventory_id: inventory.inventory_id.clone(),
                    domain: inventory.domain.clone(),
                    contents: inventory.contents.clone(),
                })
                .collect(),
            conservation: self.conservation(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_asteroid_is_deterministic() {
        assert_eq!(
            VoxelField::procedural_asteroid(42, 8),
            VoxelField::procedural_asteroid(42, 8)
        );
        assert_ne!(
            VoxelField::procedural_asteroid(42, 8).ferrite_ore,
            VoxelField::procedural_asteroid(43, 8).ferrite_ore
        );
    }

    #[test]
    fn genesis_is_conserved_and_playable() {
        let world = WorldState::genesis(7);
        assert!(world.conservation().valid);
        assert_eq!(world.grids[STARTER_GRID_ID].blocks.len(), 25);
        assert!(world.grids[STARTER_GRID_ID].power().online);
        assert!(world.voxels.occupied.len() > 1_000);
    }
}
