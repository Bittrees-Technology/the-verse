# ADR-0019: Fixed celestial registry and canonical universe addresses

**Status:** Accepted for P1.5

## Context

The local universe currently derives one asteroid field and one planet from
constants inside the simulation. Those values make the proof playable, but
they are not a durable universe identity. A worker, client, map service, or
future cell scheduler can disagree about what a coordinate means without any
schema or hash mismatch.

F-003, F-014, WORLD-002, and WORLD-005 require a persistent hierarchy, fixed
and widely separated celestial bodies, and deterministic frontier expansion.
The first registry must establish those identities without implying that the
single-process P1 worker already supports cross-cell handoff or a production
universe with thousands of active players.

## Decision

### Canonical address version 1

Every persistent spatial subject uses one normalized `UniverseAddressV1`:

```text
UniverseAddressV1 {
  universe_id
  sector: { x, y, z }       signed 128-bit integer coordinates
  cell: { x, y, z }         unsigned indexes within the sector
  local_um: { x, y, z }     signed 64-bit micrometres from cell center
}
```

The universe manifest pins `sector_edge_um`, `cell_edge_um`, and the number of
cells per sector axis. Sector coordinates use canonical base-10 strings on
JSON surfaces and 16-byte two's-complement integers in a future binary codec.
Cell indexes are in `[0, cells_per_sector_axis)`. Each local component is in
the half-open interval `[-cell_edge_um / 2, cell_edge_um / 2)`.
Dimensions must be positive, `cell_edge_um` must be even and fit the local
integer range, and `sector_edge_um` must equal the checked product of cell edge
and cells per sector axis.

Normalization uses Euclidean floor division: local overflow carries into the
cell, cell overflow carries into the sector, and negative positions normalize
identically on every platform. Non-normalized, non-finite, rounded-alias, or
wrong-universe addresses fail before mutation. A region is a derived routing
or generation label over sectors; it is not an additional coordinate and does
not introduce another seed into object identity.

Persistence and cross-service messages use the exact address. An active
physics scene derives bounded cell-local `f64` coordinates and may rebase its
presentation origin, but neither a floating origin nor a client transform is
canonical. Grid- and voxel-local coordinates remain relative to their owning
canonical address.

### Universe manifest schema 2

One immutable, content-addressed universe manifest binds:

- universe ID and seed;
- address schema and dimensional constants;
- generation-rule version and frontier policy version;
- celestial registry schema version and hash;
- content schema version, manifest version, and content hash; and
- the allowed canonical simulation schema set.

The P1.5 local universe uses universe manifest schema `2`. Its hash is
`BLAKE3("the-verse/universe-manifest/v2\0" || canonical_bytes)`. Canonical
bytes use UTF-8, sorted object keys, no duplicate keys or insignificant
whitespace, exact integer tokens, schema-bounded ASCII identifiers, and no
floating-point numbers. A manifest's hash excludes signatures; signatures and
governance publication attest to the hash rather than changing it.

### Celestial registry schema 1

The registry is an ordered collection keyed by immutable `body_id`. Each entry
contains:

- body kind (`planet`, `moon`, `asteroid`, or bounded `asteroid_field`);
- optional `parent_body_id`, required for a moon and forbidden for a planet;
- public display name, optional field ID, and `proof` or `production` scale
  class;
- normalized center address and integer-micrometre exclusion radius;
- fixed gameplay orientation;
- geometry, voxel, material, atmosphere, gravity, and resource definition IDs;
- visual descriptor ID, whose missing client asset resolves to a labelled
  fallback rather than changing authority;
- generation seed and generation-rule version where procedural content is
  used; and
- content-manifest version and content hash.

Entries are sorted by canonical body ID. A moon's parent must resolve to a
planet in the same registry. Unknown parents, self-parenting, non-planet moon
parents, and every parent cycle fail closed before hashing. Duplicate IDs or
normalized centers, unknown definitions, invalid radii, and overlapping
exclusion volumes also fail closed. Registry schema `1` uses
`BLAKE3("the-verse/celestial-registry/v1\0" || canonical_bytes)`.

The universe manifest stores the registry hash. World snapshots and event
envelopes store both hashes, so a worker cannot open, replay, or append to a
world using different celestial or universe definitions.

