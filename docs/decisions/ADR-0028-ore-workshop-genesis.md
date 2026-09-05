# ADR-0028: Seeded development ore workshop

- Status: Accepted
- Date: 2026-09-05
- Requirements: F-070, UX-005

## Decision

Add an opt-in event-zero `ore-workshop` profile. Persist richer ore coordinates
using the current voxel grade representation. Do not change the active
protocol, event schema, content manifest, or dormant protocol-19 migration.
Derive three native geological assay labels from a deterministic seeded catalog.
The catalog is presentation metadata; verified voxel grades remain authoritative
for mining, yield, depletion, inventory, and production.

Reuse the internal AGPL simulation generator from the AGPL native adapter to
avoid divergent client/server placement algorithms. This adds no third-party
dependency. The versioned local bridge method is `ore_catalog_v1`.

## Consequences

Generation targets approximately 22% rich ore with three exposed samples of
each variety. Quotas keep seed variation from making the starter field barren.
An existing world is never regenerated after a canonical event. The engineering
launcher selects a separate save directory. Ordinary genesis stays compatible.

Cuprite and cobaltite are visibly distinct deposits but still produce the same
shared ore inventory resource as ferrite. Separate metal inventory items,
recipes, and economic balances remain a future versioned migration. Older
clients can mine the same saved deposits using the existing ferrite grade.

## Verification

Test 64 seeds for density, surface discovery, clustering, and determinism. Mine
each variety through authoritative validation and reopen the snapshot/journal
to verify yield, conservation, and depletion. Exercise the native catalog on a
live verified workshop baseline and run the existing manufacturing loop.
