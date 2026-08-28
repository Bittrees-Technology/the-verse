# P1.6 durable single-cell lifecycle and background production

**Feature ID:** F-060

**Status:** Implemented and verified locally, in the hosted Linux container,
and in hosted Linux and Apple Silicon packages for the bounded P1.6 proof

**Owner:** Universe-control, simulation-worker, persistence, protocol, and
operations maintainers

The durable architecture choices are recorded in
[ADR-0022](../decisions/ADR-0022-durable-single-cell-lifecycle.md).

Implementation revision `0664130` passed the complete local release gate and
[hosted CI run 33137371577](https://github.com/Bittrees-Technology/the-verse/actions/runs/33137371577).
The hosted run includes the isolated Linux-container verifier and both native
package jobs. This is one-cell correctness and packaging evidence; it is not a
multi-cell, multi-host, or public-scale claim.

## Linked requirements and features

- WORLD-008 — Partitioned execution, as a one-cell prerequisite only
- SIM-002 — Server authority
- SIM-006 — Finite execution budgets
- SIM-008 — Power
- SIM-011 — Session-bound player authority
- IND-001 — Transformation chain
- IND-002 — Conservation
- F-011 — Durable snapshot and event recovery
- F-023 — Physical mining, refining, manufacturing, and production queues

P1.6 does not complete WORLD-008 or F-013. It proves lifecycle, scheduling, and
fencing semantics for one already generated fixed proof cell before a universe
directory assigns many cells or transfers entities between them.

## Player outcome

A player may queue supported physical production, leave the cell, and later
return to the exact authoritative result. With no gameplay session present, the
cell stops full-rate physics and advances only due production work. Restart,
duplicate scheduler delivery, or worker replacement cannot duplicate progress,
output, loss, or experience. Returning gameplay waits for bounded catch-up and
then receives one fresh authoritative view.

This is an original clean-room design built from the repository's public Verse
requirements and common distributed-systems principles. It does not copy
third-party source, assets, names, interfaces, fiction, or protected audiovisual
expression.

## Milestone boundary

### Included

- One fixed canonical proof cell and one local lifecycle coordinator.
- Durable `Sleeping`, `Activating`, `Background`, `Active`, and `Draining`
  lifecycle states with guarded transitions.
- A renewable single-host lease with a strictly increasing fencing token.
- Durable one-second production occurrences and at-least-once dispatch.
- One atomic whole-cell production event per committed occurrence.
- The same production planner and replay validator in Active and Background.
- Bounded sequential catch-up after restart or scheduler delay.
- Explicit trusted-clock, clock-discontinuity, crash, replay, and reconciliation
  behavior.
- Admission, drain, fresh-baseline, health, status, and observability rules.
- Exact schema compatibility, reset/migration, and rollback boundaries.

### Excluded

- More than one scheduled cell, a universe directory, worker placement, or
  cross-cell handoff.
- Multi-host or distributed high availability, quorum leases, or a public
  control-plane availability claim.
- Frontier materialization, sector expansion, autopilot, or multi-day travel.
- Background rigid-body physics, gravity integration, contact solving,
  character controls, oxygen, damage, combat, turrets, AI, or cleanup.
- Attack-, timer-, travel-, or arbitrary-observation-triggered wake-up.
- Death-drop expiry, derelict salvage/deletion, insurance, or outage-adjusted
  destructive timers.
- Stateful batteries, fuel consumption, machine power allocation, or priority.
- Markets, contracts, companies, AMMs, blockchain settlement, or lifecycle
  proof publication.
- Sleeping-cell production management in the browser, unrestricted sleeping-
  cell spectating, a production binary codec, or a thousands-player claim.

## Authority and ownership

The local lifecycle coordinator owns desired mode, dispatch, lease acquisition,
renewal, release, and scheduler acknowledgement. These are operational records;
they cannot create ore, progress, output, experience, inventory, or another
gameplay mutation.

The fenced simulation cell owns canonical production state, the last committed
production occurrence, event order, snapshots, replay, and conservation. Only
the current lease holder may append an event or snapshot. A scheduler delivery
is an instruction to evaluate a due occurrence, not authority to dictate its
result.

Clients submit ordinary authenticated gameplay intents only. No player,
spectator, browser, bot, or external application may construct a production
quantum, select its elapsed time, set a lifecycle mode, choose a fencing token,
or acknowledge schedule completion.

## Lifecycle model

```text
Sleeping ── due production ──> Background ── no work ──> Sleeping
    │                              │
    │ authenticated gameplay      │ authenticated gameplay
    └──────────────┬───────────────┘
                   v
              Activating ──> Active ── idle/operator ──> Draining
                                                               │
                               runnable production <───────────┤
                                      │                        │
                                      v                        v
                                 Background                 Sleeping
```

`Generated` and `Unmaterialized` remain universe-directory concerns outside
this slice. `Fenced` is a terminal worker condition rather than a canonical
gameplay mode: a process that cannot prove its live lease stops all mutation
and yields recovery to a later holder.

### Sleeping

- No cell runtime, physics loop, gameplay session, or one-second busy poll is
  required.
- The durable world snapshot, journal, lifecycle record, and optional next
  production occurrence are sufficient to recover.
- A due production occurrence may request Background. Authenticated gameplay
  ingress may request Activating.
- A public spectator request does not wake the cell or acquire a simulation
  lease. It receives a bounded `cell_not_active` status with the last durable
  public frontier. A separately cached view, if served, is explicitly labelled
  historical and is never a live interest baseline.

### Activating

- New mutation intents remain unavailable.
- The worker holds a valid lease, verifies exact universe and content roots,
  loads and replays the world, and reconciles a lagging scheduler acknowledgement
  against the canonical production frontier.
- Activation captures one trusted-time wake cut-off and commits every due
  production occurrence through that cut-off, subject to the catch-up budget.
  If backlog remains, activation yields and continues later without admitting a
  gameplay session.
- Only after catch-up, invariant validation, and a durable snapshot may the
  cell become Active and issue fresh session and interest epochs.

### Background

- The worker holds and renews the same kind of mutation lease required by
  Active.
- Only the production scheduler defined here may run. It does not construct or
  step the physics scene.
- Each due occurrence calls the same whole-cell planner used by Active, appends
  at most one atomic event, updates the canonical occurrence frontier, and then
  acknowledges dispatch.
- When no runnable work or due continuation exists, the worker snapshots,
  records Sleeping, and releases the lease.
- Gameplay ingress changes the desired mode to Active. The current holder
  completes the already claimed atomic occurrence, freezes a wake cut-off, and
  enters Activating. No active and background writer overlap.

### Active

- Existing authoritative physics, life support, input, damage, production,
  and interest replication operate normally.
- Production time comes from the durable occurrence scheduler, not from a
  process-local millisecond accumulator. Active and Background therefore share
  the same occurrence identity and planner.
- Public spectators may attach to an already Active cell, but do not count as
  gameplay activity and cannot prevent an otherwise eligible drain.

### Draining

- The worker rejects new sessions and new client mutation intents with a
  retryable lifecycle reason.
- It finishes only the physics or production atomic boundary already selected;
  it does not begin another client-controlled boundary.
- It persists the final event frontier and snapshot, invalidates active session
  and interest epochs, closes sessions with a lifecycle reason, and records the
  next eligible production occurrence.
- If runnable production remains, it enters Background. Otherwise it records
  Sleeping and releases the lease.
- A snapshot, mode record, or release failure leaves the worker fenced or
  Draining; it never reports Sleeping while it may still write.

## Durable lifecycle and lease record

Lifecycle-control schema `1` stores fields equivalent to:

```text
CellLifecycleRecord
  schema_version
  universe_id
  cell_id
  universe_manifest_hash
  celestial_registry_hash
  lifecycle_revision
  desired_mode
  observed_mode
  holder_id?
  fencing_token
  acquired_at_unix_ms?
  renewed_at_unix_ms?
  expires_at_unix_ms?
  activation_cutoff_unix_ms?
  last_world_event_sequence
  last_world_event_hash
  last_world_state_hash
  next_production_occurrence?
  acknowledged_production_sequence
  updated_at_unix_ms
```

Unknown fields, wrong roots, an invalid mode, a non-increasing revision, a zero
live fencing token, or inconsistent holder/time fields fail closed. Durable
updates use replace-and-sync or equivalent transactional semantics; a torn or
ambiguous record is not repaired by guessing.

The P1.6 proof backend uses one exclusive operating-system file lock over one
local data root. Lease metadata is renewed durably. The proof policy defaults
to a 15-second duration and renewal no later than every 5 seconds. Every write
also checks the current holder, token, and expiry. Failure or uncertainty before
expiry stops new work; reaching expiry self-fences the worker.

Expiry alone never authorizes another local process to steal a lease while the
exclusive file lock remains held. A crashed process releases the operating-
system lock; a successor then allocates a checked token strictly greater than
every recovered historical token. Token exhaustion is an explicit fatal error,
not saturation or wraparound. These rules prove safe single-host replacement,
not multi-host availability during a suspended live process.

Every event append requires its embedded `authority_fencing_token` to equal the
store's current live token. Every snapshot requires the world's operational
token to equal that token. Replay requires positive, monotonically
nondecreasing historical tokens and permits equality for events written under
one lease. The newly acquired token must exceed the recovered maximum.
Operational token changes remain outside the canonical gameplay state hash, but
their validation is mandatory before a write.

## Trusted time and production occurrences

The scheduler uses an injected trusted-clock interface. The local proof uses
host UTC only behind that interface and persists the last accepted time.
Production semantics never use a client timestamp, process uptime, frame count,
or dispatch arrival time.

Schedule-occurrence schema `1` stores fields equivalent to:

```text
ProductionScheduleOccurrence
  schema_version
  universe_id
  cell_id
  lifecycle_generation
  production_quantum_sequence
  scheduled_for_unix_ms
  universe_manifest_hash
  celestial_registry_hash
```

Its stable key is:

```text
(universe_id, cell_id, lifecycle_generation, production_quantum_sequence)
```

Sequences are positive and contiguous within a generation. While production
remains continuously runnable, scheduled time advances by exactly 1,000
milliseconds per occurrence. After an idle or paused interval cancels the
cursor, the next occurrence is re-armed exactly 1,000 milliseconds after the
new trusted runnable boundary and can therefore have a larger gap from the
last committed occurrence. Dispatch time and lateness are diagnostic only.
Backward trusted time does not reverse a cursor or make an occurrence due; a
rollback beyond the configured tolerance halts scheduling for operator
recovery. A forward jump creates a sequential backlog. It never grants one
oversized elapsed duration or skips intermediate quanta.

`lifecycle_generation` names the production-clock generation, not an ordinary
mode transition, worker process, or lease acquisition. It remains stable across
Sleeping, Background, Activating, Active, Draining, crash recovery, and fenced
worker replacement. Only a declared reset or audited migration may increment
it and restart the sequence, so a routine wake cannot make a committed
occurrence valid again.

Process downtime counts as elapsed production time only for work that was
already runnable under the last durable canonical state. The existing P1.5
aggregate qualifying-power rule has no background fuel depletion. A queue
paused for power, route, machine completeness, or destination capacity does not
wake once per second. A relevant later canonical mutation re-evaluates it and
creates the next occurrence from that new durable boundary.

When work first becomes runnable, the trusted coordinator durably arms the next
occurrence for exactly 1,000 milliseconds after that accepted boundary. This
preserves partial elapsed time without a process-local accumulator: a crash 750
milliseconds later leaves the same due timestamp, and a successor observes the
occurrence due after the remaining 250 milliseconds. Arming or cancelling a
pending occurrence and committing the canonical mutation that caused it use one
recoverable transition; recovery reconciles either side from the journal and
lifecycle record rather than inventing a new time anchor.

## Atomic whole-cell production quantum

World schema `19` stores a production-clock frontier equivalent to:

```text
ProductionClock
  lifecycle_generation
  last_committed_quantum_sequence
  last_scheduled_for_unix_ms
```

The production clock is independent of `simulation_tick`. Background
production advances registered job work by `fixed_step_hz` ticks per quantum;
it does not claim that rigid-body time advanced.

Event schema `15` adds one system payload equivalent to:

```text
ProductionQuantumCommitted
  occurrence
  elapsed_ticks
  outcomes[]

ProductionMachineOutcome
  grid_id
  machine_block_id
  job_id
  prior_status
  resulting_status
  previous_progress_ticks
  new_progress_ticks
  completed
  output_delivered
```

One event contains every queue-head outcome selected at the start of that
quantum, ordered by canonical grid ID and then block ID. Paused, blocked, and
no-progress outcomes are explicit when a claimed occurrence evaluates them.
Completion, registered loss, pending-output escrow, delivery, ledger changes,
and experience are applied atomically inside the vector. A claimed occurrence
may contain an empty vector to advance its cursor after reconciliation, but an
idle cell does not generate empty events continuously.

Live preparation and replay independently recompute the complete vector from
the prior canonical state. Missing, duplicate, unordered, extra, or altered
outcomes reject before mutation. The system event ID is derived from the stable
occurrence key and schema, and its canonical occurrence time is
`scheduled_for_unix_ms`. The fencing token and hash-chain fields still bind the
actual authority that committed it.

“Identical Active and Background advancement” means identical occurrence,
elapsed ticks, ordered production outcomes, conserved gameplay state, and
resulting canonical world hash from the same prior state. A whole serialized
envelope may carry a different valid fencing token after worker replacement;
that operational difference is not a production-rule difference.

## At-least-once dispatch and reconciliation

The canonical event journal is the authority for whether an occurrence changed
the world. Scheduler acknowledgement follows event append and sync.

- Crash before append leaves the occurrence due.
- Crash after append sync but before acknowledgement re-delivers the same key.
- Recovery sees the world occurrence frontier and acknowledges the duplicate
  without another production mutation.
- The same key with different schedule material is a fatal conflict.
- A skipped or future sequence rejects; work resumes only from the exact next
  cursor.
- Snapshot creation may lag the journal. Replay restores the same cursor before
  scheduler reconciliation.

Recovery acquires exclusive mutation authority, validates the exact pinned
roots and schemas, loads the snapshot, replays the journal while validating
historical fences and occurrences, proves the new token is greater, and only
then publishes Background or Active health. A read-only preflight may occur
before acquisition, but the worker must revalidate the complete frontier under
the acquired lease before mutation.

## Finite work and backpressure

The P1.6 proof policy sets these default operating bounds:

- at most 256 queue-bearing machines in one background-eligible proof cell;
- at most 60 exact quanta in one background dispatch;
- at most 250 milliseconds of coordinator work before yielding after the
  current atomic quantum; and
- at most one claimed but unacknowledged occurrence per cell.

The machine bound is a measured background operating envelope, not a permanent
game-design size cap. A cell outside it remains Active or rejects drain to
Background with a visible operator reason; it does not discard a structure or
partially schedule a quantum. Lower deployment limits are allowed. Higher
limits require new published evidence and a compatible policy hash.

Budget exhaustion persists the exact continuation, renews or releases the
lease safely, and yields. Backlog is reported rather than silently skipped or
combined. Arithmetic overflow, an unrepresentable due count, or inability to
finish one bounded quantum inside the lease safety margin fails closed.

## Protocol, projection, and public access

Gameplay protocol `17` reports the coordinated schemas and a retryable cell
lifecycle status. Gameplay admission during Sleeping or Background requests an
authorized wake and waits for activation; it receives no canonical snapshot
until activation succeeds. Draining rejects new admission.

Projection schema `3` and interest schema `1` remain unchanged because
lifecycle-control state is not added to an interest view. Activation and every
reconnect create fresh session and interest epochs and one new independently
verified baseline. Acknowledgements, deltas, or pending verifier stages from a
pre-drain session are invalid.

Public spectators can observe the existing bounded view only while the cell is
already Active. Their request does not change desired mode, acquire a lease,
delay drain, or schedule production. The public registry and immutable universe
manifest remain readable without waking the cell. A future historical snapshot
API must include an explicit durable frontier and staleness label.

## Compatibility and persistence

The coordinated P1.6 boundary is:

| Boundary | P1.6 value | Reason |
| --- | --- | --- |
| Gameplay protocol | `17` | Lifecycle admission/status and coordinated compatibility |
| Projection schema | `3` | Existing active-cell view material is unchanged |
| World schema | `19` | Durable production clock and occurrence frontier |
| Event schema | `15` | Atomic, occurrence-bound whole-cell quantum |
| Content schema | `11` | Existing physical-production rules are reused |
| Content manifest | `p1.5.0` | Recipes, power gate, duration, and yield are unchanged |
| Celestial registry | `1` | Fixed body identity is unchanged |
| Universe manifest | `3` | Binds lifecycle and schedule policy schema/hash |
| Interest schema | `1` | Active-cell interest material is unchanged |
| Operation fingerprint | `1` | Client intent fingerprinting is unchanged |
| Lifecycle control | `1` | Durable mode and lease record |
| Schedule occurrence | `1` | At-least-once production occurrence identity |

The product release may be named `p1.6.0` while retaining content manifest
`p1.5.0`. Operational catch-up and lease policy do not invent a new recipe or
celestial definition. The exact universe-manifest root changes because schema
`3` binds the new policy and world/event versions; trusted client pins and
portable vectors must change with it.

P1.5 proof worlds are archived and reset for the first implementation. A later
offline migration must verify the source world, introduce an unambiguous
production clock at a declared cut-off, replay both histories to the same
gameplay state, emit a signed migration receipt, and atomically install the new
manifest pointer. An older executable never opens P1.6 control, world, event,
or manifest records.

Rollback drains sessions and workers, restores the exact prior binary,
manifests, roots, and archived P1.5 data, and never reinterprets P1.6 events.

## Security and failure behavior

- A stale, expired, wrong-holder, wrong-cell, or wrong-root lease cannot append,
  snapshot, acknowledge, publish, or report healthy.
- Duplicate dispatch cannot repeat progress, output, loss, ledger credit, or
  experience.
- Missing, reordered, or forged machine outcomes reject before state mutation.
- A client cannot accelerate work by reconnecting, changing local time,
  requesting observation, or sending repeated wake requests.
- Repeated authorized wake requests coalesce around one lifecycle revision and
  one activation result.
- A clock rollback cannot replay an occurrence. A forward jump cannot create
  one unbounded event.
- Lease loss during planning discards the candidate. Lease loss after journal
  sync recovers from the durable event, not from the stale process memory.
- A failed drain cannot expose simultaneous Active and Background authority.
- Background execution cannot mutate physics, player life state, private
  visibility, or another subsystem's timers.

## Observability

Operators receive bounded metrics and structured logs for:

- desired and observed lifecycle mode, revision, and transition reason;
- lease holder, fencing token, renewal deadline, renewal latency, and stale-
  fence rejection count;
- last and next occurrence sequence, scheduled time, lateness, backlog, and
  duplicate dispatch count;
- quanta and machine outcomes per dispatch, yield reason, and processing time;
- Active-to-Draining and Activating-to-Active duration;
- snapshot and journal frontier at every transition;
- clock rollback/forward detection and trusted-time source health;
- crash-recovery and scheduler-reconciliation result; and
- halted/fenced reason and last verified universe roots.

Public status exposes mode, durable frontier, staleness, and generic health. It
does not reveal actor-private queue IDs, recipes, cargo handles, progress,
escrow, or quantities.

## Acceptance criteria

1. The same prior snapshot and occurrence through Active and Background produce
   the same ordered production outcomes, conservation view, and world hash.
2. Multiple machines advance once in grid-ID/block-ID order inside one atomic
   event; a crash cannot expose a partially applied quantum.
3. Powered progress, power pause, route pause, output block, later delivery,
   machine destruction, and grid split behave identically in both modes.
4. A restart after 750 milliseconds followed by 250 milliseconds of trusted
   elapsed time commits exactly one quantum.
5. Process downtime creates the exact sequential due occurrences for work that
   was durably runnable; it does not create one oversized elapsed event.
6. Before-write failure recovers the prior state. After-sync/before-
   acknowledgement failure recovers exactly one committed occurrence.
7. Duplicate, missing, reordered, future, wrong-cell, wrong-root, wrong-fence,
   and conflicting occurrence deliveries reject or reconcile without another
   gameplay mutation.
8. A two-process claim race has one winner. After replacement, every append and
   snapshot by the old holder fails even if that process resumes.
9. Zero, decreasing, exhausted, wrapped, or mismatched fencing tokens fail
   before mutation; valid same-lease and increasing-takeover histories replay.
10. Sleeping, Background, Activating, Active, and Draining recover exactly from
    a hard crash at every durable boundary.
11. Draining admits no new session or intent and persists the final atomic
    boundary before changing mode or releasing authority.
12. Gameplay ingress during catch-up receives no state until all occurrences
    through the wake cut-off commit and a fresh verified baseline is ready.
13. Background changes no physics tick, pose, velocity, controls, contacts,
    oxygen, life state, damage, or interest state.
14. Public spectator requests do not wake, retain, or delay the cell and cannot
    reveal private production state.
15. Paused or empty cells sleep without one-second polling; a relevant later
    canonical mutation re-evaluates paused work.
16. Long downtime respects per-dispatch machine, quantum, and time budgets,
    persists an exact continuation, and eventually reaches the same result as
    sequential Active execution.
17. Clock rollback, corrupt lifecycle data, uncertain renewal, and arithmetic
    overflow fail closed with actionable status.
18. Snapshot plus journal replay restores the exact lifecycle generation,
    occurrence frontier, queue, escrow, event frontier, and world hash.
19. Existing mining, manufacturing, inventory, multiplayer, independent-
    verifier, native, browser, packaging, and conservation suites remain green.
20. Evidence and release notes state that the proof covers one fixed cell and
    does not establish multi-cell, multi-host, or public-scale readiness.

## Test and evidence strategy

- **Unit:** Lifecycle transition table, occurrence ordering, trusted-clock
  boundaries, whole-cell planning, paused scheduling, bounds, and token
  arithmetic.
- **Property/invariant:** Conservation and experience are identical across
  Active/Background histories, duplicate dispatch, catch-up chunking, and
  restart placement.
- **Negative/replay:** Tamper every occurrence and outcome field, ordering,
  completeness, roots, lifecycle revision, clock frontier, and fencing token
  before mutation.
- **Fault injection:** Crash before and after lease allocation, renewal,
  transition write, event append/sync, schedule acknowledgement, snapshot, and
  release.
- **Cross-process:** Queue work, drain, run Background, terminate without
  graceful snapshot, restart, reconcile, activate, and observe exactly one
  output.
- **Concurrency:** Race two local workers, resume a stale worker, and coalesce
  repeated wake requests.
- **Budget/load:** Publish the 256-machine quantum and 60-quantum catch-up
  distributions, deadline margins, journal sizes, and eventual completion.
- **Client:** Fresh verified baseline after activation, stale pre-drain message
  rejection, non-waking spectator behavior, and explicit unavailable status.
- **Release:** Hosted Linux plus Apple Silicon package and cross-process gates
  pass from assembled artifacts with a controlled test clock.

## Rollout

1. Freeze ADR-0022, lifecycle-control schema `1`, occurrence schema `1`, and
   universe-manifest schema `3` hash material.
2. Add strict lease, fencing, recovery, and fault-injection primitives before
   enabling a Background transition.
3. Replace per-machine production events with the atomic whole-cell planner,
   event, replay validator, and occurrence frontier.
4. Add the optional-runtime cell host, lifecycle transitions, bounded
   scheduler, admission/drain behavior, and status metrics.
5. Update exact trust pins, verifier vectors, protocol consumers, packaging,
   and the cross-process evidence harness.
6. Archive/reset the P1.5 proof universe and run the complete local and hosted
   release gates.

No implementation may claim this milestone merely because a timer advances a
job while no client is connected. The claim requires the lease, atomic event,
idempotent dispatch, crash/replay, bounded catch-up, lifecycle, and evidence
contracts together.

## Open questions

No product decision blocks the bounded proof. A production control-plane store,
multi-host lease semantics, stale sleeping-cell public read service, additional
background systems, and permanent cell-capacity policy remain explicit later
decisions.
