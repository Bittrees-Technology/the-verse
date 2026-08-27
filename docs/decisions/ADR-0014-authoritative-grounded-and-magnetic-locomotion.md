# ADR-0014: Derive grounded and magnetic locomotion from canonical support

**Status:** Accepted

## Context

P0.9 made EVA, radial gravity, landing collision, rotation, and control delivery server authoritative. Its dynamic spherical player body and `surface_contact` convenience flag deliberately do not distinguish a floor from a wall or ceiling. They cannot safely grant walking or jump authority, and a sphere has neither a humanoid standing height nor the support probes needed for slopes, steps, and moving decks.

The existing durable input FIFO, separate received and processed sequence frontiers, shared 60 Hz player/grid event, and bounded native prediction are the correct foundations. Grounded movement must extend those boundaries without adding a client transform, a separate character clock, or a proprietary controller copied from another game.

## Decision

P0.10 uses four canonical locomotion kinds: `eva`, `airborne`, `grounded`, and `magnetic`. The server alone selects the kind and support. Jetpack-on is `eva`. Jetpack-off without a valid support is `airborne`. A gravity-aligned walkable support is `grounded`. An armed magnetic system may become `magnetic` only on a completed grid collider that satisfies proximity, relative-speed, and adhesion limits. Generic collision remains observable, but it never grants walking or jump authority by itself.

Protocol 10 keeps `SetPlayerControl` input-only and adds a boolean jump state. The queued transition persists jump alongside translation, angular input, boost, and dampeners. A false-to-true transition creates one bounded server-owned jump buffer; another jump requires a processed false state. `SetSuitMode` adds the preferred magnetic-boots setting. Clients never submit locomotion kind, support identity, ground normal, step result, jump strength, pose, velocity, elapsed time, or collision result.

Content schema 8 and manifest `p0.10.0` own standing capsule height and radius, eye height, walk and sprint speeds, ground acceleration and braking, jump speed, walkable slope and exit hysteresis, step height, ground snap, support probe distance, jump-buffer and coyote ticks, magnetic probe and catch-speed limits, adhesion acceleration, and reattachment lockout. These values are finite, bounded, and immutable for an opened universe.

World schema 13 stores canonical locomotion kind, up vector, upright body orientation, separately bounded view pitch, optional support body and collider identity, support-local anchor and normal, jump-held and buffer state, support grace, magnetic preference, and reattachment state. It retains the P0.9 received/processed frontiers and bounded control FIFO. Event schema 8 extends the existing `PlayerControlSet`, `SuitModeChanged`, and optional `PlayerPhysicsOutcome`; player and grid motion still commit atomically in one `PhysicsStepCommitted` event.

The physics adapter gains a capsule collider and a bounded deterministic capsule cast/collision-query API. Query results expose stable Verse body/collider identity, fraction or separation, point, and normal; raw Jolt identifiers never escape the adapter. The server ignores the player's own body, rejects non-finite or out-of-budget queries, canonicalizes hit ordering, and fails closed on native errors. This is a project-owned clean-room controller boundary over the pinned MIT/Apache-licensed Jolt dependency, not copied gameplay code.

A standing player uses a dynamic capsule in the same Jolt scene as the planet, voxels, and static or dynamic grids. Before each shared substep, the locomotion solver uses canonical gravity and capsule probes to classify support. A support is walkable only when its normal satisfies the content-owned slope limit relative to canonical up. Ground movement projects the upright view-forward direction and normalized input onto the support plane, then applies a bounded motor relative to the support-point velocity. The motor cannot cancel tangential gravity on a too-steep slope.

Step traversal requires three server queries: an obstruction at the intended tangent displacement, clearance for the raised capsule, and a walkable landing no higher than the content-owned step limit. A failed query leaves ordinary collision response in charge and cannot teleport or penetrate the capsule. Ground snap is similarly bounded and disabled while rising from a jump.

