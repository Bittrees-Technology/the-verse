# Authoritative hand-tool targeting

**Status:** Implemented and locally verified

## Player promise

The reticle identifies one physical surface. Mining, construction, welding, and
cutting cannot pass through a nearer asteroid voxel or grid block, and a client
cannot authorize a different target by changing an identifier in its request.
Construction attaches to the exact visible face instead of guessing from the
camera or from an asteroid direction.

## Canonical eye ray

The server reconstructs every hand-tool target from the authenticated actor's
canonical state. The actor position is the capsule center. The eye origin is:

```text
actor position + canonical up * (eye height - standing height / 2)
```

Grounded, magnetic, and airborne actors use the nonzero canonical locomotion
up vector. EVA uses the actor orientation's local positive-Y axis. EVA forward
is local negative Z. Other locomotion modes apply the canonical view pitch to
local negative Z before transforming it by actor orientation. Client-submitted
positions, rays, hit distances, normals, and outcomes are never authoritative.

## Closest-hit rules

1. Voxels and blocks are closed one-metre cubes centered on their canonical
   integer coordinate for this interaction test.
2. Voxels use exact half-cell three-dimensional DDA. A ray parallel to a cell
   boundary checks both touching columns.
3. Blocks use ray/AABB slabs after transforming the ray into the grid's rotated
   local frame.
4. The maximum inclusive surface distance is `9.0 m`.
5. Distances within `1e-9 m` are tied. A block wins a block/voxel tie; remaining
   ties use stable grid, block, or voxel identity order.
6. Equal slab or DDA entry axes resolve X, then Y, then Z.
7. An eye starting inside or touching occupied geometry records a zero-distance
   occluder with no usable entry face. Every tool action then fails closed.

## Action authorization

| Action | Required canonical result |
| --- | --- |
| Mine | Closest usable hit is the requested voxel |
| Weld | Closest usable hit is the requested block |
| Damage/cut | Closest usable hit is the requested block |
| Build | Closest usable hit is a block on the requested grid, and the requested coordinate equals that block coordinate plus its local outward face normal |

The same rule runs before event preparation and again during replay against the
prior canonical state. Wrong actor substitution, occlusion, an incorrect build
face, excessive surface range, and a missing usable face reject before world,
inventory, career, journal, or collision state can change. Existing operation
IDs and protocol payloads remain unchanged.

## Native presentation

The native client applies matching DDA, rotated-grid slab, range, tie, and
fail-closed rules to the presented camera. It derives mutually exclusive voxel
or block targets from one closest hit, retains the surface hit point and local
and world normals, rotates block highlights with their grid, derives every
construction coordinate from the hit face, and ends tool effects at the
surface. Prediction can briefly disagree with the canonical actor; the server
rejects that action safely and the next authoritative state reconciles it.

## Evidence

- Unit tests cover all face axes, rotated grids, block/voxel ties, stable
  identity ordering, exact range boundaries, origin contact, and parallel
  boundary rays.
- Simulation tests cover prepare and replay rejection, actor substitution,
  exact-face construction, non-mutation, and inclusive nine-metre targeting.
- Native headless tests cover six faces, rotated adjacency and highlighting,
  both occlusion orders, mutual exclusion, and surface effect endpoints.
- Input-only two-player and full progression scenarios aim through ordinary
  controls, reject a deliberately occluded damage request, and preserve exact
  restart recovery.

## Deliberate limits

This checkpoint does not add lag-compensated combat, rollback, projectile
ballistics, deformable block hitboxes, compound sub-block selection, tool reach
upgrades, ownership permissions, or interest-managed spatial acceleration.
Those systems must retain this server-reconstructed closest-visible-target
boundary.
