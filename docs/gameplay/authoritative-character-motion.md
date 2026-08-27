# P0.9 authoritative character motion

**Status:** Accepted implementation target

## Player promise

EVA, gravity, walking, jumping, dampeners, and collision should feel immediate, but the universe—not the client—decides where the character actually is. A modified or lagging client cannot teleport, ignore gravity, tunnel through matter, or extend tool range.

## Acceptance behavior

1. `WASD`, vertical movement, boost, dampeners, jump, mouse look, and `Q`/`E` roll produce input-only character controls.
2. The client sends no character position, velocity, collision result, grounded result, or delta time.
3. The server validates monotonic input sequences plus bounded local translation and angular vectors, then owns orientation and advances motion at the content-defined fixed step.
4. Jetpack flight supports directed acceleration, boost, gravity drift with dampeners off, and gravity-compensating braking with dampeners on.
5. Jetpack-off motion supports gravity-tangent walking, canonical ground contact, and a grounded-only jump.
6. The canonical sphere controller cannot pass through planet surface, occupied voxels, or complete and incomplete grid blocks.
7. Stale or disconnected input expires quickly, clears thrust and jump, and enables dampeners.
8. Incapacitation clears all motion. Respawn starts stationary and cannot inherit pre-death input.
9. The native client predicts locally from sequenced inputs, acknowledges and replays them against newer lightweight player states, then snaps lifecycle/large errors and smooths small presentation corrections.
10. Mining, welding, building, damaging, and inventory range checks use only the canonical server position.
11. Accepted controls and motion outcomes are durable, hash-chained, idempotent where client-authored, and exactly recoverable.
12. Browser clients may spectate the canonical pose but do not gain a special absolute-position path.
13. Character motion updates do not trigger a complete voxel/grid snapshot rebuild on every fixed step.

## Testable feel targets

- A held direction begins moving without a visible full-snapshot delay on the native client.
- Releasing movement with dampeners enabled settles cleanly; disabling dampeners preserves inertia while gravity continues to act.
- Boost is visibly faster but remains within the content-owned maximum.
- Colliding with an asteroid or grid stops penetration without camera oscillation or repeated teleport corrections.
- Walking stays tangent to gravity and jumping cannot be repeated while airborne.
- Reconnect resumes from the authoritative pose without replaying stale thrust.

## Explicit limits

This is a deterministic P0 sphere controller, not the final avatar simulation. It does not include slopes, stairs, ladders, magnetic boots, moving-platform attachment, ragdolls, collision damage, other player bodies, animation root motion, suit fuel, rollback netcode, lag compensation, authenticated ownership, or production replication bandwidth.
