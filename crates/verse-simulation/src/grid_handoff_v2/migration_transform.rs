// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant, write-free protocol-19 gameplay transformation.
//!
//! The capability in this module borrows the frozen protocol-18 source, so its
//! directory and cell locks remain held. It proves only deterministic in-memory
//! transformation. It is not a receipt, install, or activation authority.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verse_protocol::{CellKeyV1, ConservationSnapshot, InventoryDomain, PlayerLifeState};

use super::production::DraftProductionJobOriginV2;
use super::state::{DraftGridTransferCellStateV2, ValidatedDraftGridTransferCellStateV21};
use crate::event::CanonicalEvent;
use crate::identity::canonical_subject_id;
use crate::manifest_v5::ValidatedUniverseManifestV5;
use crate::model::{ContactPairKey, WorldState};
use crate::protocol19_source::{FrozenProtocol18CellEvidence, ValidatedFrozenProtocol18Source};

const IDENTITY_MAP_SCHEMA_VERSION: u32 = 1;
const PRODUCTION_ORIGIN_BLOB_SCHEMA_VERSION: u32 = 1;
const MAX_TRANSFORM_BLOB_BYTES: usize = 16 * 1_024 * 1_024;
const IDENTITY_MAP_ROOT_DOMAIN: &[u8] = b"the-verse/protocol-19-identity-map/v1\0";
const IDENTITY_SUBSET_ROOT_DOMAIN: &[u8] = b"the-verse/protocol-19-cell-identity-subset/v1\0";
const PRODUCTION_ORIGIN_ROOT_DOMAIN: &[u8] = b"the-verse/protocol-19-production-origin-map/v1\0";
const PRODUCTION_ORIGIN_SUBSET_ROOT_DOMAIN: &[u8] =
    b"the-verse/protocol-19-cell-production-origin-subset/v1\0";
const GLOBAL_CONSERVATION_ROOT_DOMAIN: &[u8] = b"the-verse/protocol-19-global-conservation/v1\0";
const NORMALIZED_GAMEPLAY_ROOT_DOMAIN: &[u8] = b"the-verse/protocol-19-normalized-gameplay/v1\0";
const ENTITY_KINDS: [&str; 6] = [
    "grid",
    "block",
    "inventory",
    "production-job",
    "death",
    "death-drop",
];

