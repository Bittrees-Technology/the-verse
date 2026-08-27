# ADR-0013: Use input-only authoritative character motion

**Status:** Accepted

## Context

The native client currently integrates EVA and walking locally, then submits an absolute `MovePlayer` position. The server bounds and collision-sweeps that proposal, but a modified client still chooses the final displacement and supplies its own frame timing. That is not an acceptable authority boundary for mining range, combat, safe zones, logistics, or multiplayer.

The local proof already has server-owned radial gravity, voxel and grid geometry, fixed-step timing, durable control inputs for grids, canonical physics outcomes, and client-side visual prediction. Character motion should use the same principles without pretending that the P0 sphere controller is a production humanoid solver.

## Decision

Protocol 8 replaces `MovePlayer { position }` with `SetPlayerControl`. A character control contains a movement epoch, monotonic input sequence, bounded local translation and angular vectors, boost, dampener, and one-shot jump inputs. It contains no position, orientation, velocity, grounded result, collision result, or elapsed time. Mouse pitch/yaw and `Q`/`E` roll are angular input; the server integrates the canonical quaternion.

Content schema 7 and manifest `p0.9.0` own walk, jetpack, boost, linear/angular acceleration, damping, jump, collision-radius, surface-clearance, fixed-step, control-send, and stale-control limits. World schema 11 stores canonical character position, orientation, velocity, grounded state, movement epoch, last processed input sequence, current bounded controls, remaining control lease, and jump latch. Event schema 6 adds durable `PlayerControlSet` and extends the fixed-step physics outcome with one optional `PlayerMotionOutcome`.

The worker advances character motion in 60 Hz substeps derived only from server time. Each substep uses canonical suit mode, control state, orientation, environment gravity, planet surface, voxels, and grids. Jetpack flight accelerates toward a server-owned target speed; boost and inertial dampening are server-owned rules. Jetpack-off motion projects input onto the local gravity tangent, applies gravity, permits a latched jump only while grounded, and probes canonical geometry for ground contact.

The proof character is a sphere. Motion sweeps from the previous canonical position to the proposed position. Planet penetration is clamped to the configured surface clearance and inward velocity is removed. A voxel or grid collision retains the prior position and applies the configured collision velocity retention. Outcomes are quantized before they enter canonical state.

Grid and player motion use one shared fixed-step phase and one atomic system event. Each committed step records the exact previous and resulting player position, orientation, velocity, grounded state, control-lease result, jump consumption, and collision result beside the grid/contact outcomes, step count, and remaining phase. Moving grids therefore cannot advance through a separate time domain from the character. Replay derives the same quantized kinematic player outcome from the prior canonical state and recorded grid outcome, rejecting any mismatch before mutation; it still applies historic Jolt grid outcomes without re-solving them.

Controls use a short server-step lease. A fresh, in-order control intent refreshes the lease even when the directional values are unchanged. Duplicate input sequences are idempotent and old epochs or out-of-order sequences cannot revive stale control. When the lease expires, translation, angular input, boost, and jump clear and dampeners engage. Disconnects and packet loss therefore cannot leave a character accelerating indefinitely. Orientation remains at the last canonical value.

Incapacitation clears velocity, motion input, jump, boost, and the control lease atomically. Respawn starts stationary with dampeners enabled. Dead players cannot submit controls. System-owned motion may settle a previously moving living character, but no character motion event is emitted for an incapacitated player.

The native client may predict the same movement for responsiveness. It samples in its fixed physics loop, keeps an input-sequence buffer, resets simulation to each newer canonical player state, discards acknowledged inputs, and replays only unacknowledged inputs. Presentation snaps large or lifecycle/epoch errors and smooths bounded small corrections. Prediction cannot extend interaction range or suppress authoritative collision. Reconnect starts from the latest canonical pose and velocity before sending controls.

Motion replication uses a lightweight versioned player-state message at a bounded cadence. Full world snapshots remain for handshake, world changes, and recovery rather than causing every character tick to rebuild every grid and voxel presentation.

## Consequences

- A client can choose bounded translation and angular intent but cannot choose character position, orientation, velocity, collision, gravity, or elapsed time.
- Mining, welding, construction, damage, and inventory range checks use a position produced by the server controller.
- Control and motion events increase local P0 event and full-snapshot traffic. P1 spatial deltas and input sequencing remain required before large-scale multiplayer.
- The deterministic sphere controller is clean-room Verse code and does not reproduce third-party implementation details.
- Exact replay across supported macOS and Linux targets depends on quantization after every substep and cross-platform evidence.

## Required evidence

- Protocol serialization proves that character-control messages contain no pose, orientation, velocity, collision, or client delta-time fields.
- Invalid, non-finite, over-limit, or non-normalized controls reject before mutation.
- Equivalent input and server time produce the exact expected jetpack, boost, drift, dampening, walking, gravity, grounded, and jump outcomes.
- Swept motion cannot tunnel through a voxel, grid, or planet surface; nearby clear motion remains possible.
- Stale controls expire into a safe dampened state, including after disconnect and restart.
- Replay rejects tampered previous/result pose, velocity, phase, grounded, collision, lease, or jump fields before mutation.
- Control retries are idempotent; before-write and after-sync motion failures recover exactly the complete prior or durable outcome.
- Incapacitation and respawn clear motion state exactly and survive journal and snapshot restart.
- Lightweight player-state updates acknowledge input sequences without triggering full world rebuilds; native prediction remains responsive, reconciles without sustained jitter, and cannot continue moving while dead or disconnected.
- The cross-process scenario moves using input only, performs a range-gated voxel action, restarts, and recovers the exact canonical position, velocity, controls, and world hash.

## Deliberate limits

P0.9 does not add crouching, ladders, magnetic boots, slopes, stairs, moving-platform attachment, ragdolls, character-to-character collision, suit fuel or power consumption, impact damage, animation-driven motion, lag compensation, rollback networking, authenticated player ownership, or multiplayer interest management. It does not claim production delayed/permissioned browser spectating. Grid cockpits and ship-control possession remain separate work.
