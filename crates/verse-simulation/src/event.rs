// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use verse_protocol::{
    IVec3, InventoryContents, PlayerDeathCause, PlayerLocomotionSnapshot, Quat, ResourceKind,
    UniverseAddress, Vec3, VoxelMaterial,
};

use crate::celestial;
use crate::model::{Block, ContactPairKey, DeathDrop, InventoryRecord, ProductionJob};

pub const EVENT_SCHEMA_NAME: &str = "verse.world_event";
pub const EVENT_SCHEMA_VERSION: u32 = 15;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case", deny_unknown_fields)]
// Stable event variants intentionally carry complete authoritative outcomes;
// boxing them would complicate the versioned replay API without changing JSON.
#[allow(clippy::large_enum_variant)]
pub enum EventPayload {
    PlayerControlSet {
        movement_epoch: u64,
        input_sequence: u64,
        linear_input: Vec3,
        angular_input: Vec3,
        boost: bool,
        dampeners: bool,
        jump: bool,
        expires_at_simulation_tick: u64,
    },
    SuitModeChanged {
        helmet_closed: bool,
        jetpack_enabled: bool,
        magnetic_boots_enabled: bool,
    },
    SuitOxygenChanged {
        player_id: String,
        previous_oxygen_milli: u16,
        new_oxygen_milli: u16,
    },
    PlayerIncapacitated {
        player_id: String,
        death_id: String,
        cause: PlayerDeathCause,
        address: UniverseAddress,
        #[serde(skip, default)]
        position: Vec3,
        previous_oxygen_milli: u16,
        dropped_inventory: Option<InventoryRecord>,
        death_drop: Option<DeathDrop>,
    },
    PlayerRespawned {
        death_id: String,
        address: UniverseAddress,
        #[serde(skip, default)]
        position: Vec3,
        suit_oxygen_milli: u16,
        helmet_closed: bool,
        jetpack_enabled: bool,
        magnetic_boots_enabled: bool,
    },
    VoxelMined {
        coordinate: IVec3,
        material: VoxelMaterial,
        ore_yield: u64,
        inventory_id: String,
    },
    OreRefined {
        inventory_id: String,
        batches: u64,
    },
    ComponentCrafted {
        inventory_id: String,
        quantity: u64,
    },
    ProductionQueued {
        job: ProductionJob,
    },
    ProductionQuantumCommitted {
        occurrence: ProductionScheduleOccurrence,
        elapsed_ticks: u64,
        outcomes: Vec<ProductionMachineOutcome>,
    },
    InventoryTransferred {
        source_inventory_id: String,
        destination_inventory_id: String,
        resource: ResourceKind,
        quantity: u64,
    },
    BlockBuilt {
        grid_id: String,
        component_inventory_id: String,
        block: Block,
    },
    BlockWelded {
        grid_id: String,
        block_id: String,
        previous_health: u16,
        new_health: u16,
        max_health: u16,
        completed_construction: bool,
    },
    GridControlSet {
        grid_id: String,
        linear_input: Vec3,
        angular_input: Vec3,
        dampeners: bool,
    },
    GridAnchorSet {
        grid_id: String,
        anchored: bool,
        reward_credited: bool,
    },
    BlockDamaged {
        grid_id: String,
        block_id: String,
        damage: u16,
    },
    PhysicsStepCommitted {
        fixed_step_hz: u16,
        step_count: u8,
        remaining_step_phase: u32,
        bodies: Vec<PhysicsBodyOutcome>,
        players: Vec<PlayerPhysicsOutcome>,
        contacts: Vec<PhysicsContactOutcome>,
        active_contacts_after: Vec<ContactPairKey>,
    },
}

