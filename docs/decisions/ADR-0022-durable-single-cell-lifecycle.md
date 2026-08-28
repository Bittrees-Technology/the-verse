# ADR-0022: Durable single-cell lifecycle and fenced background production

**Status:** Accepted

## Context

The P1.5 runtime owns one fixed active cell for the lifetime of one worker. It
holds a local writer lock, advances production from a process-local millisecond
accumulator, runs full physics while unpaused, and writes one production event
per machine. That implementation proves an active physical-industry loop but
does not prove sleeping, background execution, renewable ownership, or exact
offline progress.

The accepted architecture requires dynamically managed cell lifecycles,
durable scheduled events, exactly one writer, monotonically fenced authority,
finite execution budgets, and equivalent Active and Background production.
Moving directly to many cells would combine time, idempotency, lease, lifecycle,
handoff, replication, and placement risks without first proving their local
correctness boundary.

The complete behavior and evidence contract is the
[P1.6 durable single-cell lifecycle specification](../gameplay/durable-single-cell-lifecycle.md).

## Decision

### Bounded milestone

P1.6 proves one already generated fixed cell under one local coordinator. It is
a prerequisite for dynamic partitioning, not completion of the multi-cell
universe or handoff features.

Background mode advances physical production only. It does not step rigid-body
physics, players, gravity, contacts, controls, life support, damage, combat,
defenses, AI, travel, cleanup, markets, or another timer. The implementation is
an original clean-room Verse system and does not copy third-party code, assets,
interfaces, fiction, names, or protected presentation.

### Lifecycle state machine

The durable control plane records `Sleeping`, `Activating`, `Background`,
`Active`, and `Draining` with a monotonic lifecycle revision.

- A due production occurrence moves Sleeping to Background.
- Authenticated gameplay ingress moves Sleeping or Background through
  Activating to Active.
- Activating catches up every occurrence through one captured wake cut-off,
  validates and snapshots, and only then admits sessions.
- Active moves through Draining after the accepted idle rule or an authorized
  operator request.
- Draining stops admission and new mutation intents, completes only an already
  selected atomic boundary, persists, invalidates sessions, and moves to
  Background when runnable work remains or Sleeping otherwise.
- Background returns to Sleeping when no runnable or due work remains.
- Lease loss or uncertainty self-fences the process immediately. `Fenced` is a
  worker result, not a gameplay state that a stale holder may persist.

Public spectators may join an already Active cell but never request activation,
retain Active mode, or delay drain. A Sleeping or Background spectator request
returns bounded non-active status or an explicitly historical cached view; it
does not acquire the simulation lease.

### Control-plane and gameplay ownership

The lifecycle coordinator owns desired/observed mode, dispatch, lease records,
and schedule acknowledgement. The simulation aggregate owns production jobs,
the committed occurrence frontier, journal events, snapshots, and conservation.
The journal is authoritative if an event append and scheduler acknowledgement
are separated by a crash.

Lifecycle-control schema `1` binds the exact universe, cell, universe-manifest,
and registry roots; lifecycle revision; desired and observed mode; holder and
lease times; fencing token; last verified world frontier; next occurrence; and
acknowledged occurrence sequence. Schedule-occurrence schema `1` binds a stable
key `(universe_id, cell_id, lifecycle_generation,
production_quantum_sequence)`, exact scheduled time, and the same trust roots.

Operational lease renewal and desired mode do not enter the canonical gameplay
state hash. The canonical world does store the last committed production
occurrence so at-least-once redelivery is idempotent after replay.

### Renewable local lease and fencing

The first backend is an injectable lease interface implemented over one local
data root and one exclusive operating-system file lock. It durably records a
holder, nonzero fencing token, acquisition time, renewal time, and expiry. The
proof policy uses a 15-second lease renewed no later than every 5 seconds.

Every event append and snapshot rechecks holder, token, and expiry. The event's
embedded authority token must equal the live store token. An uncertain renewal
stops new work before the safety margin; expiry fences the worker. Expiry does
not permit time-only lease theft while a live process retains the exclusive
file lock. Crash releases the lock and permits a successor.

A successor token must be strictly greater than every recovered historical
token. Replay accepts positive nondecreasing tokens, including repeated tokens
within one lease, and rejects zero or decreasing history. Token allocation uses
checked arithmetic and fails on exhaustion. These semantics prove safe local
replacement; they do not claim distributed consensus or multi-host availability.

