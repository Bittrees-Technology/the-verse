# Universe simulation

**Status:** Proposed baseline

## Coordinate model

The Verse uses hierarchical coordinates rather than one global floating-point scene.

```text
Universe
└── generated region key
    └── sector coordinate
        └── simulation cell
            └── local double-precision frame
                └── grid or voxel-local frame
```

Proposed persistent address:

```text
UniverseAddress {
  universe_id
  region_seed
  sector_x: signed 128-bit integer
  sector_y: signed 128-bit integer
  sector_z: signed 128-bit integer
  local_position: three signed fixed-point values
}
```

Network and database representations encode 128-bit values as canonical decimal strings or 16-byte two's-complement values. Physics operates only in a bounded local frame and uses origin rebasing.

The address space is finite mathematically but has no practical reachable boundary.

## Generated celestial registry

A generated region is deterministic from:

- Universe seed.
- Sector coordinate.
- Generation-rule version.
- Approved content manifest.
- Governance-controlled frontier policy.

Planets are generated only when minimum-separation rules are satisfied. Asteroids may appear as belts, dense clusters, sparse fields, or isolated bodies. Once a generated body becomes canonical, its identity and coordinates are recorded so later generator changes cannot move it.

Planets and asteroids do not orbit in the initial model.

## Frontier expansion

New sectors become canonical when:

1. A route, survey, or expansion action requests an unmaterialized region.
2. The generator produces a candidate manifest.
3. Minimum-separation and content rules validate it.
4. The manifest is signed by the universe service.
5. The sector becomes immutable apart from authorized voxel edits and future content migrations.

Resource availability expands through frontier discovery rather than regenerating mined canonical deposits.

## Cell lifecycle

```text
Unmaterialized → Generated → Sleeping → Background → Active → Draining → Sleeping
```

- **Sleeping:** no real-time process; state is a snapshot plus scheduled events.
- **Background:** low-frequency power, travel, production, cleanup, and market-linked simulation.
- **Active:** full physics and client replication.
- **Draining:** no new entrants; transfer and snapshot complete before worker release.

Attacks, arrivals, expiring timers, or observation may wake a cell.

## Player and grid handoff

Cross-cell movement uses:

1. Source freezes the transferable entity at a tick boundary.
2. Source writes a transfer package and prepare event.
3. Destination validates schema, ownership, and capacity.
4. Universe coordinator commits the destination lease.
5. Destination activates the entity.
6. Source writes completion and removes its active copy.

A transfer operation is idempotent. At no time may two cells have write authority over the same grid.

## Dynamic and static grids

### Dynamic grid

A construct is dynamic when it is not anchored to voxel terrain or an explicitly static foundation. It may translate, rotate, dock, split, collide, and cross cells.

### Static grid

A construct becomes static or partitionable when connected to voxel terrain through an approved foundation/anchor relationship. Static structures may span multiple cells and receive interior subcells.

### Transition

Dynamic-to-static and static-to-dynamic conversion requires:

- A stable tick boundary.
- Structural connectivity calculation.
- Velocity and contact validation.
- Rebuilding spatial ownership.
- Persistence checkpoint.
- Public event.

Removing the final anchor can return a structure to dynamic simulation if the resulting construct fits the supported capital-ship model.

## No arbitrary size cap

The product does not define a maximum block count. The implementation instead uses:

- Spatial partitioning.
- Compound-collider aggregation.
- Physics sleeping.
- Network and conveyor graph partitioning.
- Interior cells.
- Distance-based update frequency.
- Per-system work queues.
- Load-shedding that preserves conservation.
- Static conversion for voxel-connected megastructures.

The P0/P1 implementation will still publish tested operating envelopes. “No arbitrary cap” does not mean every block receives full-rate simulation under unlimited load.

## Physics and damage

The authoritative cell owns:

- Rigid-body state.
- Contacts and constraints.
- Thruster and torque application.
- Structural connectivity.
- Block health.
- Grid splitting.
- Debris creation.
- Voxel impacts.
- Projectile and weapon validation.

Clients predict local presentation but accept corrections. Competitive actions must never depend solely on client collision results.

## Long-duration travel

Interplanetary travel is a durable route state, not a continuously active physics scene for days.

A route has:

- Origin and destination.
- Path segments.
- Departure time.
- Propulsion model.
- Fuel and power budget.
- Expected arrival.
- Interception windows.
- Scheduled background events.
- Last authoritative checkpoint.

The ship enters active physics when observed, intercepted, maneuvering, near a hazard, or approaching a destination. Otherwise the route service advances it analytically.

## Prototype gates

The architecture is not accepted for production until benchmarks demonstrate:

- Stable voxel edits and meshing on target Macs.
- Server-authoritative dynamic grids.
- Grid split and rejoin behavior.
- Static/dynamic transition.
- Snapshot and replay equality.
- Cell wake-up under attack.
- No duplicate ownership during handoff.
- Inventory conservation across crashes and retries.
