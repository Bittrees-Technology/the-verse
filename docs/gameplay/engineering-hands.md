# P0.4 engineering hands checkpoint

**Status:** Accepted implementation target

## Player promise

Mining and construction must feel like physical work performed against a persistent world. A mined target must leave authoritative empty volume and visible debris. A constructed block must begin as a frame, preserve its chosen orientation, pass through readable integrity stages, and become functional only after welding completes.

This remains an original clean-room implementation. Publicly observable engineering-sandbox behavior is a quality reference, not a source-code or asset dependency.

## Construction contract

1. Construction mode snaps a preview to a face-adjacent coordinate on the targeted grid.
2. `[` and `]` rotate the selected block in 90-degree yaw steps before placement; `Q` and `E` remain dedicated to character roll.
3. Placing a frame consumes the complete registered component cost exactly once.
4. A new frame begins at 25 percent integrity and is structurally present but nonfunctional.
5. Each accepted weld advances integrity by another 25 percent, clamped at the block definition's maximum health.
6. Power production, power demand, stored power, and voxel anchoring ignore unfinished blocks.
7. Completing the final weld advances the authoritative blocks-built career counter exactly once.
8. Placement and welding must be within the server-owned hand-tool range.
9. Construction completion is durable historical state. Later damage lowers integrity but never turns completed armor back into a frame, and repair never grants completion credit again.
10. Cargo identity may exist on a placed frame, but its inventory remains sealed until the final weld completes construction.

## Mining contract

1. The server validates tool range and the continued existence of the selected integer voxel.
2. An accepted edit removes that exact coordinate, grants only its definition-authorized yield, and persists before acknowledgement.
3. The native client rebuilds its smooth surface from the new snapshot and emits short-lived local rock fragments at the removed coordinate.
4. Retrying the same operation cannot remove or grant the voxel twice.
5. Restart recovery must retain the empty coordinate and the exact conservation ledger.

## Acceptance tests

- A frame-placement retry consumes one component and creates one partial block.
- Invalid orientation values and out-of-range placement are rejected without mutation.
- Three weld operations complete a 25-percent frame; a weld retry does not add integrity twice.
- An unfinished anchor cannot lock a grid even when its coordinate touches voxels.
- Orientation, current health, maximum health, durable completion state, and career credit survive journal and snapshot recovery.
- The cross-process scenario records every mined coordinate, proves each is absent, completes three staged blocks, and still proves deterministic grid splitting and recovery.
- Godot parses, completes the protocol-v6 handshake before receiving state, shows rotated holograms, distinct construction frames and damaged armor, and remeshes after accepted mining.

## Deferred

This checkpoint does not yet add arbitrary six-axis block orientation, compound blocks, per-component bill of materials, projector blueprints, deformable armor, physical character hands, collision-backed grids, or conveyor construction logistics.