Recovery occurs under exclusive mutation authority. The holder validates exact
schemas and roots, loads the snapshot, replays journal events and historical
fences, reconciles schedule acknowledgement with the canonical occurrence
frontier, proves its live token is newer, and only then reports Active or
Background health. A read-only preflight must revalidate the complete frontier
after lease acquisition.

### Trusted time and at-least-once occurrences

One injected trusted-clock interface supplies scheduler UTC. The accepted local
backend persists the last observed time. Clients, frame timing, process uptime,
and dispatch arrival time are not time authority.

Every production occurrence is a stable, positive, contiguous sequence with a
`scheduled_for_unix_ms` exactly 1,000 milliseconds after its predecessor.
Backward time never reverses a cursor or repeats an occurrence; a rollback past
the configured tolerance halts scheduling. Forward time creates exact
sequential backlog. It never creates one oversized elapsed quantum or silently
skips work.

The occurrence `lifecycle_generation` is a production-clock generation. It
does not change on an ordinary mode transition, process restart, lease renewal,
or fenced worker replacement. Only an explicit reset or audited migration may
increment it and restart the contiguous production sequence.

Process downtime accrues occurrences only for jobs durably runnable under the
unchanged canonical P1.5 power and route state. Paused or empty queues do not
wake every second. A later relevant canonical mutation schedules reevaluation.
This decision does not define outage-adjusted destructive timers.

The coordinator durably arms a runnable queue's first occurrence exactly one
second after the trusted state-change boundary. That due timestamp survives a
process crash, preserving subsecond elapsed time without treating process-local
uptime as authority. Journal and lifecycle-record reconciliation repairs a
crash between the canonical mutation and schedule arming without choosing a new
anchor.

### Atomic production quantum

Event schema `15` replaces new per-machine production advances with one
`ProductionQuantumCommitted` event for one occurrence. It contains exactly one
second expressed as the content manifest's `fixed_step_hz` and the complete
ordered outcomes for the queue heads selected at the start of the quantum.

Outcomes are sorted by grid ID then block ID and include progress, pause,
completion, registered loss, pending output, delivery, ledger, and reward
effects needed to validate the transition. The complete vector applies or
rejects atomically. A crash cannot expose a quantum in which the first machine
advanced and the second did not.

Active and Background call the same pure whole-cell planner. Replay independently
recomputes the complete vector from the prior canonical state and rejects a
missing, extra, duplicate, reordered, or altered outcome before mutation. The
event identity derives from the stable occurrence key and its canonical time is
the scheduled time. Scheduler acknowledgement occurs only after the event is
durable.

Active/Background equivalence means the same occurrence, elapsed ticks,
ordered outcome material, conserved gameplay state, and resulting world hash
from the same prior state. A valid worker replacement may change the operational
fencing token and therefore the serialized event envelope without changing the
production rule.

### Bounded catch-up

The proof policy permits at most 256 queue-bearing machines in a background-
eligible cell, 60 exact quanta per dispatch, 250 milliseconds of coordinator
work before yielding after the current atomic quantum, and one claimed but
unacknowledged occurrence.

Exceeding the machine envelope prevents drain to Background rather than
deleting structures or partially evaluating a quantum. Exhausting a catch-up
budget persists the exact continuation and yields. Backlog is never skipped or
semantically coalesced. Higher limits require a compatible policy commitment
and published evidence.

### Activation, replication, and drain

Gameplay ingress during Sleeping or Background coalesces around one activation
revision. The activation cut-off is captured before catch-up; no gameplay intent
races an occurrence due at or before that cut-off. Sessions receive no world
state until catch-up and snapshot succeed.

Activation and reconnect create fresh session and interest epochs and one new
independently verified baseline. Pre-drain deltas, acknowledgements, and pending
verification stages are invalid. Draining rejects new admission and mutation,
persists the final atomic boundary, closes sessions, and only then changes mode
or releases authority.

### Compatibility

P1.6 uses protocol `17`, projection schema `3`, world schema `19`, event schema
`15`, content schema `11`, content manifest `p1.5.0`, registry schema `1`,
universe-manifest schema `3`, interest schema `1`, operation-fingerprint schema
`1`, lifecycle-control schema `1`, and schedule-occurrence schema `1`.

Universe-manifest schema `3` binds the lifecycle/schedule policy schema and
hash in addition to the world/event tuple. Projection and interest schemas stay
unchanged because operational lifecycle data does not enter interest material.
If a later client view carries lifecycle data, that change requires a new
projection version.

