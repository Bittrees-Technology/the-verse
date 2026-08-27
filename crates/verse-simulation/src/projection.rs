// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deterministic, actor-aware interest and privacy projections.
//!
//! Interest membership is connection-local delivery state. It is deliberately
//! held outside [`WorldState`], so replay and canonical hashing never depend on
//! which clients happen to be connected.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde::ser::{
    Error as SerdeError, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant, Serializer,
};
use serde_json::Value;
use thiserror::Error;
use verse_protocol::{
    ActorPrivateSnapshot, BlockKind, BlockSnapshot, DeathDropSnapshot, EnvironmentSnapshot,
    GridMotionSnapshot, GridSnapshot, INTEREST_SCHEMA_VERSION, InterestEntityKind,
    InterestEntityPayload, InterestEntityProjection, InterestEntityRef, InterestFrameKind,
    InterestObserverClass, InterestRemoval, InterestRemovalReason, InterestSnapshot,
    InventoryDomain, InventorySnapshot, OwnedGridMassSnapshot, PROJECTION_SCHEMA_VERSION,
    PlayerLifeState, PlayerMotionSnapshot, PlayerSnapshot, ProductionJobSnapshot,
    ProductionJobStatus, ProductionQueueSnapshot, ProjectedInterestDelta, ProjectedMotionSnapshot,
    ProjectedWorldSnapshot, PublicBlockSnapshot, PublicDeathDropSnapshot, PublicGridMotionSnapshot,
    PublicGridSnapshot, PublicMachineState, PublicPlayerLifeState, PublicPlayerMotionSnapshot,
    PublicPlayerSnapshot, PublicVoxelChunkSnapshot, UniverseAddress, Vec3, VoxelSnapshot,
};

use crate::{celestial, content, model::WorldState};

const FIXED_SCALE: f64 = 1_000_000.0;
// `i64::MAX as f64` rounds up to 2^63, so this must be an exclusive bound.
const I64_UPPER_EXCLUSIVE: f64 = 9_223_372_036_854_775_808.0;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectionError {
    #[error("canonical authority graph is invalid: {0}")]
    InvalidAuthority(String),
    #[error("actor {0} is not bound to a canonical player")]
    UnboundActor(String),
    #[error("interest session is invalid: {0}")]
    InvalidSession(String),
    #[error("canonical projection invariant failed: {0}")]
    InvalidCanonicalSnapshot(String),
}

/// P1.5 observer authority. No caller-provided camera position or radius can be
/// represented here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterestObserver {
    BoundPlayer { player_id: String },
    PublicOriginSpectator,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InterestEntityIdentity {
    pub entity_id: String,
    pub kind: InterestEntityKind,
}

