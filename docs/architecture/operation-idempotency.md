# Operation idempotency and retry contract

**Status:** P1.3 implementation contract

## Client-visible rules

Every gameplay mutation belongs to exactly one authenticated actor and one
positive operation sequence. A client sends only the next authoritative
sequence and keeps the complete message unchanged until it receives a result.

| Situation | Server result | Client action |
| --- | --- | --- |
| Next sequence, valid intent | Durable receipt | Advance after authoritative acknowledgement |
| Next sequence, gameplay rejection | Typed rejection | Correct or abandon locally; sequence remains reusable |
| Retained exact retry | Original receipt | Treat as the same accepted operation |
| Retained changed message | `operation_conflict` | Stop queue and resynchronize |
| Sequence gap | `operation_sequence_gap` | Stop queue and resynchronize |
| Zero sequence | `operation_sequence_invalid` | Treat as a client defect |
| Compacted old sequence | `operation_already_committed` | Do not retry; resynchronize state |
| Missing/malformed server history | Generic fatal halt | Do not mutate until repaired |

An operation ID is diagnostic. Changing it does not create permission to reuse
an accepted sequence, and copying another player's sequence or ID has no effect
on that player's history.

## Native queue

The native client receives `committed_operation_sequence` only inside its
actor-private full snapshot. It then:

1. sets the next candidate to the committed frontier plus one;
2. permits at most one mutation awaiting a receipt;
3. stores the exact encoded payload and retries it unchanged after a bounded
   transport timeout;
4. pauses allocation across disconnect and reconnect;
5. compares the new private frontier with the pending sequence; and
6. retries, completes, or resynchronizes without guessing an outcome.

Character `input_sequence` remains separate. It orders fixed-step control
transitions inside an accepted character-control operation; it is not a
replacement for the economic operation sequence.

The P1.3 browser remains a spectator and creates no gameplay sequences.

## Server invariants

- Session binding supplies the actor; the payload cannot select it.
- Intent fingerprinting occurs before retained-receipt lookup.
- A gameplay rejection changes neither event sequence nor actor frontier.
- A durable human event advances exactly one actor frontier by exactly one.
- System physics/lifecycle events do not advance any client frontier.
- Receipt publication occurs only after event synchronization.
- Idempotent retry publishes neither world state nor another actor's receipt.
- Projection exposes only the bound actor's committed frontier.
- Retained history is ordered, contiguous, size bounded, and consistent with
  every receipt and the rolling compaction commitment.

## Verification matrix

Tests must cover:

- same actor, same sequence, byte-equivalent message;
- same actor, same sequence, changed operation ID or any payload field;
- two actors using the same sequence and operation ID independently;
- zero, gap, retained-old, compacted-old, and malformed-history paths;
- gameplay rejection followed by a corrected message at the same sequence;
- exact retained retry before and after restart;
- compacted retry before and after restart;
- append failures before write and after synchronization;
- snapshot-only, journal-only, and snapshot-plus-tail recovery;
- re-hashed event substitution, sequence reordering, and commitment tampering;
- deterministic 128-record/131,072-byte/4,096-byte bounds;
- private frontier isolation in HTTP, spectator, and other-player JSON;
- reconnect reconciliation with an in-flight native mutation; and
- long randomized campaigns that conserve inventory while histories compact.

## Deliberate limits

P1.3 does not provide production authentication, cross-cell transaction
coordination, market settlement nonces, or physical journal archival. Those
systems may build on this primitive only after defining their own commit and
reconciliation boundaries.
