# ADR-0020: Server-derived spatial interest replication

**Status:** Implemented and hosted verified for P1.5; production-scale
active-player evidence required

## Context

The P1.4 worker retains one complete cell snapshot and one absolute motion
snapshot. That bounded latest-state feed fixes backlog amplification, but every
session still receives the complete cell. Bandwidth, serialization work, and
private topology therefore grow with every entity in the cell rather than with
what one session is allowed to perceive.

ADR-0015 established immutable session-to-actor binding, authoritative input,
and deterministic multiplayer state. ADR-0016 established an audience-specific
private overlay. Neither decision defines a spatial view, delta convergence,
or a safe recovery contract. P1.5 adds those contracts without claiming that
one process can simulate thousands of full-rate players.

## Decision

### Interest schema 1

Interest state is a derived, noncanonical network view. The authoritative
world, journal, collision scene, intent validation, and economic rules never
depend on whether a client loaded an entity.

For a player session, the server derives the anchor from the immutable bound
actor and evaluates four ordered priority classes:

1. **Control critical:** the actor, controlled entity, support, contact,
   docking, constraint, pending accepted interaction, and private
   reconciliation needed to interpret the actor's state.
2. **Near physical:** public players, grids, drops, and body-local voxel chunks
   in configured spatial bands around the anchor.
3. **Selected context:** an already visible selected entity retained through a
   bounded margin so its removal is explicit.
4. **Celestial context:** public fixed-body and field summaries appropriate to
   the registry or current sector without full local physics detail.

A spectator has no actor. Its anchor and maximum bands come from a separately
authorized server-side spectator grant. Browser query parameters, camera
coordinates, entity IDs, client names, cookies, origins, and payload fields do
not create or widen a grant. Public celestial registry metadata is a separate
bounded read and does not entitle a session to dynamic entities near a body.

Content schema `11` and manifest `p1.5.0` pin an integer-metre enter radius,
larger exit radius, update cadence, and entity-class budget for each interest
band. Exact squared distances use normalized canonical addresses and integer
arithmetic. Entity IDs break ties. Membership is evaluated in canonical ID
order at committed tick boundaries.

An entity enters at or inside its class's enter radius. Once present, it stays
until strictly outside the exit radius for the configured consecutive tick
count. Mandatory dependencies ignore ordinary spatial exit until the canonical
relationship ends. This hysteresis prevents boundary jitter and is
deterministic from prior membership, authoritative state, session role, and
the pinned policy. It contains no client clock, frame rate, camera direction,
latency estimate, or hash-map iteration.

### Session and interest epochs

Each completed handshake creates an opaque `session_epoch`. It never enters
canonical state and cannot be resumed by presenting an old packet. Reconnect
creates a new session epoch and baseline.

Within the session, `interest_epoch` increments whenever the authorized
anchor, role, policy version, registry binding, or discontinuous view boundary
changes. Ordinary hysteresis entry and exit remain within the same interest
epoch. An epoch change invalidates all older baselines and deltas. Movement
epochs and actor operation sequences remain independent canonical authority
frontiers; a replication epoch cannot reset or grant either one.

### Baselines, deltas, and view hashes

Projection schema `3` and interest schema `1` define two state messages:

```text
InterestBaseline {
  projection_schema_version
  interest_schema_version
  session_epoch
  interest_epoch
  baseline_id
  observer_class
  cell_address
  local_origin_address
  registry_hash
  universe_manifest_hash
  canonical_event_sequence
  canonical_tick
  canonical_world_hash
  ordered_complete_visible_view
  view_hash
}

InterestDelta {
  session_epoch
  interest_epoch
  baseline_id
  delta_sequence
  local_origin_address
  canonical_event_sequence
  canonical_tick
  canonical_world_hash
  previous_view_hash
  ordered_enters
  ordered_component_replacements
  ordered_removals
  result_view_hash
}
```

