# P0.7 server-authoritative contact physics

**Status:** Target contract; foundation implemented and acceptance closure in progress

## Player promise

Ships, stations, characters, and mineable terrain must behave as physical objects owned by the authoritative simulation. A modified client cannot pass through a voxel, move a ship through another grid, manufacture collision damage, or choose a final transform. The result should support deliberate engineering work: mass matters, powered control changes motion, collisions create readable consequences, and a valid anchor makes a structure stable.

This checkpoint targets F-005, F-006, F-007, and F-010 and the acceptance evidence for SIM-002, SIM-003, SIM-004, SIM-006, SIM-007, and SIM-008. It does not claim production multiplayer scale.

## Authority contract

1. Grid clients submit bounded control, tool, and construction intents. They never submit a grid pose, velocity, contact, damage amount, or split result. P0.9 now folds input-only EVA, landing/contact, and rotation into the same atomic `PhysicsStepCommitted` event under [ADR-0013](../decisions/ADR-0013-input-only-authoritative-character-motion.md); no current client submits an absolute character position.
2. The server owns collision shapes, mass, inertia, friction, restitution, force application, anchoring, contact damage, and grid separation.
3. Completed blocks and physical inventory contribute to body mass. Incomplete frames are represented consistently but do not gain completed functional behavior.
4. Static voxel collision is derived from canonical occupied cells. Content pins 8×8×8-cell collision chunks with Euclidean floor ownership, stable chunk-body and cell-collider identities, and chunk-local child poses. Accepted mining atomically replaces only the owning collision body under [ADR-0011](../decisions/ADR-0011-dirty-voxel-collision-chunks.md).
5. Anchoring converts a valid voxel-connected grid to an immovable body. Removing its final valid anchor returns it to dynamic eligibility without teleporting it.
6. The living player is a dynamic 1.8 m Jolt capsule using `LinearCast` motion quality. Its optional support-aware `PlayerPhysicsOutcome` commits atomically with grid physics and contact lifecycle against the planet, voxel chunks, and grids. P0.10 derives walking, jump, radial upright alignment, steps, slopes, and magnetic attachment from server-owned stable-identity capsule queries under [ADR-0014](../decisions/ADR-0014-authoritative-grounded-and-magnetic-locomotion.md).

## Fixed-step contract

1. The simulation advances with a fixed timestep and a bounded catch-up budget.
2. The content manifest pins an integer 60 Hz schedule. Each committed batch records its solver-step count, the remaining fractional step phase, and the originating substep for every contact; replay advances the canonical tick by the recorded count and restart resumes the recorded phase.
3. Stable body and collider identifiers are sorted before insertion and before canonical outcomes are recorded.
4. Replacing one dirty voxel chunk must not recreate unrelated native bodies. Removed collider pairs are deleted from canonical active-contact state in the mining event; surviving stable pairs retain their canonical lifecycle.
5. Live physics may use floating-point Jolt calculations, but canonical committed values are finite, bounded, and quantized at the event boundary.
6. The P0 proof uses a deterministic single-thread solver configuration. Parallel production stepping requires a later benchmark and replay decision.
7. Sleeping bodies remain stable. Low-speed contact must not cause unbounded jitter, energy gain, or anchor drift.

## Recovery contract

The live solver is derived state. Each accepted tick records ordered, quantized transforms, velocities, native manifold telemetry, canonical contact lifecycle, exact integer reduced translational mass, and an explicitly labeled pairwise estimated impulse. Reduced translational mass ignores contact direction, lever arm, and rotational inertia; it is not damage evidence. Recovery applies committed outcomes rather than attempting to reproduce an earlier floating-point collision. Replay validates player contact points against the bounded swept capsule and counterpart geometry. Because Jolt `LinearCast` may report a speculative manifold along at most one configured fixed-step maximum-velocity sweep, the midpoint check admits half of that bounded separation plus contact and quantization slop; a point beyond that envelope still fails before mutation. During live commit processing, the server reconciles derived Jolt state to candidate or canonical state at the operation's defined atomic boundary: physics-step outcomes may rebuild dynamic bodies as required, while voxel edits publish ADR-0011 dirty replacement before journal append. Recovery performs a complete derived-scene rebuild. World schema 9 preserves active pairs across reconciliation. Collision-driven damage and topology outcomes require a later event-schema revision after applied post-solver impulse data is available.

