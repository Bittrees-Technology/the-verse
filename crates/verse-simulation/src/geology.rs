// SPDX-License-Identifier: AGPL-3.0-or-later

//! Integer-only, seeded ore placement. Generation is a genesis operation;
//! callers must persist its result and must never regenerate a mined field.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use verse_protocol::IVec3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OreKind {
    Ferrite,
    Cuprite,
    Cobaltite,
}

const NEIGHBORS: [IVec3; 6] = [
    IVec3::new(1, 0, 0),
    IVec3::new(-1, 0, 0),
    IVec3::new(0, 1, 0),
    IVec3::new(0, -1, 0),
    IVec3::new(0, 0, 1),
    IVec3::new(0, 0, -1),
];

fn exposed(occupied: &BTreeSet<IVec3>, point: IVec3) -> bool {
    NEIGHBORS.iter().any(|offset| {
        let (Some(x), Some(y), Some(z)) = (
            point.x.checked_add(offset.x),
            point.y.checked_add(offset.y),
            point.z.checked_add(offset.z),
        ) else {
            return true;
        };
        !occupied.contains(&IVec3::new(x, y, z))
    })
}

/// About 22% ore by volume, split 60/25/15 between common and rarer deposits.
/// Rank coherent noise independently per mineral, with stable coordinate ties.
/// Reserve a small surface sample so a beginner can discover every mineral.
#[must_use]
pub fn generate_deposits(seed: u64, occupied: &BTreeSet<IVec3>) -> BTreeMap<IVec3, OreKind> {
    let mut result = BTreeMap::new();
    for (kind, parts_per_ten_thousand, salt) in [
        (OreKind::Cobaltite, 330, 0xA409_3822_299F_31D0),
        (OreKind::Cuprite, 550, 0x082E_FA98_EC4E_6C89),
        (OreKind::Ferrite, 1320, 0x4528_21E6_38D0_1377),
    ] {
        let target = occupied.len().saturating_mul(parts_per_ten_thousand) / 10_000;
        if target == 0 {
            continue;
        }
        let mut candidates: Vec<_> = occupied
            .iter()
            .copied()
            .filter(|point| !result.contains_key(point))
            .map(|point| (crate::model::fixed_value_noise(seed ^ salt, point), point))
            .collect();
        candidates.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let starter_count = target.min(3);
        for (_, point) in candidates
            .iter()
            .filter(|(_, point)| exposed(occupied, *point))
            .take(starter_count)
        {
            result.insert(*point, kind);
        }
        let mut remaining =
            target.saturating_sub(result.values().filter(|value| **value == kind).count());
        for (_, point) in candidates {
            if remaining == 0 {
                break;
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = result.entry(point) {
                entry.insert(kind);
                remaining -= 1;
            }
        }
    }
    result
}

/// Presentation catalog for the starter asteroid; mining authority remains the snapshot.
#[must_use]
pub fn starter_deposit_catalog(seed: u64) -> Vec<(IVec3, OreKind)> {
    generate_deposits(
        seed,
        &crate::model::VoxelField::procedural_asteroid(seed, 8).occupied,
    )
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::VoxelField;

    #[test]
    fn deposits_are_seeded_balanced_discoverable_and_clustered() {
        for seed in 0..64 {
            let occupied = VoxelField::procedural_asteroid(seed, 8).occupied;
            let deposits = generate_deposits(seed, &occupied);
            assert_eq!(deposits, generate_deposits(seed, &occupied));
            assert!(deposits.keys().all(|point| occupied.contains(point)));
            assert!(deposits.len() * 100 >= occupied.len() * 21);
            assert!(deposits.len() * 100 <= occupied.len() * 22);
            for kind in [OreKind::Ferrite, OreKind::Cuprite, OreKind::Cobaltite] {
                let sites: Vec<_> = deposits
                    .iter()
                    .filter(|(_, value)| **value == kind)
                    .map(|(point, _)| *point)
                    .collect();
                assert!(
                    sites
                        .iter()
                        .filter(|point| exposed(&occupied, **point))
                        .count()
                        >= 3
                );
                let connected = sites
                    .iter()
                    .filter(|point| {
                        NEIGHBORS.iter().any(|offset| {
                            deposits.get(&IVec3::new(
                                point.x + offset.x,
                                point.y + offset.y,
                                point.z + offset.z,
                            )) == Some(&kind)
                        })
                    })
                    .count();
                assert!(
                    connected * 100 >= sites.len() * 90,
                    "seed={seed} kind={kind:?} connected={connected}/{}",
                    sites.len()
                );
            }
        }
    }

    #[test]
    fn different_seeds_change_ore_locations_on_the_same_shape() {
        let occupied = VoxelField::procedural_asteroid(42, 8).occupied;
        assert_ne!(
            generate_deposits(1, &occupied),
            generate_deposits(2, &occupied)
        );
    }

    #[test]
    fn small_and_empty_fields_have_no_invented_ore() {
        assert!(generate_deposits(1, &BTreeSet::new()).is_empty());
        let occupied = BTreeSet::from([IVec3::ZERO]);
        assert!(generate_deposits(1, &occupied).is_empty());
    }
}
