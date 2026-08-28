// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant production provenance and import-eligibility models for package v2.
//!
//! These records are private compatibility-staging material. They do not alter
//! the active world, event, scheduler, or protocol-18 production paths.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::{
    DraftGridClosureError, DraftGridClosurePackageV2, ProductionJob, WorldState, hash_json,
    valid_blake3_hex, valid_stable_id,
};
use crate::event::{PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION, ProductionScheduleOccurrence};
use crate::identity::{SUBJECT_ID_SCHEMA_VERSION, canonical_subject_id};

use super::state::ValidatedDraftGridImportBoundaryV2;

const PRODUCTION_JOB_ENTITY_KIND: &str = "production-job";
const DRAFT_IMPORTED_PRODUCTION_ELIGIBILITY_SCHEMA_VERSION: u32 = 2;
const IMPORTED_PRODUCTION_ELIGIBILITY_HASH_DOMAIN: &[u8] =
    b"the-verse/imported-production-eligibility/v2\0";
const IMPORTED_PRODUCTION_ELIGIBILITY_MAP_ROOT_DOMAIN: &[u8] =
    b"the-verse/imported-production-eligibility-map/v2\0";
const IMPORTED_PRODUCTION_QUEUE_HASH_DOMAIN: &[u8] = b"the-verse/imported-production-queue/v2\0";
const IMPORTED_PRODUCTION_CONTROLS_ROOT_DOMAIN: &[u8] =
    b"the-verse/imported-production-controls/v2\0";
const PRODUCTION_IMPORT_REARM_MILLIS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftProductionJobOriginV2 {
    subject_id_schema_version: u32,
    universe_id: String,
    creator_cell_id: String,
    event_sequence: u64,
    entity_kind: String,
    ordinal: u32,
}

impl DraftProductionJobOriginV2 {
    #[cfg(test)]
    pub(super) fn new(
        universe_id: &str,
        creator_cell_id: &str,
        event_sequence: u64,
        ordinal: u32,
    ) -> Result<(String, Self), DraftGridClosureError> {
        let origin = Self {
            subject_id_schema_version: SUBJECT_ID_SCHEMA_VERSION,
            universe_id: universe_id.to_owned(),
            creator_cell_id: creator_cell_id.to_owned(),
            event_sequence,
            entity_kind: PRODUCTION_JOB_ENTITY_KIND.to_owned(),
            ordinal,
        };
        let job_id = origin.canonical_job_id()?;
        Ok((job_id, origin))
    }

    fn canonical_job_id(&self) -> Result<String, DraftGridClosureError> {
        canonical_subject_id(
            &self.universe_id,
            &self.creator_cell_id,
            self.event_sequence,
            &self.entity_kind,
            self.ordinal,
        )
        .map_err(|source| {
            DraftGridClosureError::Invalid(format!(
                "production job origin cannot derive a canonical identity: {source}"
            ))
        })
    }

