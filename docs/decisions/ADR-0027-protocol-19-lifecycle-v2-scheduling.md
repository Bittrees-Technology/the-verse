# ADR-0027: Protocol-19 lifecycle-v2 scheduling

**Status:** Accepted

## Context

The signed protocol-19 active head selects one immutable prepared universe,
and directory v3 can issue, replace, and release cell authority. World-21 also
has an event-17 production transaction. None of those records alone proves
that an assigned worker was durably requested, that its trusted production
cursor agrees with the canonical world frontier, or that a crash between the
directory and cell stores cannot duplicate work.

The migration lifecycle file is immutable staging evidence. Rewriting it would
change receipt-bound material and permit an activated runtime to reseal its own
genesis. Runtime lifecycle state therefore needs a separate durable history.

## Decision

Each activated world-21 cell owns a lifecycle-v2 append-only journal and an
atomically replaced, hash-sealed head. The first runtime record is anchored to
the immutable lifecycle genesis hash and the signed universe activation head.
Every successor binds its predecessor, exact protocol-19 compatibility tuple,
universe and manifest, cell identity, current directory revision and document
hash, assignment generation and fence, holder, trusted-time frontier, world
event and state frontiers, production-clock generation, acknowledged sequence,
last committed schedule time, celestial-registry identity, and optional exact
next occurrence.

The activation-lock-owned universe lifecycle head is the write-ahead commit
point for those per-cell journals. It binds the signed active head, prepared
cell set, every immutable lifecycle genesis, and the exact committed child
head for every initialized cell. A prospective child successor is persisted
there before materialization in the cell. Restart may materialize only that
exact pending record; a missing, rolled-back, or unauthorized child history
fails closed. Existing active worlds may bootstrap this head only when every
child runtime is absent and all directory and world frontiers still equal the
signed migration genesis.

Directory v3 remains the only authority issuer. A lifecycle request persists a
stable operation and holder before the directory compare-and-swap. Claim and
recovery then commit the directory successor before finalizing an assigned
lifecycle record. Restart accepts only the exact prior assignment or the exact
successor named by that request. It never guesses a holder or performs another
recovery merely because lifecycle finalization lagged.

Release first records an assigned draining request. The directory then changes
that exact generation and holder to Sleeping. Only after the directory proves
the sleeping tip may lifecycle persist Sleeping. A crash at any boundary is
completed from the exact request and directory history. A nonterminal transfer
continues to pin release.

The lifecycle coordinator uses injected trusted time. Time never comes from a
client, frame count, process uptime, or dispatch arrival. A runnable production
frontier arms one exact occurrence 1,000 milliseconds after the accepted
boundary. While work remains continuously runnable, the next schedule time is
the committed schedule time plus exactly 1,000 milliseconds. An idle or paused
frontier has no occurrence and is not polled once per second.

Background dispatch starts at most 60 sequential occurrences and starts no
new quantum after 250 milliseconds of coordinator work, with at most 256
queue-bearing machines and one
unacknowledged occurrence. Each due occurrence is recorded as a pending world
commit, appended and synced through the existing event-17 journal, then
acknowledged in lifecycle history. After append-before-ack failure, the event
journal decides whether the exact occurrence committed; it is acknowledged
without applying production again. Conflicting, skipped, substituted, or
future occurrence material fails closed.

Production lifecycle generation is the world production-clock generation. It
does not change for directory claims, worker replacement, wake, background,
drain, or sleep. Directory assignment generation and fencing token remain a
separate authority domain.

The activated-world capability owns the directory and all ordered cell writer
locks for this checkpoint. Every selected cell is parsed and replayed in a
read-only preflight, together with the universe write-ahead head, before any
cell recovery write. Runtime file validation permits only the
exact prepared set or the complete named lifecycle runtime set. Unknown,
non-regular, oversized, noncanonical, forked, or resealed artifacts fail
closed.

This milestone permits production-only Background execution. It does not
admit a gameplay session, expose a raw event append, construct Active mode, run
physics or life systems in the background, or make a multi-host availability
claim. Ordinary event-17 gameplay, projection, verification, and client
cutover remain the next gate.

## Consequences

- The immutable migration receipt and prepared cell evidence never change.
- Directory authority and lifecycle intent converge after a crash without a
  second gameplay mutation or an invented successor.
- A stale holder cannot schedule, append, acknowledge, release, or report a
  healthy assigned lifecycle.
- Snapshot creation may lag the journal; snapshot plus event replay remains the
  canonical world recovery path.
- Another cell's directory transition does not invalidate a historical
  lifecycle record; new writes still require an exact current-tip capability.
- The coordinator is a bounded local correctness proof, not a distributed
  scheduler or public-scale capacity result.

## Required validation

- Canonical codec, state-machine replay, and universe write-ahead coverage must
  reject unknown fields, truncation, forks, uncommitted child successors,
  child-head rollback, wrong roots, wrong genesis, and wrong
  activation head. The current checkpoint covers incomplete sets, an
  unterminated tail, one complete append-before-head successor, bounded stale
  head temporaries, and strict active-head/genesis recovery.
- Claim and release tests must cover a directory commit before lifecycle
  finalization. Exact retry may advance no extra assignment generation or
  fencing token. Stale logical authority, backward trusted time, and a raw
  uncoordinated directory successor must fail closed.
- Production event-17 tests must prove exact occurrence application,
  redelivery rejection, conservation, replay, and absence of unrelated state
  change. The coordinator's append-before-ack crash test becomes a release gate
  as soon as the ordinary event-17 gameplay slice can create a runnable queue
  through the activated Store; migration itself intentionally imports only a
  quiescent frontier.
- Forward time must be processed sequentially within the fixed budgets. Empty
  and paused migrated cells remain Sleeping with no occurrence and no polling.
- Existing migration, activation, directory, event-17 replay, native, package,
  and conservation gates remain green.

## Non-goals

This decision does not add ordinary client event-17 payloads, Active gameplay
admission, projection schema 5, interest schema 3, browser management, remote
cell scheduling, distributed leases, frontier materialization, markets, or
blockchain settlement.

A coherent rollback of the universe head together with every implicated cell,
directory, and world artifact cannot be detected by local storage alone. The
universe lifecycle revision and head hash are therefore explicit future
checkpoint inputs for signed, remote, WORM, or chain anchoring; this milestone
does not claim protection from an attacker who can rewrite the complete local
durability domain consistently, including resealing the authoritative universe
head and every implicated child record.
