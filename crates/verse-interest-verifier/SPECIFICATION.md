<!-- SPDX-License-Identifier: Apache-2.0 -->

# The Verse interest-view hash and verifier specification, version 1

This document is the normative portable contract for gameplay protocol 16,
projection schema 3, interest schema 1, and the `interest-view/v1` domain. It
describes values, not one implementation. An encoding change requires a new
domain separator and coordinated schema negotiation.

## Input boundary

The verifier consumes the original UTF-8 JSON text bytes. It rejects invalid
UTF-8; duplicate object keys; unknown fields; invalid JSON types; integers
outside their declared Rust protocol ranges; non-finite or out-of-range
floating-point values; and configured resource-bound violations. Clients must
not pass the frame through a generic JavaScript or Godot number representation
first.

Equivalent JSON whitespace, object-key order, legal number spelling, and legal
string escaping do not affect the digest. Strings are not Unicode-normalized.

## Registry and universe-manifest commitments

The verifier independently validates the protocol-16 registry message before
establishing its immutable connection binding. Registry schema 1 is lowercase
hexadecimal BLAKE3 over:

```text
UTF8("the-verse/celestial-registry/v1\0") || canonical_integer_json(material)
```

`material` contains exactly the registry fields `schema_version`, `license`,
`universe_id`, `generation_rule_version`,
`minimum_fixed_body_surface_gap_um`, and `bodies`. It does not contain
`registry_hash`. The bodies retain their wire order and contain every field of
`CelestialBodySnapshot`; optional fields with no value are omitted according to
the protocol serializer. Schema 1 requires the registry license identifier
`CC-BY-SA-4.0` exactly.

Universe-manifest schema 2 is lowercase hexadecimal BLAKE3 over:

```text
UTF8("the-verse/universe-manifest/v2\0") || canonical_integer_json(material)
```

`material` contains exactly `schema_version`, `universe_id`, `world_seed`,
`address_schema_version`, `sector_edge_um`, `cell_edge_um`,
`cells_per_sector_axis`, `generation_rule_version`,
`frontier_policy_version`, `celestial_registry_schema_version`,
`celestial_registry_hash`, `content_schema_version`,
`content_manifest_version`, `content_hash`, `world_schema_version`, and
`event_schema_version`. It does not contain `manifest_hash`.

`canonical_integer_json` uses the same compact UTF-8 object-key and string
rules as the interest-view encoding below. All values in these two materials
are integers, strings, objects, or arrays; floating-point values and the
`fixed_1e6` transformation are forbidden. Every registry, manifest, and
content commitment must be exactly 64 lowercase hexadecimal characters.

Before a connection can install this message, its configuration pins the
expected `universe_id`, `content_hash`, `celestial_registry_hash`, and
`universe_manifest_hash`. A frame that is internally self-consistent but has
different roots fails with `binding_mismatch` before body validation. The SDK
does not carry the AGPL content catalog. For definition and generation IDs,
the exact pinned content, registry, and manifest roots are therefore the
allowlist authority; the verifier additionally requires every schema-1 ID to
be a nonempty schema-bounded ASCII identifier and enforces the structural body
kind matrix.

The manifest dimensions are independently checked before any address is
retained: all dimensions are positive, `cell_edge_um` is even and fits the
signed local range, and its checked product with `cells_per_sector_axis`
equals `sector_edge_um`. Every `UniverseAddress` must name the bound universe,
use canonical signed-i128 decimal sector strings (`0` or an optional `-`
followed by a nonzero digit and decimal digits), place each cell index below
`cells_per_sector_axis`, and place each local component in
`[-cell_edge_um / 2, cell_edge_um / 2)`. Thus `-0`, `+0`, leading zeroes, and
signed-i128 overflow are rejected rather than normalized by the verifier.

Registry bodies are in strict raw-UTF-8 `body_id` order and have unique
normalized centers. Generation and content fields bind the registry and
manifest. Parent references resolve before acceptance; moons require a planet
parent, planets forbid a parent, and missing, self, non-planet moon, or cyclic
ancestry fails closed. Surface and exclusion radii, atmosphere envelopes, and
oxygen fractions are bounded. Pairwise exclusion separation uses checked
integer address differences and squared distances; equality is accepted and
overflow or one micrometre below the configured boundary is rejected.
Schema-1 planets and moons cannot carry voxel-body definitions; asteroids must
carry both voxel-field and voxel-definition IDs; asteroid-field records cannot
carry either. Planet surface gravity must be positive. Definition IDs, visual
descriptors, generation seeds, and generation-rule bindings fail closed when
empty, malformed, or structurally incompatible with the body kind.
The sole separation exception is an asteroid whose `field_id` exactly resolves
to an `asteroid_field` body with that `body_id`. A field record's own
`field_id`, when present, equals its `body_id`. This member/field pair is
instead checked for exact containment:
`member_center_distance + member_exclusion_radius <= field_exclusion_radius`.
Every missing, aliased, wrong-kind, outside, or overflowing field relation
fails closed, and the member retains normal separation requirements from all
other bodies.

