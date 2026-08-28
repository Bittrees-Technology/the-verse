# Universe simulation

**Status:** P1.5 address/registry proof published; P1.6 one-cell lifecycle
verified; P1.7 two-cell handoff contract accepted

## Coordinate model

The Verse uses hierarchical coordinates rather than one global floating-point scene.

```text
Universe
└── sector coordinate (signed 128-bit)
    └── cell index within sector
        └── normalized local integer-micrometre position
            └── derived cell-local physics frame
                └── grid or voxel-local frame
```

The canonical persistent address is versioned by
[ADR-0019](../decisions/ADR-0019-fixed-celestial-registry.md):

```text
UniverseAddressV1 {
  universe_id
  sector: three signed 128-bit integers
  cell: three bounded indexes within the sector
  local_um: three signed 64-bit integers in the cell-centered half-open range
}
```

Universe manifest schema `2` pins the sector and cell dimensions. Network and
database JSON represents 128-bit values as canonical decimal strings; a future
binary codec uses 16-byte two's-complement values. Euclidean floor division
normalizes local overflow through cell and sector coordinates, including on
negative axes. An alternative representation of the same point is invalid.

Physics operates only in a bounded cell-local `f64` frame. Origin rebasing and
client floating origins are derived presentation state and cannot change a
canonical address or intent target. A generated region is only a routing label
over sectors; it is not another address component or identity seed.

The address space is finite mathematically but has no practical reachable boundary.

## Fixed celestial registry

Celestial registry schema `1` is an ordered, content-addressed set of immutable
body identities. Universe manifest schema `2` binds its hash together with the
universe seed, address dimensions, generation rules, frontier policy, and
content manifest. World schema `18` and event schema `14` bind both the
registry and universe-manifest hashes; recovery fails before replay when any
binding differs.

Each body records:

- Immutable body ID and kind.
- Optional parent body; moons require an existing planet parent, while missing,
  self, non-planet, and cyclic parentage fails validation.
- Public display name, visual descriptor, and proof or production scale class.
- Normalized center address and integer exclusion radius.
- Fixed gameplay orientation.
- Geometry, voxel, gravity, atmosphere, material, and resource definitions.
- Generation seed and rule version where procedural generation is used.
- Content-manifest version and hash.

P1.5 bodies do not orbit, translate, or change gameplay orientation. Cosmetic
motion cannot affect collision, gravity, atmosphere, resources, or targeting.
Voxel edits are canonical world events relative to the fixed body and do not
rewrite the registry.

Every official-client planet, moon, asteroid, or asteroid field that appears
physical resolves to a registry entry. The existing visible moon is registered
as Khepri's fixed child rather than removed or retained as untracked decorative
geometry. A missing visual asset produces a labelled proxy; it cannot hide an
authoritative collider or gravity source.

Content schema `11` and manifest `p1.5.0` require at least `3,000 m` between
the integer exclusion surfaces of every pair in the local proof. The planet
radius includes terrain and gameplay atmosphere; an asteroid field uses its
bounding radius. Equality passes and one micrometre below the boundary fails.
This proves nonoverlap for one local planet, its registered fixed moon, and the
origin asteroid field. It does not claim production planet-to-planet
separation or real-day travel.

A future generated registry remains deterministic from:

- Universe seed.
- Sector coordinate.
- Generation-rule version.
- Approved content manifest.
- Governance-controlled frontier policy.

Planets are admitted only when their active manifest's stronger production
separation rule passes. Asteroids may appear as belts, dense clusters, sparse
fields, or isolated bodies. Once published, later generator changes cannot
move or reuse a body identity.

## Frontier expansion

New sectors become canonical when:

1. A route, survey, or expansion action requests an unmaterialized region.
2. The generator produces a candidate registry extension.
3. Address normalization, minimum separation, and content rules validate it.
4. The registry and universe-manifest hashes are published and attested.
5. The sector becomes immutable apart from authorized voxel edits and explicit
   future content migrations.

Resource availability expands through frontier discovery rather than regenerating mined canonical deposits.

## Cell lifecycle

```text
Sleeping ── due production ──> Background ── no work ──> Sleeping
    │                              │
    └── authenticated gameplay ────┴──> Activating ──> Active
                                                        │
                                                idle/operator
                                                        │
                                                        v
                                                    Draining
                                                   /        \
                                  runnable work <─          ─> no work
                                      │                         │
                                      v                         v
                                 Background                  Sleeping
```

- **Sleeping:** no real-time process or busy poll; durable state is a verified
  snapshot, journal, lifecycle record, and optional next occurrence.
- **Activating:** a fenced holder recovers, reconciles and performs bounded
  production catch-up through one wake cut-off before admitting gameplay.
- **Background:** a short-lived fenced worker advances only due physical
  production through the same atomic quantum used while Active.
- **Active:** full physics, life support, damage, gameplay and client
  replication.
