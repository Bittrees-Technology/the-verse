# Data and event architecture

**Status:** Proposed service baseline; P1.5 bindings published and P1.6
lifecycle/event contract accepted

## Canonical principles

- Every economic mutation is attributable and idempotent.
- Every asset has one authoritative owner and location state.
- Derived read models can be rebuilt from durable events and snapshots.
- Destructive lifecycle events are explicit; disappearance is never an unexplained database deletion.
- Blockchain settlement is a projection of canonical events, not the primary gameplay database.
- Persistent spatial subjects use normalized canonical universe addresses;
  floating origins and session interest are derived state.
- Every opened world and appended event is bound to one universe-manifest and
  celestial-registry hash.

## Event envelope

Every canonical event includes:

```text
event_id
schema_name
schema_version
occurred_at
recorded_at
universe_id
cell_id or service_id
authority_fencing_token
actor_profile_id
actor_type: human | bot | npc | ai | system | admin
operation_id
causation_id
correlation_id
subject_ids
payload
content_manifest_version
content_manifest_hash
universe_manifest_schema_version
universe_manifest_hash
celestial_registry_schema_version
celestial_registry_hash
previous_subject_hash
event_hash
```

Administrative events additionally contain authority scope, reason code, and approving identity.

Event schema `14` requires the P1.5 universe and registry bindings. A mismatch
fails before an event is prepared, appended, replayed, or projected. Registry
and content definitions are addressed by immutable IDs and hashes rather than
being copied as mutable floating-point constants into each event.

Event schema `15` adds the P1.6 `ProductionQuantumCommitted` system event. Its
stable occurrence key is `(universe_id, cell_id, lifecycle_generation,
production_quantum_sequence)`. The event carries one exact one-second quantum
and every queue-head outcome in grid-ID/block-ID order. Live preparation and
replay independently recompute the complete vector before atomic mutation.
Event identity and canonical occurrence time derive from the occurrence; the
authority fence and hash chain bind the holder that durably committed it.

## Canonical spatial identity

Universe manifest schema `2` defines the universe, address dimensions,
generation policy, content binding, and celestial registry schema `1` hash.
`UniverseAddressV1` normalizes universe, signed 128-bit sector coordinates,
bounded cell indexes, and signed integer-micrometre local components. JSON uses
canonical decimal strings for signed 128-bit values.

Celestial entries are immutable and sorted by body ID. Supported body kinds are
planet, moon, asteroid, and bounded asteroid field. A moon requires an existing
planet parent; missing, self, non-planet, and cyclic ancestry is invalid. Fixed
body centres and gameplay orientations never change through ordinary events.
Voxel edits reference a body and body-local chunk; they do not mutate registry
identity.

Registry schema `1` and universe manifest schema `2` use domain-separated
BLAKE3 hashes over canonical floating-point-free bytes. Content schema `11` and
manifest `p1.5.0` pin the proof registry, its `3,000 m` minimum fixed-body
surface gap, and interest policy.

## Asset identity

Canonical objects use immutable IDs independent of database location.

Asset classes:

- Voxel deposit reference.
- Ore stack.
- Refined commodity stack.
- Component stack.
- Block instance.
- Grid.
- Blueprint.
- Cosmetic or avatar asset.
- Market receipt.
- Dropped inventory container.
- Salvage claim.
- Contract position.

Fungible stacks may split and merge. Split and merge events preserve quantity and lineage.

## Conservation invariant

For each conserved asset schema and operation:

```text
sum(inputs) + authorized_source
= sum(outputs) + defined_loss + authorized_sink
```

Authorized sources and sinks are versioned protocol definitions. Creative-mode creation is placed in a separate non-economic namespace and is never an authorized canonical source.

## Ownership and location states

A canonical asset has exactly one state:

- In player inventory.
- In company inventory.
- In a world container.
- Installed in a grid.
- Dropped at a location.
- In transit.
- Locked for transfer.
- In market custody.
- Tokenized in escrow.
- Consumed.
- Destroyed.
- Cleaned up.

Terminal states retain tombstone events and provenance.

## Storage model

### Simulation state

- Append-only local event journal.
- Periodic content-addressed snapshots.
- Universe-manifest and celestial-registry hash binding.
- Voxel seeds plus sparse chunk deltas.
- Grid snapshots plus post-snapshot events.

### Canonical services

- PostgreSQL transaction stores.
- Outbox pattern for publishing committed events.
- Inbox/idempotency tables for consumers.
- Double-entry ledgers for BIT-denominated obligations and custody.

### Read models