A single P1.5 registry frame contains at most 512 bodies and therefore at most
130,816 pairwise separation comparisons. Both limits are checked before any
quadratic work. A larger or effectively unbounded universe requires a future
versioned registry paging protocol; it is never represented by widening this
single synchronous frame.

The accepted dimensions and body identities remain part of the connection
binding. All later cell and local-origin headers, public player/grid/drop
payloads, actor-private player/drop payloads, and private motion addresses are
validated against them. Gravity, voxel, environment, nearest-body, and voxel-
chunk body references must resolve to the committed registry.

## Hash material

The digest is lowercase hexadecimal BLAKE3 over:

```text
UTF8("the-verse/interest-view/v1\0") || canonical_fixed_json(material)
```

`material` contains exactly these fields:

```text
projection_schema_version : u32
interest_schema_version   : u32
content_manifest_version  : string
universe_id               : string
cell_id                   : string
universe_manifest_hash    : string
celestial_registry_hash   : string
cell_address              : UniverseAddress
local_origin              : UniverseAddress
gravity_body_id           : string
voxel_body_id             : string
observer_class            : InterestObserverClass
session_epoch             : string
interest_epoch            : u64
baseline_id               : string
delta_sequence            : u64
entities                  : InterestEntityProjection[]
environment               : EnvironmentSnapshot
conservation_valid        : bool
actor_private             : ActorPrivateSnapshot | null
```

Object keys are emitted in lexicographically increasing UTF-8 byte order.
Arrays retain their schema-defined order. Output is compact UTF-8 JSON without
insignificant whitespace. JSON strings use `\"`, `\\`, `\b`, `\t`, `\n`, `\f`,
and `\r` where applicable, lowercase `\u00xx` for other ASCII controls, and raw
UTF-8 otherwise. Integers use shortest decimal notation without a leading plus,
leading zero, or negative zero.

Every value whose declared protocol type is `f32` or `f64` is replaced before
JSON serialization by:

```json
["fixed_1e6",scaled_i64]
```

The input is first parsed to the correctly rounded declared IEEE-754 type. It
is promoted to binary64 for scaling when declared `f32`. `scaled_i64` is
`round_ties_away_from_zero(value * 1_000_000)` and must be in `[-2^63, 2^63)`.
Both floating positive and negative zero produce integer zero.

Optional fields follow their protocol serializer rules. The material-level
`actor_private` key is always present and contains either its complete value or
JSON `null`. A locomotion `support` key is present with `null` when absent.
Fields marked `skip_serializing_if = Option::is_none` are omitted when absent.
Fields marked `serde(skip)`—including renderer-only convenience positions—are
never hash input.

The canonical event sequence, canonical tick, global world hash, fencing token,
frame kind, previous view hash, delta operations, and removal reasons are not
hash material. They remain protocol and frontier validation inputs.

## Canonical collections and identity

The complete `entities` array is ordered by raw UTF-8 `entity_id`, then by kind
ordinal `player < grid < voxel_chunk < death_drop`. IDs are compared by their
UTF-8 bytes without locale rules or Unicode normalization.

Each entry's kind must match its payload variant and payload identity:

- `player` -> `value.player_id`
- `grid` -> `value.grid_id`
- `voxel_chunk` -> `value.chunk_id`
- `death_drop` -> `value.drop_id`

Every entity has component schema version 3. A replacement must increase the
committed projected revision. A remove names an existing identity. An enter
names an absent identity; re-entry after a committed removal is a fresh enter.
Within each operation vector identities are canonically ordered. One delta may
mention an identity in at most one of enter, replacement, or removal.

Nested canonical collections retain these orders:

- grid blocks by `block_id` UTF-8 bytes;
- voxel-chunk voxels by integer `(x, y, z)`;
- actor-private inventories by `inventory_id`;
- actor-private death drops by `drop_id`;
- actor-private owned-grid masses by `grid_id`;
- actor-private production queues by `machine_block_id`;
- jobs inside one production queue remain FIFO and are not re-sorted.

The baseline's public `players`, `grids`, `voxel_chunks`, and `death_drops`
arrays must be the corresponding payloads of the exact complete `interest`
entity set in canonical order. Baseline `replaced` and `removed` are empty.

## Connection and binding validation

One verifier follows:

```text
await_welcome -> await_registry -> await_baseline -> current
```

The welcome must negotiate protocol 16, projection 3, interest 1, and the
compatible world/event/content/registry/manifest tuple. Its role must equal the
role configured by the client. The registry and universe manifest must bind the
same universe, schema tuple, content manifest, registry hash, and universe
manifest hash. The client-pinned universe ID, content root, registry root, and
manifest root must also match exactly. These bindings are immutable until
reset.