The first implementation archives and resets P1.5 proof data. An optional
future offline migration must declare a trusted-time cut-off, introduce one
unambiguous production frontier, prove replay equality, and emit an auditable
migration receipt. Rollback restores the matching P1.5 binary, roots, and
archived data; no older executable interprets P1.6 records.

## Alternatives considered

### Retain one event per machine

Rejected. It requires a durable intra-quantum cursor and exposes a partial
quantum across crash or lease loss. One bounded aggregate event supplies a
clear idempotency and replay boundary.

### Continue using the active process millisecond accumulator

Rejected. Subsecond phase and downtime disappear on restart, and Background
would implement a second time authority.

### Advance production with one elapsed downtime event

Rejected. Combining many seconds changes queue completion, output-capacity,
machine ordering, and future recipe interactions. Catch-up uses exact sequential
one-second quanta.

### Poll paused jobs once per second

Rejected. It creates permanent write and wake amplification without a state
change. Relevant canonical mutations re-arm paused work.

### Let public observation wake the cell

Rejected for P1.6. Anonymous traffic could consume simulation resources and
prevent drain. Public registry data remains available, and active-cell
spectating retains its existing privacy boundary.

### Steal an expired local lease by wall time

Rejected. Clock uncertainty or a suspended holder could create two writers.
The local proof requires exclusive file-lock release as well as a newer token.

### Implement a distributed scheduler and multi-cell handoff now

Deferred. A transactional or consensus-backed lease service, directory,
placement, transfers, and failure domains require separate specifications and
evidence. A local proof must not be marketed as that system.

## Consequences

### Positive

- Production can continue without full-rate physics or a gameplay session.
- Active and Background use one deterministic job state machine.
- Stable occurrences make at-least-once scheduling idempotent across crashes.
- Atomic whole-cell events remove partial-machine quantum recovery.
- Strict renewal and append-time fencing make stale authority fail closed.
- Bounded catch-up makes downtime explicit without blocking forever in one
  dispatch.
- Fresh activation baselines preserve the P1.5 convergence and privacy model.

### Negative

- P1.6 changes world, event, protocol, universe-manifest, persistence, worker,
  verifier-pin, release, and cross-process boundaries together.
- The first lease backend is safe only on one host and one shared local root.
- Long downtime may require many bounded dispatches before gameplay admission.
- A cell above the proof machine envelope cannot enter Background.
- Public spectators cannot make a sleeping proof cell live.
- P1.5 worlds require reset or a separately proven offline migration.

## Validation

- Active and Background execution from the same prior state and occurrence
  produce identical ordered production outcomes and canonical world hashes.
- Multiple machines, paused power, broken routes, full destinations, later
  delivery, splits, destruction, and conservation pass through one atomic
  quantum.
- Controlled-clock tests cover subsecond restart, long downtime, clock rollback,
  forward jumps, bounded continuation, and overflow.
- Duplicate, reordered, missing, future, wrong-root, wrong-cell, and conflicting
  occurrences reject or reconcile without duplicate work.
- Fault injection covers every lease, transition, event append/sync, schedule
  acknowledgement, snapshot, and release boundary.
- Two local workers race for the cell; only one wins, and a resumed stale holder
  cannot append, snapshot, publish, or report healthy.
- Hard-kill cross-process evidence drains, runs background work, restarts,
  reconciles, activates, and observes exactly one output.
- Background execution leaves physics ticks, players, controls, contacts,
  oxygen, damage, and interest state unchanged.
- Gameplay waits behind the activation cut-off and receives one fresh verified
  baseline; public observation does not wake or retain the cell.
- Existing multiplayer, industry, conservation, verifier, native, browser,
  packaging, and release tests remain green.

## Deliberate exclusions

This decision does not deliver multi-cell assignment, handoff, distributed
leases, public high availability, frontier generation, travel, background
physics or combat, cleanup timers, markets, contracts, blockchain integration,
sleeping-cell browser management, binary replication, or public-scale capacity.
It does not complete WORLD-008 or F-013.

## Clarifies

ADR-0018's requirement for identical Active and Background production is
defined here as identical occurrence-bound production material and gameplay
result. ADR-0018's physical recipes, power gate, queue, escrow, conservation,
split, destruction, privacy, and content-manifest decisions remain unchanged.