#[derive(Debug, Error)]
pub(crate) enum Protocol19MigrationTransformError {
    #[error("protocol-19 migration transform is invalid: {0}")]
    Invalid(String),
    #[error("protocol-19 migration transform JSON is invalid: {0}")]
    Json(String),
    #[error("protocol-19 migration transform blob exceeds its byte bound")]
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityMapEntryV1 {
    terminal_cell_id: String,
    legacy_subject_id: String,
    canonical_subject_id: String,
    creator_cell_id: String,
    creation_event_sequence: u64,
    entity_kind: String,
    ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityMapBlobV1 {
    schema_version: u32,
    universe_id: String,
    entries: Vec<IdentityMapEntryV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionOriginEntryV1 {
    terminal_cell_id: String,
    legacy_job_id: String,
    canonical_job_id: String,
    creator_cell_id: String,
    creation_event_sequence: u64,
    ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionOriginBlobV1 {
    schema_version: u32,
    universe_id: String,
    entries: Vec<ProductionOriginEntryV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct IdentityMapSubsetV1<'entries> {
    schema_version: u32,
    universe_id: &'entries str,
    terminal_cell_id: &'entries str,
    entries: &'entries [IdentityMapEntryV1],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ProductionOriginSubsetV1<'entries> {
    schema_version: u32,
    universe_id: &'entries str,
    terminal_cell_id: &'entries str,
    entries: &'entries [ProductionOriginEntryV1],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CellConservationV1 {
    cell_id: String,
    conservation: ConservationSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct NormalizedCellGameplayV1 {
    cell_id: String,
    state: WorldState,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct SubjectSets {
    grids: BTreeSet<String>,
    blocks: BTreeSet<String>,
    inventories: BTreeSet<String>,
    production_jobs: BTreeSet<String>,
    deaths: BTreeSet<String>,
    death_drops: BTreeSet<String>,
}

impl SubjectSets {
    fn typed(&self) -> [(&'static str, &BTreeSet<String>); 6] {
        [
            ("grid", &self.grids),
            ("block", &self.blocks),
            ("inventory", &self.inventories),
            ("production-job", &self.production_jobs),
            ("death", &self.deaths),
            ("death-drop", &self.death_drops),
        ]
    }

    fn contains(&self, kind: &str, id: &str) -> bool {
        self.typed()
            .into_iter()
            .find(|(candidate, _)| *candidate == kind)
            .is_some_and(|(_, ids)| ids.contains(id))
    }

    fn insert(&mut self, kind: &str, id: String) -> bool {
        match kind {
            "grid" => self.grids.insert(id),
            "block" => self.blocks.insert(id),
            "inventory" => self.inventories.insert(id),
            "production-job" => self.production_jobs.insert(id),
            "death" => self.deaths.insert(id),
            "death-drop" => self.death_drops.insert(id),
            _ => unreachable!("subject kinds are closed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CreationOrigin {
    creator_cell_id: String,
    event_sequence: u64,
    entity_kind: String,
    ordinal: u32,
}

#[derive(Debug, Default)]
struct TypedIdentityMaps {
    grids: BTreeMap<String, String>,
    blocks: BTreeMap<String, String>,
    inventories: BTreeMap<String, String>,
    production_jobs: BTreeMap<String, String>,
    deaths: BTreeMap<String, String>,
    death_drops: BTreeMap<String, String>,
}

impl TypedIdentityMaps {
    fn map_for_kind_mut(&mut self, kind: &str) -> &mut BTreeMap<String, String> {
        match kind {
            "grid" => &mut self.grids,
            "block" => &mut self.blocks,
            "inventory" => &mut self.inventories,
            "production-job" => &mut self.production_jobs,
            "death" => &mut self.deaths,
            "death-drop" => &mut self.death_drops,
            _ => unreachable!("subject kinds are closed"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Protocol19TransformedCell {
    cell_key: CellKeyV1,
    cell_id: String,
    state: DraftGridTransferCellStateV2,
    identity_subset_root: String,
    production_origin_root: String,
}

impl Protocol19TransformedCell {
    pub(crate) fn cell_key(&self) -> &CellKeyV1 {
        &self.cell_key
    }

    pub(crate) fn cell_id(&self) -> &str {
        &self.cell_id
    }

    pub(crate) fn validate<'state, 'manifest>(
        &'state self,
        manifest: &'manifest ValidatedUniverseManifestV5,
    ) -> Result<
        ValidatedDraftGridTransferCellStateV21<'state, 'manifest>,
        Protocol19MigrationTransformError,
    > {
        self.state
            .validate_world_v21(manifest)
            .map_err(|source| Protocol19MigrationTransformError::Invalid(source.to_string()))
    }

    pub(crate) fn world_state_hash(&self) -> &str {
        self.state.state_hash()
    }

    pub(crate) fn event_sequence(&self) -> u64 {
        self.state.base().event_sequence
    }

    pub(crate) fn event_head_hash(&self) -> &str {
        &self.state.base().last_event_hash
    }

    pub(crate) fn authority_fencing_token(&self) -> u64 {
        self.state.base().fencing_token
    }

    pub(crate) fn active_world_hash(&self) -> Result<String, Protocol19MigrationTransformError> {
        self.state
            .calculate_active_world_hash()
            .map_err(|source| Protocol19MigrationTransformError::Invalid(source.to_string()))
    }

    pub(crate) fn identity_subset_root(&self) -> &str {
        &self.identity_subset_root
    }

    pub(crate) fn production_origin_root(&self) -> &str {
        &self.production_origin_root
    }

    pub(crate) fn contains_aggregate(
        &self,
        kind: crate::cell_directory::MobileAggregateKind,
        id: &str,
    ) -> bool {
        match kind {
            crate::cell_directory::MobileAggregateKind::Player => {
                self.state.base().player.by_id.contains_key(id)
            }
            crate::cell_directory::MobileAggregateKind::Grid => {
                self.state.base().grids.contains_key(id)
            }
        }
    }

    pub(crate) fn resident_aggregates(
        &self,
    ) -> Vec<(crate::cell_directory::MobileAggregateKind, String)> {
        self.state
            .base()
            .player
            .by_id
            .keys()
            .map(|id| {
                (
                    crate::cell_directory::MobileAggregateKind::Player,
                    id.clone(),
                )
            })
            .chain(
                self.state
                    .base()
                    .grids
                    .keys()
                    .map(|id| (crate::cell_directory::MobileAggregateKind::Grid, id.clone())),
            )
            .collect()
    }
}

/// Non-Serde proof of a deterministic, write-free transformation. Borrowing
/// `source` keeps every frozen protocol-18 lock held for the capability's
/// complete lifetime.
#[derive(Debug)]
pub(crate) struct ValidatedProtocol19MigrationTransform<'source> {
    source: &'source ValidatedFrozenProtocol18Source,
    target_manifest_hash: String,
    cells: Vec<Protocol19TransformedCell>,
    identity_map_bytes: Vec<u8>,
    identity_map_root: String,
    identity_map_entry_count: u64,
    production_origin_bytes: Vec<u8>,
    production_origin_root: String,
    production_origin_count: u64,
    global_conservation_root: String,
    normalized_gameplay_root: String,
    grid_identity_map: BTreeMap<(String, String), String>,
}

impl<'source> ValidatedProtocol19MigrationTransform<'source> {
    pub(crate) fn derive(
        source: &'source ValidatedFrozenProtocol18Source,
        target_manifest: &ValidatedUniverseManifestV5,
    ) -> Result<Self, Protocol19MigrationTransformError> {
        if source.world_seed() != target_manifest.world_seed()
            || source.universe_id() != target_manifest.universe_id()
            || source.cells().is_empty()
        {
            return Err(Protocol19MigrationTransformError::Invalid(
                "source and target manifest do not identify one nonempty universe".into(),
            ));
        }

        let mut identity_entries = Vec::new();
        let mut production_entries = Vec::new();
        let mut cells = Vec::with_capacity(source.cells().len());
        let mut normalized_cells = Vec::with_capacity(source.cells().len());
        let mut source_conservation = Vec::with_capacity(source.cells().len());
        let mut target_conservation = Vec::with_capacity(source.cells().len());
        let source_cell_ids = source
            .cells()
            .iter()
            .map(|cell| cell.cell_id().to_owned())
            .collect::<BTreeSet<_>>();
        let mut generated_subject_ids = BTreeSet::new();
        let mut preserved_subject_ids = BTreeSet::new();

        for cell in source.cells() {
            if !cell.state().player_transfer_locks.is_empty()
                || !cell.state().player_transfer_reservations.is_empty()
            {
                return Err(Protocol19MigrationTransformError::Invalid(format!(
                    "cell {} retains an in-flight player transfer",
                    cell.cell_id()
                )));
            }
            let (entries, maps) = derive_cell_identity_map(source.universe_id(), cell)?;
            for entry in &entries {
                if !generated_subject_ids.insert(entry.canonical_subject_id.clone()) {
                    return Err(Protocol19MigrationTransformError::Invalid(
                        "two transformed subjects derived the same canonical identity".into(),
                    ));
                }
            }
            preserved_subject_ids.extend(preserved_identity_references(cell));
            let event_zero_subjects = subject_sets(cell.event_zero_state());
            let terminal_subjects = subject_sets(cell.state());
            for (kind, ids) in event_zero_subjects.typed() {
                for id in ids {
                    if terminal_subjects.contains(kind, id) {
                        preserved_subject_ids.insert(id.clone());
                    }
                }
            }

            let mut target_base = cell.state().clone();
            rewrite_state(&mut target_base, &maps)?;
            target_manifest
                .manifest_hash()
                .clone_into(&mut target_base.universe_manifest_hash);

            let mut origins = BTreeMap::new();
            for entry in entries
                .iter()
                .filter(|entry| entry.entity_kind == "production-job")
            {
                let (job_id, origin) = DraftProductionJobOriginV2::new(
                    source.universe_id(),
                    &entry.creator_cell_id,
                    entry.creation_event_sequence,
                    entry.ordinal,
                )
                .map_err(|source| Protocol19MigrationTransformError::Invalid(source.to_string()))?;
                if job_id != entry.canonical_subject_id
                    || origins.insert(job_id.clone(), origin).is_some()
                {
                    return Err(Protocol19MigrationTransformError::Invalid(
                        "production provenance does not exactly cover transformed jobs".into(),
                    ));
                }
                production_entries.push(ProductionOriginEntryV1 {
                    terminal_cell_id: cell.cell_id().to_owned(),
                    legacy_job_id: entry.legacy_subject_id.clone(),
                    canonical_job_id: job_id,
                    creator_cell_id: entry.creator_cell_id.clone(),
                    creation_event_sequence: entry.creation_event_sequence,
                    ordinal: entry.ordinal,
                });
            }

            let state = DraftGridTransferCellStateV2::new_world_v21_with_production_origins(
                target_base,
                origins,
                target_manifest,
            )
            .map_err(|source| Protocol19MigrationTransformError::Invalid(source.to_string()))?;
            state
                .validate_world_v21(target_manifest)
                .map_err(|source| {
                    Protocol19MigrationTransformError::Invalid(format!(
                        "cell {} target validation failed: {source}",
                        cell.cell_id()
                    ))
                })?;

            if state.base().fencing_token != cell.authority_fencing_token() {
                return Err(Protocol19MigrationTransformError::Invalid(format!(
                    "cell {} transformation changed its exact authority fence",
                    cell.cell_id()
                )));
            }
            let mut restored = state.base().clone();
            inverse_rewrite_state(&mut restored, &maps)?;
            source
                .source_manifest_hash()
                .clone_into(&mut restored.universe_manifest_hash);
            if restored != *cell.state() {
                return Err(Protocol19MigrationTransformError::Invalid(format!(
                    "cell {} does not restore the exact frozen frontier and authority",
                    cell.cell_id()
                )));
            }

            let mut normalized = restored;
            normalized.fencing_token = 0;

            if entries
                .iter()
                .any(|entry| entry.terminal_cell_id != cell.cell_id())
            {
                return Err(Protocol19MigrationTransformError::Invalid(
                    "identity subset contains an entry for another terminal cell".into(),
                ));
            }
            let identity_subset_root = hash_json(
                IDENTITY_SUBSET_ROOT_DOMAIN,
                &IdentityMapSubsetV1 {
                    schema_version: IDENTITY_MAP_SCHEMA_VERSION,
                    universe_id: source.universe_id(),
                    terminal_cell_id: cell.cell_id(),
                    entries: &entries,
                },
            )?;
            let production_subset = production_entries
                .iter()
                .filter(|entry| entry.terminal_cell_id == cell.cell_id())
                .cloned()
                .collect::<Vec<_>>();
            if production_subset
                .iter()
                .any(|entry| entry.terminal_cell_id != cell.cell_id())
            {
                return Err(Protocol19MigrationTransformError::Invalid(
                    "production subset contains an entry for another terminal cell".into(),
                ));
            }
            let production_origin_root = hash_json(
                PRODUCTION_ORIGIN_SUBSET_ROOT_DOMAIN,
                &ProductionOriginSubsetV1 {
                    schema_version: PRODUCTION_ORIGIN_BLOB_SCHEMA_VERSION,
                    universe_id: source.universe_id(),
                    terminal_cell_id: cell.cell_id(),
                    entries: &production_subset,
                },
            )?;
            source_conservation.push(CellConservationV1 {
                cell_id: cell.cell_id().to_owned(),
                conservation: cell.state().conservation(),
            });
            target_conservation.push(CellConservationV1 {
                cell_id: cell.cell_id().to_owned(),
                conservation: state.base().conservation(),
            });
            normalized_cells.push(NormalizedCellGameplayV1 {
                cell_id: cell.cell_id().to_owned(),
                state: normalized,
            });
            identity_entries.extend(entries);
            cells.push(Protocol19TransformedCell {
                cell_key: cell.cell_key().clone(),
                cell_id: cell.cell_id().to_owned(),
                state,
                identity_subset_root,
                production_origin_root,
            });
        }

        ensure_generated_ids_do_not_collide(&generated_subject_ids, &preserved_subject_ids)?;

        identity_entries.sort();
        production_entries.sort();
        if source_conservation != target_conservation
            || cells
                .windows(2)
                .any(|pair| pair[0].cell_id >= pair[1].cell_id)
        {
            return Err(Protocol19MigrationTransformError::Invalid(
                "transformation changed conservation or cell ordering".into(),
            ));
        }
        let identity_blob = IdentityMapBlobV1 {
            schema_version: IDENTITY_MAP_SCHEMA_VERSION,
            universe_id: source.universe_id().to_owned(),
            entries: identity_entries,
        };
        let production_blob = ProductionOriginBlobV1 {
            schema_version: PRODUCTION_ORIGIN_BLOB_SCHEMA_VERSION,
            universe_id: source.universe_id().to_owned(),
            entries: production_entries,
        };
        let grid_identity_map = identity_blob
            .entries
            .iter()
            .filter(|entry| entry.entity_kind == "grid")
            .map(|entry| {
                (
                    (
                        entry.terminal_cell_id.clone(),
                        entry.legacy_subject_id.clone(),
                    ),
                    entry.canonical_subject_id.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let identity_map_bytes = canonical_blob_bytes(&identity_blob)?;
        let production_origin_bytes = canonical_blob_bytes(&production_blob)?;
        let decoded_identity =
            validate_identity_blob(&identity_map_bytes, source.universe_id(), &source_cell_ids)?;
        let decoded_production = validate_production_blob(
            &production_origin_bytes,
            source.universe_id(),
            &source_cell_ids,
        )?;
        validate_blob_projection(&decoded_identity, &decoded_production)?;
        let identity_map_root = hash_json(IDENTITY_MAP_ROOT_DOMAIN, &identity_blob)?;
        let production_origin_root = hash_json(PRODUCTION_ORIGIN_ROOT_DOMAIN, &production_blob)?;
        let global_conservation_root =
            hash_json(GLOBAL_CONSERVATION_ROOT_DOMAIN, &source_conservation)?;
        let normalized_gameplay_root =
            hash_json(NORMALIZED_GAMEPLAY_ROOT_DOMAIN, &normalized_cells)?;

        Ok(Self {
            source,
            target_manifest_hash: target_manifest.manifest_hash().to_owned(),
            cells,
            identity_map_entry_count: u64::try_from(identity_blob.entries.len()).map_err(|_| {
                Protocol19MigrationTransformError::Invalid("identity count overflowed".into())
            })?,
            identity_map_bytes,
            identity_map_root,
            production_origin_count: u64::try_from(production_blob.entries.len()).map_err(
                |_| {
                    Protocol19MigrationTransformError::Invalid("production count overflowed".into())
                },
            )?,
            production_origin_bytes,
            production_origin_root,
            global_conservation_root,
            normalized_gameplay_root,
            grid_identity_map,
        })
    }

    pub(crate) fn source(&self) -> &'source ValidatedFrozenProtocol18Source {
        self.source
    }

    pub(crate) fn target_manifest_hash(&self) -> &str {
        &self.target_manifest_hash
    }

    pub(crate) fn cells(&self) -> &[Protocol19TransformedCell] {
        &self.cells
    }

    pub(crate) fn identity_map_bytes(&self) -> &[u8] {
        &self.identity_map_bytes
    }

    pub(crate) fn identity_map_root(&self) -> &str {
        &self.identity_map_root
    }

    pub(crate) const fn identity_map_entry_count(&self) -> u64 {
        self.identity_map_entry_count
    }

    pub(crate) fn production_origin_bytes(&self) -> &[u8] {
        &self.production_origin_bytes
    }

    pub(crate) fn production_origin_root(&self) -> &str {
        &self.production_origin_root
    }

    pub(crate) const fn production_origin_count(&self) -> u64 {
        self.production_origin_count
    }

    pub(crate) fn global_conservation_root(&self) -> &str {
        &self.global_conservation_root
    }

    pub(crate) fn normalized_gameplay_root(&self) -> &str {
        &self.normalized_gameplay_root
    }

    pub(crate) fn target_aggregate_id<'id>(
        &'id self,
        kind: crate::cell_directory::MobileAggregateKind,
        terminal_cell_id: &str,
        legacy_id: &'id str,
    ) -> &'id str {
        match kind {
            crate::cell_directory::MobileAggregateKind::Player => legacy_id,
            crate::cell_directory::MobileAggregateKind::Grid => self
                .grid_identity_map
                .get(&(terminal_cell_id.to_owned(), legacy_id.to_owned()))
                .map_or(legacy_id, String::as_str),
        }
    }
}

fn subject_sets(state: &WorldState) -> SubjectSets {
    let mut subjects = SubjectSets::default();
    for (grid_id, grid) in &state.grids {
        subjects.grids.insert(grid_id.clone());
        subjects.blocks.extend(grid.blocks.keys().cloned());
    }
    subjects.inventories.extend(
        state
            .inventories
            .iter()
            .filter(|(_, inventory)| !matches!(inventory.domain, InventoryDomain::Player { .. }))
            .map(|(inventory_id, _)| inventory_id.clone()),
    );
    subjects.production_jobs.extend(
        state
            .production_queues
            .values()
            .flatten()
            .map(|job| job.job_id.clone()),
    );
    for player in state.player.by_id.values() {
        if let PlayerLifeState::Incapacitated { death_id, .. } = &player.life_state {
            subjects.deaths.insert(death_id.clone());
        }
    }
    for (drop_id, drop) in &state.death_drops {
        subjects.death_drops.insert(drop_id.clone());
        subjects.deaths.insert(drop.death_id.clone());
    }
    subjects
}

fn preserved_identity_references(cell: &FrozenProtocol18CellEvidence) -> BTreeSet<String> {
    preserved_identity_references_from(cell.state(), cell.events())
}

fn preserved_identity_references_from(
    state: &WorldState,
    events: &[CanonicalEvent],
) -> BTreeSet<String> {
    let mut identities = BTreeSet::new();
    for (player_id, player) in &state.player.by_id {
        identities.insert(player_id.clone());
        identities.insert(player.inventory_id.clone());
    }
    identities.extend(
        state
            .grids
            .values()
            .map(|grid| grid.owner_player_id.clone()),
    );
    identities.extend(
        state
            .production_queues
            .values()
            .flatten()
            .map(|job| job.owner_player_id.clone()),
    );
    identities.extend(
        state
            .death_drops
            .values()
            .map(|drop| drop.owner_player_id.clone()),
    );
    for inventory in state.inventories.values() {
        match &inventory.domain {
            InventoryDomain::Player { player_id }
            | InventoryDomain::Dropped {
                owner_player_id: player_id,
                ..
            } => {
                identities.insert(player_id.clone());
            }
            InventoryDomain::Cargo { .. } => {}
        }
    }
    identities.extend(state.processed_operations.keys().cloned());
    identities.extend(
        events
            .iter()
            .filter_map(|event| event.actor_player_id.clone()),
    );
    identities
}

fn ensure_generated_ids_do_not_collide(
    generated_subject_ids: &BTreeSet<String>,
    preserved_subject_ids: &BTreeSet<String>,
) -> Result<(), Protocol19MigrationTransformError> {
    if generated_subject_ids
        .iter()
        .any(|subject_id| preserved_subject_ids.contains(subject_id))
    {
        return Err(Protocol19MigrationTransformError::Invalid(
            "a transformed subject collides with a preserved player, actor, or event-zero identity"
                .into(),
        ));
    }
    Ok(())
}

fn derive_cell_identity_map(
    universe_id: &str,
    cell: &FrozenProtocol18CellEvidence,
) -> Result<(Vec<IdentityMapEntryV1>, TypedIdentityMaps), Protocol19MigrationTransformError> {
    if cell.event_archive_entry_count() != cell.event_sequence()
        || usize::try_from(cell.event_sequence()).ok() != Some(cell.events().len())
    {
        return Err(Protocol19MigrationTransformError::Invalid(format!(
            "cell {} archive count differs from its event frontier",
            cell.cell_id()
        )));
    }
    let initial = subject_sets(cell.event_zero_state());
    let origins = replay_creation_origins(
        cell.cell_id(),
        cell.event_zero_state(),
        cell.events(),
        cell.state(),
    )?;
    let terminal = subject_sets(cell.state());
    let mut entries = Vec::new();
    let mut maps = TypedIdentityMaps::default();
    for (kind, ids) in terminal.typed() {
        for legacy_id in ids {
            if initial.contains(kind, legacy_id) {
                continue;
            }
            let origin = origins
                .get(&(kind.to_owned(), legacy_id.clone()))
                .ok_or_else(|| {
                    Protocol19MigrationTransformError::Invalid(format!(
                        "live {kind} {legacy_id} lacks replay-derived creation provenance"
                    ))
                })?;
            let canonical_id = canonical_subject_id(
                universe_id,
                &origin.creator_cell_id,
                origin.event_sequence,
                &origin.entity_kind,
                origin.ordinal,
            )
            .map_err(|source| Protocol19MigrationTransformError::Invalid(source.to_string()))?;
            if maps
                .map_for_kind_mut(kind)
                .insert(legacy_id.clone(), canonical_id.clone())
                .is_some()
            {
                return Err(Protocol19MigrationTransformError::Invalid(
                    "one terminal subject has two creation origins".into(),
                ));
            }
            entries.push(IdentityMapEntryV1 {
                terminal_cell_id: cell.cell_id().to_owned(),
                legacy_subject_id: legacy_id.clone(),
                canonical_subject_id: canonical_id,
                creator_cell_id: origin.creator_cell_id.clone(),
                creation_event_sequence: origin.event_sequence,
                entity_kind: origin.entity_kind.clone(),
                ordinal: origin.ordinal,
            });
        }
    }
    entries.sort();
    Ok((entries, maps))
}

fn replay_creation_origins(
    cell_id: &str,
    event_zero_state: &WorldState,
    events: &[CanonicalEvent],
    terminal_state: &WorldState,
) -> Result<BTreeMap<(String, String), CreationOrigin>, Protocol19MigrationTransformError> {
    let mut replay = event_zero_state.clone();
    let mut previous = subject_sets(event_zero_state);
    let mut ever_seen = previous.clone();
    let mut origins = BTreeMap::<(String, String), CreationOrigin>::new();
    for event in events {
        if event.cell_id != cell_id {
            return Err(Protocol19MigrationTransformError::Invalid(
                "creation replay event belongs to another cell".into(),
            ));
        }
        replay
            .apply_event(event)
            .map_err(|source| Protocol19MigrationTransformError::Invalid(source.to_string()))?;
        let next = subject_sets(&replay);
        record_created_subjects(
            &event.cell_id,
            event.event_sequence,
            &previous,
            &next,
            &mut ever_seen,
            &mut origins,
        )?;
        previous = next;
    }
    // A later sleeping claim may advance only the operational lease fence;
    // the frozen validator has already proved that this does not change the
    // event-derived gameplay frontier.
    replay.fencing_token = terminal_state.fencing_token;
    if &replay != terminal_state {
        return Err(Protocol19MigrationTransformError::Invalid(format!(
            "cell {cell_id} transform replay differs from its frozen frontier"
        )));
    }
    Ok(origins)
}

fn record_created_subjects(
    creator_cell_id: &str,
    event_sequence: u64,
    previous: &SubjectSets,
    next: &SubjectSets,
    ever_seen: &mut SubjectSets,
    origins: &mut BTreeMap<(String, String), CreationOrigin>,
) -> Result<(), Protocol19MigrationTransformError> {
    for (kind, ids) in next.typed() {
        for (index, id) in ids
            .iter()
            .filter(|id| !previous.contains(kind, id))
            .enumerate()
        {
            if ever_seen.contains(kind, id) {
                return Err(Protocol19MigrationTransformError::Invalid(format!(
                    "{kind} identity {id} disappeared and was later reused"
                )));
            }
            let ordinal = u32::try_from(index).map_err(|_| {
                Protocol19MigrationTransformError::Invalid(
                    "one event created too many subjects of one entity kind".into(),
                )
            })?;
            ever_seen.insert(kind, id.clone());
            if origins
                .insert(
                    (kind.to_owned(), id.clone()),
                    CreationOrigin {
                        creator_cell_id: creator_cell_id.to_owned(),
                        event_sequence,
                        entity_kind: kind.to_owned(),
                        ordinal,
                    },
                )
                .is_some()
            {
                return Err(Protocol19MigrationTransformError::Invalid(
                    "one subject has more than one creation provenance".into(),
                ));
            }
        }
    }
    Ok(())
}

fn mapped(map: &BTreeMap<String, String>, value: &mut String) {
    if let Some(next) = map.get(value) {
        *value = next.clone();
    }
}

fn rewrite_state(
    state: &mut WorldState,
    maps: &TypedIdentityMaps,
) -> Result<(), Protocol19MigrationTransformError> {
    rewrite_state_with_maps(state, maps)
}

fn inverse_rewrite_state(
    state: &mut WorldState,
    maps: &TypedIdentityMaps,
) -> Result<(), Protocol19MigrationTransformError> {
    let inverse = |map: &BTreeMap<String, String>| {
        map.iter()
            .map(|(legacy, canonical)| (canonical.clone(), legacy.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    rewrite_state_with_maps(
        state,
        &TypedIdentityMaps {
            grids: inverse(&maps.grids),
            blocks: inverse(&maps.blocks),
            inventories: inverse(&maps.inventories),
            production_jobs: inverse(&maps.production_jobs),
            deaths: inverse(&maps.deaths),
            death_drops: inverse(&maps.death_drops),
        },
    )
}

fn rewrite_state_with_maps(
    state: &mut WorldState,
    maps: &TypedIdentityMaps,
) -> Result<(), Protocol19MigrationTransformError> {
    let mut grids = BTreeMap::new();
    for (_, mut grid) in std::mem::take(&mut state.grids) {
        mapped(&maps.grids, &mut grid.grid_id);
        let mut blocks = BTreeMap::new();
        for (_, mut block) in std::mem::take(&mut grid.blocks) {
            mapped(&maps.blocks, &mut block.block_id);
            if let Some(inventory_id) = &mut block.inventory_id {
                mapped(&maps.inventories, inventory_id);
            }
            if blocks.insert(block.block_id.clone(), block).is_some() {
                return Err(Protocol19MigrationTransformError::Invalid(
                    "block identity rewrite collided".into(),
                ));
            }
        }
        grid.blocks = blocks;
        if grids.insert(grid.grid_id.clone(), grid).is_some() {
            return Err(Protocol19MigrationTransformError::Invalid(
                "grid identity rewrite collided".into(),
            ));
        }
    }
    state.grids = grids;

    let mut inventories = BTreeMap::new();
    for (_, mut inventory) in std::mem::take(&mut state.inventories) {
        mapped(&maps.inventories, &mut inventory.inventory_id);
        if let InventoryDomain::Cargo { block_id } = &mut inventory.domain {
            mapped(&maps.blocks, block_id);
        }
        if inventories
            .insert(inventory.inventory_id.clone(), inventory)
            .is_some()
        {
            return Err(Protocol19MigrationTransformError::Invalid(
                "inventory identity rewrite collided".into(),
            ));
        }
    }
    state.inventories = inventories;

    let mut queues = BTreeMap::new();
    for (mut machine_id, mut queue) in std::mem::take(&mut state.production_queues) {
        mapped(&maps.blocks, &mut machine_id);
        for job in &mut queue {
            mapped(&maps.production_jobs, &mut job.job_id);
            mapped(&maps.blocks, &mut job.machine_block_id);
            mapped(&maps.inventories, &mut job.source_inventory_id);
            mapped(&maps.inventories, &mut job.destination_inventory_id);
        }
        if queues.insert(machine_id, queue).is_some() {
            return Err(Protocol19MigrationTransformError::Invalid(
                "production machine identity rewrite collided".into(),
            ));
        }
    }
    state.production_queues = queues;

    let mut drops = BTreeMap::new();
    for (_, mut drop) in std::mem::take(&mut state.death_drops) {
        mapped(&maps.death_drops, &mut drop.drop_id);
        mapped(&maps.deaths, &mut drop.death_id);
        mapped(&maps.inventories, &mut drop.inventory_id);
        if drops.insert(drop.drop_id.clone(), drop).is_some() {
            return Err(Protocol19MigrationTransformError::Invalid(
                "death-drop identity rewrite collided".into(),
            ));
        }
    }
    state.death_drops = drops;
    for player in state.player.by_id.values_mut() {
        if let PlayerLifeState::Incapacitated { death_id, .. } = &mut player.life_state {
            mapped(&maps.deaths, death_id);
        }
        if let Some(support) = &mut player.locomotion.support {
            mapped(&maps.grids, &mut support.body_id);
            mapped(&maps.blocks, &mut support.collider_id);
        }
    }
    let mut contact_pairs = BTreeSet::new();
    for mut pair in std::mem::take(&mut state.active_contact_pairs) {
        mapped(&maps.grids, &mut pair.body_a);
        mapped(&maps.blocks, &mut pair.collider_a);
        mapped(&maps.grids, &mut pair.body_b);
        mapped(&maps.blocks, &mut pair.collider_b);
        canonicalize_contact_pair(&mut pair);
        if !contact_pairs.insert(pair) {
            return Err(Protocol19MigrationTransformError::Invalid(
                "contact identity rewrite collided".into(),
            ));
        }
    }
    state.active_contact_pairs = contact_pairs;
    Ok(())
}

fn canonicalize_contact_pair(pair: &mut ContactPairKey) {
    if (&pair.body_b, &pair.collider_b) < (&pair.body_a, &pair.collider_a) {
        std::mem::swap(&mut pair.body_a, &mut pair.body_b);
        std::mem::swap(&mut pair.collider_a, &mut pair.collider_b);
    }
}

fn canonical_blob_bytes<T: Serialize>(
    value: &T,
) -> Result<Vec<u8>, Protocol19MigrationTransformError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|source| Protocol19MigrationTransformError::Json(source.to_string()))?;
    if bytes.is_empty() || bytes.len() > MAX_TRANSFORM_BLOB_BYTES {
        return Err(Protocol19MigrationTransformError::TooLarge);
    }
    Ok(bytes)
}

fn validate_identity_blob(
    bytes: &[u8],
    universe_id: &str,
    terminal_cell_ids: &BTreeSet<String>,
) -> Result<IdentityMapBlobV1, Protocol19MigrationTransformError> {
    if bytes.is_empty() || bytes.len() > MAX_TRANSFORM_BLOB_BYTES {
        return Err(Protocol19MigrationTransformError::TooLarge);
    }
    let blob = serde_json::from_slice::<IdentityMapBlobV1>(bytes)
        .map_err(|source| Protocol19MigrationTransformError::Json(source.to_string()))?;
    if blob.schema_version != IDENTITY_MAP_SCHEMA_VERSION
        || blob.universe_id != universe_id
        || blob.entries.windows(2).any(|pair| pair[0] >= pair[1])
        || blob.entries.iter().any(|entry| {
            !terminal_cell_ids.contains(&entry.terminal_cell_id)
                || !terminal_cell_ids.contains(&entry.creator_cell_id)
                || !ENTITY_KINDS.contains(&entry.entity_kind.as_str())
                || entry.legacy_subject_id.is_empty()
                || entry.legacy_subject_id == entry.canonical_subject_id
        })
        || blob
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.terminal_cell_id.as_str(),
                    entry.entity_kind.as_str(),
                    entry.legacy_subject_id.as_str(),
                )
            })
            .collect::<BTreeSet<_>>()
            .len()
            != blob.entries.len()
        || blob
            .entries
            .iter()
            .map(|entry| entry.canonical_subject_id.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            != blob.entries.len()
        || blob.entries.iter().any(|entry| {
            canonical_subject_id(
                universe_id,
                &entry.creator_cell_id,
                entry.creation_event_sequence,
                &entry.entity_kind,
                entry.ordinal,
            )
            .map_or(true, |derived| derived != entry.canonical_subject_id)
        })
        || canonical_blob_bytes(&blob)? != bytes
    {
        return Err(Protocol19MigrationTransformError::Invalid(
            "identity map blob is not canonical".into(),
        ));
    }
    Ok(blob)
}

pub(crate) fn recover_identity_map_root(
    bytes: &[u8],
    universe_id: &str,
    cell_ids: &BTreeSet<String>,
) -> Result<String, Protocol19MigrationTransformError> {
    let blob = validate_identity_blob(bytes, universe_id, cell_ids)?;
    hash_json(IDENTITY_MAP_ROOT_DOMAIN, &blob)
}

fn validate_production_blob(
    bytes: &[u8],
    universe_id: &str,
    terminal_cell_ids: &BTreeSet<String>,
) -> Result<ProductionOriginBlobV1, Protocol19MigrationTransformError> {
    if bytes.is_empty() || bytes.len() > MAX_TRANSFORM_BLOB_BYTES {
        return Err(Protocol19MigrationTransformError::TooLarge);
    }
    let blob = serde_json::from_slice::<ProductionOriginBlobV1>(bytes)
        .map_err(|source| Protocol19MigrationTransformError::Json(source.to_string()))?;
    if blob.schema_version != PRODUCTION_ORIGIN_BLOB_SCHEMA_VERSION
        || blob.universe_id != universe_id
        || blob.entries.windows(2).any(|pair| pair[0] >= pair[1])
        || blob.entries.iter().any(|entry| {
            !terminal_cell_ids.contains(&entry.terminal_cell_id)
                || !terminal_cell_ids.contains(&entry.creator_cell_id)
                || entry.legacy_job_id.is_empty()
                || entry.legacy_job_id == entry.canonical_job_id
        })
        || blob
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.terminal_cell_id.as_str(),
                    entry.legacy_job_id.as_str(),
                )
            })
            .collect::<BTreeSet<_>>()
            .len()
            != blob.entries.len()
        || blob
            .entries
            .iter()
            .map(|entry| entry.canonical_job_id.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            != blob.entries.len()
        || blob.entries.iter().any(|entry| {
            canonical_subject_id(
                universe_id,
                &entry.creator_cell_id,
                entry.creation_event_sequence,
                "production-job",
                entry.ordinal,
            )
            .map_or(true, |derived| derived != entry.canonical_job_id)
        })
        || canonical_blob_bytes(&blob)? != bytes
    {
        return Err(Protocol19MigrationTransformError::Invalid(
            "production origin blob is not canonical".into(),
        ));
    }
    Ok(blob)
}

pub(crate) fn recover_production_origin_root(
    bytes: &[u8],
    universe_id: &str,
    cell_ids: &BTreeSet<String>,
) -> Result<String, Protocol19MigrationTransformError> {
    let blob = validate_production_blob(bytes, universe_id, cell_ids)?;
    hash_json(PRODUCTION_ORIGIN_ROOT_DOMAIN, &blob)
}

fn validate_blob_projection(
    identity: &IdentityMapBlobV1,
    production: &ProductionOriginBlobV1,
) -> Result<(), Protocol19MigrationTransformError> {
    let expected = identity
        .entries
        .iter()
        .filter(|entry| entry.entity_kind == "production-job")
        .map(|entry| ProductionOriginEntryV1 {
            terminal_cell_id: entry.terminal_cell_id.clone(),
            legacy_job_id: entry.legacy_subject_id.clone(),
            canonical_job_id: entry.canonical_subject_id.clone(),
            creator_cell_id: entry.creator_cell_id.clone(),
            creation_event_sequence: entry.creation_event_sequence,
            ordinal: entry.ordinal,
        })
        .collect::<Vec<_>>();
    if identity.universe_id != production.universe_id || expected != production.entries {
        return Err(Protocol19MigrationTransformError::Invalid(
            "production origin blob is not the exact production-job projection of the identity map"
                .into(),
        ));
    }
    Ok(())
}

fn hash_json<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<String, Protocol19MigrationTransformError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|source| Protocol19MigrationTransformError::Json(source.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use verse_protocol::{
        InventoryContents, LocomotionSupportSnapshot, PlayerDeathCause, ProductionRecipeKind, Vec3,
    };

    use crate::event::EventPayload;
    use crate::model::{
        ActorOperationHistory, DeathDrop, InventoryRecord, ProductionJob, STARTER_GRID_ID,
    };

    #[test]
    fn canonical_blob_decoders_reject_aliases_and_unknown_fields() {
        let universe_id = "the-verse-proof-universe";
        let blob = IdentityMapBlobV1 {
            schema_version: IDENTITY_MAP_SCHEMA_VERSION,
            universe_id: universe_id.into(),
            entries: Vec::new(),
        };
        let bytes = canonical_blob_bytes(&blob).expect("blob encodes");
        let terminal_cells = BTreeSet::new();
        validate_identity_blob(&bytes, universe_id, &terminal_cells)
            .expect("canonical blob validates");

        let mut whitespace = bytes.clone();
        whitespace.push(b' ');
        assert!(validate_identity_blob(&whitespace, universe_id, &terminal_cells).is_err());

        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes).expect("blob JSON parses");
        value["invented"] = serde_json::json!(true);
        let unknown = serde_json::to_vec(&value).expect("changed blob encodes");
        assert!(validate_identity_blob(&unknown, universe_id, &terminal_cells).is_err());
    }

    #[test]
    fn blob_validation_rejects_unknown_kinds_duplicate_sources_and_projection_drift() {
        let universe_id = "the-verse-proof-universe";
        let cell_id = "11".repeat(32);
        let terminal_cells = BTreeSet::from([cell_id.clone()]);
        let entry = |entity_kind: &str, legacy_subject_id: &str, ordinal: u32| IdentityMapEntryV1 {
            terminal_cell_id: cell_id.clone(),
            legacy_subject_id: legacy_subject_id.into(),
            canonical_subject_id: canonical_subject_id(
                universe_id,
                &cell_id,
                7,
                entity_kind,
                ordinal,
            )
            .expect("canonical test ID derives"),
            creator_cell_id: cell_id.clone(),
            creation_event_sequence: 7,
            entity_kind: entity_kind.into(),
            ordinal,
        };

        let unknown = IdentityMapBlobV1 {
            schema_version: IDENTITY_MAP_SCHEMA_VERSION,
            universe_id: universe_id.into(),
            entries: vec![entry("invented-kind", "legacy", 0)],
        };
        assert!(
            validate_identity_blob(
                &canonical_blob_bytes(&unknown).expect("unknown blob encodes"),
                universe_id,
                &terminal_cells,
            )
            .is_err()
        );

        let mut duplicates = vec![
            entry("block", "legacy-duplicate", 0),
            entry("block", "legacy-duplicate", 1),
        ];
        duplicates.sort();
        let duplicate_blob = IdentityMapBlobV1 {
            schema_version: IDENTITY_MAP_SCHEMA_VERSION,
            universe_id: universe_id.into(),
            entries: duplicates,
        };
        assert!(
            validate_identity_blob(
                &canonical_blob_bytes(&duplicate_blob).expect("duplicate blob encodes"),
                universe_id,
                &terminal_cells,
            )
            .is_err()
        );

        let identity = IdentityMapBlobV1 {
            schema_version: IDENTITY_MAP_SCHEMA_VERSION,
            universe_id: universe_id.into(),
            entries: vec![entry("production-job", "legacy-job", 0)],
        };
        let production = ProductionOriginBlobV1 {
            schema_version: PRODUCTION_ORIGIN_BLOB_SCHEMA_VERSION,
            universe_id: universe_id.into(),
            entries: Vec::new(),
        };
        assert!(validate_blob_projection(&identity, &production).is_err());
    }

    #[test]
    fn canonical_ids_include_creator_cell_even_at_equal_frontiers() {
        let left =
            canonical_subject_id("the-verse-proof-universe", &"11".repeat(32), 41, "block", 0)
                .expect("left ID derives");
        let right =
            canonical_subject_id("the-verse-proof-universe", &"22".repeat(32), 41, "block", 0)
                .expect("right ID derives");
        assert_ne!(left, right);
    }

    #[test]
    fn preserved_offline_owners_and_archived_actors_block_generated_id_collisions() {
        let mut state = WorldState::genesis(808);
        let generated_id = canonical_subject_id(&state.universe_id, &state.cell_id, 7, "grid", 0)
            .expect("generated test identity derives");
        state
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("starter grid exists")
            .owner_player_id = generated_id.clone();
        state.inventories.insert(
            "inventory-offline-owner".into(),
            InventoryRecord {
                inventory_id: "inventory-offline-owner".into(),
                domain: InventoryDomain::Dropped {
                    reason: "collision-test".into(),
                    owner_player_id: "player-offline-inventory".into(),
                },
                contents: InventoryContents::default(),
                capacity_liters: 1,
            },
        );
        state.production_queues.insert(
            "block-refinery".into(),
            VecDeque::from([ProductionJob {
                job_id: "job-offline-owner".into(),
                operation_id: "offline-owner-job".into(),
                owner_player_id: "player-offline-job".into(),
                machine_block_id: "block-refinery".into(),
                recipe: ProductionRecipeKind::Refining,
                content_manifest_version: state.content_manifest_version.clone(),
                batches: 1,
                source_inventory_id: "inventory-industry-cargo-starter".into(),
                destination_inventory_id: "inventory-industry-cargo-starter".into(),
                progress_ticks: 0,
                duration_ticks: 120,
                reserved_inputs: InventoryContents::default(),
                pending_outputs: InventoryContents::default(),
                queued_event_sequence: 1,
            }]),
        );
        state.processed_operations.insert(
            "player-offline-operation".into(),
            ActorOperationHistory::default(),
        );
        let event = CanonicalEvent::new(
            1,
            state.content_manifest_version.clone(),
            state.universe_manifest_hash.clone(),
            state.celestial_registry_hash.clone(),
            state.universe_id.clone(),
            state.cell_id.clone(),
            1,
            Some("player-archived-event".into()),
            "human",
            Some("archived-operation".into()),
            Some(1),
            Some("0".repeat(64)),
            "",
            EventPayload::SuitModeChanged {
                helmet_closed: true,
                jetpack_enabled: true,
                magnetic_boots_enabled: false,
            },
        );

        let preserved = preserved_identity_references_from(&state, &[event]);
        for expected in [
            generated_id.as_str(),
            "player-offline-inventory",
            "player-offline-job",
            "player-offline-operation",
            "player-archived-event",
        ] {
            assert!(preserved.contains(expected));
        }
        let error =
            ensure_generated_ids_do_not_collide(&BTreeSet::from([generated_id]), &preserved)
                .expect_err("generated collision rejects");
        assert!(error.to_string().contains("collides with a preserved"));
    }

    #[test]
    fn typed_rewrite_updates_every_live_reference_and_is_exactly_reversible() {
        let mut state = WorldState::genesis(807);
        let player_id = state.player.player_id.clone();
        let player_address = state.player.address.clone();
        let old_grid = crate::model::STARTER_GRID_ID.to_owned();
        let old_block = "block-cargo".to_owned();
        let old_inventory = "inventory-cargo-starter".to_owned();
        let old_job = "production-job-legacy".to_owned();
        let old_death = "death-legacy".to_owned();
        let old_drop = "drop-legacy".to_owned();
        let old_drop_inventory = "inventory-drop-legacy".to_owned();

        state.production_queues.insert(
            old_block.clone(),
            VecDeque::from([ProductionJob {
                job_id: old_job.clone(),
                operation_id: "migration-transform-job".into(),
                owner_player_id: player_id.clone(),
                machine_block_id: old_block.clone(),
                recipe: ProductionRecipeKind::Refining,
                content_manifest_version: state.content_manifest_version.clone(),
                batches: 1,
                source_inventory_id: old_inventory.clone(),
                destination_inventory_id: old_inventory.clone(),
                progress_ticks: 0,
                duration_ticks: 1,
                reserved_inputs: InventoryContents::default(),
                pending_outputs: InventoryContents::default(),
                queued_event_sequence: 1,
            }]),
        );
        state.inventories.insert(
            old_drop_inventory.clone(),
            InventoryRecord {
                inventory_id: old_drop_inventory.clone(),
                domain: InventoryDomain::Dropped {
                    reason: "migration-test".into(),
                    owner_player_id: player_id.clone(),
                },
                contents: InventoryContents::default(),
                capacity_liters: 1,
            },
        );
        state.death_drops.insert(
            old_drop.clone(),
            DeathDrop {
                drop_id: old_drop.clone(),
                death_id: old_death.clone(),
                inventory_id: old_drop_inventory.clone(),
                owner_player_id: player_id.clone(),
                address: player_address,
                position: state.player.position,
                created_event_sequence: 1,
                cause: PlayerDeathCause::OxygenDepleted,
            },
        );
        state.player.life_state = PlayerLifeState::Incapacitated {
            death_id: old_death.clone(),
            cause: PlayerDeathCause::OxygenDepleted,
        };
        state.player.locomotion.support = Some(LocomotionSupportSnapshot {
            body_id: old_grid.clone(),
            collider_id: old_block.clone(),
            local_anchor: Vec3::ZERO,
            local_normal: Vec3::new(0.0, 1.0, 0.0),
        });
        state.active_contact_pairs.insert(ContactPairKey {
            body_a: old_grid.clone(),
            collider_a: old_block.clone(),
            body_b: "planet-body".into(),
            collider_b: "voxel-collider".into(),
        });
        let original = state.clone();
        let maps = TypedIdentityMaps {
            grids: BTreeMap::from([(old_grid, "zz-grid-canonical".into())]),
            blocks: BTreeMap::from([(old_block, "block-canonical".into())]),
            inventories: BTreeMap::from([
                (old_inventory, "inventory-canonical".into()),
                (old_drop_inventory, "inventory-drop-canonical".into()),
            ]),
            production_jobs: BTreeMap::from([(old_job, "production-job-canonical".into())]),
            deaths: BTreeMap::from([(old_death, "death-canonical".into())]),
            death_drops: BTreeMap::from([(old_drop, "death-drop-canonical".into())]),
        };

        rewrite_state(&mut state, &maps).expect("typed state rewrites");
        assert!(state.grids.contains_key("zz-grid-canonical"));
        assert!(state.inventories.contains_key("inventory-canonical"));
        assert!(state.death_drops.contains_key("death-drop-canonical"));
        assert_eq!(
            state.production_queues["block-canonical"][0].job_id,
            "production-job-canonical"
        );
        assert_eq!(
            state
                .player
                .locomotion
                .support
                .as_ref()
                .unwrap()
                .collider_id,
            "block-canonical"
        );
        assert!(
            state.active_contact_pairs.iter().all(|pair| {
                (&pair.body_a, &pair.collider_a) <= (&pair.body_b, &pair.collider_b)
            })
        );
        assert!(
            state
                .active_contact_pairs
                .iter()
                .any(|pair| pair.body_a == "planet-body" && pair.body_b == "zz-grid-canonical")
        );
        inverse_rewrite_state(&mut state, &maps).expect("typed state inverse rewrites");
        assert_eq!(state, original);
    }

    #[test]
    fn replay_diff_assigns_ordinals_before_filtering_terminal_subjects() {
        let mut event_zero = WorldState::genesis(809);
        event_zero.player.helmet_closed = true;
        event_zero.player.suit_oxygen_milli = 5;
        let payload = event_zero
            .oxygen_incapacitation_payload()
            .expect("canonical incapacitation derives");
        let event = event_zero.prepare_system_event(payload);
        let mut terminal = event_zero.clone();
        terminal
            .apply_event(&event)
            .expect("canonical incapacitation applies");

        let origins = replay_creation_origins(
            &event_zero.cell_id,
            &event_zero,
            std::slice::from_ref(&event),
            &terminal,
        )
        .expect("creation provenance replays");
        let created = origins.values().cloned().collect::<BTreeSet<_>>();
        assert_eq!(created.len(), 3);
        assert!(created.iter().any(|origin| {
            origin.entity_kind == "death" && origin.event_sequence == 1 && origin.ordinal == 0
        }));
        assert!(created.iter().any(|origin| {
            origin.entity_kind == "death-drop" && origin.event_sequence == 1 && origin.ordinal == 0
        }));
        assert!(created.iter().any(|origin| {
            origin.entity_kind == "inventory" && origin.event_sequence == 1 && origin.ordinal == 0
        }));
    }

    #[test]
    fn replay_diff_rejects_a_disappeared_identity_reappearing() {
        let previous = SubjectSets::default();
        let mut first = SubjectSets::default();
        first.blocks.insert("block-reused".into());
        let mut ever_seen = SubjectSets::default();
        let mut origins = BTreeMap::new();
        record_created_subjects(
            &"11".repeat(32),
            1,
            &previous,
            &first,
            &mut ever_seen,
            &mut origins,
        )
        .expect("first creation records");

        let disappeared = SubjectSets::default();
        let error = record_created_subjects(
            &"11".repeat(32),
            2,
            &disappeared,
            &first,
            &mut ever_seen,
            &mut origins,
        )
        .expect_err("reused identity rejects");
        assert!(
            error
                .to_string()
                .contains("disappeared and was later reused")
        );
    }
}