A baseline is complete for that session's currently authorized view, not for
the cell or universe. Delta sequence begins at `1` and is contiguous per
baseline. Every entity projection carries an immutable entity ID, kind,
monotonic projected revision, and versioned components. An enter carries the
complete allowed entity projection. Re-entry after removal also carries a new
complete projection and never reuses cached components. Canonical entity IDs
are not reassigned to different subjects. Component replacement is absolute
for the named versioned component; it is not an arithmetic patch whose result
depends on hidden intermediate packets. Collections and fields use canonical
schema order.

`local_origin_address` is a derived normalized address used to decode bounded
local transforms. Changing it is an explicit absolute rebase inside the delta;
it moves no canonical entity and grants no new interest. A rebase whose
resulting view cannot be expressed from the acknowledged baseline uses a new
interest epoch and baseline.

A removal names only an entity previously visible to that session and carries
exactly one bounded reason:

```text
InterestRemovalReason = out_of_interest | destroyed | transferred
```

`out_of_interest` is not destruction. `destroyed` and `transferred` reflect a
canonical transition already authorized for public projection. A removal
contains no destination cell, hidden owner, attacker, inventory, cause,
coordinates, or other metadata.

The view hash commits only to the complete resulting projected view and its
schema, universe/registry binding, session epoch, interest epoch, and baseline
or delta frontier:

```text
BLAKE3("the-verse/interest-view/v1\0" || canonical_projected_view_bytes)
```

Projection schema `3` defines canonical integer or fixed-point wire encodings
for every hashed spatial and physical value. Clients retain those exact wire
values for hashing and convert separate copies to rendering floats. Non-finite,
noncanonical, or out-of-range values fail projection; renderer serialization
is never view-hash input.

Hidden entities, other actors' private fields, authentication material, socket
metadata, the canonical event sequence, and the canonical whole-world hash are
absent from the projected-view hash input. The protocol carries the canonical
event/tick frontier and global canonical commitment alongside the view hash so
existing authoritative reconciliation can identify the exact source state.
Subset clients use `view_hash` to converge their permitted representation; they
must not treat the global commitment as a description of hidden entities or
attempt to infer hidden state from it. The global frontier, commitment, packet
timing, and canonical tick reveal that out-of-view activity may have occurred,
so P1.5 preserves field confidentiality but does not claim traffic-analysis or
zero-knowledge secrecy.

The client applies a delta only when the independent verifier confirms every
epoch, baseline ID, sequence, previous view hash, trusted root, and recomputed
resulting view hash. Otherwise it discards the staged result and requests a new
baseline or closes the stream according to the bounded error class. Only a
successful presentation commit releases the verifier's exact acknowledgement.
It acknowledges the resulting hash, never a claimed gameplay outcome. The
server never trusts a client hash as authority.

### Authority and privacy

- Only the server computes membership and audience projection.
- Interest cannot authorize targeting, mining, combat, inventory access,
  market access, construction, or control. Every intent reconstructs and
  validates against canonical state.
- Actor-private inventory, production, control, oxygen, drops, operation
  history, and exact owned mass remain visible only to the bound actor, even
  when another player can see the public entity.
- The bound actor's carried inventory and control reconciliation are control
  critical. Remote owned cargo, exact grid mass, and production details are not
  an ownership-wide visual channel; their owning public grid or machine must
  also be inside the authorized active-cell view.
- An unseen entity produces no ID, removal, count, projected-view-hash
  contribution, or per-entity error. The separately carried global commitment
  remains the documented aggregate activity side channel. A newly visible
  entity receives a full public projection, not its hidden history.
- Projection failure closes the stream with a generic error. It never falls
  back to a canonical or another actor's snapshot.
- Shared caches may hold canonical dirty markers or wholly public registry
  data. Session baselines, deltas, view hashes, and overlays require a complete
  audience-and-epoch cache key and bounded lifetime.

### Backpressure and recovery

Receipts, fatal errors, and handshake control are bounded separately from state
replication. Per session, the worker retains one acknowledged baseline
frontier, one coalesced structural target, and the newest absolute motion
target. Superseded motion is discarded. A structural change, enter, or removal
must be represented before any later motion that assumes it.

