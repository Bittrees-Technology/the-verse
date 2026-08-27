# P0.7 server-authoritative contact physics

**Status:** Target contract; foundation implemented and acceptance closure in progress

## Player promise

Ships, stations, characters, and mineable terrain must behave as physical objects owned by the authoritative simulation. A modified client cannot pass through a voxel, move a ship through another grid, manufacture collision damage, or choose a final transform. The result should support deliberate engineering work: mass matters, powered control changes motion, collisions create readable consequences, and a valid anchor makes a structure stable.

This checkpoint targets F-005, F-006, F-007, and F-010 and the acceptance evidence for SIM-002, SIM-003, SIM-004, SIM-006, SIM-007, and SIM-008. It does not claim production multiplayer scale.

## Authority contract

1. Grid clients submit bounded control, tool, and construction intents. They never submit a grid pose, velocity, contact, damage amount, or split result. The current P0 character client still proposes an absolute position that is range-, planet-, voxel-, and grid-collision checked; input-only character simulation remains required.
2. The server owns collision shapes, mass, inertia, friction, restitution, force application, anchoring, contact damage, and grid separation.
3. Completed blocks and physical inventory contribute to body mass. Incomplete frames are represented consistently but do not gain completed functional behavior.
4. Static voxel collision is derived from canonical occupied cells. Render edits are dirty-chunk scoped. The current physics adapter atomically rebuilds the complete static compound after a voxel edit; dirty collision-body replacement remains required.
5. Anchoring converts a valid voxel-connected grid to an immovable body. Removing its final valid anchor returns it to dynamic eligibility without teleporting it.
6. Player motion is swept or subdivided against authoritative voxel and grid volumes so a series of individually valid intents cannot tunnel through matter.

## Fixed-step contract

1. The simulation advances with a fixed timestep and a bounded catch-up budget.
2. The content manifest pins an integer 60 Hz schedule. Each committed batch records its solver-step count, the remaining fractional step phase, and the originating substep for every contact; replay advances the canonical tick by the recorded count and restart resumes the recorded phase.
3. Stable body and collider identifiers are sorted before insertion and before canonical outcomes are recorded.
4. Live physics may use floating-point Jolt calculations, but canonical committed values are finite, bounded, and quantized at the event boundary.
5. The P0 proof uses a deterministic single-thread solver configuration. Parallel production stepping requires a later benchmark and replay decision.
6. Sleeping bodies remain stable. Low-speed contact must not cause unbounded jitter, energy gain, or anchor drift.

## Recovery contract

The live solver is derived state. Each accepted tick records ordered, quantized transforms, velocities, native manifold telemetry, canonical contact lifecycle, exact integer reduced translational mass, and an explicitly labeled pairwise estimated impulse. Reduced translational mass ignores contact direction, lever arm, and rotational inertia; it is not damage evidence. Recovery applies committed outcomes rather than attempting to reproduce an earlier floating-point collision. After every live commit and after recovery, the server rebuilds Jolt bodies from the recovered canonical snapshot. World schema 9 preserves active pairs across that rebuild. Collision-driven damage and topology outcomes require a later event-schema revision after applied post-solver impulse data is available.

An interruption before the journal commit retains the prior state. An interruption after the durable commit recovers the new state. Neither path may duplicate damage, blocks, inventory, or grid identities.

## Current checkpoint evidence

Implemented and tested:

- An isolated, pinned, license-recorded Jolt 5.3 adapter with static/dynamic compound bodies, stable leaf identities, bounded raw manifold capture, bounded forces/torques, and single-thread fixed stepping.
- Protocol v6 grid controls, explicit construction-completion state, full grid quaternion snapshots, exact block/cargo-derived mass, authoritative grid–voxel response, swept player–voxel rejection, quantized committed body/contact outcomes, canonical contact lifecycle across rebuild/restart, exact graceful restart, and truncated final-journal recovery.
- Sparse dirty render chunks, a complete native mining/building/restart scenario, and initial Apple Silicon compound-body measurements.

Still required for P0.7 acceptance:

- A project-owned Jolt/JoltC post-solve callback that exposes applied impulses (including the winning CCD path), followed by server-derived collision damage and atomic damage/split outcomes. The current pairwise estimate is telemetry only.
- Dirty collision chunk replacement, runtime grid–grid momentum tolerance, anchor/contact stability, mass-response, player–grid, and persistence failpoint tests.
- Repeatable edit-to-remesh, Ubuntu, network, multi-body, and large-grid evidence plus a native Linux artifact.

## Acceptance scenarios

- A dynamic grid cannot pass through occupied asteroid voxels or another grid.
- Two dynamic grids exchange momentum within the published P0 tolerance.
- A resting grid remains stable on a contact surface for the benchmark interval.
- An anchored grid does not move under ordinary control or contact forces.
- Removing the last valid anchor creates one eligible dynamic body with conserved blocks and inventory.
- An impact above the configured resistance produces server-owned damage; any resulting split conserves topology and inventories.
- Mining a surface cell removes its collider after the accepted edit and rebuilds only influenced chunks.
- Restart during a physics commit recovers either the complete prior tick or the complete committed tick.
- macOS and Ubuntu reports publish tick time by dynamic-body and completed-block count.

## Deliberate limits

P0.7 proves one authoritative cell and a bounded body count. It does not add production interest management, inter-cell travel, safe zones, offline destruction, airtight rooms, markets, blockchain custody, or editable planet terrain. Those systems must build on this authority and recovery contract rather than bypass it.
