# P0.9 authoritative EVA physics

**Status:** Accepted implementation target

## Player promise

EVA thrust, six-axis rotation, gravity, dampeners, and landing contact should feel immediate, but the universe—not the client—decides where the character actually is. A modified or lagging client cannot teleport, ignore gravity, tunnel through matter, or extend tool range.

## Acceptance behavior

1. `WASD`, vertical movement, boost, dampeners, mouse look, and `Q`/`E` roll produce input-only character controls. P0.9 has no jump input.
2. The client sends no character position, orientation, velocity, collision result, surface-contact result, or delta time.
3. Every control names the server-owned `movement_epoch` and a monotonic `input_sequence`. The server rejects another epoch or an old sequence, validates bounded local translation and angular vectors, then owns orientation and advances motion at the content-defined fixed step.
4. Jetpack flight supports directed acceleration, boost, gravity drift with dampeners off, and gravity-compensating braking with dampeners on.
5. With the jetpack off, translation input produces no walking thrust. Existing velocity and gravity remain authoritative, so the character can fall, land, and come to rest without gaining grounded locomotion.
6. The living character is one content-sized dynamic Jolt sphere using `LinearCast` motion quality. Jolt advances its position, orientation, linear velocity, angular velocity, and collision response in the same substeps as the static planet, voxel bodies, and static or dynamic grids.
7. `surface_contact` is true when the final substep's canonical active-contact set contains the player body and false otherwise. Stable body/collider identity and normals remain in the ordinary physics-contact outcome. This convenience signal is not a grounded, walking, or jump permission.
8. Fresh controls carry an 18-tick lease at 60 Hz. Expiry clears translation, angular input, and boost, and enables dampeners, so stale or disconnected input cannot keep accelerating or rotating the character.
9. Incapacitation clears all motion. Respawn starts stationary and cannot inherit pre-death input.
10. The native client predicts locally from sequenced inputs, acknowledges and replays them against newer lightweight player states, then snaps lifecycle/epoch/large errors and smooths small presentation corrections.
11. Mining, welding, building, damaging, and inventory range checks use only the canonical server position.
12. `PlayerControlSet` persists accepted client input. The same `PhysicsStepCommitted` event that commits grid physics contains one optional `PlayerPhysicsOutcome`, keeping player and grid motion in one atomic time domain.
13. Accepted controls and physics outcomes are durable, hash-chained, idempotent where client-authored, and exactly recoverable without rerunning historic Jolt casts.
14. Browser clients may spectate the canonical pose, velocity, and surface-contact boolean but do not gain a special absolute-position path or need a physics runtime.
15. Character motion updates do not trigger a complete voxel/grid snapshot rebuild on every fixed step.

## Testable feel targets

- A held direction begins moving without a visible full-snapshot delay on the native client.
- Releasing movement with dampeners enabled settles cleanly; disabling dampeners preserves inertia while gravity continues to act.
- Boost is visibly faster but remains within the content-owned maximum.
- Mouse pitch/yaw follows the rolled local frame, while held or tapped `Q`/`E` produces authoritative local-forward roll.
- Jolt `LinearCast` collision against the planet, asteroid, or grid prevents tunneling and produces a stable landing without repeated teleport corrections.
- Disabling the jetpack permits ballistic drift and gravity-driven landing, but does not enable walking or jumping.
- Reconnect resumes from the authoritative pose without replaying stale thrust.

## Explicit limits

This is a bounded P0 sphere-cast controller, not the final avatar simulation. Jetpack-off walking, jump, and a canonical grounded-locomotion state are P0.10-or-later work, together with slopes, stairs, ladders, magnetic boots, and moving-platform attachment. P0.9 also excludes ragdolls, collision damage, other player bodies, animation root motion, suit fuel, rollback netcode, lag compensation, authenticated ownership, and production replication bandwidth.