An interruption before the journal commit retains the prior state. An interruption after the durable commit recovers the new state. A dirty-chunk failure before native publication preserves the prior usable scene; a persistence failure after native publication halts writes until recovery derives the scene from durable state. Neither path may duplicate damage, blocks, inventory, or grid identities.

## Current checkpoint evidence

Implemented and tested:

- An isolated, pinned, license-recorded Jolt 5.3 adapter with static/dynamic compound bodies, stable leaf identities, bounded raw manifold capture, bounded forces/torques, and single-thread fixed stepping.
- Protocol v6 grid controls, explicit construction-completion state, full grid quaternion snapshots, exact block/cargo-derived mass, authoritative grid–voxel response, swept player–voxel rejection, quantized committed body/contact outcomes, canonical contact lifecycle across rebuild/restart, exact graceful restart, and truncated final-journal recovery.
- Authoritative runtime evidence for equal-mass grid–grid momentum exchange and exact restart, cargo mass reducing acceleration under the same powered control, a two-second settle plus two-second resting-contact observation, immovable anchored bodies under impact followed by conservative release and restart, and swept player rejection against axis-aligned and rotated grids. The P0 equal-mass head-on tolerance is at most `1 kg m/s` absolute total linear-momentum error after canonical commit quantization. During the resting observation, translation drift is at most `0.1 mm`, linear speed at most `1 mm/s`, and angular speed at most `0.001 rad/s`. Adapter coverage separately proves unequal-density collision response.
- In-process journal failpoints prove that a returned failure before write preserves the prior physics tick and a returned failure after journal synchronization recovers the complete new tick and resumes from it. Torn final records are truncated. Atomic snapshots synchronize the replacement file and parent directory after rename.
- Content `p0.7.3` partitions voxel collision into stable 8×8×8 bodies. Mining patches one body exactly once, prunes only the removed collider's active pairs, preserves surviving lifecycle, rejects final anchored support without mutation, and reconstructs identical collision fingerprints across before-write or after-sync recovery. Native staging failure restores the prior body, catalog, and stepable scene.
- Sparse dirty render chunks, a complete native mining/building/restart scenario, and initial Apple Silicon compound-body measurements.

Still required for P0.7 acceptance:

- A project-owned Jolt/JoltC post-solve callback that exposes applied impulses (including the winning CCD path), followed by server-derived collision damage and atomic damage/split outcomes. The current pairwise estimate is telemetry only.
- Subprocess crash injection across journal, state-publish, derived-scene rebuild, and snapshot boundaries.
- Repeatable edit-to-remesh, Ubuntu, network, multi-body, and large-grid evidence plus a native Linux artifact.

## Acceptance scenarios

- A dynamic grid cannot pass through occupied asteroid voxels or another grid.
- Two dynamic grids exchange momentum within the published P0 tolerance.
- A resting grid remains stable on a contact surface for the benchmark interval.
- An anchored grid does not move under ordinary control or contact forces.
- Removing the last valid anchor creates one eligible dynamic body with conserved blocks and inventory.
- An impact above the configured resistance produces server-owned damage; any resulting split conserves topology and inventories.
- Mining a surface cell removes its collider and any stale active pair containing it, atomically replaces only its owning collision chunk, preserves every surviving pair and unrelated body state, and recovers the same identities after restart. P0 tests the candidate voxel field with the canonical anchor predicate and rejects removing a currently anchored grid's final support without mutating world, scene, or journal; the player must release that anchor first.
- Restart during a physics commit recovers either the complete prior tick or the complete committed tick.
- macOS and Ubuntu reports publish tick time by dynamic-body and completed-block count.

## Deliberate limits

P0.7 proves one authoritative cell and a bounded body count. It does not add production interest management, inter-cell travel, safe zones, offline destruction, airtight rooms, markets, blockchain custody, or editable planet terrain. Those systems must build on this authority and recovery contract rather than bypass it.