### Fixed bodies and mutable terrain

P1.5 celestial centers and gameplay orientations do not translate, orbit, or
change after registry publication. Cosmetic client animation may not affect
gravity, atmosphere, collision, targeting, resource location, or addresses.
Planet and asteroid voxel edits are canonical world events relative to the
fixed body; edits never rewrite the registry entry or its hash.

Adding a frontier body appends a new, separately published registry version or
creates a new universe-manifest revision through an explicit migration. It
does not regenerate or move an existing body. Mined deposits do not silently
return when generation rules change.

### P1.5 local separation proof

Content schema `11` and manifest `p1.5.0` pin a local-proof minimum fixed-body
surface gap of `3,000 m`. Validation operates on normalized integer centres and
exclusion radii. It uses checked widened integer squared-distance comparison,
rejecting overflow, and proves without floating-point rounding that for every
pair:

```text
center_distance >= radius_a + radius_b + 3,000 m
```

The local planet's exclusion radius includes its surface, maximum terrain, and
gameplay atmosphere envelope. An asteroid field uses a bounding exclusion
radius. Equality is valid; one micrometre below the boundary is invalid.

This is a configurable proof-fixture collision and identity threshold, not the
product's final interplanetary spacing. P1.5 contains only one planet and
therefore cannot demonstrate planet-to-planet travel measured in real-world
days. The current visible moon is registered as a fixed child body rather than
remaining decorative geometry. Later production manifests must publish larger
planet-separation and travel evidence without moving bodies already opened
under this manifest.

### Compatibility boundary

The coordinated P1.5 boundary is:

| Interface | Version |
| --- | --- |
| Gameplay protocol | `16` |
| Projection schema | `3` |
| World schema | `18` |
| Event schema | `14` |
| Content schema | `11` |
| Content manifest | `p1.5.0` |
| Celestial registry | `1` |
| Universe manifest | `2` |
| Interest schema | `1` |

Protocol `16` exposes universe, registry, and interest compatibility during
handshake before any world state. Projection schema `3` carries normalized
addresses and registry references. World schema `18` binds the two manifest
hashes and canonical cell address. Event schema `14` binds the same hashes to
new events. No component may accept a partially upgraded combination.

## Migration and rollback

P1.4 local worlds and journals have no registry binding. The P1.5 proof
archives and resets them; it does not infer celestial identity from unversioned
floating-point constants. A future persistent-universe migration must be an
offline, deterministic copy that assigns normalized addresses, validates every
body and entity, writes a migration receipt containing old and new hashes, and
atomically switches the universe-manifest pointer only after replay equality.

Rollback restores the prior binaries, manifests, and prior read-only world as
one unit. A P1.4 binary must reject world schema `18`; changing only the
registry pointer or reopening P1.5 data with P1.4 rules is forbidden. Once a
public body identity is accepted, rollback cannot reuse that ID at another
address.

## Consequences

- Celestial identity is durable, inspectable, and bound to recovery.
- Floating-origin rendering can evolve without changing persistent addresses.
- Fixed bodies eliminate orbital handoff from the first scale slice.
- Every cell and public map can verify that it uses the same universe.
- Registry changes become explicit migrations rather than generator drift.
- Exact integer addressing adds normalization and overflow tests at every
  ingress boundary.

## Required evidence

- Golden vectors cover positive and negative normalization, sector carries,
  boundary aliases, JSON and binary integer encoding, and overflow rejection.
- Registry bytes and hashes match on macOS and Linux.
- Pairwise separation accepts equality and rejects one micrometre below it.
- Moon validation accepts only an existing planet parent and rejects missing,
  self, non-planet, and cyclic ancestry.
- Reordered entries, altered definitions, wrong content, or wrong universe
  hashes fail before snapshot load or journal append.
- Restart and replay preserve addresses, registry references, and world hash.
- Client origin rebasing does not alter an intent target or canonical address.
- Public registry reads expose no dynamic actor-private inventory or control
  state.

## Deliberate exclusions

P1.5 does not implement procedural frontier admission, multi-process cell
scheduling, cross-cell handoff, analytical interplanetary travel, orbital
mechanics, moving planets, seamless planet-scale terrain, or production
planet-to-planet distance. Those systems must preserve this address and
registry identity contract.
