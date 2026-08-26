# Data and event architecture

**Status:** Proposed baseline

## Canonical principles

- Every economic mutation is attributable and idempotent.
- Every asset has one authoritative owner and location state.
- Derived read models can be rebuilt from durable events and snapshots.
- Destructive lifecycle events are explicit; disappearance is never an unexplained database deletion.
- Blockchain settlement is a projection of canonical events, not the primary gameplay database.

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
previous_subject_hash
event_hash
```

Administrative events additionally contain authority scope, reason code, and approving identity.

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

1. Loads the latest verified snapshot.
2. Validates its content hash and schema.
3. Replays events after the snapshot.
4. Rebuilds derived physics structures.
5. Confirms aggregate hashes.
6. Acquires a new fenced lease.
7. Resumes writes.

Cleanup timers and long travel use durable scheduled events and never depend on one process's wall clock.

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