- **Draining:** no new entrants or intents; the selected atomic boundary,
  session invalidation and snapshot finish before mode change or lease release.

Lease loss or uncertainty immediately fences a worker; `Fenced` is a worker
result, not a state that stale authority may persist. In P1.6, only
authenticated gameplay ingress, an authorized operator request, or a durable
production occurrence wakes the fixed proof cell. Public spectators do not
wake it or keep it Active. Attacks, travel arrivals, other expiring timers,
cleanup and market-linked work remain later lifecycle triggers.

P1.5 implements only the active local cell. P1.6 is the bounded implemented slice
defined in [the durable single-cell lifecycle contract](../gameplay/durable-single-cell-lifecycle.md):
one fixed cell, one local coordinator, renewable single-host fencing, and
production-only background execution. It does not complete dynamic assignment,
multi-cell scheduling, handoff, distributed availability, or WORLD-008.

## Player and grid handoff

P1.7 binds each cell to a canonical normalized `CellKeyV1` and deterministic
cell ID. A durable local universe directory owns cell assignment generations
and mobile-aggregate placement generations; each cell independently retains
its P1.6 lease and fencing token. Both fences are required because valid leases
for two different cells cannot alone prevent both from claiming one grid.

Cross-cell movement follows this durable saga:

1. The source derives boundary crossing, freezes the complete isolated closure
   at an atomic tick/production boundary, and synchronizes a content-addressed
   package.
2. The destination validates that exact package into durable non-live
   quarantine.
3. The directory compare-and-swap validates both assignments, source prepare,
   destination receipt, package hash, and prior placement generation. It moves
   placement to the destination at generation `N+1`; this commit is the only
   authority-transfer point.
4. The destination imports idempotently, reconstructs physics and schedules,
   validates and snapshots.
5. The source finalizes an audit tombstone and can never unlock the old
   placement.
6. The gateway replaces all source movement/interest state with one
   transfer-linked, independently verified destination baseline before controls
   resume.

Before directory commit recovery may abort back to the exact source state.
After commit it is roll-forward only. Duplicate or reordered delivery cannot
create a second mutation, and the package conserves cargo, installed
components, production queues and escrow, ownership, actor history, physics,
and lineage. Unsupported anchored, externally constrained, boundary-spanning,
or oversized aggregates remain source-authoritative and report a specific
retryable condition.

The bounded proof and its exclusions are specified by
[F-061](../gameplay/durable-two-cell-handoff.md) and
[ADR-0023](../decisions/ADR-0023-durable-two-cell-handoff.md). It does not yet
solve general cross-cell physics, static structures, or partitioned capital
ships.

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

This route model is not implemented by P1.5. The local fixed-body registry must
not be presented as evidence of seamless interplanetary travel.

## Schema and migration boundary

P1.5 is one coordinated compatibility set: protocol `16`, projection schema
`3`, world schema `18`, event schema `14`, content schema `11`, content manifest
`p1.5.0`, celestial registry schema `1`, universe manifest schema `2`, and
interest schema `1`. Partial combinations fail closed before state delivery or
journal replay.

The local P1.4 proof is archived and reset because it has no registry binding.
A future persistent migration must normalize every address offline, validate
all bodies and subjects, record old and new hashes, and switch manifests only
after replay equality. Rollback restores the previous binary, manifests, and
read-only world together; it never opens world schema `18` under older rules.

P1.6 is a second coordinated boundary: protocol `17`, projection schema `3`,
world schema `19`, event schema `15`, content schema `11`, content manifest
`p1.5.0`, registry schema `1`, universe manifest schema `3`, interest schema
`1`, lifecycle-control schema `1`, and schedule-occurrence schema `1`.
Universe manifest `3` binds the lifecycle/schedule policy. The first proof
archives and resets P1.5 data; any later offline migration must introduce an
unambiguous occurrence frontier and prove replay equality.

P1.7 is a third coordinated boundary: protocol `18`, projection schema `4`,
world schema `20`, event schema `16`, content schema `11`, content manifest
`p1.5.0`, registry schema `1`, universe manifest schema `4`, interest schema
`2`, operation fingerprint schema `2`, lifecycle-control schema `1`,
production-occurrence schema `1`, cell-directory schema `1`, and transfer
schema `1`. Manifest `4` binds the cell-key, directory, placement, package,
projection, interest, and retry policies. The first proof archives and resets
P1.6 data; a later migration must create stable cell keys and placements,
issue universe-unique subject IDs, preserve retained operation conflicts, and
prove replay and cross-cell conservation equality.

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
- Canonical positive and negative address normalization vectors.
- Cross-platform registry and universe-manifest hash equality.
- Fixed-body separation at the exact accepted and rejected boundaries.
- Registry mismatch rejection before load, replay, or append.
- Atomic whole-cell production equivalence between Active and Background.
- Hard-crash occurrence reconciliation and stale-fence rejection.
- Bounded catch-up and fresh activation baselines for one fixed cell.