impl InterestEntityIdentity {
    fn new(kind: InterestEntityKind, entity_id: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Membership {
    projected_revision: u64,
    content_fingerprint: u64,
    outside_since_tick: Option<u64>,
}

/// A worker owns one cursor per authenticated connection.
#[derive(Debug, Clone, PartialEq)]
pub struct InterestProjectionState {
    session_epoch: String,
    interest_epoch: u64,
    baseline_id: String,
    delta_sequence: u64,
    observer: InterestObserver,
    members: BTreeMap<InterestEntityIdentity, Membership>,
    payloads: BTreeMap<InterestEntityIdentity, CandidatePayload>,
    view_hash: Option<String>,
    last_evaluated_tick: Option<u64>,
    environment: Option<EnvironmentSnapshot>,
    actor_private: Option<ActorPrivateSnapshot>,
}

impl InterestProjectionState {
    #[must_use]
    pub fn bound_player(session_epoch: impl Into<String>, player_id: impl Into<String>) -> Self {
        Self::new(
            session_epoch.into(),
            InterestObserver::BoundPlayer {
                player_id: player_id.into(),
            },
        )
    }

    #[must_use]
    pub fn public_origin_spectator(session_epoch: impl Into<String>) -> Self {
        Self::new(
            session_epoch.into(),
            InterestObserver::PublicOriginSpectator,
        )
    }

    fn new(session_epoch: String, observer: InterestObserver) -> Self {
        let baseline_id = digest(format!("interest-baseline-v1\0{session_epoch}\01").as_bytes());
        Self {
            session_epoch,
            interest_epoch: 1,
            baseline_id,
            delta_sequence: 0,
            observer,
            members: BTreeMap::new(),
            payloads: BTreeMap::new(),
            view_hash: None,
            last_evaluated_tick: None,
            environment: None,
            actor_private: None,
        }
    }

    #[must_use]
    pub fn observer(&self) -> &InterestObserver {
        &self.observer
    }

    #[must_use]
    pub fn visible_entity_count(&self) -> usize {
        self.members.len()
    }

    #[must_use]
    pub fn session_epoch(&self) -> &str {
        &self.session_epoch
    }

    #[must_use]
    pub const fn interest_epoch(&self) -> u64 {
        self.interest_epoch
    }

    #[must_use]
    pub fn baseline_id(&self) -> &str {
        &self.baseline_id
    }

    #[must_use]
    pub const fn delta_sequence(&self) -> u64 {
        self.delta_sequence
    }

    #[must_use]
    pub fn view_hash(&self) -> Option<&str> {
        self.view_hash.as_deref()
    }

    /// Invalidates the prior baseline after an explicit resync decision. A
    /// worker can project into a clone and install that clone only after ack,
    /// keeping the acknowledged cursor immutable while a frame is in flight.
    pub fn fresh_baseline(&mut self) -> Result<(), ProjectionError> {
        self.interest_epoch = self
            .interest_epoch
            .checked_add(1)
            .ok_or_else(|| ProjectionError::InvalidSession("interest epoch exhausted".into()))?;
        self.baseline_id = digest(
            format!(
                "interest-baseline-v1\0{}\0{}",
                self.session_epoch, self.interest_epoch
            )
            .as_bytes(),
        );
        self.delta_sequence = 0;
        self.members.clear();
        self.payloads.clear();
        self.view_hash = None;
        self.last_evaluated_tick = None;
        self.environment = None;
        self.actor_private = None;
        Ok(())
    }

    fn contains(&self, kind: InterestEntityKind, id: &str) -> bool {
        self.members
            .contains_key(&InterestEntityIdentity::new(kind, id))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum CandidatePayload {
    Player(PublicPlayerSnapshot),
    Grid(PublicGridSnapshot),
    VoxelChunk(PublicVoxelChunkSnapshot),
    DeathDrop(PublicDeathDropSnapshot),
}

impl CandidatePayload {
    fn identity(&self) -> InterestEntityIdentity {
        match self {
            Self::Player(value) => {
                InterestEntityIdentity::new(InterestEntityKind::Player, &value.player_id)
            }
            Self::Grid(value) => {
                InterestEntityIdentity::new(InterestEntityKind::Grid, &value.grid_id)
            }
            Self::VoxelChunk(value) => {
                InterestEntityIdentity::new(InterestEntityKind::VoxelChunk, &value.chunk_id)
            }
            Self::DeathDrop(value) => {
                InterestEntityIdentity::new(InterestEntityKind::DeathDrop, &value.drop_id)
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    address: UniverseAddress,
    content_fingerprint: u64,
    control_critical: bool,
    payload: CandidatePayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SpatialBucketKey {
    x: i128,
    y: i128,
    z: i128,
}

/// Immutable, prebuilt public projection material for one authoritative world
/// revision. Owning the canonical snapshot and indices lets a worker share the
/// source across sessions and project outside the authoritative runtime lock.
pub struct ProjectionSource {
    canonical: verse_protocol::WorldSnapshot,
    candidates: BTreeMap<InterestEntityIdentity, Candidate>,
    spatial_buckets: BTreeMap<SpatialBucketKey, Vec<InterestEntityIdentity>>,
    support_entities: BTreeMap<String, InterestEntityIdentity>,
    actor_private: BTreeMap<String, ActorPrivateSnapshot>,
    block_grids: BTreeMap<String, String>,
    spectator_anchor: UniverseAddress,
    spectator_environment: verse_protocol::EnvironmentSnapshot,
}

impl std::fmt::Debug for ProjectionSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProjectionSource")
            .field("universe_id", &self.canonical.universe_id)
            .field("cell_id", &self.canonical.cell_id)
            .field("event_sequence", &self.canonical.event_sequence)
            .field("simulation_tick", &self.canonical.simulation_tick)
            .field("world_hash", &self.canonical.world_hash)
            .finish_non_exhaustive()
    }
}

struct CandidateQuery {
    candidates: BTreeMap<InterestEntityIdentity, Candidate>,
    #[cfg(test)]
    stats: CandidateQueryStats,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CandidateQueryStats {
    bucket_lookups: usize,
    spatial_candidates_visited: usize,
    exact_candidates_visited: usize,
}

struct Selection {
    members: BTreeMap<InterestEntityIdentity, Membership>,
    payloads: BTreeMap<InterestEntityIdentity, CandidatePayload>,
    entered: Vec<InterestEntityRef>,
    replaced: Vec<InterestEntityRef>,
    removed: Vec<InterestRemoval>,
    frame_kind: InterestFrameKind,
    delta_sequence: u64,
    previous_view_hash: Option<String>,
}

/// Official protocol-16 state stream result. A session begins with one
/// baseline and then emits only deltas from the same cursor.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum ProjectedInterestFrame {
    Baseline(ProjectedWorldSnapshot),
    Delta(ProjectedInterestDelta),
}

impl ProjectionSource {
    /// Projects the next protocol-16 frame while reusing this source's
    /// canonical snapshot, public payloads, and spatial index.
    pub fn project_interest_frame(
        &self,
        cursor: &mut InterestProjectionState,
        removal_reasons: &BTreeMap<InterestEntityIdentity, InterestRemovalReason>,
    ) -> Result<ProjectedInterestFrame, ProjectionError> {
        let previous_environment = cursor.environment.clone();
        let previous_private = cursor.actor_private.clone();
        let projected =
            self.project_interest_world_snapshot_with_removals(cursor, removal_reasons)?;
        if projected.interest.frame_kind == InterestFrameKind::Baseline {
            return Ok(ProjectedInterestFrame::Baseline(projected));
        }
        let environment = (previous_environment.as_ref() != Some(&projected.environment))
            .then(|| projected.environment.clone());
        let (actor_private, actor_private_motion) =
            delta_private_components(previous_private.as_ref(), projected.actor_private.as_ref());
        Ok(ProjectedInterestFrame::Delta(ProjectedInterestDelta {
            projection_schema_version: projected.projection_schema_version,
            schema_version: projected.schema_version,
            content_manifest_version: projected.content_manifest_version,
            universe_id: projected.universe_id,
            cell_id: projected.cell_id,
            universe_manifest_hash: projected.universe_manifest_hash,
            celestial_registry_hash: projected.celestial_registry_hash,
            cell_address: projected.cell_address,
            gravity_body_id: projected.gravity_body_id,
            voxel_body_id: projected.voxel_body_id,
            event_sequence: projected.event_sequence,
            simulation_tick: projected.simulation_tick,
            world_hash: projected.world_hash,
            environment,
            conservation_valid: Some(projected.conservation_valid),
            interest: projected.interest,
            actor_private,
            actor_private_motion,
        }))
    }

    pub fn project_interest_baseline(
        &self,
        cursor: &mut InterestProjectionState,
    ) -> Result<ProjectedWorldSnapshot, ProjectionError> {
        if cursor.view_hash.is_some() {
            return Err(ProjectionError::InvalidSession(
                "baseline requested from an initialized cursor; call fresh_baseline first".into(),
            ));
        }
        match self.project_interest_frame(cursor, &BTreeMap::new())? {
            ProjectedInterestFrame::Baseline(value) => Ok(value),
            ProjectedInterestFrame::Delta(_) => Err(ProjectionError::InvalidSession(
                "uninitialized cursor emitted a delta".into(),
            )),
        }
    }

    pub fn project_interest_delta(
        &self,
        cursor: &mut InterestProjectionState,
        removal_reasons: &BTreeMap<InterestEntityIdentity, InterestRemovalReason>,
    ) -> Result<ProjectedInterestDelta, ProjectionError> {
        if cursor.view_hash.is_none() {
            return Err(ProjectionError::InvalidSession(
                "delta requested before a baseline".into(),
            ));
        }
        match self.project_interest_frame(cursor, removal_reasons)? {
            ProjectedInterestFrame::Delta(value) => Ok(value),
            ProjectedInterestFrame::Baseline(_) => Err(ProjectionError::InvalidSession(
                "initialized cursor emitted a baseline".into(),
            )),
        }
    }

    pub fn project_interest_world_snapshot(
        &self,
        cursor: &mut InterestProjectionState,
    ) -> Result<ProjectedWorldSnapshot, ProjectionError> {
        self.project_interest_world_snapshot_with_removals(cursor, &BTreeMap::new())
    }

    pub fn project_interest_world_snapshot_with_removals(
        &self,
        cursor: &mut InterestProjectionState,
        removal_reasons: &BTreeMap<InterestEntityIdentity, InterestRemovalReason>,
    ) -> Result<ProjectedWorldSnapshot, ProjectionError> {
        validate_session(cursor)?;
        let actor = bound_actor(cursor);
        let canonical = &self.canonical;
        let anchor = self.interest_anchor(cursor)?;
        let local_origin = anchor.clone();
        let observer_environment = self.observer_environment(cursor)?;
        if removal_reasons
            .keys()
            .any(|identity| self.candidates.contains_key(identity))
        {
            return Err(ProjectionError::InvalidCanonicalSnapshot(
                "a canonical entity cannot carry destroyed or transferred removal evidence".into(),
            ));
        }
        let query = self.candidates(cursor, &anchor)?;
        let selection = select(
            cursor,
            &query.candidates,
            &anchor,
            canonical.simulation_tick,
            removal_reasons,
        )?;
        let visible = selection.members.keys().cloned().collect::<BTreeSet<_>>();

        let mut players = Vec::new();
        let mut grids = Vec::new();
        let mut voxel_chunks = Vec::new();
        let mut death_drops = Vec::new();
        for (identity, payload) in &selection.payloads {
            if !visible.contains(identity) {
                continue;
            }
            match payload {
                CandidatePayload::Player(value) => players.push(value.clone()),
                CandidatePayload::Grid(value) => grids.push(value.clone()),
                CandidatePayload::VoxelChunk(value) => voxel_chunks.push(value.clone()),
                CandidatePayload::DeathDrop(value) => death_drops.push(value.clone()),
            }
        }
        let actor_private = actor
            .map(|actor| self.filtered_actor_private(actor, &visible))
            .transpose()?;
        let entity_refs = selection
            .members
            .iter()
            .map(|(identity, member)| entity_ref(identity, member.projected_revision))
            .collect::<Vec<_>>();
        let complete_view_entities = entity_refs
            .iter()
            .map(|entity| complete_entity_projection(entity, &selection.payloads))
            .collect::<Result<Vec<_>, _>>()?;
        let observer_class = observer_class(&cursor.observer);
        let view_hash = fixed_hash(&ViewHashMaterial {
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            interest_schema_version: INTEREST_SCHEMA_VERSION,
            content_manifest_version: &canonical.content_manifest_version,
            universe_id: &canonical.universe_id,
            cell_id: &canonical.cell_id,
            universe_manifest_hash: &canonical.universe_manifest_hash,
            celestial_registry_hash: &canonical.celestial_registry_hash,
            cell_address: &canonical.cell_address,
            local_origin: &local_origin,
            gravity_body_id: &canonical.gravity_body_id,
            voxel_body_id: &canonical.voxel_body_id,
            observer_class,
            session_epoch: &cursor.session_epoch,
            interest_epoch: cursor.interest_epoch,
            baseline_id: &cursor.baseline_id,
            delta_sequence: selection.delta_sequence,
            entities: &complete_view_entities,
            environment: &observer_environment,
            conservation_valid: canonical.conservation.valid,
            actor_private: actor_private.as_ref(),
        })?;
        let entered = selection
            .entered
            .iter()
            .map(|entity| complete_entity_projection(entity, &selection.payloads))
            .collect::<Result<Vec<_>, _>>()?;
        let replaced = selection
            .replaced
            .iter()
            .map(|entity| complete_entity_projection(entity, &selection.payloads))
            .collect::<Result<Vec<_>, _>>()?;
        let interest = InterestSnapshot {
            schema_version: INTEREST_SCHEMA_VERSION,
            frame_kind: selection.frame_kind,
            session_epoch: cursor.session_epoch.clone(),
            interest_epoch: cursor.interest_epoch,
            baseline_id: cursor.baseline_id.clone(),
            delta_sequence: selection.delta_sequence,
            observer_class,
            cell_address: canonical.cell_address.clone(),
            local_origin_address: local_origin,
            registry_hash: canonical.celestial_registry_hash.clone(),
            universe_manifest_hash: canonical.universe_manifest_hash.clone(),
            canonical_event_sequence: canonical.event_sequence,
            canonical_tick: canonical.simulation_tick,
            canonical_world_hash: canonical.world_hash.clone(),
            previous_view_hash: selection.previous_view_hash,
            view_hash: view_hash.clone(),
            entered,
            replaced,
            removed: selection.removed,
        };
        cursor.members = selection.members;
        cursor.payloads = selection.payloads;
        cursor.delta_sequence = selection.delta_sequence;
        cursor.view_hash = Some(view_hash);
        cursor.last_evaluated_tick = Some(canonical.simulation_tick);
        cursor.environment = Some(observer_environment.clone());
        cursor.actor_private.clone_from(&actor_private);

        Ok(ProjectedWorldSnapshot {
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            schema_version: canonical.schema_version,
            content_manifest_version: canonical.content_manifest_version.clone(),
            universe_id: canonical.universe_id.clone(),
            cell_id: canonical.cell_id.clone(),
            universe_manifest_hash: canonical.universe_manifest_hash.clone(),
            celestial_registry_hash: canonical.celestial_registry_hash.clone(),
            cell_address: canonical.cell_address.clone(),
            gravity_body_id: canonical.gravity_body_id.clone(),
            voxel_body_id: canonical.voxel_body_id.clone(),
            event_sequence: canonical.event_sequence,
            simulation_tick: canonical.simulation_tick,
            fencing_token: canonical.fencing_token,
            world_hash: canonical.world_hash.clone(),
            players,
            environment: observer_environment,
            voxel_chunks,
            grids,
            death_drops,
            conservation_valid: canonical.conservation.valid,
            interest,
            actor_private,
        })
    }

    fn interest_anchor(
        &self,
        cursor: &InterestProjectionState,
    ) -> Result<UniverseAddress, ProjectionError> {
        match &cursor.observer {
            InterestObserver::BoundPlayer { player_id } => self
                .canonical
                .players
                .iter()
                .find(|player| player.player_id == *player_id)
                .map(|player| player.address.clone())
                .ok_or_else(|| ProjectionError::UnboundActor(player_id.clone())),
            InterestObserver::PublicOriginSpectator => Ok(self.spectator_anchor.clone()),
        }
    }

    fn observer_environment(
        &self,
        cursor: &InterestProjectionState,
    ) -> Result<verse_protocol::EnvironmentSnapshot, ProjectionError> {
        match &cursor.observer {
            InterestObserver::BoundPlayer { player_id } => self
                .canonical
                .players
                .iter()
                .find(|player| player.player_id == *player_id)
                .and_then(|player| player.environment.clone())
                .ok_or_else(|| ProjectionError::UnboundActor(player_id.clone())),
            InterestObserver::PublicOriginSpectator => Ok(self.spectator_environment.clone()),
        }
    }

    fn filtered_actor_private(
        &self,
        actor: &str,
        visible: &BTreeSet<InterestEntityIdentity>,
    ) -> Result<ActorPrivateSnapshot, ProjectionError> {
        let mut private = self
            .actor_private
            .get(actor)
            .cloned()
            .ok_or_else(|| ProjectionError::UnboundActor(actor.into()))?;
        let visible_grids = visible_ids(visible, InterestEntityKind::Grid);
        let visible_drops = visible_ids(visible, InterestEntityKind::DeathDrop);
        private
            .death_drops
            .retain(|drop| visible_drops.contains(drop.drop_id.as_str()));
        let visible_drop_inventories = private
            .death_drops
            .iter()
            .map(|drop| drop.inventory_id.as_str())
            .collect::<BTreeSet<_>>();
        private
            .inventories
            .retain(|inventory| match &inventory.domain {
                InventoryDomain::Player { player_id } => player_id == actor,
                InventoryDomain::Cargo { block_id } => self
                    .block_grids
                    .get(block_id)
                    .is_some_and(|grid_id| visible_grids.contains(grid_id.as_str())),
                InventoryDomain::Dropped { .. } => {
                    visible_drop_inventories.contains(inventory.inventory_id.as_str())
                }
            });
        private
            .owned_grid_masses
            .retain(|mass| visible_grids.contains(mass.grid_id.as_str()));
        private.production_queues.retain(|queue| {
            self.block_grids
                .get(&queue.machine_block_id)
                .is_some_and(|grid_id| visible_grids.contains(grid_id.as_str()))
        });
        Ok(private)
    }

    fn candidates(
        &self,
        cursor: &InterestProjectionState,
        anchor: &UniverseAddress,
    ) -> Result<CandidateQuery, ProjectionError> {
        let radius_um = spatial_query_radius_um()?;
        let bounds = spatial_bucket_bounds(&self.canonical.cell_address, anchor, radius_um)?;
        let mut identities = BTreeSet::new();
        #[cfg(test)]
        let mut bucket_lookups = 0_usize;
        #[cfg(test)]
        let mut spatial_candidates_visited = 0_usize;
        for x in bounds.0.x..=bounds.1.x {
            for y in bounds.0.y..=bounds.1.y {
                for z in bounds.0.z..=bounds.1.z {
                    #[cfg(test)]
                    {
                        bucket_lookups += 1;
                    }
                    if let Some(bucket) = self.spatial_buckets.get(&SpatialBucketKey { x, y, z }) {
                        #[cfg(test)]
                        {
                            spatial_candidates_visited += bucket.len();
                        }
                        identities.extend(bucket.iter().cloned());
                    }
                }
            }
        }

        // A prior member must still be evaluated after moving beyond the
        // indexed query radius so hysteresis and removal semantics converge.
        identities.extend(
            cursor
                .members
                .keys()
                .filter(|identity| self.candidates.contains_key(*identity))
                .cloned(),
        );

        let mut critical = BTreeSet::new();
        if let Some(actor) = bound_actor(cursor) {
            let actor_identity =
                InterestEntityIdentity::new(InterestEntityKind::Player, actor.to_owned());
            identities.insert(actor_identity.clone());
            critical.insert(actor_identity);
            if let Some(support) = self
                .canonical
                .players
                .iter()
                .find(|player| player.player_id == actor)
                .and_then(|player| player.locomotion.support.as_ref())
                .and_then(|support| self.support_entities.get(&support.body_id))
            {
                identities.insert(support.clone());
                critical.insert(support.clone());
            }
        }

        let mut candidates = BTreeMap::new();
        for identity in identities {
            let Some(source) = self.candidates.get(&identity) else {
                continue;
            };
            let mut candidate = source.clone();
            candidate.control_critical = critical.contains(&identity);
            candidates.insert(identity, candidate);
        }
        #[cfg(test)]
        let exact_candidates_visited = candidates.len();
        Ok(CandidateQuery {
            candidates,
            #[cfg(test)]
            stats: CandidateQueryStats {
                bucket_lookups,
                spatial_candidates_visited,
                exact_candidates_visited,
            },
        })
    }
}

impl WorldState {
    /// Builds public projection material once so a worker can reuse it for all
    /// sessions projected from the same immutable authoritative state.
    pub fn projection_source(&self) -> Result<ProjectionSource, ProjectionError> {
        let canonical = self.snapshot();
        let mut candidates = BTreeMap::new();
        let mut support_entities = BTreeMap::new();
        for player in &canonical.players {
            add_candidate(
                &mut candidates,
                player.address.clone(),
                false,
                CandidatePayload::Player(public_player(player, &canonical.cell_address)?),
            )?;
        }
        for grid in &canonical.grids {
            let identity = InterestEntityIdentity::new(InterestEntityKind::Grid, &grid.grid_id);
            add_candidate(
                &mut candidates,
                grid.address.clone(),
                false,
                CandidatePayload::Grid(public_grid(self, grid, &canonical.cell_address)?),
            )?;
            support_entities.insert(grid.grid_id.clone(), identity);
        }
        for chunk in public_voxel_chunks(&canonical.voxel_body_id, &canonical.voxels)? {
            let address = chunk_address(self.world_seed, &canonical.voxel_body_id, &chunk)?;
            let (x, y, z) = chunk_coordinates(&chunk)?;
            let identity =
                InterestEntityIdentity::new(InterestEntityKind::VoxelChunk, &chunk.chunk_id);
            add_candidate(
                &mut candidates,
                address,
                false,
                CandidatePayload::VoxelChunk(chunk),
            )?;
            support_entities.insert(format!("voxel-chunk-{x}-{y}-{z}"), identity);
        }
        for drop in &canonical.death_drops {
            add_candidate(
                &mut candidates,
                drop.address.clone(),
                false,
                CandidatePayload::DeathDrop(public_drop(drop, &canonical.cell_address)?),
            )?;
        }

        let mut spatial_buckets = BTreeMap::<SpatialBucketKey, Vec<_>>::new();
        for (identity, candidate) in &candidates {
            let bucket = spatial_bucket_key(&canonical.cell_address, &candidate.address)?;
            spatial_buckets
                .entry(bucket)
                .or_default()
                .push(identity.clone());
        }
        let all_entities = candidates.keys().cloned().collect::<BTreeSet<_>>();
        let inventory_owners = self.projection_inventory_owners(None)?;
        let actor_private = canonical
            .players
            .iter()
            .map(|player| {
                self.actor_private(
                    &player.player_id,
                    &canonical,
                    &inventory_owners,
                    &all_entities,
                )
                .map(|private| (player.player_id.clone(), private))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let block_grids = canonical
            .grids
            .iter()
            .flat_map(|grid| {
                grid.blocks
                    .iter()
                    .map(move |block| (block.block_id.clone(), grid.grid_id.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let anchor = content::manifest().interest.public_spectator_anchor_um;
        let spectator_anchor = celestial::address_from_origin_offset_um(
            &canonical.cell_address,
            [
                i128::from(anchor.x),
                i128::from(anchor.y),
                i128::from(anchor.z),
            ],
        )
        .map_err(|source| {
            ProjectionError::InvalidCanonicalSnapshot(format!(
                "spectator anchor is invalid: {source}"
            ))
        })?;
        let spectator_position =
            celestial::local_position_from_address(&canonical.cell_address, &spectator_anchor)
                .map_err(|source| {
                    ProjectionError::InvalidCanonicalSnapshot(format!(
                        "spectator environment position cannot be derived: {source}"
                    ))
                })?;
        let spectator_environment = self.environment_at(spectator_position);
        Ok(ProjectionSource {
            canonical,
            candidates,
            spatial_buckets,
            support_entities,
            actor_private,
            block_grids,
            spectator_anchor,
            spectator_environment,
        })
    }

    /// Backward-compatible fresh baseline. Stateful network streams retain an
    /// [`InterestProjectionState`] and use `project_interest_world_snapshot`.
    pub fn project_world_snapshot(
        &self,
        actor_player_id: Option<&str>,
    ) -> Result<ProjectedWorldSnapshot, ProjectionError> {
        let mut cursor = compatibility_cursor(actor_player_id);
        self.project_interest_world_snapshot(&mut cursor)
    }

    pub fn project_interest_world_snapshot(
        &self,
        cursor: &mut InterestProjectionState,
    ) -> Result<ProjectedWorldSnapshot, ProjectionError> {
        self.project_interest_world_snapshot_with_removals(cursor, &BTreeMap::new())
    }

    /// Emits the explicit protocol-16 stream family. Callers must not mix this
    /// with the protocol-15 `Snapshot`/`MotionState` compatibility family.
    pub fn project_interest_frame(
        &self,
        cursor: &mut InterestProjectionState,
        removal_reasons: &BTreeMap<InterestEntityIdentity, InterestRemovalReason>,
    ) -> Result<ProjectedInterestFrame, ProjectionError> {
        self.projection_source()?
            .project_interest_frame(cursor, removal_reasons)
    }

    pub fn project_interest_baseline(
        &self,
        cursor: &mut InterestProjectionState,
    ) -> Result<ProjectedWorldSnapshot, ProjectionError> {
        if cursor.view_hash.is_some() {
            return Err(ProjectionError::InvalidSession(
                "baseline requested from an initialized cursor; call fresh_baseline first".into(),
            ));
        }
        match self.project_interest_frame(cursor, &BTreeMap::new())? {
            ProjectedInterestFrame::Baseline(value) => Ok(value),
            ProjectedInterestFrame::Delta(_) => Err(ProjectionError::InvalidSession(
                "uninitialized cursor emitted a delta".into(),
            )),
        }
    }

    /// Cumulative absolute delta from the view held by `cursor` to current
    /// canonical state. Project into a cloned cursor and install it on ack.
    pub fn project_interest_delta(
        &self,
        cursor: &mut InterestProjectionState,
        removal_reasons: &BTreeMap<InterestEntityIdentity, InterestRemovalReason>,
    ) -> Result<ProjectedInterestDelta, ProjectionError> {
        if cursor.view_hash.is_none() {
            return Err(ProjectionError::InvalidSession(
                "delta requested before a baseline".into(),
            ));
        }
        match self.project_interest_frame(cursor, removal_reasons)? {
            ProjectedInterestFrame::Delta(value) => Ok(value),
            ProjectedInterestFrame::Baseline(_) => Err(ProjectionError::InvalidSession(
                "initialized cursor emitted a baseline".into(),
            )),
        }
    }

    pub fn project_interest_world_snapshot_with_removals(
        &self,
        cursor: &mut InterestProjectionState,
        removal_reasons: &BTreeMap<InterestEntityIdentity, InterestRemovalReason>,
    ) -> Result<ProjectedWorldSnapshot, ProjectionError> {
        self.projection_source()?
            .project_interest_world_snapshot_with_removals(cursor, removal_reasons)
    }

    #[cfg(test)]
    fn project_interest_world_snapshot_from_source(
        &self,
        source: &ProjectionSource,
        cursor: &mut InterestProjectionState,
        removal_reasons: &BTreeMap<InterestEntityIdentity, InterestRemovalReason>,
    ) -> Result<ProjectedWorldSnapshot, ProjectionError> {
        validate_session(cursor)?;
        let actor = bound_actor(cursor);
        let inventory_owners = self.projection_inventory_owners(actor)?;
        let canonical = &source.canonical;
        let anchor = self.interest_anchor(cursor)?;
        let local_origin = anchor.clone();
        let observer_position =
            celestial::local_position_from_address(&canonical.cell_address, &anchor).map_err(
                |source| {
                    ProjectionError::InvalidCanonicalSnapshot(format!(
                        "observer environment position cannot be derived: {source}"
                    ))
                },
            )?;
        let observer_environment = self.environment_at(observer_position);
        if removal_reasons
            .keys()
            .any(|identity| source.candidates.contains_key(identity))
        {
            return Err(ProjectionError::InvalidCanonicalSnapshot(
                "a canonical entity cannot carry destroyed or transferred removal evidence".into(),
            ));
        }
        let query = source.candidates(cursor, &anchor)?;
        let selection = select(
            cursor,
            &query.candidates,
            &anchor,
            canonical.simulation_tick,
            removal_reasons,
        )?;
        let visible = selection.members.keys().cloned().collect::<BTreeSet<_>>();

        let mut players = Vec::new();
        let mut grids = Vec::new();
        let mut voxel_chunks = Vec::new();
        let mut death_drops = Vec::new();
        for (identity, payload) in &selection.payloads {
            if !visible.contains(identity) {
                continue;
            }
            match payload {
                CandidatePayload::Player(value) => players.push(value.clone()),
                CandidatePayload::Grid(value) => grids.push(value.clone()),
                CandidatePayload::VoxelChunk(value) => voxel_chunks.push(value.clone()),
                CandidatePayload::DeathDrop(value) => death_drops.push(value.clone()),
            }
        }
        let actor_private = actor
            .map(|actor| self.actor_private(actor, canonical, &inventory_owners, &visible))
            .transpose()?;
        let entity_refs = selection
            .members
            .iter()
            .map(|(identity, member)| entity_ref(identity, member.projected_revision))
            .collect::<Vec<_>>();
        let complete_view_entities = entity_refs
            .iter()
            .map(|entity| complete_entity_projection(entity, &selection.payloads))
            .collect::<Result<Vec<_>, _>>()?;
        let observer_class = observer_class(&cursor.observer);
        let view_hash = fixed_hash(&ViewHashMaterial {
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            interest_schema_version: INTEREST_SCHEMA_VERSION,
            content_manifest_version: &canonical.content_manifest_version,
            universe_id: &canonical.universe_id,
            cell_id: &canonical.cell_id,
            universe_manifest_hash: &canonical.universe_manifest_hash,
            celestial_registry_hash: &canonical.celestial_registry_hash,
            cell_address: &canonical.cell_address,
            local_origin: &local_origin,
            gravity_body_id: &canonical.gravity_body_id,
            voxel_body_id: &canonical.voxel_body_id,
            observer_class,
            session_epoch: &cursor.session_epoch,
            interest_epoch: cursor.interest_epoch,
            baseline_id: &cursor.baseline_id,
            delta_sequence: selection.delta_sequence,
            entities: &complete_view_entities,
            environment: &observer_environment,
            conservation_valid: canonical.conservation.valid,
            actor_private: actor_private.as_ref(),
        })?;
        let entered = selection
            .entered
            .iter()
            .map(|entity| complete_entity_projection(entity, &selection.payloads))
            .collect::<Result<Vec<_>, _>>()?;
        let replaced = selection
            .replaced
            .iter()
            .map(|entity| complete_entity_projection(entity, &selection.payloads))
            .collect::<Result<Vec<_>, _>>()?;
        let interest = InterestSnapshot {
            schema_version: INTEREST_SCHEMA_VERSION,
            frame_kind: selection.frame_kind,
            session_epoch: cursor.session_epoch.clone(),
            interest_epoch: cursor.interest_epoch,
            baseline_id: cursor.baseline_id.clone(),
            delta_sequence: selection.delta_sequence,
            observer_class,
            cell_address: canonical.cell_address.clone(),
            local_origin_address: local_origin,
            registry_hash: canonical.celestial_registry_hash.clone(),
            universe_manifest_hash: canonical.universe_manifest_hash.clone(),
            canonical_event_sequence: canonical.event_sequence,
            canonical_tick: canonical.simulation_tick,
            canonical_world_hash: canonical.world_hash.clone(),
            previous_view_hash: selection.previous_view_hash,
            view_hash: view_hash.clone(),
            entered,
            replaced,
            removed: selection.removed,
        };
        cursor.members = selection.members;
        cursor.payloads = selection.payloads;
        cursor.delta_sequence = selection.delta_sequence;
        cursor.view_hash = Some(view_hash);
        cursor.last_evaluated_tick = Some(canonical.simulation_tick);
        cursor.environment = Some(observer_environment.clone());
        cursor.actor_private.clone_from(&actor_private);

        Ok(ProjectedWorldSnapshot {
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            schema_version: canonical.schema_version,
            content_manifest_version: canonical.content_manifest_version.clone(),
            universe_id: canonical.universe_id.clone(),
            cell_id: canonical.cell_id.clone(),
            universe_manifest_hash: canonical.universe_manifest_hash.clone(),
            celestial_registry_hash: canonical.celestial_registry_hash.clone(),
            cell_address: canonical.cell_address.clone(),
            gravity_body_id: canonical.gravity_body_id.clone(),
            voxel_body_id: canonical.voxel_body_id.clone(),
            event_sequence: canonical.event_sequence,
            simulation_tick: canonical.simulation_tick,
            fencing_token: canonical.fencing_token,
            world_hash: canonical.world_hash.clone(),
            players,
            environment: observer_environment,
            voxel_chunks,
            grids,
            death_drops,
            conservation_valid: canonical.conservation.valid,
            interest,
            actor_private,
        })
    }

    /// Compatibility motion baseline filtered by the same authoritative
    /// membership policy. Structural stateful streams use world deltas first.
    pub fn project_motion_snapshot(
        &self,
        actor_player_id: Option<&str>,
    ) -> Result<ProjectedMotionSnapshot, ProjectionError> {
        let mut cursor = compatibility_cursor(actor_player_id);
        let baseline = self.project_interest_world_snapshot(&mut cursor)?;
        let canonical = self.motion_snapshot();
        let mut players = canonical
            .players
            .iter()
            .filter(|p| cursor.contains(InterestEntityKind::Player, &p.player_id))
            .map(public_player_motion)
            .collect::<Vec<_>>();
        players.sort_by(|a, b| a.player_id.cmp(&b.player_id));
        let mut grids = canonical
            .grids
            .iter()
            .filter(|g| cursor.contains(InterestEntityKind::Grid, &g.grid_id))
            .map(public_grid_motion)
            .collect::<Vec<_>>();
        grids.sort_by(|a, b| a.grid_id.cmp(&b.grid_id));
        let actor_private = bound_actor(&cursor)
            .map(|actor| {
                canonical
                    .players
                    .iter()
                    .find(|p| p.player_id == actor)
                    .cloned()
                    .ok_or_else(|| ProjectionError::UnboundActor(actor.into()))
            })
            .transpose()?;
        Ok(ProjectedMotionSnapshot {
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            universe_manifest_hash: canonical.universe_manifest_hash,
            celestial_registry_hash: canonical.celestial_registry_hash,
            event_sequence: canonical.event_sequence,
            simulation_tick: canonical.simulation_tick,
            world_hash: canonical.world_hash,
            players,
            grids,
            interest: baseline.interest,
            actor_private,
        })
    }

    #[cfg(test)]
    fn interest_anchor(
        &self,
        cursor: &InterestProjectionState,
    ) -> Result<UniverseAddress, ProjectionError> {
        match &cursor.observer {
            InterestObserver::BoundPlayer { player_id } => self
                .player
                .get(player_id)
                .map(|player| player.address.clone())
                .ok_or_else(|| ProjectionError::UnboundActor(player_id.clone())),
            InterestObserver::PublicOriginSpectator => {
                let anchor = content::manifest().interest.public_spectator_anchor_um;
                celestial::address_from_origin_offset_um(
                    &self.cell_address,
                    [
                        i128::from(anchor.x),
                        i128::from(anchor.y),
                        i128::from(anchor.z),
                    ],
                )
                .map_err(|source| {
                    ProjectionError::InvalidCanonicalSnapshot(format!(
                        "spectator anchor is invalid: {source}"
                    ))
                })
            }
        }
    }

    fn actor_private(
        &self,
        actor: &str,
        canonical: &verse_protocol::WorldSnapshot,
        owners: &BTreeMap<String, String>,
        visible: &BTreeSet<InterestEntityIdentity>,
    ) -> Result<ActorPrivateSnapshot, ProjectionError> {
        let player = canonical
            .players
            .iter()
            .find(|p| p.player_id == actor)
            .cloned()
            .ok_or_else(|| ProjectionError::UnboundActor(actor.into()))?;
        let grid_ids = visible_ids(visible, InterestEntityKind::Grid);
        let drop_ids = visible_ids(visible, InterestEntityKind::DeathDrop);
        let block_ids = canonical
            .grids
            .iter()
            .filter(|g| g.owner_player_id == actor && grid_ids.contains(g.grid_id.as_str()))
            .flat_map(|g| g.blocks.iter().map(|b| b.block_id.as_str()))
            .collect::<BTreeSet<_>>();
        let drop_inventory_ids = canonical
            .death_drops
            .iter()
            .filter(|d| d.owner_player_id == actor && drop_ids.contains(d.drop_id.as_str()))
            .map(|d| d.inventory_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut inventories = canonical
            .inventories
            .iter()
            .filter(|i| {
                owners
                    .get(&i.inventory_id)
                    .is_some_and(|owner| owner == actor)
                    && match &i.domain {
                        InventoryDomain::Player { player_id } => player_id == actor,
                        InventoryDomain::Cargo { block_id } => {
                            block_ids.contains(block_id.as_str())
                        }
                        InventoryDomain::Dropped { .. } => {
                            drop_inventory_ids.contains(i.inventory_id.as_str())
                        }
                    }
            })
            .cloned()
            .collect::<Vec<InventorySnapshot>>();
        inventories.sort_by(|a, b| a.inventory_id.cmp(&b.inventory_id));
        let mut death_drops = canonical
            .death_drops
            .iter()
            .filter(|d| d.owner_player_id == actor && drop_ids.contains(d.drop_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        death_drops.sort_by(|a, b| a.drop_id.cmp(&b.drop_id));
        let mut owned_grid_masses = canonical
            .grids
            .iter()
            .filter(|g| g.owner_player_id == actor && grid_ids.contains(g.grid_id.as_str()))
            .map(|g| OwnedGridMassSnapshot {
                grid_id: g.grid_id.clone(),
                mass_kg: g.mass_kg,
            })
            .collect::<Vec<_>>();
        owned_grid_masses.sort_by(|a, b| a.grid_id.cmp(&b.grid_id));
        let production_queues = self
            .production_queues
            .iter()
            .filter(|(id, _)| block_ids.contains(id.as_str()))
            .filter_map(|(machine, queue)| {
                let jobs = queue
                    .iter()
                    .enumerate()
                    .filter(|(_, job)| job.owner_player_id == actor)
                    .map(|(index, job)| ProductionJobSnapshot {
                        job_id: job.job_id.clone(),
                        owner_player_id: job.owner_player_id.clone(),
                        machine_block_id: job.machine_block_id.clone(),
                        recipe: job.recipe,
                        batches: job.batches,
                        source_inventory_id: job.source_inventory_id.clone(),
                        destination_inventory_id: job.destination_inventory_id.clone(),
                        progress_ticks: job.progress_ticks,
                        duration_ticks: job.duration_ticks,
                        status: self.production_job_status(machine, index),
                        reserved_inputs: job.reserved_inputs.clone(),
                        pending_outputs: job.pending_outputs.clone(),
                    })
                    .collect::<Vec<_>>();
                (!jobs.is_empty()).then(|| ProductionQueueSnapshot {
                    machine_block_id: machine.clone(),
                    jobs,
                })
            })
            .collect();
        Ok(ActorPrivateSnapshot {
            player,
            committed_operation_sequence: self.last_operation_sequence(actor),
            inventories,
            death_drops,
            owned_grid_masses,
            production_queues,
        })
    }

    fn projection_inventory_owners(
        &self,
        actor: Option<&str>,
    ) -> Result<BTreeMap<String, String>, ProjectionError> {
        self.validate_player_roster()
            .map_err(ProjectionError::InvalidAuthority)?;
        if let Some(actor) = actor
            && self.player.get(actor).is_none()
        {
            return Err(ProjectionError::UnboundActor(actor.into()));
        }
        self.inventories
            .keys()
            .map(|id| {
                self.inventory_owner_player_id(id)
                    .map(|owner| (id.clone(), owner.into()))
                    .map_err(ProjectionError::InvalidAuthority)
            })
            .collect()
    }
}

#[derive(Serialize)]
struct ViewHashMaterial<'a> {
    projection_schema_version: u32,
    interest_schema_version: u32,
    content_manifest_version: &'a str,
    universe_id: &'a str,
    cell_id: &'a str,
    universe_manifest_hash: &'a str,
    celestial_registry_hash: &'a str,
    cell_address: &'a UniverseAddress,
    local_origin: &'a UniverseAddress,
    gravity_body_id: &'a str,
    voxel_body_id: &'a str,
    observer_class: InterestObserverClass,
    session_epoch: &'a str,
    interest_epoch: u64,
    baseline_id: &'a str,
    delta_sequence: u64,
    entities: &'a [InterestEntityProjection],
    environment: &'a verse_protocol::EnvironmentSnapshot,
    conservation_valid: bool,
    actor_private: Option<&'a ActorPrivateSnapshot>,
}

fn compatibility_cursor(actor: Option<&str>) -> InterestProjectionState {
    match actor {
        Some(actor) => InterestProjectionState::bound_player(
            format!(
                "compat-{}",
                &digest(format!("actor\0{actor}").as_bytes())[..16]
            ),
            actor,
        ),
        None => InterestProjectionState::public_origin_spectator("compat-public-origin"),
    }
}

fn bound_actor(cursor: &InterestProjectionState) -> Option<&str> {
    match &cursor.observer {
        InterestObserver::BoundPlayer { player_id } => Some(player_id),
        InterestObserver::PublicOriginSpectator => None,
    }
}

const fn observer_class(observer: &InterestObserver) -> InterestObserverClass {
    match observer {
        InterestObserver::BoundPlayer { .. } => InterestObserverClass::BoundPlayer,
        InterestObserver::PublicOriginSpectator => InterestObserverClass::PublicOriginSpectator,
    }
}

fn validate_session(cursor: &InterestProjectionState) -> Result<(), ProjectionError> {
    if cursor.session_epoch.trim().is_empty() || cursor.session_epoch.len() > 256 {
        return Err(ProjectionError::InvalidSession(
            "session_epoch must contain 1..=256 bytes".into(),
        ));
    }
    if cursor.interest_epoch == 0 || cursor.baseline_id.is_empty() {
        return Err(ProjectionError::InvalidSession(
            "interest frontier is incomplete".into(),
        ));
    }
    Ok(())
}

fn visible_ids(
    values: &BTreeSet<InterestEntityIdentity>,
    kind: InterestEntityKind,
) -> BTreeSet<&str> {
    values
        .iter()
        .filter(|value| value.kind == kind)
        .map(|value| value.entity_id.as_str())
        .collect()
}

fn add_candidate(
    values: &mut BTreeMap<InterestEntityIdentity, Candidate>,
    address: UniverseAddress,
    control_critical: bool,
    payload: CandidatePayload,
) -> Result<(), ProjectionError> {
    celestial::validate_universe_address(&address, &address.universe_id).map_err(|source| {
        ProjectionError::InvalidCanonicalSnapshot(format!("entity address is invalid: {source}"))
    })?;
    let identity = payload.identity();
    let content_fingerprint = projected_revision(&(&address, payload_for_hash(&payload)))?;
    let candidate = Candidate {
        address,
        content_fingerprint,
        control_critical,
        payload,
    };
    if values.insert(identity, candidate).is_some() {
        return Err(ProjectionError::InvalidCanonicalSnapshot(
            "duplicate interest entity identity".into(),
        ));
    }
    Ok(())
}

fn payload_for_hash(payload: &CandidatePayload) -> InterestEntityPayload {
    match payload {
        CandidatePayload::Player(value) => InterestEntityPayload::Player(value.clone()),
        CandidatePayload::Grid(value) => InterestEntityPayload::Grid(value.clone()),
        CandidatePayload::VoxelChunk(value) => InterestEntityPayload::VoxelChunk(value.clone()),
        CandidatePayload::DeathDrop(value) => InterestEntityPayload::DeathDrop(value.clone()),
    }
}

fn complete_entity_projection(
    entity: &InterestEntityRef,
    payloads: &BTreeMap<InterestEntityIdentity, CandidatePayload>,
) -> Result<InterestEntityProjection, ProjectionError> {
    let identity = InterestEntityIdentity::new(entity.kind, &entity.entity_id);
    let payload = payloads.get(&identity).ok_or_else(|| {
        ProjectionError::InvalidCanonicalSnapshot(format!(
            "visible payload {} is missing",
            entity.entity_id
        ))
    })?;
    Ok(InterestEntityProjection {
        entity_id: entity.entity_id.clone(),
        kind: entity.kind,
        projected_revision: entity.projected_revision,
        component_schema_version: PROJECTION_SCHEMA_VERSION,
        payload: payload_for_hash(payload),
    })
}

fn entity_ref(identity: &InterestEntityIdentity, revision: u64) -> InterestEntityRef {
    InterestEntityRef {
        entity_id: identity.entity_id.clone(),
        kind: identity.kind,
        projected_revision: revision,
    }
}

struct Eligible {
    priority: u8,
    distance_squared_um: u128,
    identity: InterestEntityIdentity,
    member: Membership,
    payload: CandidatePayload,
    control_critical: bool,
}

fn interest_band(
    kind: InterestEntityKind,
) -> Result<&'static content::InterestBandDefinition, ProjectionError> {
    content::manifest()
        .interest
        .entity_bands
        .iter()
        .find(|band| band.kind == kind)
        .ok_or_else(|| {
            ProjectionError::InvalidCanonicalSnapshot(format!(
                "interest band for {kind:?} is missing"
            ))
        })
}

fn squared_distance_um(
    left: &UniverseAddress,
    right: &UniverseAddress,
) -> Result<u128, ProjectionError> {
    let offset = celestial::relative_offset_um(left, right).map_err(|source| {
        ProjectionError::InvalidCanonicalSnapshot(format!(
            "interest address distance is invalid: {source}"
        ))
    })?;
    let mut squared = 0_u128;
    for component in offset {
        let magnitude = component.unsigned_abs();
        let Some(term) = magnitude.checked_mul(magnitude) else {
            return Err(ProjectionError::InvalidCanonicalSnapshot(
                "interest distance overflow".into(),
            ));
        };
        let Some(next) = squared.checked_add(term) else {
            return Err(ProjectionError::InvalidCanonicalSnapshot(
                "interest distance overflow".into(),
            ));
        };
        squared = next;
    }
    Ok(squared)
}

fn radius_squared_um(radius_m: u32) -> u128 {
    let micrometres = u128::from(radius_m) * 1_000_000;
    micrometres * micrometres
}

fn spatial_bucket_edge_um() -> Result<i128, ProjectionError> {
    let edge_m = content::manifest().interest.spatial_bucket_edge_m;
    if edge_m == 0 {
        return Err(ProjectionError::InvalidCanonicalSnapshot(
            "interest spatial bucket edge must be positive".into(),
        ));
    }
    Ok(i128::from(edge_m) * 1_000_000)
}

fn spatial_query_radius_um() -> Result<i128, ProjectionError> {
    let policy = &content::manifest().interest;
    let selected_context_radius_m = policy
        .enter_radius_m
        .checked_add(policy.selected_context_margin_m)
        .ok_or_else(|| {
            ProjectionError::InvalidCanonicalSnapshot(
                "selected-context interest radius overflowed".into(),
            )
        })?;
    let radius_m = policy
        .entity_bands
        .iter()
        .map(|band| band.exit_radius_m)
        .chain([policy.exit_radius_m, selected_context_radius_m])
        .max()
        .ok_or_else(|| {
            ProjectionError::InvalidCanonicalSnapshot("interest policy has no query radius".into())
        })?;
    Ok(i128::from(radius_m) * 1_000_000)
}

fn spatial_bucket_key(
    origin: &UniverseAddress,
    address: &UniverseAddress,
) -> Result<SpatialBucketKey, ProjectionError> {
    let offset = celestial::relative_offset_um(origin, address).map_err(|source| {
        ProjectionError::InvalidCanonicalSnapshot(format!(
            "interest spatial bucket address is invalid: {source}"
        ))
    })?;
    let edge = spatial_bucket_edge_um()?;
    Ok(SpatialBucketKey {
        x: offset[0].div_euclid(edge),
        y: offset[1].div_euclid(edge),
        z: offset[2].div_euclid(edge),
    })
}

fn spatial_bucket_bounds(
    origin: &UniverseAddress,
    anchor: &UniverseAddress,
    radius_um: i128,
) -> Result<(SpatialBucketKey, SpatialBucketKey), ProjectionError> {
    let offset = celestial::relative_offset_um(origin, anchor).map_err(|source| {
        ProjectionError::InvalidCanonicalSnapshot(format!(
            "interest spatial anchor is invalid: {source}"
        ))
    })?;
    let edge = spatial_bucket_edge_um()?;
    let component_bounds = |component: i128| {
        let minimum = component.checked_sub(radius_um).ok_or_else(|| {
            ProjectionError::InvalidCanonicalSnapshot(
                "interest spatial query lower bound overflowed".into(),
            )
        })?;
        let maximum = component.checked_add(radius_um).ok_or_else(|| {
            ProjectionError::InvalidCanonicalSnapshot(
                "interest spatial query upper bound overflowed".into(),
            )
        })?;
        Ok::<_, ProjectionError>((minimum.div_euclid(edge), maximum.div_euclid(edge)))
    };
    let x = component_bounds(offset[0])?;
    let y = component_bounds(offset[1])?;
    let z = component_bounds(offset[2])?;
    Ok((
        SpatialBucketKey {
            x: x.0,
            y: y.0,
            z: z.0,
        },
        SpatialBucketKey {
            x: x.1,
            y: y.1,
            z: z.1,
        },
    ))
}

fn select(
    prior: &InterestProjectionState,
    candidates: &BTreeMap<InterestEntityIdentity, Candidate>,
    anchor: &UniverseAddress,
    canonical_tick: u64,
    removals: &BTreeMap<InterestEntityIdentity, InterestRemovalReason>,
) -> Result<Selection, ProjectionError> {
    let policy = &content::manifest().interest;
    let first_frame = prior.view_hash.is_none();
    if prior
        .last_evaluated_tick
        .is_some_and(|last| canonical_tick < last)
    {
        return Err(ProjectionError::InvalidSession(
            "canonical tick regressed behind the acknowledged interest frontier".into(),
        ));
    }
    if removals
        .keys()
        .any(|identity| candidates.contains_key(identity))
    {
        return Err(ProjectionError::InvalidCanonicalSnapshot(
            "a canonical entity cannot carry destroyed or transferred removal evidence".into(),
        ));
    }
    let frame_kind = if first_frame {
        InterestFrameKind::Baseline
    } else {
        InterestFrameKind::Delta
    };
    let delta_sequence = if first_frame {
        0
    } else {
        prior
            .delta_sequence
            .checked_add(1)
            .ok_or_else(|| ProjectionError::InvalidSession("delta sequence exhausted".into()))?
    };
    let changed_revision = delta_sequence
        .checked_add(1)
        .ok_or_else(|| ProjectionError::InvalidSession("projected revision exhausted".into()))?;
    let mut eligible = Vec::new();
    for (identity, candidate) in candidates {
        if removals.contains_key(identity) {
            continue;
        }
        let band = interest_band(identity.kind)?;
        let old = prior.members.get(identity);
        let structural_changed = old.is_some()
            && prior
                .payloads
                .get(identity)
                .is_some_and(|previous| structural_payload_changed(previous, &candidate.payload));
        let due = first_frame
            || candidate.control_critical
            || structural_changed
            || canonical_tick.is_multiple_of(u64::from(band.update_interval_ticks));
        let distance = squared_distance_um(anchor, &candidate.address)?;
        let enter = radius_squared_um(band.enter_radius_m);
        let exit = radius_squared_um(band.exit_radius_m);
        let (include, outside_since_tick, priority) = if candidate.control_critical {
            (true, None, 0)
        } else if distance <= enter {
            (true, None, if old.is_some() { 1 } else { 2 })
        } else if let Some(member) = old {
            if distance <= exit {
                (true, None, 1)
            } else {
                // If delivery was blocked awaiting an ACK, use the first tick
                // after the acknowledged evaluation as the privacy-safe lower
                // bound. This prevents frame pacing from extending visibility.
                let first_unobserved_tick =
                    prior.last_evaluated_tick.map_or(canonical_tick, |last| {
                        if canonical_tick > last {
                            last + 1
                        } else {
                            canonical_tick
                        }
                    });
                let since = member.outside_since_tick.unwrap_or(first_unobserved_tick);
                let elapsed = canonical_tick
                    .checked_sub(since)
                    .and_then(|value| value.checked_add(1))
                    .ok_or_else(|| {
                        ProjectionError::InvalidSession("interest exit duration overflowed".into())
                    })?;
                (
                    elapsed < u64::from(policy.exit_consecutive_ticks),
                    Some(since),
                    1,
                )
            }
        } else {
            (false, None, 3)
        };
        if include {
            let use_current_payload = due || old.is_none();
            let projected_revision = match old {
                Some(member) if !use_current_payload => member.projected_revision,
                Some(member)
                    if member.content_fingerprint == candidate.content_fingerprint
                        && prior.payloads.get(identity) == Some(&candidate.payload) =>
                {
                    member.projected_revision
                }
                Some(_) | None => changed_revision,
            };
            let content_fingerprint = if use_current_payload {
                candidate.content_fingerprint
            } else {
                old.ok_or_else(|| {
                    ProjectionError::InvalidSession(format!(
                        "deferred membership is missing for {}",
                        identity.entity_id
                    ))
                })?
                .content_fingerprint
            };
            let payload = if use_current_payload {
                candidate.payload.clone()
            } else {
                prior.payloads.get(identity).cloned().ok_or_else(|| {
                    ProjectionError::InvalidSession(format!(
                        "visible payload frontier is missing for {}",
                        identity.entity_id
                    ))
                })?
            };
            eligible.push(Eligible {
                priority,
                distance_squared_um: distance,
                identity: identity.clone(),
                member: Membership {
                    projected_revision,
                    content_fingerprint,
                    outside_since_tick,
                },
                payload,
                control_critical: candidate.control_critical,
            });
        }
    }
    eligible.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.distance_squared_um.cmp(&right.distance_squared_um))
            .then_with(|| left.identity.cmp(&right.identity))
    });
    let critical = eligible.iter().filter(|item| item.control_critical).count();
    if critical > policy.maximum_visible_entities {
        return Err(ProjectionError::InvalidCanonicalSnapshot(
            "control-critical interest exceeds the global budget".into(),
        ));
    }
    let mut kind_counts = BTreeMap::<InterestEntityKind, usize>::new();
    let mut members = BTreeMap::new();
    let mut payloads = BTreeMap::new();
    for item in eligible {
        let band = interest_band(item.identity.kind)?;
        let count = *kind_counts.get(&item.identity.kind).unwrap_or(&0);
        if !item.control_critical
            && (members.len() >= policy.maximum_visible_entities || count >= band.maximum_entities)
        {
            continue;
        }
        kind_counts.insert(item.identity.kind, count + 1);
        payloads.insert(item.identity.clone(), item.payload);
        members.insert(item.identity, item.member);
    }

    let mut entered = Vec::new();
    let mut replaced = Vec::new();
    for (identity, member) in &members {
        let value = entity_ref(identity, member.projected_revision);
        match prior.members.get(identity) {
            None => entered.push(value),
            Some(old) if old.projected_revision != member.projected_revision => {
                replaced.push(value);
            }
            Some(_) => {}
        }
    }
    if first_frame {
        entered = members
            .iter()
            .map(|(identity, member)| entity_ref(identity, member.projected_revision))
            .collect();
        replaced.clear();
    }
    let mut removed = prior
        .members
        .keys()
        .filter(|identity| !members.contains_key(*identity))
        .map(|identity| InterestRemoval {
            entity_id: identity.entity_id.clone(),
            kind: identity.kind,
            reason: removals.get(identity).copied().unwrap_or_else(|| {
                if candidates.contains_key(identity) {
                    InterestRemovalReason::OutOfInterest
                } else {
                    InterestRemovalReason::Destroyed
                }
            }),
        })
        .collect::<Vec<_>>();
    removed.sort_by(|left, right| {
        left.entity_id
            .cmp(&right.entity_id)
            .then_with(|| left.kind.cmp(&right.kind))
    });
    Ok(Selection {
        members,
        payloads,
        entered,
        replaced,
        removed,
        frame_kind,
        delta_sequence,
        previous_view_hash: prior.view_hash.clone(),
    })
}

fn structural_payload_changed(previous: &CandidatePayload, current: &CandidatePayload) -> bool {
    match (previous, current) {
        (CandidatePayload::Player(previous), CandidatePayload::Player(current)) => {
            previous.player_id != current.player_id
                || previous.life_state != current.life_state
                || previous.helmet_closed != current.helmet_closed
                || previous.jetpack_enabled != current.jetpack_enabled
        }
        (CandidatePayload::Grid(previous), CandidatePayload::Grid(current)) => {
            previous.grid_id != current.grid_id
                || previous.owner_player_id != current.owner_player_id
                || previous.anchored != current.anchored
                || previous.power != current.power
                || previous.blocks != current.blocks
        }
        (CandidatePayload::VoxelChunk(previous), CandidatePayload::VoxelChunk(current)) => {
            previous != current
        }
        (CandidatePayload::DeathDrop(previous), CandidatePayload::DeathDrop(current)) => {
            previous != current
        }
        _ => true,
    }
}

fn public_life_state(value: &PlayerLifeState) -> PublicPlayerLifeState {
    match value {
        PlayerLifeState::Alive => PublicPlayerLifeState::Alive,
        PlayerLifeState::Incapacitated { .. } => PublicPlayerLifeState::Incapacitated,
    }
}

fn render_position(
    origin: &UniverseAddress,
    address: &UniverseAddress,
) -> Result<Vec3, ProjectionError> {
    celestial::local_position_from_address(origin, address).map_err(|source| {
        ProjectionError::InvalidCanonicalSnapshot(format!(
            "renderer position cannot be derived from exact address: {source}"
        ))
    })
}

fn public_player(
    player: &PlayerSnapshot,
    origin: &UniverseAddress,
) -> Result<PublicPlayerSnapshot, ProjectionError> {
    Ok(PublicPlayerSnapshot {
        player_id: player.player_id.clone(),
        address: player.address.clone(),
        position: render_position(origin, &player.address)?,
        orientation: player.orientation,
        linear_velocity: player.linear_velocity,
        angular_velocity: player.angular_velocity,
        surface_contact: player.surface_contact,
        locomotion_kind: player.locomotion.kind,
        life_state: public_life_state(&player.life_state),
        helmet_closed: player.helmet_closed,
        jetpack_enabled: player.jetpack_enabled,
    })
}

fn public_machine_state(world: &WorldState, block: &BlockSnapshot) -> Option<PublicMachineState> {
    if !matches!(block.kind, BlockKind::Refinery | BlockKind::Assembler) {
        return None;
    }
    let Some(queue) = world.production_queues.get(&block.block_id) else {
        return Some(PublicMachineState::Idle);
    };
    if queue.is_empty() {
        return Some(PublicMachineState::Idle);
    }
    Some(match world.production_job_status(&block.block_id, 0) {
        ProductionJobStatus::Running => PublicMachineState::Operating,
        ProductionJobStatus::OutputBlocked => PublicMachineState::Blocked,
        ProductionJobStatus::Queued
        | ProductionJobStatus::PausedPower
        | ProductionJobStatus::PausedRoute => PublicMachineState::Paused,
    })
}

fn public_block(world: &WorldState, block: &BlockSnapshot) -> PublicBlockSnapshot {
    PublicBlockSnapshot {
        block_id: block.block_id.clone(),
        coordinate: block.coordinate,
        kind: block.kind,
        orientation: block.orientation,
        health: block.health,
        max_health: block.max_health,
        construction_complete: block.construction_complete,
        machine_state: public_machine_state(world, block),
    }
}

fn public_grid(
    world: &WorldState,
    grid: &GridSnapshot,
    origin: &UniverseAddress,
) -> Result<PublicGridSnapshot, ProjectionError> {
    let mut blocks = grid
        .blocks
        .iter()
        .map(|block| public_block(world, block))
        .collect::<Vec<_>>();
    blocks.sort_by(|left, right| left.block_id.cmp(&right.block_id));
    Ok(PublicGridSnapshot {
        grid_id: grid.grid_id.clone(),
        owner_player_id: grid.owner_player_id.clone(),
        address: grid.address.clone(),
        position: render_position(origin, &grid.address)?,
        orientation: grid.orientation,
        linear_velocity: grid.linear_velocity,
        angular_velocity: grid.angular_velocity,
        anchored: grid.anchored,
        power: grid.power,
        blocks,
    })
}

fn public_drop(
    drop: &DeathDropSnapshot,
    _origin: &UniverseAddress,
) -> Result<PublicDeathDropSnapshot, ProjectionError> {
    celestial::validate_universe_address(&drop.address, &drop.address.universe_id).map_err(
        |source| {
            ProjectionError::InvalidCanonicalSnapshot(format!(
                "death-drop address is invalid: {source}"
            ))
        },
    )?;
    Ok(PublicDeathDropSnapshot {
        drop_id: drop.drop_id.clone(),
        address: drop.address.clone(),
    })
}

fn public_voxel_chunks(
    body_id: &str,
    voxels: &[VoxelSnapshot],
) -> Result<Vec<PublicVoxelChunkSnapshot>, ProjectionError> {
    let edge = i32::from(content::manifest().physics.voxel_collision_chunk_edge_cells);
    let mut chunks = BTreeMap::<(i32, i32, i32), Vec<VoxelSnapshot>>::new();
    for voxel in voxels {
        let key = (
            voxel.coordinate.x.div_euclid(edge),
            voxel.coordinate.y.div_euclid(edge),
            voxel.coordinate.z.div_euclid(edge),
        );
        chunks.entry(key).or_default().push(voxel.clone());
    }
    chunks
        .into_iter()
        .map(|((x, y, z), mut voxels)| {
            voxels.sort_by_key(|voxel| voxel.coordinate);
            let chunk_id = format!("{body_id}:chunk:{x}:{y}:{z}");
            let revision = projected_revision(&(&chunk_id, body_id, &voxels))?;
            Ok(PublicVoxelChunkSnapshot {
                chunk_id,
                body_id: body_id.into(),
                revision,
                voxels,
            })
        })
        .collect()
}

fn chunk_coordinates(chunk: &PublicVoxelChunkSnapshot) -> Result<(i32, i32, i32), ProjectionError> {
    let first = chunk.voxels.first().ok_or_else(|| {
        ProjectionError::InvalidCanonicalSnapshot(format!(
            "voxel chunk {} is empty",
            chunk.chunk_id
        ))
    })?;
    let edge = i32::from(content::manifest().physics.voxel_collision_chunk_edge_cells);
    Ok((
        first.coordinate.x.div_euclid(edge),
        first.coordinate.y.div_euclid(edge),
        first.coordinate.z.div_euclid(edge),
    ))
}

fn chunk_address(
    world_seed: u64,
    body_id: &str,
    chunk: &PublicVoxelChunkSnapshot,
) -> Result<UniverseAddress, ProjectionError> {
    let body = celestial::body_snapshot(world_seed, body_id);
    let (x, y, z) = chunk_coordinates(chunk)?;
    let edge = i128::from(content::manifest().physics.voxel_collision_chunk_edge_cells);
    let center_um = |coordinate: i32| (i128::from(coordinate) * edge + edge / 2) * 1_000_000;
    celestial::address_from_origin_offset_um(
        &body.center,
        [center_um(x), center_um(y), center_um(z)],
    )
    .map_err(|source| {
        ProjectionError::InvalidCanonicalSnapshot(format!(
            "voxel chunk address cannot normalize: {source}"
        ))
    })
}

fn public_player_motion(player: &PlayerMotionSnapshot) -> PublicPlayerMotionSnapshot {
    PublicPlayerMotionSnapshot {
        player_id: player.player_id.clone(),
        address: player.address.clone(),
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

fn private_structure_equal(left: &ActorPrivateSnapshot, right: &ActorPrivateSnapshot) -> bool {
    left.committed_operation_sequence == right.committed_operation_sequence
        && left.inventories == right.inventories
        && left.death_drops == right.death_drops
        && left.owned_grid_masses == right.owned_grid_masses
        && left.production_queues == right.production_queues
        && left.player.player_id == right.player.player_id
        && left.player.inventory_id == right.player.inventory_id
        && left.player.experience == right.player.experience
        && left.player.level == right.player.level
        && left.player.next_level_experience == right.player.next_level_experience
        && left.player.career == right.player.career
        && left.player.suit_oxygen_milli == right.player.suit_oxygen_milli
        && left.player.critical_oxygen_milli == right.player.critical_oxygen_milli
        && left.player.helmet_closed == right.player.helmet_closed
}

fn private_player_motion(player: &PlayerSnapshot) -> PlayerMotionSnapshot {
    PlayerMotionSnapshot {
        player_id: player.player_id.clone(),
        address: player.address.clone(),
        position: player.position,
        orientation: player.orientation,
        linear_velocity: player.linear_velocity,
        angular_velocity: player.angular_velocity,
        surface_contact: player.surface_contact,
        locomotion: player.locomotion.clone(),
        movement_epoch: player.movement_epoch,
        last_received_input_sequence: player.last_received_input_sequence,
        last_processed_input_sequence: player.last_processed_input_sequence,
        control_linear_input: player.control_linear_input,
        control_angular_input: player.control_angular_input,
        boost: player.boost,
        dampeners: player.dampeners,
        jump: player.jump,
        control_expires_at_simulation_tick: player.control_expires_at_simulation_tick,
        jetpack_enabled: player.jetpack_enabled,
        life_state: player.life_state.clone(),
        environment: player.environment.clone(),
    }
}

fn delta_private_components(
    previous: Option<&ActorPrivateSnapshot>,
    current: Option<&ActorPrivateSnapshot>,
) -> (Option<ActorPrivateSnapshot>, Option<PlayerMotionSnapshot>) {
    let Some(current) = current else {
        return (None, None);
    };
    let Some(previous) = previous else {
        return (Some(current.clone()), None);
    };
    if !private_structure_equal(previous, current) {
        return (Some(current.clone()), None);
    }
    if previous.player != current.player {
        return (None, Some(private_player_motion(&current.player)));
    }
    (None, None)
}

fn public_grid_motion(grid: &GridMotionSnapshot) -> PublicGridMotionSnapshot {
    PublicGridMotionSnapshot {
        grid_id: grid.grid_id.clone(),
        address: grid.address.clone(),
        position: grid.position,
        orientation: grid.orientation,
        linear_velocity: grid.linear_velocity,
        angular_velocity: grid.angular_velocity,
    }
}

fn projected_revision<T: Serialize>(value: &T) -> Result<u64, ProjectionError> {
    let hash = fixed_hash(value)?;
    let digest = blake3::hash(hash.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    Ok(u64::from_le_bytes(bytes))
}

fn fixed_hash<T: Serialize>(value: &T) -> Result<String, ProjectionError> {
    value.serialize(FiniteValidator).map_err(|source| {
        ProjectionError::InvalidCanonicalSnapshot(format!(
            "projection hash material contains an invalid scalar: {source}"
        ))
    })?;
    let value = serde_json::to_value(value).map_err(|source| {
        ProjectionError::InvalidCanonicalSnapshot(format!(
            "projection hash material cannot serialize: {source}"
        ))
    })?;
    let fixed = fixed_value(value)?;
    let bytes = serde_json::to_vec(&fixed).map_err(|source| {
        ProjectionError::InvalidCanonicalSnapshot(format!(
            "fixed projection hash material cannot serialize: {source}"
        ))
    })?;
    let mut domain = b"the-verse/interest-view/v1\0".to_vec();
    domain.extend(bytes);
    Ok(digest(&domain))
}

fn fixed_value(value: Value) -> Result<Value, ProjectionError> {
    match value {
        Value::Number(number) if number.is_f64() => {
            let scalar = number.as_f64().ok_or_else(|| {
                ProjectionError::InvalidCanonicalSnapshot(
                    "projection contains an invalid float".into(),
                )
            })?;
            let scaled = fixed_scalar(scalar)?;
            Ok(Value::Array(vec![
                Value::String("fixed_1e6".into()),
                Value::Number(scaled.into()),
            ]))
        }
        Value::Array(values) => values
            .into_iter()
            .map(fixed_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| fixed_value(value).map(|value| (key, value)))
            .collect::<Result<serde_json::Map<_, _>, _>>()
            .map(Value::Object),
        other => Ok(other),
    }
}

fn fixed_scalar(value: f64) -> Result<i64, ProjectionError> {
    if !value.is_finite() {
        return Err(ProjectionError::InvalidCanonicalSnapshot(
            "projection contains a non-finite value".into(),
        ));
    }
    let scaled = (value * FIXED_SCALE).round();
    if !scaled.is_finite() || scaled < i64::MIN as f64 || scaled >= I64_UPPER_EXCLUSIVE {
        return Err(ProjectionError::InvalidCanonicalSnapshot(
            "projection value exceeds fixed-point range".into(),
        ));
    }
    Ok(scaled as i64)
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[derive(Debug)]
struct FiniteError(String);

impl std::fmt::Display for FiniteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for FiniteError {}

impl SerdeError for FiniteError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

#[derive(Clone, Copy)]
struct FiniteValidator;

struct FiniteCompound;

impl Serializer for FiniteValidator {
    type Ok = ();
    type Error = FiniteError;
    type SerializeSeq = FiniteCompound;
    type SerializeTuple = FiniteCompound;
    type SerializeTupleStruct = FiniteCompound;
    type SerializeTupleVariant = FiniteCompound;
    type SerializeMap = FiniteCompound;
    type SerializeStruct = FiniteCompound;
    type SerializeStructVariant = FiniteCompound;

    fn serialize_f32(self, value: f32) -> Result<(), Self::Error> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(FiniteError("non-finite f32".into()))
        }
    }

    fn serialize_f64(self, value: f64) -> Result<(), Self::Error> {
        if !value.is_finite() {
            return Err(FiniteError("non-finite f64".into()));
        }
        let scaled = value * FIXED_SCALE;
        if !scaled.is_finite() || scaled < i64::MIN as f64 || scaled > i64::MAX as f64 {
            return Err(FiniteError("f64 exceeds fixed-point range".into()));
        }
        Ok(())
    }

    fn serialize_bool(self, _: bool) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i8(self, _: i8) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i16(self, _: i16) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i32(self, _: i32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i64(self, _: i64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i128(self, _: i128) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u8(self, _: u8) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u16(self, _: u16) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u32(self, _: u32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u64(self, _: u64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u128(self, _: u128) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_char(self, _: char) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_str(self, _: &str) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_none(self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_unit(self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(FiniteCompound)
    }
}

macro_rules! finite_element {
    ($trait_name:ident, $method:ident) => {
        impl $trait_name for FiniteCompound {
            type Ok = ();
            type Error = FiniteError;
            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
                value.serialize(FiniteValidator)
            }
            fn end(self) -> Result<(), Self::Error> {
                Ok(())
            }
        }
    };
}

finite_element!(SerializeSeq, serialize_element);
finite_element!(SerializeTuple, serialize_element);
finite_element!(SerializeTupleStruct, serialize_field);

impl SerializeTupleVariant for FiniteCompound {
    type Ok = ();
    type Error = FiniteError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(FiniteValidator)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl SerializeMap for FiniteCompound {
    type Ok = ();
    type Error = FiniteError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        key.serialize(FiniteValidator)
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(FiniteValidator)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl SerializeStruct for FiniteCompound {
    type Ok = ();
    type Error = FiniteError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(FiniteValidator)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl SerializeStructVariant for FiniteCompound {
    type Ok = ();
    type Error = FiniteError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(FiniteValidator)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use verse_interest_verifier::{ErrorCode, InterestVerifier, StageKind, VerifierConfig};
    use verse_protocol::{
        BlockKind, CELESTIAL_REGISTRY_SCHEMA_VERSION, IVec3, InventoryContents, InventoryDomain,
        PROTOCOL_VERSION, PlayerDeathCause, Quat, ServerMessage, SessionRole,
        UNIVERSE_MANIFEST_SCHEMA_VERSION,
    };

    use super::*;
    use crate::model::{Block, DeathDrop, Grid, InventoryRecord};

    fn address(world: &WorldState, x: f64) -> UniverseAddress {
        world
            .address_for_active_position(Vec3::new(x, 0.0, 0.0))
            .expect("fixture address")
    }

    fn place_player(world: &mut WorldState, player_id: &str, x: f64) {
        let exact = address(world, x);
        let player = world.player.get_mut(player_id).expect("fixture player");
        player.address = exact;
        player.position = Vec3::new(x, 0.0, 0.0);
        world.simulation_tick += 1;
    }

    fn place_grid(world: &mut WorldState, grid_id: &str, x: f64) {
        let exact = address(world, x);
        let grid = world.grids.get_mut(grid_id).expect("fixture grid");
        grid.address = exact;
        grid.position = Vec3::new(x, 0.0, 0.0);
    }

    fn place_drop(world: &mut WorldState, drop_id: &str, x: f64) {
        let exact = address(world, x);
        let drop = world.death_drops.get_mut(drop_id).expect("fixture drop");
        drop.address = exact;
        drop.position = Vec3::new(x, 0.0, 0.0);
    }

    fn verifier_for(world: &WorldState, role: SessionRole) -> InterestVerifier {
        let manifest = content::manifest();
        InterestVerifier::new(VerifierConfig::new(
            role,
            crate::WORLD_SCHEMA_VERSION,
            crate::EVENT_SCHEMA_VERSION,
            manifest.schema_version,
            &manifest.manifest_version,
            crate::content::manifest_hash(),
            &world.universe_id,
            &world.celestial_registry_hash,
            &world.universe_manifest_hash,
        ))
        .expect("verifier config")
    }

    fn commit_wire(verifier: &mut InterestVerifier, message: &ServerMessage) -> StageKind {
        let raw = serde_json::to_vec(message).expect("wire frame serializes");
        let token = verifier.stage(&raw).expect("server frame verifies");
        let kind = verifier.pending_kind().expect("pending kind");
        let raw_value: serde_json::Value =
            serde_json::from_slice(&raw).expect("raw message parses as JSON");
        let sanitized_value: serde_json::Value = serde_json::from_str(
            verifier
                .pending_sanitized_json()
                .expect("pending sanitized message"),
        )
        .expect("sanitized message parses as JSON");
        assert_eq!(sanitized_value, raw_value);
        let outcome = verifier.commit(token).expect("verified frame commits");
        assert_eq!(outcome.kind, kind);
        kind
    }

    fn establish_verifier(verifier: &mut InterestVerifier, world: &WorldState, role: SessionRole) {
        let manifest = content::manifest();
        let welcome = ServerMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            world_schema_version: crate::WORLD_SCHEMA_VERSION,
            event_schema_version: crate::EVENT_SCHEMA_VERSION,
            content_schema_version: manifest.schema_version,
            content_manifest_version: manifest.manifest_version.clone(),
            celestial_registry_schema_version: CELESTIAL_REGISTRY_SCHEMA_VERSION,
            universe_manifest_schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
            interest_schema_version: INTEREST_SCHEMA_VERSION,
            server_name: "projection parity test".into(),
            session_role: role,
        };
        assert_eq!(commit_wire(verifier, &welcome), StageKind::Welcome);
        let registry = ServerMessage::Registry {
            registry: Box::new(crate::registry_snapshot(world.world_seed).expect("registry")),
            universe_manifest: Box::new(
                crate::universe_manifest(
                    world.world_seed,
                    crate::WORLD_SCHEMA_VERSION,
                    crate::EVENT_SCHEMA_VERSION,
                )
                .expect("universe manifest"),
            ),
        };
        assert_eq!(commit_wire(verifier, &registry), StageKind::Registry);
    }

    fn world_with_two_actors() -> WorldState {
        let mut world = WorldState::genesis(41);
        place_player(&mut world, "player-local", 0.0);
        let mut remote = world.player.primary().clone();
        remote.player_id = "player-remote".into();
        remote.inventory_id = "inventory-player-remote".into();
        remote.address = address(&world, 30.0);
        remote.position = Vec3::new(30.0, 0.0, 0.0);
        remote.experience = 275;
        remote.suit_oxygen_milli = 412;
        remote.last_received_input_sequence = 91;
        remote.last_processed_input_sequence = 89;
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

        let remote_block = Block::new("block-remote", IVec3::ZERO, BlockKind::Structural);
        world.ledger.genesis_installed_components += remote_block.component_cost;
        world.grids.insert(
            "grid-remote".into(),
            Grid {
                grid_id: "grid-remote".into(),
                owner_player_id: "player-remote".into(),
                anchor_reward_eligible: true,
                address: address(&world, 40.0),
                position: Vec3::new(40.0, 0.0, 0.0),
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
        for (owner, suffix, x) in [
            ("player-local", "local", 2.0),
            ("player-remote", "remote", 22.0),
        ] {
            let inventory_id = format!("inventory-drop-{suffix}");
            let drop_id = format!("drop-{suffix}");
            world.inventories.insert(
                inventory_id.clone(),
                InventoryRecord {
                    inventory_id: inventory_id.clone(),
                    domain: InventoryDomain::Dropped {
                        reason: "player_death".into(),
                        owner_player_id: owner.into(),
                    },
                    contents: InventoryContents::default(),
                    capacity_liters: 8_000,
                },
            );
            world.death_drops.insert(
                drop_id.clone(),
                DeathDrop {
                    drop_id,
                    death_id: format!("death-{suffix}"),
                    inventory_id,
                    owner_player_id: owner.into(),
                    address: address(&world, x),
                    position: Vec3::new(x, 0.0, 0.0),
                    created_event_sequence: world.event_sequence,
                    cause: PlayerDeathCause::OxygenDepleted,
                },
            );
        }
        world.validate_player_roster().expect("valid fixture");
        world
    }

    fn has_player(snapshot: &ProjectedWorldSnapshot, id: &str) -> bool {
        snapshot.players.iter().any(|player| player.player_id == id)
    }

    fn removed_reason(delta: &ProjectedInterestDelta, id: &str) -> InterestRemovalReason {
        delta
            .interest
            .removed
            .iter()
            .find(|value| value.entity_id == id)
            .expect("expected removal")
            .reason
    }

    #[test]
    fn independent_verifier_accepts_real_player_baseline_motion_rebase_and_receipt() {
        let mut world = world_with_two_actors();
        let role = SessionRole::Player {
            player_id: "player-local".into(),
        };
        let mut verifier = verifier_for(&world, role.clone());
        establish_verifier(&mut verifier, &world, role);

        let mut cursor =
            InterestProjectionState::bound_player("independent-player-session", "player-local");
        let baseline = world
            .project_interest_baseline(&mut cursor)
            .expect("real player baseline");
        assert_eq!(
            commit_wire(
                &mut verifier,
                &ServerMessage::InterestBaseline {
                    baseline: Box::new(baseline),
                },
            ),
            StageKind::Baseline
        );
        let first = verifier.committed_view().expect("baseline view");
        assert!(first.has_actor_private);

        place_player(&mut world, "player-local", 1.25);
        world
            .player
            .get_mut("player-local")
            .expect("local player")
            .linear_velocity = Vec3::new(1.0, 0.0, 0.0);
        let delta = world
            .project_interest_delta(&mut cursor, &BTreeMap::new())
            .expect("real motion/rebase delta");
        assert_ne!(
            delta.interest.local_origin_address, delta.cell_address,
            "the player-local origin should be a body-local rebase rather than the cell address"
        );
        assert_eq!(
            commit_wire(
                &mut verifier,
                &ServerMessage::InterestDelta {
                    delta: Box::new(delta),
                },
            ),
            StageKind::Delta
        );
        assert_eq!(
            verifier
                .committed_view()
                .expect("delta view")
                .delta_sequence,
            first.delta_sequence + 1
        );

        let receipt = ServerMessage::IntentAccepted {
            receipt: verse_protocol::IntentReceipt {
                operation_sequence: 9_007_199_254_740_993,
                operation_id: "receipt-over-js-safe-integer".into(),
                event_sequence: world.event_sequence,
                code: "projection_parity".into(),
                message: "typed receipt remains exact".into(),
            },
        };
        assert_eq!(
            commit_wire(&mut verifier, &receipt),
            StageKind::IntentAccepted
        );
    }

    #[test]
    fn independent_verifier_rejects_tampering_and_accepts_a_real_recovery_baseline() {
        let world = world_with_two_actors();
        let role = SessionRole::Spectator;
        let mut verifier = verifier_for(&world, role.clone());
        establish_verifier(&mut verifier, &world, role);
        let mut cursor =
            InterestProjectionState::public_origin_spectator("independent-spectator-session");
        let baseline = world
            .project_interest_baseline(&mut cursor)
            .expect("real spectator baseline");

        let mut tampered = baseline.clone();
        tampered.environment.altitude_m += 1.0;
        let error = verifier
            .stage(
                &serde_json::to_vec(&ServerMessage::InterestBaseline {
                    baseline: Box::new(tampered),
                })
                .expect("tampered frame serializes"),
            )
            .expect_err("included-field tampering is rejected");
        assert_eq!(error.code(), ErrorCode::HashMismatch);
        assert!(verifier.committed_view().is_none());

        let mut side_channel_only = baseline.clone();
        side_channel_only.world_hash = "excluded-global-world-hash".into();
        side_channel_only.interest.canonical_world_hash = side_channel_only.world_hash.clone();
        assert_eq!(
            commit_wire(
                &mut verifier,
                &ServerMessage::InterestBaseline {
                    baseline: Box::new(side_channel_only),
                },
            ),
            StageKind::Baseline,
            "documented global side channels are not projected-view hash input"
        );
        let first = verifier.committed_view().expect("first committed view");

        cursor.fresh_baseline().expect("fresh recovery cursor");
        let recovery = world
            .project_interest_baseline(&mut cursor)
            .expect("real recovery baseline");
        assert_eq!(recovery.interest.session_epoch, first.session_epoch);
        assert!(recovery.interest.interest_epoch > first.interest_epoch);
        assert_ne!(recovery.interest.baseline_id, first.baseline_id);
        assert_eq!(
            commit_wire(
                &mut verifier,
                &ServerMessage::InterestBaseline {
                    baseline: Box::new(recovery),
                },
            ),
            StageKind::Baseline
        );
    }

    #[test]
    fn per_kind_boundary_hysteresis_prevents_flapping_and_reentry_is_complete() {
        let mut world = world_with_two_actors();
        place_player(&mut world, "player-remote", 2_000.0);
        let mut cursor = InterestProjectionState::bound_player("session-a", "player-local");
        let baseline = world
            .project_interest_baseline(&mut cursor)
            .expect("baseline");
        assert!(has_player(&baseline, "player-remote"));
        let baseline_revision = baseline
            .interest
            .entered
            .iter()
            .find(|value| value.entity_id == "player-remote")
            .expect("remote baseline")
            .projected_revision;

        place_player(&mut world, "player-remote", 2_249.0);
        let band = world
            .project_interest_delta(&mut cursor, &BTreeMap::new())
            .expect("band delta");
        assert!(band.interest.removed.is_empty());
        assert!(
            band.interest
                .replaced
                .iter()
                .any(|value| value.entity_id == "player-remote")
        );
        let band_revision = band
            .interest
            .replaced
            .iter()
            .find(|value| value.entity_id == "player-remote")
            .expect("remote replacement")
            .projected_revision;
        assert!(band_revision > baseline_revision);

        place_player(&mut world, "player-remote", 2_251.0);
        let first_outside = world
            .project_interest_delta(&mut cursor, &BTreeMap::new())
            .expect("first outside");
        assert!(first_outside.interest.removed.is_empty());
        world.simulation_tick += 1;
        let second_outside = world
            .project_interest_delta(&mut cursor, &BTreeMap::new())
            .expect("second outside");
        assert_eq!(
            removed_reason(&second_outside, "player-remote"),
            InterestRemovalReason::OutOfInterest
        );

        place_player(&mut world, "player-remote", 2_100.0);
        let no_reentry = world
            .project_interest_delta(&mut cursor, &BTreeMap::new())
            .expect("outside enter band");
        assert!(
            !no_reentry
                .interest
                .entered
                .iter()
                .any(|value| value.entity_id == "player-remote")
        );
        place_player(&mut world, "player-remote", 1_999.0);
        let reentry = world
            .project_interest_delta(&mut cursor, &BTreeMap::new())
            .expect("reentry");
        let entry = reentry
            .interest
            .entered
            .iter()
            .find(|value| value.entity_id == "player-remote")
            .expect("complete re-entry");
        assert!(matches!(entry.payload, InterestEntityPayload::Player(_)));
        assert!(entry.projected_revision > band_revision);
    }

    #[test]
    fn bound_anchor_is_canonical_and_two_observers_see_different_subsets() {
        let mut world = world_with_two_actors();
        place_player(&mut world, "player-local", 5_000.0);
        place_player(&mut world, "player-remote", 100.0);
        place_grid(&mut world, "grid-remote", 110.0);
        let mut bound = InterestProjectionState::bound_player("bound", "player-local");
        let bound_view = world
            .project_interest_baseline(&mut bound)
            .expect("bound view");
        assert!(has_player(&bound_view, "player-local"));
        assert!(!has_player(&bound_view, "player-remote"));
        assert!(
            !bound_view
                .grids
                .iter()
                .any(|grid| grid.grid_id == "grid-remote")
        );

        let mut spectator = InterestProjectionState::public_origin_spectator("spectator");
        let public_view = world
            .project_interest_baseline(&mut spectator)
            .expect("spectator view");
        assert!(has_player(&public_view, "player-remote"));
        assert!(!has_player(&public_view, "player-local"));
        assert!(
            public_view
                .grids
                .iter()
                .any(|grid| grid.grid_id == "grid-remote")
        );
        assert_ne!(
            bound_view.interest.view_hash,
            public_view.interest.view_hash
        );
    }

    #[test]
    fn spectator_environment_and_view_hash_do_not_follow_a_hidden_primary_player() {
        let mut first_world = world_with_two_actors();
        place_player(&mut first_world, "player-local", 5_000.0);
        let mut second_world = first_world.clone();
        place_player(&mut second_world, "player-local", 6_000.0);

        let mut first_cursor = InterestProjectionState::public_origin_spectator("same-session");
        let mut second_cursor = InterestProjectionState::public_origin_spectator("same-session");
        let first = first_world
            .project_interest_baseline(&mut first_cursor)
            .expect("first spectator baseline");
        let second = second_world
            .project_interest_baseline(&mut second_cursor)
            .expect("second spectator baseline");

        assert!(!has_player(&first, "player-local"));
        assert!(!has_player(&second, "player-local"));
        assert_eq!(first.environment, second.environment);
        assert_eq!(first.interest.view_hash, second.interest.view_hash);
    }

    #[test]
    fn ordinary_hysteresis_members_are_not_culled_by_the_selected_context_budget() {
        let world = WorldState::genesis(41);
        let anchor = world.cell_address.clone();
        let mut initial = BTreeMap::new();
        for index in 0..65 {
            let drop_id = format!("hysteresis-drop-{index:02}");
            let drop_address =
                celestial::address_from_origin_offset_um(&anchor, [2_000_000_000, 0, 0])
                    .expect("inner-boundary address");
            add_candidate(
                &mut initial,
                drop_address.clone(),
                false,
                CandidatePayload::DeathDrop(PublicDeathDropSnapshot {
                    drop_id,
                    address: drop_address,
                }),
            )
            .expect("candidate");
        }
        let mut cursor = InterestProjectionState::public_origin_spectator("hysteresis-session");
        let baseline =
            select(&cursor, &initial, &anchor, 1, &BTreeMap::new()).expect("initial membership");
        assert_eq!(baseline.members.len(), 65);
        cursor.members = baseline.members;
        cursor.payloads = baseline.payloads;
        cursor.view_hash = Some("prior-view".into());
        cursor.last_evaluated_tick = Some(1);

        let mut retained = BTreeMap::new();
        for index in 0..65 {
            let drop_id = format!("hysteresis-drop-{index:02}");
            let drop_address =
                celestial::address_from_origin_offset_um(&anchor, [2_100_000_000, 0, 0])
                    .expect("hysteresis-band address");
            add_candidate(
                &mut retained,
                drop_address.clone(),
                false,
                CandidatePayload::DeathDrop(PublicDeathDropSnapshot {
                    drop_id,
                    address: drop_address,
                }),
            )
            .expect("candidate");
        }
        let delta = select(&cursor, &retained, &anchor, 2, &BTreeMap::new())
            .expect("hysteresis membership");
        assert_eq!(delta.members.len(), 65);
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn acknowledged_tick_gap_expires_outside_member_without_a_far_replacement() {
        let mut world = world_with_two_actors();
        let mut cursor = InterestProjectionState::bound_player("slow-ack", "player-local");
        world
            .project_interest_baseline(&mut cursor)
            .expect("baseline");
        place_player(&mut world, "player-remote", 2_251.0);
        world.simulation_tick += 10;
        let delta = world
            .project_interest_delta(&mut cursor, &BTreeMap::new())
            .expect("cumulative slow-client delta");
        assert_eq!(
            removed_reason(&delta, "player-remote"),
            InterestRemovalReason::OutOfInterest
        );
        assert!(
            !delta
                .interest
                .replaced
                .iter()
                .any(|value| value.entity_id == "player-remote")
        );
    }

    #[test]
    fn newcomer_cannot_evict_a_still_valid_hysteresis_member_at_kind_cap() {
        let world = WorldState::genesis(41);
        let anchor = world.cell_address.clone();
        let band = interest_band(InterestEntityKind::DeathDrop).expect("drop band");
        let mut initial = BTreeMap::new();
        for index in 0..band.maximum_entities {
            let drop_id = format!("retained-drop-{index:04}");
            add_candidate(
                &mut initial,
                anchor.clone(),
                false,
                CandidatePayload::DeathDrop(PublicDeathDropSnapshot {
                    drop_id,
                    address: anchor.clone(),
                }),
            )
            .expect("candidate");
        }
        let mut cursor = InterestProjectionState::public_origin_spectator("cap-session");
        let baseline =
            select(&cursor, &initial, &anchor, 1, &BTreeMap::new()).expect("full-cap baseline");
        cursor.members = baseline.members;
        cursor.payloads = baseline.payloads;
        cursor.delta_sequence = baseline.delta_sequence;
        cursor.view_hash = Some("prior-view".into());
        cursor.last_evaluated_tick = Some(1);

        let retained_id =
            InterestEntityIdentity::new(InterestEntityKind::DeathDrop, "retained-drop-0000");
        let mut next = initial;
        let retained = next.get_mut(&retained_id).expect("retained member");
        retained.address = celestial::address_from_origin_offset_um(
            &anchor,
            [i128::from(band.enter_radius_m + 1) * 1_000_000, 0, 0],
        )
        .expect("hysteresis-band address");
        add_candidate(
            &mut next,
            anchor.clone(),
            false,
            CandidatePayload::DeathDrop(PublicDeathDropSnapshot {
                drop_id: "new-near-drop".into(),
                address: anchor.clone(),
            }),
        )
        .expect("new near candidate");
        let selection =
            select(&cursor, &next, &anchor, 2, &BTreeMap::new()).expect("capacity selection");
        assert!(selection.members.contains_key(&retained_id));
        assert!(
            !selection.members.contains_key(&InterestEntityIdentity::new(
                InterestEntityKind::DeathDrop,
                "new-near-drop",
            ))
        );
        assert!(selection.removed.is_empty());
    }

    #[test]
    fn unique_entity_churn_retains_only_the_bounded_visible_cursor_state() {
        let world = WorldState::genesis(41);
        let anchor = world.cell_address.clone();
        let mut cursor = InterestProjectionState::public_origin_spectator("churn-session");
        let mut last_revision = 0;
        for tick in 1..=2_048_u64 {
            let drop_id = format!("ephemeral-drop-{tick}");
            let drop_address = celestial::address_from_origin_offset_um(&anchor, [0, 0, 0])
                .expect("visible address");
            let mut candidates = BTreeMap::new();
            add_candidate(
                &mut candidates,
                drop_address.clone(),
                false,
                CandidatePayload::DeathDrop(PublicDeathDropSnapshot {
                    drop_id,
                    address: drop_address,
                }),
            )
            .expect("candidate");
            let selection = select(&cursor, &candidates, &anchor, tick, &BTreeMap::new())
                .expect("bounded churn selection");
            assert_eq!(selection.members.len(), 1);
            assert_eq!(selection.payloads.len(), 1);
            let revision = selection
                .members
                .values()
                .next()
                .expect("visible member")
                .projected_revision;
            assert!(revision > last_revision);
            last_revision = revision;
            cursor.members = selection.members;
            cursor.payloads = selection.payloads;
            cursor.delta_sequence = selection.delta_sequence;
            cursor.view_hash = Some(format!("view-{tick}"));
            cursor.last_evaluated_tick = Some(tick);
        }
        assert_eq!(cursor.visible_entity_count(), 1);
        assert_eq!(cursor.payloads.len(), 1);
    }

    #[test]
    fn spatial_query_cost_is_independent_of_irrelevant_far_entities() {
        let base = world_with_two_actors();
        let mut crowded = base.clone();
        let far_address = address(&crowded, 6_000.0);
        for index in 0..2_048_u32 {
            let inventory_id = format!("inventory-far-drop-{index:04}");
            let drop_id = format!("far-drop-{index:04}");
            crowded.inventories.insert(
                inventory_id.clone(),
                InventoryRecord {
                    inventory_id: inventory_id.clone(),
                    domain: InventoryDomain::Dropped {
                        reason: "player_death".into(),
                        owner_player_id: "player-local".into(),
                    },
                    contents: InventoryContents::default(),
                    capacity_liters: 8_000,
                },
            );
            crowded.death_drops.insert(
                drop_id.clone(),
                DeathDrop {
                    drop_id,
                    death_id: format!("far-death-{index:04}"),
                    inventory_id,
                    owner_player_id: "player-local".into(),
                    address: far_address.clone(),
                    position: Vec3::new(6_000.0, 0.0, 0.0),
                    created_event_sequence: crowded.event_sequence,
                    cause: PlayerDeathCause::OxygenDepleted,
                },
            );
        }
        crowded
            .validate_player_roster()
            .expect("crowded authority graph");

        let cursor = InterestProjectionState::bound_player("indexed", "player-local");
        let anchor = base.interest_anchor(&cursor).expect("base anchor");
        let base_source = base.projection_source().expect("base source");
        let base_query = base_source
            .candidates(&cursor, &anchor)
            .expect("base query");
        let crowded_source = crowded.projection_source().expect("crowded source");
        let crowded_anchor = crowded.interest_anchor(&cursor).expect("crowded anchor");
        let crowded_query = crowded_source
            .candidates(&cursor, &crowded_anchor)
            .expect("crowded query");

        assert_eq!(
            crowded_source.candidates.len(),
            base_source.candidates.len() + 2_048
        );
        assert_eq!(crowded_query.stats, base_query.stats);
        assert_eq!(
            crowded_query.candidates.keys().collect::<Vec<_>>(),
            base_query.candidates.keys().collect::<Vec<_>>()
        );
        let base_selection = select(
            &cursor,
            &base_query.candidates,
            &anchor,
            base_source.canonical.simulation_tick,
            &BTreeMap::new(),
        )
        .expect("base selection");
        let crowded_selection = select(
            &cursor,
            &crowded_query.candidates,
            &crowded_anchor,
            crowded_source.canonical.simulation_tick,
            &BTreeMap::new(),
        )
        .expect("crowded selection");
        assert_eq!(
            crowded_selection.members.keys().collect::<Vec<_>>(),
            base_selection.members.keys().collect::<Vec<_>>()
        );
        assert!(
            crowded_query
                .candidates
                .keys()
                .all(|identity| !identity.entity_id.starts_with("far-drop-"))
        );
    }

    #[test]
    fn cached_projection_source_matches_direct_private_and_public_projection() {
        let world = world_with_two_actors();
        let source = world.projection_source().expect("projection source");
        let mut direct_cursor =
            InterestProjectionState::bound_player("source-equivalence", "player-local");
        let mut cached_cursor = direct_cursor.clone();

        let direct = world
            .project_interest_world_snapshot_from_source(
                &source,
                &mut direct_cursor,
                &BTreeMap::new(),
            )
            .expect("direct projection");
        let cached = source
            .project_interest_world_snapshot(&mut cached_cursor)
            .expect("cached projection");

        assert_eq!(cached, direct);
        assert_eq!(cached_cursor, direct_cursor);
    }

    #[test]
    fn structural_voxel_change_bypasses_the_lower_motion_update_cadence() {
        let mut world = world_with_two_actors();
        let mut cursor = InterestProjectionState::bound_player("voxel-structural", "player-local");
        let baseline = world
            .projection_source()
            .expect("baseline source")
            .project_interest_baseline(&mut cursor)
            .expect("baseline");
        let coordinate = baseline
            .voxel_chunks
            .iter()
            .flat_map(|chunk| &chunk.voxels)
            .map(|voxel| voxel.coordinate)
            .next()
            .expect("visible proof chunk has voxels");
        world.voxels.remove(coordinate).expect("voxel removes");
        world.event_sequence += 1;
        world.simulation_tick += 1;
        let voxel_band = interest_band(InterestEntityKind::VoxelChunk).expect("voxel band");
        assert!(
            !world
                .simulation_tick
                .is_multiple_of(u64::from(voxel_band.update_interval_ticks))
        );

        let delta = world
            .projection_source()
            .expect("changed source")
            .project_interest_delta(&mut cursor, &BTreeMap::new())
            .expect("structural delta");
        assert!(delta.interest.replaced.iter().any(|entity| {
            entity.kind == InterestEntityKind::VoxelChunk
                && matches!(&entity.payload, InterestEntityPayload::VoxelChunk(chunk)
                    if chunk.voxels.iter().all(|voxel| voxel.coordinate != coordinate))
        }));
    }

    #[test]
    fn exact_addresses_not_renderer_floats_decide_membership() {
        let mut world = world_with_two_actors();
        place_player(&mut world, "player-remote", 5_000.0);
        world
            .player
            .get_mut("player-remote")
            .expect("remote")
            .position = Vec3::ZERO;
        let mut cursor = InterestProjectionState::bound_player("exact", "player-local");
        let result = world.project_interest_baseline(&mut cursor);
        assert!(matches!(result, Err(ProjectionError::InvalidAuthority(_))));

        let left =
            celestial::address_from_origin_offset_um(&world.cell_address, [-10_000_500_000, 0, 0])
                .expect("negative carry");
        let right =
            celestial::address_from_origin_offset_um(&world.cell_address, [-9_999_500_000, 0, 0])
                .expect("neighbor cell carry");
        assert_eq!(
            squared_distance_um(&left, &right).expect("distance"),
            1_000_000_u128.pow(2)
        );
    }

    #[test]
    fn projections_are_stable_ordered_private_and_non_mutating() {
        let world = world_with_two_actors();
        let before = world.clone();
        let mut first_cursor = InterestProjectionState::bound_player("same", "player-local");
        let mut second_cursor = InterestProjectionState::bound_player("same", "player-local");
        let first = world
            .project_interest_baseline(&mut first_cursor)
            .expect("first");
        let second = world
            .project_interest_baseline(&mut second_cursor)
            .expect("second");
        assert_eq!(first, second);
        assert_eq!(world, before);
        assert!(first.interest.entered.windows(2).all(|values| {
            (values[0].entity_id.as_str(), values[0].kind)
                <= (values[1].entity_id.as_str(), values[1].kind)
        }));
        let private = first.actor_private.expect("private overlay");
        assert_eq!(private.player.player_id, "player-local");
        assert!(
            !private
                .inventories
                .iter()
                .any(|value| value.inventory_id == "inventory-player-remote")
        );
        assert!(
            !private
                .death_drops
                .iter()
                .any(|value| value.drop_id == "drop-remote")
        );
        let public_drop = first
            .death_drops
            .iter()
            .find(|value| value.drop_id == "drop-local")
            .expect("local salvage marker is public");
        let public_drop = serde_json::to_value(public_drop).expect("public drop serializes");
        assert_eq!(
            public_drop
                .as_object()
                .expect("public drop is an object")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["address", "drop_id"])
        );
        for private_field in [
            "owner_player_id",
            "inventory_id",
            "death_id",
            "cause",
            "created_event_sequence",
        ] {
            assert!(public_drop.get(private_field).is_none());
        }
    }

    #[test]
    fn private_assets_are_cleared_when_their_public_authority_link_is_not_visible() {
        let mut world = world_with_two_actors();
        place_grid(&mut world, "grid-starter", 5_000.0);
        place_grid(&mut world, "grid-industry-starter", 5_100.0);
        place_drop(&mut world, "drop-local", 5_200.0);
        let mut cursor = InterestProjectionState::bound_player("private", "player-local");
        let private = world
            .project_interest_baseline(&mut cursor)
            .expect("baseline")
            .actor_private
            .expect("private overlay");
        assert_eq!(
            private
                .inventories
                .iter()
                .map(|value| value.inventory_id.as_str())
                .collect::<Vec<_>>(),
            ["inventory-player-local"]
        );
        assert!(private.death_drops.is_empty());
        assert!(private.owned_grid_masses.is_empty());
        assert!(private.production_queues.is_empty());
    }

    #[test]
    fn global_commitment_and_tick_are_side_channels_not_view_hash_input() {
        let world = world_with_two_actors();
        let mut changed = world.clone();
        changed.event_sequence += 1;
        changed.simulation_tick += 1;
        let mut first = InterestProjectionState::public_origin_spectator("same");
        let mut second = InterestProjectionState::public_origin_spectator("same");
        let left = world.project_interest_baseline(&mut first).expect("left");
        let right = changed
            .project_interest_baseline(&mut second)
            .expect("right");
        assert_ne!(left.world_hash, right.world_hash);
        assert_ne!(left.interest.canonical_tick, right.interest.canonical_tick);
        assert_eq!(left.interest.view_hash, right.interest.view_hash);
    }

    #[test]
    fn deltas_omit_unchanged_environment_and_split_private_motion_from_structure() {
        let mut world = world_with_two_actors();
        let mut spectator = InterestProjectionState::public_origin_spectator("sparse-public");
        world
            .project_interest_baseline(&mut spectator)
            .expect("spectator baseline");
        world.event_sequence += 1;
        world.simulation_tick += 1;
        let public_delta = world
            .project_interest_delta(&mut spectator, &BTreeMap::new())
            .expect("spectator delta");
        assert!(public_delta.environment.is_none());
        assert!(public_delta.actor_private.is_none());
        assert!(public_delta.actor_private_motion.is_none());

        let mut player = InterestProjectionState::bound_player("sparse-player", "player-local");
        world
            .project_interest_baseline(&mut player)
            .expect("player baseline");
        world
            .player
            .get_mut("player-local")
            .expect("local player")
            .linear_velocity = Vec3::new(1.0, 0.0, 0.0);
        world.simulation_tick += 1;
        let motion = world
            .project_interest_delta(&mut player, &BTreeMap::new())
            .expect("motion delta");
        assert!(motion.actor_private.is_none());
        assert_eq!(
            motion
                .actor_private_motion
                .as_ref()
                .expect("private motion")
                .linear_velocity,
            Vec3::new(1.0, 0.0, 0.0)
        );

        let local = world.player.get_mut("player-local").expect("local player");
        local.helmet_closed = !local.helmet_closed;
        world.simulation_tick += 1;
        let structural = world
            .project_interest_delta(&mut player, &BTreeMap::new())
            .expect("private structural delta");
        assert!(structural.actor_private.is_some());
        assert!(structural.actor_private_motion.is_none());
    }

    #[test]
    fn removal_reasons_are_bounded_and_transfer_requires_worker_evidence() {
        let mut world = world_with_two_actors();
        let mut cursor = InterestProjectionState::bound_player("remove", "player-local");
        world
            .project_interest_baseline(&mut cursor)
            .expect("baseline");
        world.player.by_id.remove("player-remote");
        world.inventories.remove("inventory-player-remote");
        world.grids.remove("grid-remote");
        world.death_drops.remove("drop-remote");
        world.inventories.remove("inventory-drop-remote");
        world.simulation_tick += 1;
        let grid = InterestEntityIdentity::new(InterestEntityKind::Grid, "grid-remote");
        let delta = world
            .project_interest_delta(
                &mut cursor,
                &BTreeMap::from([(grid, InterestRemovalReason::Transferred)]),
            )
            .expect("delta");
        assert_eq!(
            removed_reason(&delta, "grid-remote"),
            InterestRemovalReason::Transferred
        );
        assert_eq!(
            removed_reason(&delta, "player-remote"),
            InterestRemovalReason::Destroyed
        );
    }

    #[test]
    fn removal_evidence_cannot_hide_or_relabel_an_existing_canonical_entity() {
        let world = world_with_two_actors();
        let mut cursor = InterestProjectionState::bound_player("evidence", "player-local");
        world
            .project_interest_baseline(&mut cursor)
            .expect("baseline");
        for reason in [
            InterestRemovalReason::Destroyed,
            InterestRemovalReason::Transferred,
        ] {
            let result = world.project_interest_delta(
                &mut cursor,
                &BTreeMap::from([(
                    InterestEntityIdentity::new(InterestEntityKind::Grid, "grid-remote"),
                    reason,
                )]),
            );
            assert!(matches!(
                result,
                Err(ProjectionError::InvalidCanonicalSnapshot(ref message))
                    if message.contains("canonical entity")
            ));
        }
    }

    #[test]
    fn frontier_rebase_invalidates_prior_baseline_and_delta_chain() {
        let world = world_with_two_actors();
        let mut cursor = InterestProjectionState::public_origin_spectator("frontier");
        let baseline = world
            .project_interest_baseline(&mut cursor)
            .expect("baseline");
        assert_eq!(cursor.delta_sequence(), 0);
        let prior_baseline = baseline.interest.baseline_id;
        let prior_epoch = cursor.interest_epoch();
        cursor.fresh_baseline().expect("rebase");
        assert_eq!(cursor.interest_epoch(), prior_epoch + 1);
        assert_ne!(cursor.baseline_id(), prior_baseline);
        assert!(cursor.view_hash().is_none());
        assert!(
            world
                .project_interest_delta(&mut cursor, &BTreeMap::new())
                .is_err()
        );
    }

    #[test]
    fn fixed_hash_rejects_non_finite_values() {
        assert!(matches!(
            fixed_hash(&Vec3::new(f64::NAN, 0.0, 0.0)),
            Err(ProjectionError::InvalidCanonicalSnapshot(_))
        ));
    }

    #[test]
    fn fixed_scalar_rounding_and_i64_boundaries_are_unambiguous() {
        assert_eq!(fixed_scalar(0.000_000_5).expect("positive half step"), 1);
        assert_eq!(fixed_scalar(-0.000_000_5).expect("negative half step"), -1);
        assert_eq!(fixed_scalar(0.000_000_499).expect("below half step"), 0);

        let upper_exclusive = 9_223_372_036_854_775_808.0 / FIXED_SCALE;
        assert!(matches!(
            fixed_scalar(upper_exclusive),
            Err(ProjectionError::InvalidCanonicalSnapshot(_))
        ));
        let lower_inclusive = -9_223_372_036_854_775_808.0 / FIXED_SCALE;
        assert_eq!(
            fixed_scalar(lower_inclusive).expect("i64 minimum is representable"),
            i64::MIN
        );
    }

    #[test]
    fn interest_distance_overflow_fails_closed_instead_of_saturating() {
        let origin = celestial::cell_origin_address();
        let mut far = origin.clone();
        far.sector.x = "1000000".into();
        assert!(matches!(
            squared_distance_um(&origin, &far),
            Err(ProjectionError::InvalidCanonicalSnapshot(ref message))
                if message == "interest distance overflow"
        ));

        far.sector.x = i128::MAX.to_string();
        assert!(matches!(
            squared_distance_um(&origin, &far),
            Err(ProjectionError::InvalidCanonicalSnapshot(_))
        ));
    }

    #[test]
    fn equal_truncated_fingerprint_cannot_suppress_a_changed_payload() {
        let world = world_with_two_actors();
        let mut cursor = InterestProjectionState::bound_player("collision", "player-local");
        world
            .project_interest_baseline(&mut cursor)
            .expect("baseline");
        let identity = InterestEntityIdentity::new(InterestEntityKind::Player, "player-local");
        let source = world.projection_source().expect("projection source");
        let anchor = world.interest_anchor(&cursor).expect("anchor");
        let mut candidates = source
            .candidates(&cursor, &anchor)
            .expect("candidates")
            .candidates;
        let candidate = candidates
            .get_mut(&identity)
            .expect("local player candidate");
        candidate.content_fingerprint = cursor.members[&identity].content_fingerprint;
        candidate.control_critical = true;
        let CandidatePayload::Player(player) = &mut candidate.payload else {
            panic!("candidate is a player")
        };
        player.linear_velocity.x += 1.0;

        let selection = select(
            &cursor,
            &candidates,
            &anchor,
            source.canonical.simulation_tick + 1,
            &BTreeMap::new(),
        )
        .expect("selection");
        assert!(
            selection
                .replaced
                .iter()
                .any(|value| value.entity_id == "player-local")
        );
        assert_ne!(selection.payloads[&identity], cursor.payloads[&identity]);
    }
}
