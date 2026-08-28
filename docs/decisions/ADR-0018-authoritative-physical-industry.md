# ADR-0018: Authoritative physical industry and conveyor production

**Status:** Accepted

## Context

The P1.1 proof can refine ore and fabricate a component immediately inside any
actor-owned functional inventory. That path proves actor isolation,
conservation, idempotency, and recovery, but it does not make production
physical. It has no refinery or assembler block, conveyor connectivity, machine
queue, production duration, job escrow, or power interruption.

F-023 and IND-001 require extraction, hauling, refining, manufacturing,
assembly, and construction to form one authoritative work loop. Core production
invariants also require registered inputs, outputs, loss, energy, machine time,
and content-manifest selection to survive retries and crashes.

The first physical-industry implementation must improve the single-cell
gameplay loop without binding production to active physics replication. Future
background cells must be able to advance the same canonical state through the
same event contract.

The complete player-facing and verification contract is the
[P1.4 physical-industry specification](../gameplay/physical-industry.md).

## Decision

### Physical topology

- Completed cargo, refinery, assembler, and conveyor blocks expose conveyor
  ports on all six faces.
- Two ports connect only when completed blocks occupy face-adjacent integer
  coordinates on the same grid. Edge, corner, world-space, proximity, and
  cross-grid adjacency do not connect.
- A deterministic graph traversal over canonical block-coordinate and block-ID
  order resolves connectivity. The graph is derived and may be cached, but the
  cache is never authoritative.
- Unfinished, destroyed, or separated blocks do not participate. A structural
  split recomputes each resulting grid independently.

### Inventories and jobs

- A production request identifies one completed machine block, one registered
  recipe, a positive batch quantity, one source cargo inventory, and one
  destination cargo inventory.
- Source and destination remain explicit cargo inventories in this slice.
  Refineries and assemblers do not gain general-purpose player-visible internal
  inventories.
- The source, destination, and machine must belong to the authenticated actor
  and must resolve to one completed conveyor component on one grid.
- Each machine owns one canonical first-in, first-out queue containing at most
  32 jobs. Queue order is immutable in this slice.
- Accepted enqueue atomically removes the exact registered inputs from the
  source inventory and places them in job escrow. There is no cancellation in
  this slice.
- Job escrow is a canonical asset location included in conservation. It is not
  a cargo inventory and cannot be selected by generic transfer intents.
- Completion atomically applies the recipe registered by the job's content
  manifest. If the destination has capacity, output is delivered exactly once.
  Otherwise output remains in job escrow with an `output_blocked` state and is
  delivered exactly once after capacity and connectivity become valid.

### Time and power

- Production has a separate canonical one-second scheduler. One scheduler
  quantum is exactly the active content manifest's fixed-step rate expressed as
  integer simulation ticks.
- A production-advance event records integer elapsed ticks and the complete
  canonical job outcomes for that quantum. Active and future background cells
  must prepare and replay the same event for the same prior state and elapsed
  ticks.
- Only the first queued unfinished job on a machine may advance.
- Progress advances only while the machine is complete, its queue remains
  associated with it, and its grid passes the existing qualifying-power gate.
  Loss of power pauses progress without consuming inputs again or resetting
  elapsed work.
- This slice deliberately retains the current aggregate qualifying-power model.
  Stateful battery discharge, per-machine allocation, and power priority are
  deferred and must not be implied by the UI.

### Damage, destruction, and splits

- A queue and every escrowed quantity always resolve to exactly one live machine
  and grid, or to one conserved dropped container created by an explicit event.
- A split moves a machine, its queue, and its escrow to the unique fragment
  containing that machine. It never clones, reorders, or completes a job.
- Destroying the machine atomically removes its queue and creates one dropped
  inventory containing all reserved inputs and pending outputs. Recipe-defined
  loss already committed at completion is not recreated.
- Destroying or separating a conveyor or cargo endpoint pauses affected jobs.
  It does not erase their progress or escrow. A later valid route may resume or
  deliver them.

### Authority and visibility

- The simulation cell owns conveyor resolution, queue admission, input
  reservation, time, power qualification, completion, delivery, damage
  consequences, persistence, and recovery. Clients submit intents only.
- Queues, job identities, recipe quantities, progress, source and destination
  inventory identities, and escrow are actor-private. Public grid state may
  expose visible machine kind and broad operating presentation without private
  quantities or queue detail.
- Owner-wide terminal access remains a temporary P1.4 proof rule. Physical
  terminal proximity, cockpit possession, radio range, delegated operators,
  company authority, and hostile capture are deferred. The UI and documentation
  must identify owner-wide access as temporary rather than as a conveyor rule.

### Compatibility

- P1.4 uses protocol `15`, projection schema `2`, world schema `17`, event
  schema `13`, content schema `10`, and content manifest `p1.4.0`.
- The operation fingerprint schema remains `1`; the protocol version and full
  decoded message already domain-separate new intent shapes.
- Existing direct refine and craft wire intents remain recognized only as
  rejected legacy/development paths after the native client and end-to-end
  scenario use machine queues. They cannot mutate a P1.4 world.
- P1.3 proof saves and journals are archived and reset. No migration invents
  machines, conveyor routes, queue ownership, or job escrow.

## Alternatives considered

### Refine or craft in any inventory

Rejected as the final interaction because it bypasses machinery, power, time,
and physical logistics. It remains useful only as the superseded proof path.

### Give each machine unrestricted internal storage

Deferred. Explicit cargo endpoints keep the first graph, authority, capacity,
and destruction rules small while still requiring physical routing.

### Reserve inputs only when processing completes

Rejected because an accepted queue would not own the resources it promises to
transform. Concurrent transfers could starve or reorder work, and crash
recovery would be less legible.

### Couple production to physics-step outcomes

Rejected because sleeping and background cells do not run full-rate rigid-body
physics. A separate production event permits equivalent active and background
advancement.

### Add cancellation immediately

Deferred. Correct cancellation requires a destination for refunds when the
original source is full, destroyed, disconnected, or transferred. Omitting it
keeps the first escrow state machine conservative.

## Consequences

### Positive

- Refining and manufacturing become physical, time-bearing engineering work.
- Inputs and outputs remain conserved through retries, power loss, crashes,
  splits, blocked capacity, and destruction.
- The scheduler can later run in background cells without changing the job
  state machine.
- Queue privacy extends the existing actor-private inventory boundary.

### Negative

- P1.4 requires coordinated protocol, projection, world, event, content, native
  UI, persistence, and end-to-end changes.
- Owner-wide access and the current power aggregate remain temporary realism
  limits.
- A job cannot be cancelled in this slice.
- Full-face ports are intentionally less expressive than later directional,
  size-specific, or filtered conveyor systems.

## Validation

- Unit tests prove graph connectivity, FIFO bounds, job state transitions,
  integer durations, capacity blocking, and exact delivery.
- Conservation and property tests include every escrow state and machine
  destruction.
- Multiplayer tests prove owner authority and actor-private queue projection.
- Replay tests restart before and after enqueue, power loss, completion, blocked
  output, delivery, split, and destruction.
- Cross-process gameplay mines ore, routes it through refinery and assembler,
  fabricates a component, and constructs a block without a direct production
  mutation.
- Active and synthetic background advancement produce identical production
  events from the same prior state and elapsed tick count.

## Supersedes

The immediate proof-production behavior in P1.1 for P1.4 canonical worlds. It
does not supersede P1.1 actor ownership, P1.2 private projection, or P1.3
operation idempotency.
