# ADR-0017: Bounded conflict-safe operation idempotency

**Status:** Accepted for P1.3 implementation

## Context

The P1.2 simulation stores every accepted receipt forever under
`(actor_player_id, operation_id)`. This makes an exact retry safe, but it has
three production-blocking weaknesses:

- reusing an operation ID with a different payload returns the old receipt
  without identifying the conflict;
- the receipt map grows without a bound for the lifetime of a universe; and
- a newly installed or reconnected client has no authoritative mutation
  frontier from which to allocate identifiers safely.

Inventory conservation limits the damage of many duplicate attempts, but it
does not make ambiguous replay identity acceptable. Marketplace custody and
later cross-cell handoff require a stronger primitive before they can rely on
gameplay receipts.

## Decision

Protocol 14 assigns every mutating client message both:

- `operation_sequence`: a positive, contiguous counter scoped to the bound
  actor; and
- `operation_id`: a caller-readable diagnostic identifier that is not the
  canonical ordering key.

The server derives the actor from the session as before. It computes a typed
BLAKE3 intent fingerprint over a versioned domain containing the protocol,
fingerprint schema, universe, cell, actor, and complete decoded client
message. The operation ID and sequence are therefore covered by the
fingerprint along with every payload field.

World schema 16 stores one `ActorOperationHistory` per canonical player:

```text
committed_through
compacted_through
compacted_history_hash
retained[operation_sequence] = {
  operation_id,
  intent_fingerprint,
  receipt
}
```

Event schema 12 stores the actor, operation sequence, operation ID, and intent
fingerprint on every human event. System events carry none of those fields.
Content schema 9 and manifest `p1.1.0` remain unchanged because the resource,
recipe, reward, physics, and survival rules do not change.

## Acceptance algorithm

For actor history with frontier `N`:

1. Sequence zero is invalid.
2. A retained sequence with the same fingerprint returns its original durable
   receipt without appending, mutating, or publishing another update.
3. A retained sequence with another fingerprint is an operation conflict.
4. A sequence at or below the compacted frontier is reported as already
   committed but no longer replayable; it is never executed again.
5. A sequence at or below `N` that is neither retained nor compacted is
   malformed canonical history and fails closed.
6. A sequence above `N + 1` is a gap and does not mutate state.
7. Exactly `N + 1` may proceed through normal authorization and preparation.
   A rejected gameplay request does not consume the sequence. Only a durably
   appended accepted event advances the frontier.

The initial clients serialize mutations per actor. A future protocol may add
an explicit reservation window, but accepting gaps or concurrent speculative
commits is outside P1.3.

## Bounded retention and commitment

Each actor retains at most 128 recent processed operations, at most 131,072
serialized bytes across the retained history, and at most 4,096 serialized
bytes for one retained record. Crossing any bound compacts the oldest complete
record in sequence order.

Compaction advances `compacted_through` and folds a versioned canonical record
into `compacted_history_hash`:

```text
BLAKE3(domain || prior_hash || sequence || fingerprint || canonical_receipt)
```

Compaction never removes a noncontiguous record and never changes
`committed_through`. The rolling hash is a deterministic audit commitment, not
a proof that can reconstruct an evicted receipt.

Snapshots contain the bounded history and commitment. Journal replay applies
human events in actor-sequence order and deterministically reaches the same
bounded result. Existing snapshot sequencing allows replay to skip journal
events already represented by the snapshot. Physical journal segment archival
is separate operational work; this decision bounds live lookup and snapshot
state, not the current append-only audit log on disk.

## Private synchronization

The actor-private full projection exposes the actor's committed operation
frontier. It does not expose retained fingerprints, receipts, operation IDs,
compaction hashes, or another actor's frontier. Accepted receipts and rejected
messages echo the submitted sequence only to the requesting connection.

After a full snapshot, a client allocates `committed_through + 1`. It keeps the
complete in-flight payload until the exact receipt arrives. On connection loss
it requests a new full projection before deciding whether to retry. Conflict,
gap, and already-compacted responses force resynchronization rather than an
automatic payload rewrite.

## Failure and recovery

The event append remains the commit point. Failure before durable append leaves
the frontier reusable. Failure after synchronization but before the caller
observes a receipt recovers the event and frontier from the journal; an exact
retry returns the retained receipt or, after sufficient later activity, an
already-committed compacted response. Neither path repeats the mutation.

Replay validates contiguous actor sequences, fingerprints, receipt identity,
retention bounds, and the compaction commitment before accepting recovered
state. Invalid snapshots or replays fail closed.

## Consequences

- Exact retries are safe and payload substitution becomes visible.
- Actor receipt memory and snapshot size have deterministic limits.
- Operation ID strings remain useful in logs without carrying ordering
  authority.
- Clients must migrate every mutating message and maintain a serialized
  per-actor queue.
- Old protocol clients and world/event schemas are rejected explicitly.
- A compacted retry cannot receive its historical success details; the client
  must reconcile from authoritative state.

## Rejected alternatives

- **Keep an unbounded operation-ID map.** It leaks memory and snapshot size for
  every active actor.
- **Use operation ID alone with a payload hash.** It detects substitution but
  still needs unbounded negative knowledge to prove an old ID was committed.
- **Use a global sequence.** Independent actors would contend on one client
  frontier and leak ordering authority across sessions.
- **Consume rejected sequences.** Network and validation errors would create
  needless holes and make safe correction harder.
- **Use a probabilistic filter for compacted IDs.** False positives could
  discard legitimate economic work.