The worker may recompute one cumulative delta from the acknowledged view to
the latest authorized view. It may not concatenate an unbounded packet history
or drop a removal. Byte, entity, serialization-time, and unacknowledged-age
limits are server configured and measured. When any bound is exceeded, an
epoch or hash mismatch occurs, or the client falls behind the retained
frontier, the worker discards pending state and sends one fresh current
baseline. It does not loop through historical baselines.

Backpressure can reduce motion frequency or representation detail only through
a versioned server policy. It cannot expand interest, omit authority-relevant
state, change canonical simulation, or turn hidden state into a client-side
guess. A connection that cannot accept the bounded recovery baseline is closed
rather than consuming unbounded memory.

### Compatibility boundary

P1.5 uses gameplay protocol `16`, projection schema `3`, world schema `18`,
event schema `14`, content schema `11`, content manifest `p1.5.0`, celestial
registry schema `1`, universe manifest schema `2`, and interest schema `1`.
Handshake compatibility includes every value before a baseline is sent.

Protocol `15` clients cannot negotiate an interest stream. The P1.4 complete
JSON transport may remain available only as an explicitly configured local
diagnostic endpoint with the same audience projection; it is not a public
fallback and cannot share a socket with protocol `16` state.

## Relationship to earlier decisions

ADR-0015 is accepted as the actor-binding and deterministic multiplayer-cell
foundation. This decision replaces only its placeholder for future
interest-managed deltas; it does not change input authority, admission,
disconnect, operation ordering, or the one-writer cell model.

This decision preserves ADR-0016's audience rules and its canonical commitment
for authoritative reconciliation. Projection schema `3` adds the view hash so
subset clients no longer misuse that global commitment as their projected-state
hash. The P1.4 latest-state feed remains valid evidence for bounded coalescing,
but it is not the P1.5 wire contract.

## Migration and rollback

Interest state is not persisted in world schema `18` or event schema `14`.
Upgrade drains protocol `15` sessions, deploys the coordinated schema set, and
requires every protocol `16` session to begin with a new baseline. No baseline,
delta, session epoch, or acknowledgement survives process restart.

Rollback drains protocol `16` sessions and restores the complete P1.4 service
and data set. It cannot reinterpret protocol `16` packets as protocol `15`.
Because interest is derived, rollback does not rewrite canonical events; the
world/registry migration rules in ADR-0019 still apply.

## Required evidence

- Golden membership tests cover exact enter and exit boundaries, hysteresis,
  negative sector coordinates, ties, dependency retention, and reconnect.
- Two players and one spectator receive disjoint private overlays and distinct
  spatial views without hidden IDs, counts, projected hashes, or tombstones,
  apart from the documented global canonical commitment side channel.
- Delay, duplication, reordering, loss, and stale epochs either converge to the
  expected view hash or request one bounded baseline.
- A structural enter/removal cannot be overtaken by later motion.
- Slow-consumer tests bound retained messages, bytes, serialization work, and
  baseline frequency under a published impairment profile.
- Intent tests prove an unloaded or forged entity ID cannot bypass canonical
  targeting or authority checks.
- Benchmarks publish membership, projection, encoding, and bandwidth
  distributions for `2`, `8`, `16`, `32`, and `64` active players plus
  synthetic nearby entities on the reference Mac and hosted Linux runner.
- Multi-cell and thousand-participant claims remain blocked until separate
  scheduler, handoff, and universe-level load evidence exists.

## Deliberate exclusions

P1.5 does not deliver a final binary codec, cross-cell subscriptions, handoff,
lag compensation, rollback simulation, occlusion culling, fog-of-war secrecy,
arbitrary remote cameras, peer-to-peer state, or thousands of players in one
physics process. It does not make hidden state the primary anti-cheat control.
Those systems must preserve server-derived interest, audience projection, and
baseline/hash convergence.