Jump consumes one buffered edge only while grounded, magnetic, or inside the coyote window. It detaches support, inherits the exact support-point velocity, adds the content-owned speed along canonical up, and starts the magnetic reattachment lockout. Holding jump cannot repeat the impulse. Activating the jetpack detaches while retaining world velocity.

For a dynamic grid support, the server stores a support-local foot anchor and normal. Support-point velocity is grid linear velocity plus angular velocity crossed with the world anchor offset. Standing still preserves the local anchor while the grid translates or rotates; relative walking changes it. Jump and detach inherit support-point velocity. A stable collider that moves to a split grid may rebind by collider identity; destruction or implausible support causes `airborne` without a pose teleport.

Magnetic support uses the contacted grid normal as canonical up. Adhesion counters only bounded normal separation and does not erase tangential forces. It never attaches to a planet or voxel, never catches a surface above the relative-speed limit, and releases on jump, jetpack activation, boot disablement, support destruction, or excessive separation. Suit-power consumption is deferred until canonical suit power exists.

Body yaw is integrated around canonical up. View pitch is canonical but separate from the upright body orientation; `Q` and `E` roll the body only in EVA. Radial up changes are parallel-transported onto the next tangent plane to avoid quaternion flips. The native camera may rate-limit visual horizon correction, but tool rays and movement use the server-owned orientation and view pitch.

Historic replay does not rerun Jolt queries. It validates the prior canonical state, processed input transition, support existence and identity, support-local anchor plausibility, locomotion transition, finite normalized up/orientation, support-aware pose and velocity envelopes, and contact evidence before applying the committed quantized outcome. Live execution performs the queries and stages all grid/player results before appending the single physics event. Existing before-write and after-sync recovery rules remain unchanged.

## Consequences

- A wall or ceiling contact cannot authorize walking or jumping.
- A player can walk on planets, voxels, stations, and moving ships while the server owns support and relative velocity.
- Prediction becomes mode- and support-relative. Support changes, lifecycle changes, epoch changes, and large errors snap; small corrections on the same support smooth in support space.
- Capsule casts and step probes add bounded native work per active grounded player. Multiplayer cell budgets and degradation policy must account for them before P1 scale.
- The upright body and separate view pitch change world, event, protocol, and content schemas. P0 worlds have no automatic migration.

## Required evidence

- Flat walking reaches configured speed without radial sinking; normalized diagonal speed differs by less than one percent.
- Sprint and release reach the expected tier and brake relative to support without reversal or oscillation.
- Back-to-back jump press and release survive the FIFO and produce exactly one impulse; delay, duplication, reconnect, and restart cannot duplicate it.
- A slope below the limit is walkable, one above it rejects uphill motor force, and hysteresis prevents state flicker.
- A step at the configured height succeeds only with capsule clearance and a walkable landing; a higher step fails without penetration or camera teleport.
- Fixtures on all six planetary axes keep body up aligned with radial up, and pole traversal has no quaternion flip.
- Magnetic attachment accepts only a completed grid collider under its catch limit and detaches exactly once for each defined release condition.
- A stationary player on translating and rotating grids retains the support-local anchor within tolerance and inherits support-point velocity on jump.
- Grid split/rebind and block destruction produce deterministic support transitions without teleportation.
- Replay, journal/snapshot restart, append failpoints, and impairment tests preserve jump, support, locomotion, and moving-grid state with the exact world hash.
- Native feedback distinguishes `EVA`, `FREEFALL`, `GROUND`, `MAG-LOCK`, and `BOOTS ARMED`; opening a menu neutralizes input while gravity and support motion continue.

## Deliberate limits

P0.10 does not include crouching, prone movement, ladders, ragdolls, animation-root authority, character-to-character collision, impact damage, suit fuel or power consumption, artificial-gravity generators, cockpit possession, rollback networking, lag compensation, or production multiplayer replication. Ladders require canonical climbable block metadata and a dedicated acceptance checkpoint. Magnetic boots in P0.10 are a locomotion rule, not a substitute for ship gravity or pressurization.
