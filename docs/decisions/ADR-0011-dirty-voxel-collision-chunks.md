# ADR-0011: Replace dirty voxel collision chunks atomically

**Status:** Accepted

## Context

Mineable terrain is canonical voxel state, while Jolt bodies are derived state. Rebuilding every static voxel collider and every dynamic grid after one accepted mining edit discards unrelated native-body activation and contact caches, performs work proportional to the whole cell, and cannot satisfy the P0.7 requirement that an edit rebuild only influenced collision chunks.

The chunk boundary is also observable in canonical contact identity. A replacement must therefore preserve stable identifiers, remain safe when native shape creation fails, and recover from the durable event rather than persisting Jolt internals.

## Decision

Partition occupied voxel cells into fixed cubic collision chunks. Content schema 5 adds the required integer `voxel_collision_chunk_edge_cells` field, and manifest `p0.7.3` pins it to eight, producing 8×8×8-cell chunks. Startup rejects a missing, zero, or unsupported value. Chunk coordinates use Euclidean floor division so negative world coordinates have one unambiguous owner.

Each nonempty chunk is one static Jolt compound body named `voxel-chunk-{x}-{y}-{z}`. Its body pose is the chunk's world-space origin, and every occupied cell is a unit-cube child named `voxel-{x}-{y}-{z}` with a chunk-local pose. Chunk bodies and child colliders are sorted by those stable identifiers before native creation.

An accepted voxel removal derives the affected chunk from the coordinate and prepares that chunk from the candidate next world state by enumerating exactly its 8×8×8 coordinates; it never scans unrelated occupied cells. The physics adapter validates the complete replacement-body specification against scene limits without rescanning unrelated bodies, creates and adds the replacement before removing the previous native body, and publishes its safe specification only after native creation succeeds. If validation or creation fails, the previous body and safe scene specification remain usable. Removing the final occupied cell removes the chunk body. The P0 mutation path is delete-only; future voxel placement must use the same stable identity and atomic replacement rules.

The contact-listener catalog reuses the stable body slot when a chunk is replaced. No solver step or callback may overlap replacement. Unchanged native bodies are not recreated. Live apply and replay delete canonical active-contact pairs when the removed collider appears on either pair side; every surviving pair remains bit-for-bit unchanged, so the next committed step may remain canonically `persisted` even though Jolt rebuilt the dirty chunk's native contact cache.

P0 evaluates every currently valid anchored grid against the existing voxel field while logically excluding the target cell, using the same `anchor_touches` predicate without cloning the complete voxel set. If any such grid would lose its final support, mining is rejected with an explicit instruction to release the anchor first. Rejection leaves voxel state, inventory, active pairs, processed operations, the derived scene, and the journal unchanged. This avoids silently leaving a static grid without voxel support or partially updating two derived bodies. A later atomic multi-body topology transaction may replace this restriction.

The worker updates the derived chunk before appending the canonical mining event. Failure before native publication leaves both the prior canonical state and prior scene usable. If journal append or later persistence fails after native publication, authoritative writes halt; restart reconstructs the complete scene from whichever world state is durable. Jolt body IDs, sleep state, and caches are never persisted.

## Consequences

- Collision-shape derivation enumerates exactly 512 coordinates, and native replacement is bounded to one eight-cell chunk instead of the complete voxel field. The existing transactional world-state clone remains global and is not covered by this claim.
- Unrelated grids and voxel chunks retain their native body instances and activation state across a mining edit.
- Replacing a body requires temporary native capacity for one additional body. If Jolt cannot allocate it, the edit fails before durability and preserves the old scene.
- Canonical contact identity now includes the stable voxel chunk body ID. The content-version change rejects worlds produced under the former single-body identity without requiring a new world, event, or protocol payload shape.
- Recovery still performs one complete derived-scene build, which is correct because no native state is authoritative.
- Smooth render meshing may dirty neighboring chunks for surface reconstruction; exact unit-cell collision dirties only the coordinate's owning collision chunk.

## Required evidence

- Adapter tests prove successful replacement, final-body removal, preservation of unrelated body state, and failure atomicity with an injected failure after replacement creation but before publication.
- Simulation tests prove the 512-coordinate enumeration cap, negative-coordinate chunk ownership, stable body/collider identity, either-side removal of stale active pairs with bit-for-bit preservation of survivors through live apply and replay, one dirty-chunk replacement per accepted mine, retry idempotency, conservation, collision against remaining cells, no-mutation anchored-support rejection, and exact restart.
- Before-write and after-sync mining failpoints recover the exact prior or mined state and matching derived collider fingerprint. A `p0.7.2` world is rejected before replay; `p0.7.3` restart reproduces identical chunk and collider identities.
- The cross-process mining scenario continues to prove exact coordinate removal and recovery under content `p0.7.3`.
