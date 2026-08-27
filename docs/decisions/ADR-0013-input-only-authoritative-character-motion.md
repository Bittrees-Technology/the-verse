# ADR-0013: Use one atomic input-only EVA physics step

**Status:** Accepted

## Context

The native client currently integrates EVA and provisional surface locomotion locally, then submits an absolute `MovePlayer` position. The server bounds and collision-sweeps that proposal, but a modified client still chooses the final displacement and supplies its own frame timing. That is not an acceptable authority boundary for mining range, combat, safe zones, logistics, or multiplayer.

The local proof already has server-owned radial gravity, voxel and grid geometry, fixed-step timing, durable control inputs for grids, canonical Jolt physics outcomes, and client-side visual prediction. Character motion should enter that same atomic physics step without pretending that the P0 sphere-cast controller is a grounded humanoid solver.

## Decision

Protocol 8 replaces `MovePlayer { position }` with `SetPlayerControl`. A character control contains a server-owned `movement_epoch`, a monotonic `input_sequence`, bounded local translation and angular vectors, boost, and dampener input. It contains no position, orientation, velocity, surface-contact result, collision result, elapsed time, or jump input. Mouse pitch/yaw and `Q`/`E` roll are angular input; the server integrates and normalizes the canonical quaternion in the character's local frame.

Content schema 7 and manifest `p0.9.0` own character mass and collision radius; thrust, boost, linear/angular damping and acceleration; normal and boost speed limits; maximum angular speed; and the control-lease length. World schema 11 stores canonical character position, orientation, linear and angular velocity, `movement_epoch`, last processed `input_sequence`, current bounded controls, the absolute simulation tick at which the control expires, and the current `surface_contact` boolean. Event schema 6 adds durable `PlayerControlSet` and extends `PhysicsStepCommitted` with one optional `PlayerPhysicsOutcome`; there is no separate player-motion system event.

The worker advances character and grid physics together in 60 Hz substeps derived only from server time. A living character is one content-sized, content-massed dynamic Jolt sphere with `LinearCast` motion quality. It occupies the same derived scene as the static spherical planet body, voxel-chunk bodies, and static or dynamic grid bodies. Each substep converts canonical local translation and angular controls into bounded world-space force and torque. Jetpack flight accelerates toward a server-owned target speed; boost and inertial dampening are server-owned rules. With the jetpack off, translation input supplies no walking force: existing velocity and gravity remain active so the character may fall and land. Jolt integrates the body's pose, linear/angular velocity, continuous collision detection, and contact response atomically with grid motion.

The character body and collider have stable Verse identifiers. `surface_contact` is a convenience boolean derived from whether the final substep's canonical active-contact set contains that player body. Stable body/collider identities, contact phase, and quantized manifold telemetry remain in the ordinary `PhysicsStepCommitted` contact outcomes. A later final substep without a player contact clears the boolean. It is contact evidence for presentation and future locomotion work, not grounded-locomotion authority. Player body outcomes are finite, bounded, normalized, and quantized before they enter canonical state.

Grid and player motion use one shared fixed-step phase and one atomic `PhysicsStepCommitted` system event. Its optional `PlayerPhysicsOutcome` records player identity; resulting position, orientation, linear/angular velocity, and surface-contact boolean; resulting bounded controls, boost, and dampeners; and the absolute control-expiry tick. `PhysicsStepCommitted` records the shared step count, remaining phase, body outcomes, contact outcomes, and active-contact set. The player field is absent when no living character participates. `movement_epoch` and `input_sequence` remain durable in `PlayerControlSet` and canonical player state rather than being duplicated in every system outcome. Live execution uses Jolt; replay validates bounds, contact identity, and the prior canonical state, then applies the committed quantized outcome without rerunning historic Jolt solves.

Controls use an 18-tick server-step lease, equal to 300 milliseconds at the pinned 60 Hz step. `PlayerControlSet` records the absolute expiry tick. A fresh, in-order control intent refreshes the lease even when the directional values are unchanged. Operation retries are idempotent; an input sequence is monotonic within its movement epoch, and another epoch or an old sequence cannot revive stale control. When the lease expires, translation, angular input, and boost clear and dampeners engage. Disconnects and packet loss therefore cannot leave a character accelerating or rotating indefinitely.