    fn validate_for_job(
        &self,
        package_universe_id: &str,
        package_source_cell_id: &str,
        package_source_event_sequence: u64,
        job: &ProductionJob,
    ) -> Result<(), DraftGridClosureError> {
        if self.subject_id_schema_version != SUBJECT_ID_SCHEMA_VERSION
            || self.universe_id != package_universe_id
            || self.entity_kind != PRODUCTION_JOB_ENTITY_KIND
            || !valid_blake3_hex(&self.creator_cell_id)
            || self.event_sequence == 0
            || self.event_sequence != job.queued_event_sequence
            || self.canonical_job_id()? != job.job_id
            || (self.creator_cell_id == package_source_cell_id
                && self.event_sequence > package_source_event_sequence)
        {
            return Err(DraftGridClosureError::Invalid(
                "production job origin does not bind its canonical creation event".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn frontier_is_valid_in_cell(
        &self,
        cell_id: &str,
        local_event_sequence: u64,
        job: &ProductionJob,
    ) -> bool {
        self.event_sequence == job.queued_event_sequence
            && (self.creator_cell_id != cell_id || self.event_sequence <= local_event_sequence)
    }
}

pub(super) fn validate_production_job_origins(
    package_universe_id: &str,
    package_source_cell_id: &str,
    package_source_event_sequence: u64,
    production_queues: &BTreeMap<String, VecDeque<ProductionJob>>,
    origins: &BTreeMap<String, DraftProductionJobOriginV2>,
) -> Result<(), DraftGridClosureError> {
    let jobs = production_queues
        .values()
        .flatten()
        .map(|job| (job.job_id.as_str(), job))
        .collect::<BTreeMap<_, _>>();
    if jobs.len() != production_queues.values().map(VecDeque::len).sum::<usize>()
        || origins.len() != jobs.len()
        || origins.keys().map(String::as_str).ne(jobs.keys().copied())
    {
        return Err(DraftGridClosureError::Invalid(
            "production job origins must exactly cover the unique transferred job set".into(),
        ));
    }
    for (job_id, origin) in origins {
        let job = jobs.get(job_id.as_str()).ok_or_else(|| {
            DraftGridClosureError::Invalid(
                "production job origin references a job outside the transferred closure".into(),
            )
        })?;
        origin.validate_for_job(
            package_universe_id,
            package_source_cell_id,
            package_source_event_sequence,
            job,
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftImportedProductionEligibilityV2 {
    schema_version: u32,
    transfer_id: String,
    package_hash: String,
    universe_id: String,
    universe_manifest_hash: String,
    celestial_registry_hash: String,
    machine_block_id: String,
    ordered_job_ids: Vec<String>,
    queue_hash: String,
    destination_cell_id: String,
    destination_assignment_generation: u64,
    destination_fencing_token: u64,
    import_event_sequence: u64,
    import_event_hash: String,
    trusted_import_unix_ms: u64,
    eligible_at_unix_ms: u64,
    destination_production_lifecycle_generation: u64,
    eligibility_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DraftProductionImportAuthorityV2 {
    destination_assignment_generation: u64,
    destination_fencing_token: u64,
    import_event_sequence: u64,
    import_event_hash: String,
    trusted_import_unix_ms: u64,
    destination_production_lifecycle_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DraftImportedProductionDecisionV2 {
    TransferPaused,
    ReleaseAndEvaluate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DraftProductionMachineControlKindV2 {
    Evaluate,
    TransferPaused,
    ReleaseAndEvaluate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftProductionMachineControlV2 {
    grid_id: String,
    machine_block_id: String,
    kind: DraftProductionMachineControlKindV2,
    eligibility_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftImportedProductionOccurrenceControlsV2 {
    occurrence: ProductionScheduleOccurrence,
    machines: Vec<DraftProductionMachineControlV2>,
    controls_root: String,
}

impl DraftImportedProductionEligibilityV2 {
    fn derive_for_queue(
        package: &DraftGridClosurePackageV2,
        machine_block_id: &str,
        authority: &DraftProductionImportAuthorityV2,
    ) -> Result<Self, DraftGridClosureError> {
        package.validate_wire()?;
        authority.validate_for_package(package)?;
        let queue = package
            .production_queues
            .get(machine_block_id)
            .ok_or_else(|| {
                DraftGridClosureError::Invalid(
                    "import eligibility machine is not an exact packaged queue".into(),
                )
            })?;
        if queue.is_empty() {
            return Err(DraftGridClosureError::Invalid(
                "import eligibility cannot bind an empty machine queue".into(),
            ));
        }
        let ordered_job_ids = queue.iter().map(|job| job.job_id.clone()).collect();
        let queue_hash = hash_json(IMPORTED_PRODUCTION_QUEUE_HASH_DOMAIN, queue)?;
        let eligible_at_unix_ms = authority
            .trusted_import_unix_ms
            .checked_add(PRODUCTION_IMPORT_REARM_MILLIS)
            .ok_or_else(|| {
                DraftGridClosureError::Unsupported(
                    "production import re-arm time overflowed".into(),
                )
            })?;
        let mut record = Self {
            schema_version: DRAFT_IMPORTED_PRODUCTION_ELIGIBILITY_SCHEMA_VERSION,
            transfer_id: package.transfer_id.clone(),
            package_hash: package.package_hash.clone(),
            universe_id: package.universe_id.clone(),
            universe_manifest_hash: package.universe_manifest_hash.clone(),
            celestial_registry_hash: package.celestial_registry_hash.clone(),
            machine_block_id: machine_block_id.to_owned(),
            ordered_job_ids,
            queue_hash,
            destination_cell_id: package.destination_cell_id.clone(),
            destination_assignment_generation: authority.destination_assignment_generation,
            destination_fencing_token: authority.destination_fencing_token,
            import_event_sequence: authority.import_event_sequence,
            import_event_hash: authority.import_event_hash.clone(),
            trusted_import_unix_ms: authority.trusted_import_unix_ms,
            eligible_at_unix_ms,
            destination_production_lifecycle_generation: authority
                .destination_production_lifecycle_generation,
            eligibility_hash: String::new(),
        };
        record.eligibility_hash = record.calculate_hash()?;
        record.validate_for_import(package, authority)?;
        Ok(record)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.eligibility_hash.clear();
        hash_json(IMPORTED_PRODUCTION_ELIGIBILITY_HASH_DOMAIN, &material)
    }

    pub(super) fn validate(&self) -> Result<(), DraftGridClosureError> {
        let unique_job_ids = self.ordered_job_ids.iter().collect::<BTreeSet<_>>();
        if self.schema_version != DRAFT_IMPORTED_PRODUCTION_ELIGIBILITY_SCHEMA_VERSION
            || !valid_stable_id(&self.transfer_id)
            || !valid_blake3_hex(&self.package_hash)
            || self.universe_id.trim().is_empty()
            || !valid_blake3_hex(&self.universe_manifest_hash)
            || !valid_blake3_hex(&self.celestial_registry_hash)
            || !valid_stable_id(&self.machine_block_id)
            || self.ordered_job_ids.is_empty()
            || unique_job_ids.len() != self.ordered_job_ids.len()
            || self
                .ordered_job_ids
                .iter()
                .any(|job_id| !valid_stable_id(job_id))
            || !valid_blake3_hex(&self.queue_hash)
            || !valid_blake3_hex(&self.destination_cell_id)
            || self.destination_assignment_generation == 0
            || self.destination_fencing_token == 0
            || self.import_event_sequence == 0
            || !valid_blake3_hex(&self.import_event_hash)
            || self.trusted_import_unix_ms == 0
            || self
                .trusted_import_unix_ms
                .checked_add(PRODUCTION_IMPORT_REARM_MILLIS)
                != Some(self.eligible_at_unix_ms)
            || self.destination_production_lifecycle_generation == 0
            || !valid_blake3_hex(&self.eligibility_hash)
            || self.eligibility_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "imported production eligibility identity, queue, frontier, or hash is invalid"
                    .into(),
            ));
        }
        Ok(())
    }

    pub(super) fn validate_persisted_in_world(
        &self,
        world: &WorldState,
        current_queue: &VecDeque<ProductionJob>,
    ) -> Result<(), DraftGridClosureError> {
        self.validate()?;
        if self.universe_id != world.universe_id
            || self.universe_manifest_hash != world.universe_manifest_hash
            || self.celestial_registry_hash != world.celestial_registry_hash
            || self.destination_cell_id != world.cell_id
            || world.fencing_token < self.destination_fencing_token
            || self.destination_production_lifecycle_generation
                != world.production_clock.lifecycle_generation
            || self
                .ordered_job_ids
                .iter()
                .map(String::as_str)
                .ne(current_queue.iter().map(|job| job.job_id.as_str()))
            || self.queue_hash != hash_json(IMPORTED_PRODUCTION_QUEUE_HASH_DOMAIN, current_queue)?
        {
            return Err(DraftGridClosureError::Invalid(
                "persisted import eligibility no longer binds its destination world and queue"
                    .into(),
            ));
        }
        Ok(())
    }

    pub(super) fn transfer_id(&self) -> &str {
        &self.transfer_id
    }

    pub(super) fn package_hash(&self) -> &str {
        &self.package_hash
    }

    pub(super) fn machine_block_id(&self) -> &str {
        &self.machine_block_id
    }

    pub(super) fn eligibility_hash(&self) -> &str {
        &self.eligibility_hash
    }

    pub(super) fn contains_job_id(&self, job_id: &str) -> bool {
        self.ordered_job_ids
            .iter()
            .any(|candidate| candidate == job_id)
    }

    pub(super) fn eligible_at_unix_ms(&self) -> u64 {
        self.eligible_at_unix_ms
    }

    pub(super) fn validate_release_occurrence(
        &self,
        occurrence: &ProductionScheduleOccurrence,
    ) -> Result<(), DraftGridClosureError> {
        self.validate()?;
        if occurrence.schema_version != PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION
            || occurrence.universe_id != self.universe_id
            || occurrence.cell_id != self.destination_cell_id
            || occurrence.lifecycle_generation != self.destination_production_lifecycle_generation
            || occurrence.production_quantum_sequence == 0
            || occurrence.scheduled_for_unix_ms < self.eligible_at_unix_ms
            || occurrence.universe_manifest_hash != self.universe_manifest_hash
            || occurrence.celestial_registry_hash != self.celestial_registry_hash
        {
            return Err(DraftGridClosureError::Invalid(
                "released eligibility does not bind its exact production occurrence".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn validate_persisted_import_boundary(
        &self,
        transfer_id: &str,
        package_hash: &str,
        boundary: &ValidatedDraftGridImportBoundaryV2,
    ) -> Result<(), DraftGridClosureError> {
        self.validate()?;
        if self.transfer_id != transfer_id
            || self.package_hash != package_hash
            || self.destination_assignment_generation
                != boundary.destination_assignment_generation()
            || self.destination_fencing_token != boundary.destination_fencing_token()
            || self.import_event_sequence != boundary.import_event_sequence()
            || self.import_event_hash != boundary.import_event_hash()
            || self.trusted_import_unix_ms != boundary.trusted_import_unix_ms()
            || self.destination_production_lifecycle_generation
                != boundary.destination_production_lifecycle_generation()
        {
            return Err(DraftGridClosureError::Invalid(
                "persisted import eligibility changed its sealed import boundary".into(),
            ));
        }
        Ok(())
    }

    fn validate_for_import(
        &self,
        package: &DraftGridClosurePackageV2,
        authority: &DraftProductionImportAuthorityV2,
    ) -> Result<(), DraftGridClosureError> {
        self.validate()?;
        authority.validate_for_package(package)?;
        let queue = package
            .production_queues
            .get(&self.machine_block_id)
            .ok_or_else(|| {
                DraftGridClosureError::Invalid(
                    "import eligibility references a machine outside the exact package".into(),
                )
            })?;
        let expected_job_ids = queue
            .iter()
            .map(|job| job.job_id.as_str())
            .collect::<Vec<_>>();
        if self.transfer_id != package.transfer_id
            || self.package_hash != package.package_hash
            || self.universe_id != package.universe_id
            || self.universe_manifest_hash != package.universe_manifest_hash
            || self.celestial_registry_hash != package.celestial_registry_hash
            || self.destination_cell_id != package.destination_cell_id
            || self.destination_assignment_generation != authority.destination_assignment_generation
            || self.destination_fencing_token != authority.destination_fencing_token
            || self.import_event_sequence != authority.import_event_sequence
            || self.import_event_hash != authority.import_event_hash
            || self.trusted_import_unix_ms != authority.trusted_import_unix_ms
            || self.destination_production_lifecycle_generation
                != authority.destination_production_lifecycle_generation
            || self
                .ordered_job_ids
                .iter()
                .map(String::as_str)
                .ne(expected_job_ids)
            || self.queue_hash != hash_json(IMPORTED_PRODUCTION_QUEUE_HASH_DOMAIN, queue)?
        {
            return Err(DraftGridClosureError::Invalid(
                "imported production eligibility does not bind the exact package queue and authority"
                    .into(),
            ));
        }
        Ok(())
    }

    pub(super) fn decision_for_occurrence(
        &self,
        current_ordered_job_ids: &[String],
        occurrence: &ProductionScheduleOccurrence,
    ) -> Result<DraftImportedProductionDecisionV2, DraftGridClosureError> {
        self.validate()?;
        if self
            .ordered_job_ids
            .iter()
            .map(String::as_str)
            .ne(current_ordered_job_ids.iter().map(String::as_str))
            || occurrence.schema_version != PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION
            || occurrence.universe_id != self.universe_id
            || occurrence.cell_id != self.destination_cell_id
            || occurrence.lifecycle_generation != self.destination_production_lifecycle_generation
            || occurrence.production_quantum_sequence == 0
            || occurrence.universe_manifest_hash != self.universe_manifest_hash
            || occurrence.celestial_registry_hash != self.celestial_registry_hash
        {
            return Err(DraftGridClosureError::Invalid(
                "production occurrence does not match the imported queue boundary".into(),
            ));
        }
        Ok(
            if occurrence.scheduled_for_unix_ms < self.eligible_at_unix_ms {
                DraftImportedProductionDecisionV2::TransferPaused
            } else {
                DraftImportedProductionDecisionV2::ReleaseAndEvaluate
            },
        )
    }

    #[cfg(test)]
    fn test_fixture() -> Self {
        let mut record = Self {
            schema_version: DRAFT_IMPORTED_PRODUCTION_ELIGIBILITY_SCHEMA_VERSION,
            transfer_id: "transfer-production-1".into(),
            package_hash: "11".repeat(32),
            universe_id: "the-verse-proof-universe".into(),
            universe_manifest_hash: "22".repeat(32),
            celestial_registry_hash: "33".repeat(32),
            machine_block_id: "block-refinery".into(),
            ordered_job_ids: vec!["production-job-a".into(), "production-job-b".into()],
            queue_hash: "66".repeat(32),
            destination_cell_id: "44".repeat(32),
            destination_assignment_generation: 7,
            destination_fencing_token: 9,
            import_event_sequence: 31,
            import_event_hash: "55".repeat(32),
            trusted_import_unix_ms: 1_800_000_000_000,
            eligible_at_unix_ms: 1_800_000_001_000,
            destination_production_lifecycle_generation: 4,
            eligibility_hash: String::new(),
        };
        record.eligibility_hash = record.calculate_hash().expect("test record hashes");
        record
    }

    #[cfg(test)]
    pub(super) fn resealed_with_ordered_job_ids_for_test(
        &self,
        ordered_job_ids: Vec<String>,
    ) -> Self {
        let mut record = self.clone();
        record.ordered_job_ids = ordered_job_ids;
        record.eligibility_hash.clear();
        record.eligibility_hash = record.calculate_hash().expect("test record reseals");
        record
    }

    #[cfg(test)]
    pub(super) fn resealed_with_trusted_import_unix_ms_for_test(
        &self,
        trusted_import_unix_ms: u64,
    ) -> Self {
        let mut record = self.clone();
        record.trusted_import_unix_ms = trusted_import_unix_ms;
        record.eligible_at_unix_ms = trusted_import_unix_ms
            .checked_add(PRODUCTION_IMPORT_REARM_MILLIS)
            .expect("test trusted import time remains in range");
        record.eligibility_hash.clear();
        record.eligibility_hash = record.calculate_hash().expect("test record reseals");
        record
    }
}

impl DraftImportedProductionOccurrenceControlsV2 {
    fn calculate_root(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.controls_root.clear();
        hash_json(IMPORTED_PRODUCTION_CONTROLS_ROOT_DOMAIN, &material)
    }

    pub(super) fn validate_for_world(
        &self,
        world: &WorldState,
        eligibilities: &BTreeMap<String, DraftImportedProductionEligibilityV2>,
    ) -> Result<(), DraftGridClosureError> {
        self.validate_canonical()?;
        validate_next_occurrence_for_world(world, &self.occurrence)?;
        if self.machines.len() != world.production_queues.len() {
            return Err(DraftGridClosureError::Invalid(
                "imported production controls do not bind one exact cell occurrence".into(),
            ));
        }
        let expected = derive_imported_production_occurrence_controls(
            world,
            eligibilities,
            self.occurrence.clone(),
        )?;
        if self != &expected {
            return Err(DraftGridClosureError::Invalid(
                "imported production controls differ from the canonical machine decisions".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn validate_canonical(&self) -> Result<(), DraftGridClosureError> {
        if self.machines.is_empty()
            || self.machines.iter().any(|control| {
                !valid_stable_id(&control.grid_id)
                    || !valid_stable_id(&control.machine_block_id)
                    || match control.kind {
                        DraftProductionMachineControlKindV2::Evaluate => {
                            control.eligibility_hash.is_some()
                        }
                        DraftProductionMachineControlKindV2::TransferPaused
                        | DraftProductionMachineControlKindV2::ReleaseAndEvaluate => control
                            .eligibility_hash
                            .as_ref()
                            .is_none_or(|hash| !valid_blake3_hex(hash)),
                    }
            })
            || self.machines.windows(2).any(|pair| {
                (&pair[0].grid_id, &pair[0].machine_block_id)
                    >= (&pair[1].grid_id, &pair[1].machine_block_id)
            })
            || !valid_blake3_hex(&self.controls_root)
            || self.controls_root != self.calculate_root()?
        {
            return Err(DraftGridClosureError::Invalid(
                "imported production controls are not canonical ordered decisions".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn occurrence(&self) -> &ProductionScheduleOccurrence {
        &self.occurrence
    }

    pub(super) fn machines(&self) -> &[DraftProductionMachineControlV2] {
        &self.machines
    }

    pub(super) fn controls_root(&self) -> &str {
        &self.controls_root
    }
}

impl DraftProductionMachineControlV2 {
    pub(super) fn grid_id(&self) -> &str {
        &self.grid_id
    }

    pub(super) fn machine_block_id(&self) -> &str {
        &self.machine_block_id
    }

    pub(super) fn kind(&self) -> DraftProductionMachineControlKindV2 {
        self.kind
    }

    pub(super) fn eligibility_hash(&self) -> Option<&str> {
        self.eligibility_hash.as_deref()
    }
}

fn validate_next_occurrence_for_world(
    world: &WorldState,
    occurrence: &ProductionScheduleOccurrence,
) -> Result<(), DraftGridClosureError> {
    let expected_sequence = world
        .production_clock
        .last_committed_quantum_sequence
        .checked_add(1)
        .ok_or_else(|| {
            DraftGridClosureError::Unsupported("production occurrence sequence is exhausted".into())
        })?;
    let time_is_valid = if world.production_clock.last_committed_quantum_sequence == 0 {
        occurrence.scheduled_for_unix_ms > 0
    } else {
        world
            .production_clock
            .last_scheduled_for_unix_ms
            .checked_add(PRODUCTION_IMPORT_REARM_MILLIS)
            .is_some_and(|earliest| occurrence.scheduled_for_unix_ms >= earliest)
    };
    if occurrence.schema_version != PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION
        || occurrence.universe_id != world.universe_id
        || occurrence.cell_id != world.cell_id
        || occurrence.lifecycle_generation != world.production_clock.lifecycle_generation
        || occurrence.production_quantum_sequence != expected_sequence
        || occurrence.universe_manifest_hash != world.universe_manifest_hash
        || occurrence.celestial_registry_hash != world.celestial_registry_hash
        || !time_is_valid
    {
        return Err(DraftGridClosureError::Invalid(
            "production occurrence is not the exact next destination cell quantum".into(),
        ));
    }
    Ok(())
}

pub(super) fn derive_imported_production_occurrence_controls(
    world: &WorldState,
    eligibilities: &BTreeMap<String, DraftImportedProductionEligibilityV2>,
    occurrence: ProductionScheduleOccurrence,
) -> Result<DraftImportedProductionOccurrenceControlsV2, DraftGridClosureError> {
    validate_next_occurrence_for_world(world, &occurrence)?;
    for (machine_id, eligibility) in eligibilities {
        let queue = world.production_queues.get(machine_id).ok_or_else(|| {
            DraftGridClosureError::Invalid(
                "import eligibility has no queue in the destination occurrence".into(),
            )
        })?;
        if machine_id != eligibility.machine_block_id() {
            return Err(DraftGridClosureError::Invalid(
                "import eligibility key changed before scheduler evaluation".into(),
            ));
        }
        eligibility.validate_persisted_in_world(world, queue)?;
    }

    let mut scheduled = world
        .production_queues
        .keys()
        .map(|machine_block_id| {
            world
                .block_grid(machine_block_id)
                .map(|(grid, _)| (grid.grid_id.clone(), machine_block_id.clone()))
                .ok_or_else(|| {
                    DraftGridClosureError::Invalid(
                        "imported production control references a missing machine".into(),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    scheduled.sort();
    let mut machines = Vec::with_capacity(scheduled.len());
    for (grid_id, machine_block_id) in scheduled {
        let queue = &world.production_queues[&machine_block_id];
        let (kind, eligibility_hash) = if let Some(eligibility) =
            eligibilities.get(&machine_block_id)
        {
            let ordered_job_ids = queue
                .iter()
                .map(|job| job.job_id.clone())
                .collect::<Vec<_>>();
            let decision = eligibility.decision_for_occurrence(&ordered_job_ids, &occurrence)?;
            let kind = match decision {
                DraftImportedProductionDecisionV2::TransferPaused => {
                    DraftProductionMachineControlKindV2::TransferPaused
                }
                DraftImportedProductionDecisionV2::ReleaseAndEvaluate => {
                    DraftProductionMachineControlKindV2::ReleaseAndEvaluate
                }
            };
            (kind, Some(eligibility.eligibility_hash().to_owned()))
        } else {
            (DraftProductionMachineControlKindV2::Evaluate, None)
        };
        machines.push(DraftProductionMachineControlV2 {
            grid_id,
            machine_block_id,
            kind,
            eligibility_hash,
        });
    }
    let mut controls = DraftImportedProductionOccurrenceControlsV2 {
        occurrence,
        machines,
        controls_root: String::new(),
    };
    controls.controls_root = controls.calculate_root()?;
    Ok(controls)
}

impl DraftProductionImportAuthorityV2 {
    /// Builds the private production capability after the destination import
    /// transaction has derived every field from validated directory, cell
    /// event, live-fence, lifecycle, and trusted-clock evidence.
    pub(super) fn from_committed_import(
        package: &DraftGridClosurePackageV2,
        boundary: &ValidatedDraftGridImportBoundaryV2,
    ) -> Result<Self, DraftGridClosureError> {
        let authority = Self {
            destination_assignment_generation: boundary.destination_assignment_generation(),
            destination_fencing_token: boundary.destination_fencing_token(),
            import_event_sequence: boundary.import_event_sequence(),
            import_event_hash: boundary.import_event_hash().to_owned(),
            trusted_import_unix_ms: boundary.trusted_import_unix_ms(),
            destination_production_lifecycle_generation: boundary
                .destination_production_lifecycle_generation(),
        };
        authority.validate_for_package(package)?;
        Ok(authority)
    }

    #[cfg(test)]
    pub(super) fn new(
        package: &DraftGridClosurePackageV2,
        destination_assignment_generation: u64,
        destination_fencing_token: u64,
        import_event_sequence: u64,
        import_event_hash: String,
        trusted_import_unix_ms: u64,
        destination_production_lifecycle_generation: u64,
    ) -> Result<Self, DraftGridClosureError> {
        let authority = Self {
            destination_assignment_generation,
            destination_fencing_token,
            import_event_sequence,
            import_event_hash,
            trusted_import_unix_ms,
            destination_production_lifecycle_generation,
        };
        authority.validate_for_package(package)?;
        Ok(authority)
    }

    fn validate_for_package(
        &self,
        package: &DraftGridClosurePackageV2,
    ) -> Result<(), DraftGridClosureError> {
        if self.destination_assignment_generation < package.destination_assignment_generation
            || self.destination_fencing_token < package.destination_fencing_token
            || self.import_event_sequence == 0
            || !valid_blake3_hex(&self.import_event_hash)
            || self.trusted_import_unix_ms == 0
            || self.destination_production_lifecycle_generation == 0
            || self
                .trusted_import_unix_ms
                .checked_add(PRODUCTION_IMPORT_REARM_MILLIS)
                .is_none()
        {
            return Err(DraftGridClosureError::Invalid(
                "production import authority, event, trusted time, or lifecycle is invalid".into(),
            ));
        }
        Ok(())
    }
}

pub(super) fn derive_imported_production_eligibilities(
    package: &DraftGridClosurePackageV2,
    authority: &DraftProductionImportAuthorityV2,
) -> Result<BTreeMap<String, DraftImportedProductionEligibilityV2>, DraftGridClosureError> {
    package.validate_wire()?;
    authority.validate_for_package(package)?;
    package
        .production_queues
        .keys()
        .map(|machine_block_id| {
            DraftImportedProductionEligibilityV2::derive_for_queue(
                package,
                machine_block_id,
                authority,
            )
            .map(|record| (machine_block_id.clone(), record))
        })
        .collect()
}

pub(super) fn validate_imported_production_eligibilities(
    package: &DraftGridClosurePackageV2,
    authority: &DraftProductionImportAuthorityV2,
    records: &BTreeMap<String, DraftImportedProductionEligibilityV2>,
) -> Result<(), DraftGridClosureError> {
    let expected = derive_imported_production_eligibilities(package, authority)?;
    if records != &expected {
        return Err(DraftGridClosureError::Invalid(
            "imported production eligibility map is not the exact packaged machine set".into(),
        ));
    }
    Ok(())
}

pub(super) fn imported_production_eligibility_map_root(
    records: &BTreeMap<String, DraftImportedProductionEligibilityV2>,
) -> Result<String, DraftGridClosureError> {
    for (machine_id, record) in records {
        record.validate()?;
        if machine_id != record.machine_block_id() {
            return Err(DraftGridClosureError::Invalid(
                "import eligibility map key does not match its machine identity".into(),
            ));
        }
    }
    hash_json(IMPORTED_PRODUCTION_ELIGIBILITY_MAP_ROOT_DOMAIN, records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use verse_protocol::{InventoryContents, ProductionRecipeKind};

    fn eligibility() -> DraftImportedProductionEligibilityV2 {
        DraftImportedProductionEligibilityV2::test_fixture()
    }

    fn occurrence(scheduled_for_unix_ms: u64) -> ProductionScheduleOccurrence {
        let boundary = eligibility();
        ProductionScheduleOccurrence {
            schema_version: PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
            universe_id: boundary.universe_id,
            cell_id: boundary.destination_cell_id,
            lifecycle_generation: boundary.destination_production_lifecycle_generation,
            production_quantum_sequence: 12,
            scheduled_for_unix_ms,
            universe_manifest_hash: boundary.universe_manifest_hash,
            celestial_registry_hash: boundary.celestial_registry_hash,
        }
    }

    #[test]
    fn whole_cell_controls_use_canonical_grid_then_machine_order() {
        let mut world = WorldState::genesis(801);
        let industry = world
            .grids
            .remove("grid-industry-starter")
            .expect("starter industry grid exists");
        let mut first_grid = industry.clone();
        first_grid.grid_id = "grid-a".into();
        first_grid
            .blocks
            .retain(|block_id, _| block_id == "block-refinery");
        let mut second_grid = industry;
        second_grid.grid_id = "grid-z".into();
        second_grid
            .blocks
            .retain(|block_id, _| block_id == "block-assembler");
        world.grids.insert(first_grid.grid_id.clone(), first_grid);
        world.grids.insert(second_grid.grid_id.clone(), second_grid);
        let job = |job_id: &str, machine_block_id: &str| ProductionJob {
            job_id: job_id.into(),
            operation_id: format!("operation-{job_id}"),
            owner_player_id: world.player.primary_player_id.clone(),
            machine_block_id: machine_block_id.into(),
            recipe: ProductionRecipeKind::Refining,
            content_manifest_version: world.content_manifest_version.clone(),
            batches: 1,
            source_inventory_id: world.player.inventory_id.clone(),
            destination_inventory_id: world.player.inventory_id.clone(),
            progress_ticks: 0,
            duration_ticks: 60,
            reserved_inputs: InventoryContents::default(),
            pending_outputs: InventoryContents::default(),
            queued_event_sequence: 1,
        };
        world.production_queues.insert(
            "block-assembler".into(),
            VecDeque::from([job("job-assembler", "block-assembler")]),
        );
        world.production_queues.insert(
            "block-refinery".into(),
            VecDeque::from([job("job-refinery", "block-refinery")]),
        );
        let occurrence = world
            .next_production_occurrence_at(1_800_000_000_000)
            .expect("occurrence derives");
        let controls =
            derive_imported_production_occurrence_controls(&world, &BTreeMap::new(), occurrence)
                .expect("controls derive");
        assert_eq!(
            controls
                .machines()
                .iter()
                .map(|control| (control.grid_id(), control.machine_block_id()))
                .collect::<Vec<_>>(),
            vec![("grid-a", "block-refinery"), ("grid-z", "block-assembler")]
        );
    }

    #[test]
    fn equal_local_event_sequences_in_distinct_cells_produce_distinct_job_ids() {
        let (origin_job, origin) =
            DraftProductionJobOriginV2::new("the-verse-proof-universe", &"11".repeat(32), 41, 0)
                .expect("origin job derives");
        let (east_job, east) =
            DraftProductionJobOriginV2::new("the-verse-proof-universe", &"22".repeat(32), 41, 0)
                .expect("east job derives");
        assert_ne!(origin_job, east_job);
        assert_ne!(origin.creator_cell_id, east.creator_cell_id);
    }

    #[test]
    fn import_boundary_pauses_before_one_second_and_releases_at_exact_boundary() {
        let boundary = eligibility();
        let queue = boundary.ordered_job_ids.clone();
        assert_eq!(boundary.eligible_at_unix_ms, 1_800_000_001_000);
        assert_eq!(
            boundary
                .decision_for_occurrence(&queue, &occurrence(1_800_000_000_999))
                .expect("pre-boundary decision derives"),
            DraftImportedProductionDecisionV2::TransferPaused
        );
        assert_eq!(
            boundary
                .decision_for_occurrence(&queue, &occurrence(1_800_000_001_000))
                .expect("boundary decision derives"),
            DraftImportedProductionDecisionV2::ReleaseAndEvaluate
        );
    }

    #[test]
    fn changed_queue_lifecycle_or_overflow_fails_closed() {
        let mut changed_queue = eligibility();
        changed_queue.ordered_job_ids.reverse();
        assert!(changed_queue.validate().is_err());

        let mut wrong_generation = occurrence(1_800_000_001_000);
        wrong_generation.lifecycle_generation += 1;
        let queue = eligibility().ordered_job_ids;
        assert!(
            eligibility()
                .decision_for_occurrence(&queue, &wrong_generation)
                .is_err()
        );

        let mut overflow = eligibility();
        overflow.trusted_import_unix_ms = u64::MAX;
        overflow.eligibility_hash = overflow.calculate_hash().expect("overflow record reseals");
        assert!(overflow.validate().is_err());
    }
}
