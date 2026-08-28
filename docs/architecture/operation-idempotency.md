# Operation idempotency and retry contract

**Status:** P1.3 implementation contract; P1.7 cross-cell extension accepted

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

## P1.7 retry continuity across cells

Operation fingerprint schema `2` removes the current cell ID from the
fingerprint. It remains bound to the immutable universe and authenticated
actor, protocol and fingerprint schemas, positive operation sequence, and
exact canonical message bytes. Changing the route after an accepted operation
therefore cannot turn the same sequence and payload into a new mutation.

The player's retained receipt suffix, rolling compaction commitment, committed
frontier, pending input queue, and movement/input frontiers move inside the
content-addressed transfer package. The destination validates and imports them
before it admits actor intents. A lost source receipt immediately before
handoff returns the original receipt after import; changed material at that
sequence remains `operation_conflict`, including after compaction and restart.

The transfer saga is a system operation and consumes no client operation
sequence. Controls received under the retired source movement epoch reject
without advancing the destination frontier. A gateway or client may retry a
message but cannot rewrite its sequence, fingerprint material, destination, or
placement generation.

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
- lost receipt immediately before handoff, destination retry after import, and
  changed-payload conflict across source/destination restart;
- long randomized campaigns that conserve inventory while histories compact.

## Deliberate limits

P1.3 alone does not provide production authentication, cross-cell transaction
coordination, market settlement nonces, or physical journal archival. P1.7
adds only the bounded two-cell operation-history movement described above; the
directory and transfer package retain their own commit and reconciliation
boundaries.