- Search index.
- Public market index.
- Inventory view.
- Universe map.
- Provenance graph.
- Economic dashboard.
- Blockchain reconciliation view.

Read models are never write authorities.

The universe map is a read model over immutable registry entries. Live spatial
interest baselines, deltas, membership, epochs, acknowledgements, and view
hashes are disposable per-session projections and are never written to the
canonical journal or economic stores.

## Event ordering

Global total ordering is neither required nor scalable. Ordering guarantees are:

- Strict monotonic order per authoritative aggregate.
- Tick order within a cell.
- Prepare/commit order for transfers.
- Chain block/log order per network.
- Explicit causal links across services.

Conflicts are rejected using aggregate versions and fencing tokens.

## Snapshot and recovery

A worker recovery process:

1. Performs an optional bounded read-only preflight without publishing health.
2. Acquires exclusive mutation authority and a checked, strictly newer fencing
   token for the exact universe and cell.
3. Under that lease, loads and validates the latest snapshot, lifecycle record,
   schemas, roots and hashes.
4. Replays later events, requiring positive nondecreasing historical fences,
   exact occurrence order and per-event binding equality.
5. Proves the live token exceeds every recovered historical token and rebuilds
   derived physics structures from normalized cell-local addresses.
6. Reconciles at-least-once schedule acknowledgement against the canonical
   committed-occurrence frontier.
7. Confirms aggregate hashes, renews the lease and only then publishes Active
   or Background health and resumes writes.

Every append requires the event fence to equal the current live store fence;
every snapshot requires the same equality. Holder, expiry, root, token or
renewal uncertainty fails before mutation. A stale worker cannot append after a
successor acquires a higher token. Token exhaustion is fatal rather than
saturating or wrapping.

Cleanup timers and long travel use durable scheduled events and never depend on one process's wall clock.

P1.6 production uses an injected trusted clock, a durable next occurrence and
a canonical committed frontier. Dispatch is at least once: append and sync
precede acknowledgement, and redelivery of a committed key reconciles without
another gameplay mutation. Forward clock jumps create exact sequential backlog
processed under finite budgets; backward discontinuity cannot reverse or repeat
the cursor. Paused or empty production does not schedule a one-second busy poll.

Session reconnect never restores replication state from persistence. Protocol
`16` creates new session and interest epochs and one fresh projection schema
`3` baseline. The baseline/delta stream carries the canonical event/tick
frontier and global canonical commitment alongside a view hash over only the
audience-authorized subset.

## P1.5 compatibility and migration

The P1.5 set is protocol `16`, projection schema `3`, world schema `18`, event
schema `14`, content schema `11`, content manifest `p1.5.0`, celestial registry
schema `1`, universe manifest schema `2`, and interest schema `1`. Partial
combinations are rejected.

The local P1.4 proof is archived and reset because its snapshots and events do
not bind a registry. A future persistent migration runs offline, normalizes
every address, validates all parent and separation rules, produces a receipt
with old/new hashes, proves replay equality, and atomically changes the active
manifest pointer. Rollback restores matching prior binaries, manifests, and
read-only data; an older executable never interprets a newer world or journal.

## P1.6 compatibility and migration

P1.6 admits protocol `17`, projection schema `3`, world schema `19`, event
schema `15`, content schema `11`, content manifest `p1.5.0`, registry schema
`1`, universe manifest schema `3`, interest schema `1`, operation fingerprint
schema `1`, lifecycle-control schema `1`, and schedule-occurrence schema `1` as
one coordinated set. Partial combinations fail closed.

The first proof archives and resets P1.5 data. A future offline migration must
declare its trusted-time cut-off, introduce one unambiguous production-clock
generation and frontier, prove replay equality, and atomically switch the
manifest pointer. Rollback restores the exact P1.5 binary, roots and archived
data; it never reinterprets a P1.6 record.

## Settlement batches

Eligible events are normalized into leaves:

```text
leaf = hash(
  settlement_schema_version,
  universe_id,
  event_id,
  event_type,
  subject_id,
  quantity_or_state_hash,
  actor,
  occurred_at
)
```

Leaves form a Merkle tree. The root, range, schema, and retrievable content manifest are posted to an approved settlement contract. A proof API returns the event, leaf, sibling path, root transaction, and confirmation state.

## Data retention

- Ownership, market, governance, creative-admin, and settlement history: permanent.
- Canonical transformation lineage: permanent or content-addressed archival.
- Raw high-frequency physics telemetry: sampled and time-limited.
- Chat and personal information: separate retention policy.
- Security logs: protected access and defined retention.