An accepted `PlayerControlSet` is appended before its control state is published. For a live physics step, the worker stages and validates the complete grid and optional player outcome before appending the single `PhysicsStepCommitted` event. Failure before durability restores or rebuilds the derived scene to the prior canonical state; failure after journal synchronization recovers the committed event and rebuilds Jolt from it. Neither boundary can publish a partial player-only or grid-only step.

Incapacitation clears linear/angular velocity, motion input, boost, surface contact, and the control lease atomically. Respawn advances the server-owned movement epoch, resets the input sequence, and starts from the server-selected point with identity orientation, zero velocity, and dampeners enabled. Dead players cannot submit controls. A `PhysicsStepCommitted` event contains no player outcome for an incapacitated player.

The native client may predict the same movement for responsiveness. It samples in its fixed physics loop, keeps an input-sequence buffer, resets simulation to each newer canonical player state, discards acknowledged inputs, and replays only unacknowledged inputs. Presentation snaps large or lifecycle/epoch errors and smooths bounded small corrections. Prediction cannot extend interaction range or suppress authoritative collision. Reconnect starts from the latest canonical pose and velocity before sending controls.

Motion replication uses a lightweight versioned player-state message at a bounded cadence. Full world snapshots remain for handshake, world changes, and recovery rather than causing every character tick to rebuild every grid and voxel presentation.

## Consequences

- A client can choose bounded translation and angular intent but cannot choose character position, orientation, velocity, collision, gravity, or elapsed time.
- Mining, welding, construction, damage, and inventory range checks use a position produced by the server controller.
- Control and motion events increase local P0 event and full-snapshot traffic. P1 authenticated session ownership, spatial deltas, interest management, and backpressure remain required before large-scale multiplayer.
- The bounded sphere-cast controller is clean-room Verse code and does not reproduce third-party implementation details.
- Historic replay applies committed outcomes rather than depending on cross-platform reproduction of Jolt floating-point queries. Live macOS and Linux results still require quantization after every substep and cross-platform tolerance evidence.

## Required evidence

- Protocol serialization proves that character-control messages contain no pose, orientation, velocity, collision, or client delta-time fields.
- Invalid, non-finite, or over-limit controls reject before mutation.
- Equivalent input and server time produce the exact expected EVA, boost, drift, dampening, gravity, landing, and local-frame rotation outcomes.
- The dynamic Jolt sphere's `LinearCast` motion quality prevents tunneling through the static planet, a voxel, or a grid while nearby clear motion remains possible.
- Landing produces the expected canonical contact pair and surface-contact boolean without enabling walking or jump behavior.
- Stale controls expire into a safe dampened state, including after disconnect and restart.
- Replay rejects a mismatched prior canonical state or tampered result pose, velocity, phase, movement epoch, input sequence, surface contact, collision, or lease fields before mutation.
- Control retries are idempotent; before-write and after-sync motion failures recover exactly the complete prior or durable outcome.
- Incapacitation and respawn clear motion state exactly and survive journal and snapshot restart.
- Lightweight player-state updates acknowledge input sequences without triggering full world rebuilds; native prediction remains responsive, reconciles without sustained jitter, and cannot continue moving while dead or disconnected.
- The cross-process scenario moves using input only, performs a range-gated voxel action, restarts, and recovers the exact canonical position, velocity, controls, and world hash.

## Deliberate limits

P0.9 does not add walking, jump, or a canonical grounded-locomotion state. Those capabilities, plus slopes, stairs, ladders, magnetic boots, and moving-platform attachment, are explicitly P0.10-or-later work. Crouching, ragdolls, character-to-character collision, suit fuel or power consumption, impact damage, animation-driven motion, lag compensation, rollback networking, authenticated player ownership, and multiplayer interest management also remain later. P0.9 does not claim production delayed/permissioned browser spectating. Grid cockpits and ship-control possession remain separate work.