pub const PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION: u32 =
    verse_protocol::PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionScheduleOccurrence {
    pub schema_version: u32,
    pub universe_id: String,
    pub cell_id: String,
    pub lifecycle_generation: u64,
    pub production_quantum_sequence: u64,
    pub scheduled_for_unix_ms: u64,
    pub universe_manifest_hash: String,
    pub celestial_registry_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionMachineOutcomeKind {
    Advanced,
    Completed,
    CompletedAndDelivered,
    OutputDelivered,
    PausedPower,
    PausedRoute,
    PausedMachine,
    OutputBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionMachineOutcome {
    pub grid_id: String,
    pub machine_block_id: String,
    pub job_id: String,
    pub kind: ProductionMachineOutcomeKind,
    pub previous_progress_ticks: u64,
    pub new_progress_ticks: u64,
    pub destination_inventory_id: String,
    pub outputs: InventoryContents,
}

impl ProductionMachineOutcome {
    pub fn changes_state(&self) -> bool {
        matches!(
            self.kind,
            ProductionMachineOutcomeKind::Advanced
                | ProductionMachineOutcomeKind::Completed
                | ProductionMachineOutcomeKind::CompletedAndDelivered
                | ProductionMachineOutcomeKind::OutputDelivered
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
// These independent booleans are explicit fields in the versioned event
// schema; combining them into an enum would remove valid input combinations.
#[allow(clippy::struct_excessive_bools)]
pub struct PlayerPhysicsOutcome {
    pub player_id: String,
    pub address: UniverseAddress,
    #[serde(skip, default)]
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub surface_contact: bool,
    pub locomotion: PlayerLocomotionSnapshot,
    pub control_linear_input: Vec3,
    pub control_angular_input: Vec3,
    pub boost: bool,
    pub dampeners: bool,
    pub jump: bool,
    pub control_expires_at_simulation_tick: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicsBodyOutcome {
    pub grid_id: String,
    pub address: UniverseAddress,
    #[serde(skip, default)]
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicsContactOutcome {
    pub substep_index: u8,
    pub body_a_id: String,
    pub collider_a_id: String,
    pub body_b_id: String,
    pub collider_b_id: String,
    pub point_address: UniverseAddress,
    #[serde(skip, default)]
    pub point: Vec3,
    pub normal: Vec3,
    pub penetration_m: f64,
    pub closing_speed_mm_per_second: u64,
    pub estimated_normal_impulse_millinewton_seconds: u64,
    pub reduced_translational_mass_grams: u64,
    pub phase: PhysicsContactPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicsContactPhase {
    Began,
    Persisted,
}

impl EventPayload {
    pub fn hydrate_spatial_poses(&mut self, cell_address: &UniverseAddress) -> Result<(), String> {
        let hydrate = |address: &UniverseAddress| {
            celestial::local_position_from_address(cell_address, address)
                .map_err(|source| format!("event spatial address is invalid: {source}"))
        };
        match self {
            Self::PlayerIncapacitated {
                address,
                position,
                death_drop,
                ..
            } => {
                *position = hydrate(address)?;
                if let Some(drop) = death_drop {
                    drop.position = hydrate(&drop.address)?;
                }
            }
            Self::PlayerRespawned {
                address, position, ..
            } => *position = hydrate(address)?,
            Self::PhysicsStepCommitted {
                bodies,
                players,
                contacts,
                ..
            } => {
                for body in bodies {
                    body.position = hydrate(&body.address)?;
                }
                for player in players {
                    player.position = hydrate(&player.address)?;
                }
                for contact in contacts {
                    contact.point = hydrate(&contact.point_address)?;
                }
            }
            Self::PlayerControlSet { .. }
            | Self::SuitModeChanged { .. }
            | Self::SuitOxygenChanged { .. }
            | Self::VoxelMined { .. }
            | Self::OreRefined { .. }
            | Self::ComponentCrafted { .. }
            | Self::ProductionQueued { .. }
            | Self::ProductionQuantumCommitted { .. }
            | Self::InventoryTransferred { .. }
            | Self::BlockBuilt { .. }
            | Self::BlockWelded { .. }
            | Self::GridControlSet { .. }
            | Self::GridAnchorSet { .. }
            | Self::BlockDamaged { .. } => {}
        }
        Ok(())
    }
}

impl EventPayload {
    pub fn experience_reward(&self) -> u64 {
        let rewards = &crate::content::manifest().experience_rewards;
        match self {
            Self::VoxelMined { ore_yield, .. } => ore_yield.saturating_mul(rewards.mined_ore_unit),
            Self::OreRefined { batches, .. } => batches.saturating_mul(rewards.refining_batch),
            Self::ComponentCrafted { quantity, .. } => {
                quantity.saturating_mul(rewards.crafted_component)
            }
            Self::InventoryTransferred { .. } => rewards.inventory_transfer,
            Self::BlockBuilt { .. } => rewards.frame_placed,
            Self::BlockWelded {
                completed_construction,
                ..
            } => {
                if *completed_construction {
                    rewards.construction_completed
                } else {
                    rewards.weld_progress_or_repair
                }
            }
            Self::GridAnchorSet {
                reward_credited: true,
                ..
            } => rewards.first_anchor_engagement,
            Self::BlockDamaged { .. } => rewards.block_damage,
            Self::PlayerControlSet { .. }
            | Self::SuitModeChanged { .. }
            | Self::SuitOxygenChanged { .. }
            | Self::PlayerIncapacitated { .. }
            | Self::PlayerRespawned { .. }
            | Self::ProductionQueued { .. }
            | Self::ProductionQuantumCommitted { .. }
            | Self::GridControlSet { .. }
            | Self::GridAnchorSet { .. }
            | Self::PhysicsStepCommitted { .. } => 0,
        }
    }

    pub fn receipt(&self) -> (&'static str, String) {
        match self {
            Self::PlayerControlSet { input_sequence, .. } => (
                "player_control_set",
                format!("Character control {input_sequence} accepted"),
            ),
            Self::SuitModeChanged {
                helmet_closed,
                jetpack_enabled,
                magnetic_boots_enabled,
            } => (
                "suit_mode_changed",
                format!(
                    "Helmet {} — jetpack {} — magnetic boots {}",
                    if *helmet_closed { "sealed" } else { "open" },
                    if *jetpack_enabled {
                        "online"
                    } else {
                        "offline"
                    },
                    if *magnetic_boots_enabled {
                        "armed"
                    } else {
                        "off"
                    },
                ),
            ),
            Self::SuitOxygenChanged {
                new_oxygen_milli, ..
            } => (
                "suit_oxygen_changed",
                format!("Suit oxygen {}%", u32::from(*new_oxygen_milli) / 10),
            ),
            Self::PlayerIncapacitated { .. } => (
                "player_incapacitated",
                "Life support failed — recovery required".into(),
            ),
            Self::PlayerRespawned { .. } => (
                "player_respawned",
                "Recovery complete — suit inventory is empty".into(),
            ),
            Self::VoxelMined { ore_yield, .. } => ("voxel_mined", format!("Mined {ore_yield} ore")),
            Self::OreRefined { batches, .. } => (
                "ore_refined",
                format!("Refined {batches} batch(es) into {batches} material"),
            ),
            Self::ComponentCrafted { quantity, .. } => (
                "component_crafted",
                format!("Crafted {quantity} component(s)"),
            ),
            Self::ProductionQueued { job } => (
                "production_queued",
                format!("Queued {:?} batch(es): {}", job.recipe, job.batches),
            ),
            Self::ProductionQuantumCommitted { outcomes, .. } => {
                let completed = outcomes.iter().filter(|outcome| {
                    matches!(
                        outcome.kind,
                        ProductionMachineOutcomeKind::Completed
                            | ProductionMachineOutcomeKind::CompletedAndDelivered
                    )
                });
                if completed.count() > 0 {
                    ("production_completed", "Machine work completed".into())
                } else {
                    ("production_advanced", "Machine work advanced".into())
                }
            }
            Self::InventoryTransferred {
                resource, quantity, ..
            } => (
                "inventory_transferred",
                format!("Transferred {quantity} {resource:?}"),
            ),
            Self::BlockBuilt { block, .. } => (
                "block_frame_placed",
                format!("Placed {:?} frame", block.kind),
            ),
            Self::BlockWelded {
                new_health,
                max_health,
                ..
            } => {
                let percent = u32::from(*new_health) * 100 / u32::from(*max_health);
                (
                    "block_welded",
                    format!("Welded block to {percent}% integrity"),
                )
            }
            Self::GridControlSet { .. } => ("grid_control_set", "Grid control accepted".into()),
            Self::GridAnchorSet { anchored, .. } => (
                "grid_anchor_set",
                if *anchored {
                    "Grid anchored".into()
                } else {
                    "Grid released".into()
                },
            ),
            Self::BlockDamaged { .. } => ("block_damaged", "Damage applied".into()),
            Self::PhysicsStepCommitted { .. } => {
                ("physics_step_committed", "Physics step committed".into())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalEvent {
    pub schema_name: String,
    pub schema_version: u32,
    pub content_manifest_version: String,
    pub universe_manifest_hash: String,
    pub celestial_registry_hash: String,
    pub event_id: String,
    pub event_sequence: u64,
    pub occurred_at_unix_ms: u64,
    pub universe_id: String,
    pub cell_id: String,
    pub authority_fencing_token: u64,
    /// Canonical player actor for human mutations. System events have no
    /// player actor; credentials and connection identity never enter replay.
    pub actor_player_id: Option<String>,
    pub actor_type: String,
    pub operation_id: Option<String>,
    pub operation_sequence: Option<u64>,
    pub intent_fingerprint: Option<String>,
    pub previous_event_hash: String,
    pub payload: EventPayload,
    pub event_hash: String,
}

#[derive(Serialize)]
struct EventHashMaterial<'a> {
    schema_name: &'a str,
    schema_version: u32,
    content_manifest_version: &'a str,
    universe_manifest_hash: &'a str,
    celestial_registry_hash: &'a str,
    event_id: &'a str,
    event_sequence: u64,
    occurred_at_unix_ms: u64,
    universe_id: &'a str,
    cell_id: &'a str,
    authority_fencing_token: u64,
    actor_player_id: &'a Option<String>,
    actor_type: &'a str,
    operation_id: &'a Option<String>,
    operation_sequence: &'a Option<u64>,
    intent_fingerprint: &'a Option<String>,
    previous_event_hash: &'a str,
    payload: &'a EventPayload,
}

impl CanonicalEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_sequence: u64,
        content_manifest_version: impl Into<String>,
        universe_manifest_hash: impl Into<String>,
        celestial_registry_hash: impl Into<String>,
        universe_id: impl Into<String>,
        cell_id: impl Into<String>,
        authority_fencing_token: u64,
        actor_player_id: Option<String>,
        actor_type: impl Into<String>,
        operation_id: Option<String>,
        operation_sequence: Option<u64>,
        intent_fingerprint: Option<String>,
        previous_event_hash: impl Into<String>,
        payload: EventPayload,
    ) -> Self {
        let occurred_at_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        let mut event = Self {
            schema_name: EVENT_SCHEMA_NAME.into(),
            schema_version: EVENT_SCHEMA_VERSION,
            content_manifest_version: content_manifest_version.into(),
            universe_manifest_hash: universe_manifest_hash.into(),
            celestial_registry_hash: celestial_registry_hash.into(),
            event_id: Uuid::new_v4().to_string(),
            event_sequence,
            occurred_at_unix_ms,
            universe_id: universe_id.into(),
            cell_id: cell_id.into(),
            authority_fencing_token,
            actor_player_id,
            actor_type: actor_type.into(),
            operation_id,
            operation_sequence,
            intent_fingerprint,
            previous_event_hash: previous_event_hash.into(),
            payload,
            event_hash: String::new(),
        };
        event.event_hash = event.calculate_hash();
        event
    }

    pub fn calculate_hash(&self) -> String {
        let material = EventHashMaterial {
            schema_name: &self.schema_name,
            schema_version: self.schema_version,
            content_manifest_version: &self.content_manifest_version,
            universe_manifest_hash: &self.universe_manifest_hash,
            celestial_registry_hash: &self.celestial_registry_hash,
            event_id: &self.event_id,
            event_sequence: self.event_sequence,
            occurred_at_unix_ms: self.occurred_at_unix_ms,
            universe_id: &self.universe_id,
            cell_id: &self.cell_id,
            authority_fencing_token: self.authority_fencing_token,
            actor_player_id: &self.actor_player_id,
            actor_type: &self.actor_type,
            operation_id: &self.operation_id,
            operation_sequence: &self.operation_sequence,
            intent_fingerprint: &self.intent_fingerprint,
            previous_event_hash: &self.previous_event_hash,
            payload: &self.payload,
        };
        let bytes = serde_json::to_vec(&material).expect("event hash material serializes");
        blake3::hash(&bytes).to_hex().to_string()
    }

    pub fn hash_is_valid(&self) -> bool {
        self.event_hash == self.calculate_hash()
    }

    pub(crate) fn retime_and_rehash(&mut self, occurred_at_unix_ms: u64) {
        self.occurred_at_unix_ms = occurred_at_unix_ms;
        self.event_hash = self.calculate_hash();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_hash_detects_payload_tampering() {
        let mut event = CanonicalEvent::new(
            1,
            "p1.5.0",
            "1".repeat(64),
            "2".repeat(64),
            "universe",
            "cell",
            9,
            Some("player".into()),
            "human",
            Some("op-1".into()),
            Some(1),
            Some("0".repeat(64)),
            "",
            EventPayload::PlayerControlSet {
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
                expires_at_simulation_tick: 18,
            },
        );
        assert!(event.hash_is_valid());
        event.payload = EventPayload::PlayerControlSet {
            movement_epoch: 1,
            input_sequence: 1,
            linear_input: Vec3::new(0.0, 1.0, 0.0),
            angular_input: Vec3::ZERO,
            boost: false,
            jump: false,
            dampeners: true,
            expires_at_simulation_tick: 18,
        };
        assert!(!event.hash_is_valid());
    }

    #[test]
    fn event_hash_detects_universe_binding_tampering() {
        let event = CanonicalEvent::new(
            1,
            "p1.5.0",
            "1".repeat(64),
            "2".repeat(64),
            "universe",
            "cell",
            9,
            None,
            "system",
            None,
            None,
            None,
            "",
            EventPayload::SuitOxygenChanged {
                player_id: "player".into(),
                previous_oxygen_milli: 1_000,
                new_oxygen_milli: 995,
            },
        );
        assert!(event.hash_is_valid());

        let mut universe_tampered = event.clone();
        universe_tampered.universe_manifest_hash = "3".repeat(64);
        assert!(!universe_tampered.hash_is_valid());

        let mut registry_tampered = event;
        registry_tampered.celestial_registry_hash = "4".repeat(64);
        assert!(!registry_tampered.hash_is_valid());
    }

    #[test]
    fn canonical_spatial_event_hashes_exact_addresses_and_omits_derived_poses() {
        let origin = celestial::cell_origin_address();
        let address = celestial::address_from_origin_offset_um(&origin, [1_234_567, -8_765_432, 9])
            .expect("event address canonicalizes");
        let position = celestial::local_position_from_address(&origin, &address)
            .expect("event address hydrates");
        let payload = EventPayload::PhysicsStepCommitted {
            fixed_step_hz: 60,
            step_count: 1,
            remaining_step_phase: 0,
            bodies: vec![PhysicsBodyOutcome {
                grid_id: "grid-spatial".into(),
                address: address.clone(),
                position,
                orientation: Quat::IDENTITY,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
            }],
            players: Vec::new(),
            contacts: vec![PhysicsContactOutcome {
                substep_index: 0,
                body_a_id: "grid-spatial".into(),
                collider_a_id: "block-spatial".into(),
                body_b_id: "voxel".into(),
                collider_b_id: "voxel-spatial".into(),
                point_address: address.clone(),
                point: position,
                normal: Vec3::new(1.0, 0.0, 0.0),
                penetration_m: 0.0,
                closing_speed_mm_per_second: 0,
                estimated_normal_impulse_millinewton_seconds: 0,
                reduced_translational_mass_grams: 1,
                phase: PhysicsContactPhase::Began,
            }],
            active_contacts_after: Vec::new(),
        };
        let event = CanonicalEvent::new(
            1,
            "p1.5.0",
            "1".repeat(64),
            "2".repeat(64),
            origin.universe_id.clone(),
            "cell-origin",
            9,
            None,
            "system",
            None,
            None,
            None,
            "",
            payload,
        );
        let encoded = serde_json::to_value(&event).expect("event serializes");
        let body = &encoded["payload"]["bodies"][0];
        let contact = &encoded["payload"]["contacts"][0];
        assert_eq!(body["address"], serde_json::to_value(&address).unwrap());
        assert!(body.get("position").is_none());
        assert_eq!(
            contact["point_address"],
            serde_json::to_value(&address).unwrap()
        );
        assert!(contact.get("point").is_none());

        let mut decoded = serde_json::from_value::<CanonicalEvent>(encoded.clone())
            .expect("canonical spatial event deserializes");
        assert!(decoded.hash_is_valid());
        let hash = decoded.event_hash.clone();
        decoded
            .payload
            .hydrate_spatial_poses(&origin)
            .expect("event poses hydrate from exact addresses");
        let EventPayload::PhysicsStepCommitted {
            bodies, contacts, ..
        } = &mut decoded.payload
        else {
            unreachable!();
        };
        assert_eq!(bodies[0].position, position);
        assert_eq!(contacts[0].point, position);
        bodies[0].position.x += 100.0;
        contacts[0].point.y -= 100.0;
        assert_eq!(decoded.calculate_hash(), hash);
        assert_eq!(
            serde_json::to_value(decoded).expect("hydrated event remains canonical"),
            encoded
        );
    }

    #[test]
    fn p11_rewards_close_repeatable_work_loops() {
        let transfer = EventPayload::InventoryTransferred {
            source_inventory_id: "suit".into(),
            destination_inventory_id: "cargo".into(),
            resource: ResourceKind::Ore,
            quantity: 1,
        };
        let repair = EventPayload::BlockWelded {
            grid_id: "grid".into(),
            block_id: "block".into(),
            previous_health: 50,
            new_health: 75,
            max_health: 100,
            completed_construction: false,
        };
        let damage = EventPayload::BlockDamaged {
            grid_id: "grid".into(),
            block_id: "block".into(),
            damage: 35,
        };
        let anchor = EventPayload::GridAnchorSet {
            grid_id: "grid".into(),
            anchored: true,
            reward_credited: true,
        };
        let repeated_anchor = EventPayload::GridAnchorSet {
            grid_id: "grid".into(),
            anchored: true,
            reward_credited: false,
        };

        assert_eq!(transfer.experience_reward(), 0);
        assert_eq!(repair.experience_reward(), 0);
        assert_eq!(damage.experience_reward(), 0);
        assert_eq!(anchor.experience_reward(), 40);
        assert_eq!(repeated_anchor.experience_reward(), 0);
    }
}
