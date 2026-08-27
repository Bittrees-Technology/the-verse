# P0.9 authoritative EVA physics

**Status:** Implemented and verified on macOS and hosted Ubuntu

## Player promise

EVA thrust, six-axis rotation, gravity, dampeners, and landing contact should feel immediate, but the universe—not the client—decides where the character actually is. A modified or lagging client cannot teleport, ignore gravity, tunnel through matter, or extend tool range.

## Acceptance behavior

1. `WASD`, vertical movement, boost, dampeners, mouse look, and `Q`/`E` roll produce input-only character controls. P0.9 has no jump input.
2. The client sends no character position, orientation, velocity, collision result, surface-contact result, or delta time.
3. Every control names the server-owned `movement_epoch` and a monotonic `input_sequence`. The server rejects another epoch, a sequence at or behind the received frontier, non-finite vectors, and excessive vectors before mutation. Durable acceptance advances `last_received_input_sequence`; fixed-step consumption advances the separate `last_processed_input_sequence` used by client reconciliation.
4. Jetpack flight supports directed acceleration, boost, gravity drift with dampeners off, and gravity-compensating braking with dampeners on.
5. With the jetpack off, translation input produces no walking thrust. Existing velocity and gravity remain authoritative, so the character can fall, land, and come to rest without gaining grounded locomotion.
6. The living character is one content-sized dynamic Jolt sphere using `LinearCast` motion quality. Jolt advances its position, orientation, linear velocity, angular velocity, and collision response in the same substeps as the static planet, voxel bodies, and static or dynamic grids.
7. `surface_contact` is true when the final substep's canonical active-contact set contains the player body and false otherwise. Stable body/collider identity and normals remain in the ordinary physics-contact outcome. This convenience signal is not a grounded, walking, or jump permission.
8. Accepted control transitions enter a persisted bounded FIFO and at most one transition is consumed per 60 Hz substep. This preserves a press followed by a release even when both arrive between worker polls. Each transition carries an 18-tick lease from receipt; an expired queued entry advances the processed frontier without reviving its motion. Expiry of the active control clears translation, angular input, and boost, and enables dampeners.
9. Incapacitation clears all motion. Respawn starts stationary and cannot inherit pre-death input.
10. The native client predicts locally from sequenced inputs. It allocates new sequences after the received frontier, queues `Q`/`E` key edges so a complete short tap survives between physics samples, retains inputs until the processed frontier acknowledges fixed-step consumption, and replays only the remaining prediction history against each newer lightweight player state. Held or otherwise non-neutral input refreshes its lease; unchanged canonical-neutral input remains silent instead of creating idle journal traffic. Lifecycle, epoch, reconnect, history-gap, and large-error corrections snap; small presentation corrections smooth.
11. Mining, welding, building, damaging, and inventory range checks use only the canonical server position.
12. Event schema 7 persists accepted `PlayerControlSet` transitions. World schema 12 stores the received and processed frontiers plus the pending FIFO. The same `PhysicsStepCommitted` event that commits grid physics contains one optional `PlayerPhysicsOutcome`, keeping player and grid motion in one atomic time domain.
13. Accepted controls and physics outcomes are durable, hash-chained, idempotent where client-authored, and exactly recoverable without rerunning historic Jolt casts. Live and replay validation enforce finite values, global solver bounds, conservative fixed-step translation and rotation envelopes, and bounded planet/contact penetration; replay applies the committed quantized outcome and does not claim to derive its exact velocity by solving physics again.
14. Browser clients may spectate the canonical pose, velocity, and surface-contact boolean but do not gain a special absolute-position path or need a physics runtime.
15. Character motion updates do not trigger a complete voxel/grid snapshot rebuild on every fixed step.

## Testable feel targets

- A held direction begins moving without a visible full-snapshot delay on the native client.
- Releasing movement with dampeners enabled settles cleanly; disabling dampeners preserves inertia while gravity continues to act.
- Boost is visibly faster but remains within the content-owned maximum.
- Mouse pitch/yaw follows the rolled local frame, while held or tapped `Q`/`E` produces authoritative local-forward roll.
- A press and release received before the next worker poll are applied on successive authoritative substeps rather than collapsing into the final state.
- Jolt `LinearCast` collision against the planet, asteroid, or grid prevents tunneling and produces a stable landing without repeated teleport corrections.
- Disabling the jetpack permits ballistic drift and gravity-driven landing, but does not enable walking or jumping.
- Reconnect resumes from the authoritative pose without replaying stale thrust.

## Verification status

The macOS local gate is green for protocol serialization and rejection, queued press/release consumption, float32-safe control bounds, received-versus-processed acknowledgement, Jolt collision and landing fixtures, conservative replay envelopes, control and physics failpoint recovery, death/respawn clearing, input-only range-gated work, oxygen lifecycle, exact restart hash recovery, and the live native acknowledgement path. Its deterministic Godot impairment harness exercises delayed and skipped motion states, acknowledgement replay, correction smoothing and snaps, menu-open gravity, death/disconnect gating, short `Q`/`E` taps, idle control silence, bounded production prediction buffers, and motion-only updates without structural rebuilds.

The quantized EVA fixture runs in the ordinary Rust test suite and is green on the reference Mac and in [hosted Ubuntu run 33047681929](https://github.com/Bittrees-Technology/the-verse/actions/runs/33047681929). That run also passed the headless Godot impairment and live-client gates plus the release-container smoke. A native Linux direct-download artifact and published Linux performance baseline remain required before the overall P0 exit evidence is complete.

## Explicit limits

This is a bounded P0 sphere-cast controller, not the final avatar simulation. Jetpack-off walking, jump, and a canonical grounded-locomotion state are P0.10-or-later work, together with slopes, stairs, ladders, magnetic boots, and moving-platform attachment. P0.9 also excludes ragdolls, collision damage, other player bodies, animation root motion, suit fuel, rollback netcode, lag compensation, authenticated ownership, and production replication bandwidth.