Typed intent receipts, intent rejections, and fatal messages may be staged and
sanitized for presentation in protocol-valid phases. They never change the
committed view and never produce an interest acknowledgement.

## Actor-private structural linkage

Actor-private collection ordering, address, and resource checks are necessary
but not sufficient. After a baseline entity set or complete resulting delta
entity set is reconstructed, the verifier validates the full private overlay
against that resulting public set. A retained private overlay is checked again
after public removals; prior validity cannot authorize a now-hidden grid,
block, machine, or drop.

For a bound player:

- the private player ID equals the immutable session actor, resolves to the
  visible public player, and has the same canonical address;
- exactly one private inventory has the player's `inventory_id` and a `player`
  domain naming that actor; every other `player` domain is invalid;
- every `cargo` inventory names a block identity that occurs exactly once in
  the public view, is a cargo block, and belongs to a visible actor-owned grid;
  one cargo block cannot back multiple private inventory records;
- every private death drop names the actor, agrees in ID and address with a
  visible public death drop, and names exactly one actor-owned `dropped`
  inventory; dropped inventories and private death drops are one-to-one;
- every private grid mass resolves to a visible grid owned by the actor;
- every production queue resolves to exactly one visible actor-owned refinery
  or assembler block; and
- queue jobs have unique IDs within the queue, name the actor and queue
  machine, use the recipe supported by that machine kind, and resolve both
  endpoint inventory IDs inside the same validated actor-private overlay.

A spectator has no private overlay. Missing, foreign, ambiguous, orphaned, or
cross-linked authority edges fail with `invalid_private_linkage` before hash
acceptance, presentation staging, state mutation, or acknowledgement.

Every state frame's outer header, interest header, and established binding must
agree wherever a value is repeated. The interest frame kind must match its
server message kind. A spectator has observer class `public_origin_spectator`
and no actor-private state or private motion. A player has observer class
`bound_player`, and any private player ID must equal the welcome role's player
ID. Protocol-15 `snapshot` and `motion_state` messages are rejected.

## Baseline transition

A baseline must have delta sequence zero, no previous view hash, a new nonempty
session epoch and baseline ID, complete entered entities, no replacements or
removals, complete environment and conservation state, and optional complete
actor-private state. A bound-player baseline requires actor-private state for
the exact welcomed player; a spectator baseline forbids it. The verifier
constructs the complete material, computes the digest, and compares it with
`interest.view_hash` using exact lowercase hexadecimal bytes.

The first baseline enters `current`. A later baseline is accepted only as a
bounded recovery replacement for the same connection, role, and immutable
registry/manifest binding. It retains the session epoch, advances the interest
epoch, uses a new baseline ID, and atomically replaces the complete prior view
only after verification and commit.

Successful verification creates a pending state only. Committed state changes
only when the pending stage token is committed.

## Delta transition

A delta must match the committed session epoch, interest epoch, baseline ID,
observer, universe/registry bindings, and cell. Its sequence is the committed
sequence plus one, and `previous_view_hash` exactly matches the committed
digest. Canonical event and simulation-tick frontiers cannot regress even
though they are not hash material. A changed `local_origin_address` is an
explicit absolute rebase: it becomes the resulting view's hash input and moves
no canonical entity.

The verifier stages operations over the complete committed entity map. Omitted
environment, conservation, and actor-private fields retain their committed
values. A complete `actor_private` replaces the prior private overlay. It is
invalid to carry both a complete private replacement and private motion.

Private motion requires an existing private player with the same identity and
replaces exactly these fields:

```text
address, orientation, linear_velocity, angular_velocity, surface_contact,
locomotion, movement_epoch, last_received_input_sequence,
last_processed_input_sequence, control_linear_input, control_angular_input,
boost, dampeners, jump, control_expires_at_simulation_tick, jetpack_enabled,
life_state, environment
```

All other private player fields retain their committed values. Renderer-only
`position` is absent from both inputs and hash material.

The resulting complete material uses the delta sequence and current staged
values. Its digest must exactly equal `interest.view_hash`. Success creates one
pending state without changing the committed state.

## Commit, acknowledgement, and failure

Only one pending stage is allowed. Its opaque local token is one-use and valid
only for that verifier generation. Discarding, resetting, or committing it
invalidates the token.

Commit atomically installs the pending verified state and, for a baseline or
delta, emits the exact compact JSON serialization of:

```text
ClientMessage::AcknowledgeInterest {
  session_epoch,
  interest_epoch,
  baseline_id,
  delta_sequence,
  view_hash
}
```

The acknowledgement values come only from committed verified state. Failed
parsing, validation, hashing, presentation staging, discard, reset, timeout, or
an invalid commit token emits no acknowledgement and leaves committed state
unchanged.
