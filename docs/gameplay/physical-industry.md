# P1.4 physical refining and manufacturing

**Feature ID:** F-023

**Status:** Implemented and locally/hosted verified for one active simulation
cell; background-cell equivalence pending

**Owner:** Core simulation, worker, protocol, and native-client maintainers

The durable architecture choices are recorded in
[ADR-0018](../decisions/ADR-0018-authoritative-physical-industry.md).

## Linked requirements

- SIM-002 — Server authority
- SIM-007 — Destruction
- SIM-008 — Power
- SIM-011 — Session-bound player authority
- IND-001 — Transformation chain
- IND-002 — Conservation
- CHAIN-002 — Internal inventory
- CHAIN-003 — Lifecycle proofs

## User outcome

A player mines ore, hauls it into cargo, builds and welds a connected refinery,
conveyor line, and assembler, then schedules work that visibly takes time and
requires power. Refined material and components appear only after authoritative
machine work. Power loss pauses a job, disconnected logistics block work, a
full destination retains output safely, and recovery never duplicates or loses
the player's materials.

This is an original implementation of common engineering-sandbox mechanics. It
does not copy third-party source, assets, names, interface designs, or protected
expression.

## Scope

### Included

- Completed cargo, refinery, assembler, and conveyor blocks with six full-face
  ports.
- Deterministic same-grid face-adjacency networks.
- Explicit source and destination cargo inventories.
- One actor-private FIFO of at most 32 jobs per machine.
- Atomic input reservation into canonical job escrow.
- Registered integer-tick production durations and recipe loss.
- One-second canonical production scheduling.
- Existing qualifying-power gate, pause, and resume.
- Output escrow and exactly-once delivery after capacity or route recovery.
- Queue and escrow behavior under damage, destruction, and grid splits.
- Native inventory and production terminal views, with P1.5 delivery limited
  to machines and cargo in the authorized active-cell interest view.
- Exact persistence, replay, idempotency, and conservation evidence.

### Excluded

- Job cancellation, reordering, cooperative queues, or priority controls.
- Machine upgrades, yield modules, speed modules, or alternate recipes.
- Directional, small/large-port, filtered, sorter, tube-pressure, or cross-grid
  conveyor mechanics.
- Stateful fuel use, battery discharge, charging, power allocation, or load
  priority.
- Physical terminal proximity, cockpit possession, radio or antenna range,
  delegated operators, company permissions, and hostile capture.
- Sleeping-cell and offline production execution. The event contract supports
  it, but dynamic cell scheduling must implement it later.
- Browser production management, market custody, contracts, and blockchain
  settlement.

## State authority

The authoritative simulation cell owns:

- functional block and grid topology;
- derived conveyor connectivity;
- machine queues and job identity;
- recipe and content-manifest selection;
- reserved input and pending output escrow;
- elapsed production ticks and job state;
- power qualification;
- inventory capacity and delivery;
- damage, split, destruction, and drop outcomes;
- event order, snapshots, replay, and conservation.

The native client selects a visible authorized machine and cargo endpoints,
submits bounded intents, and renders replicated state. It never advances time,
chooses output, reports connectivity, or writes inventory.

## Data model

### Content definitions

Content schema `10` and manifest `p1.4.0` add:

```text
BlockDefinition
  kind: cargo | refinery | assembler | conveyor | ...
  conveyor_ports: six-face bitset
  power_requirement
  component_cost
  mass_grams

ProductionRecipe
  recipe_id
  machine_kind: refinery | assembler
  input: registered resource quantities
  output: registered resource quantities
  defined_loss: registered resource quantities
  duration_ticks_per_batch
```

All quantities and durations are positive bounded integers. Recipe validation
rejects output-producing cycles that have no registered input, energy, time, or
loss basis. Every queue entry pins `p1.4.0`; replay never silently substitutes a
new recipe.

### Canonical world state

```text
ProductionQueue
  machine_block_id
  grid_id
  owner_player_id
  jobs: ordered list, maximum 32

ProductionJob
  job_id
  operation_id
  actor_player_id
  recipe_id
  content_manifest_version
  batches
  source_inventory_id
  destination_inventory_id
  reserved_inputs
  pending_outputs
  committed_loss
  required_ticks
  completed_ticks
  state: queued | running | paused_power | paused_route | output_blocked
```

`reserved_inputs` and `pending_outputs` are mutually appropriate to the job
stage. Their quantities are included in the canonical conservation view. A job
with completed transformation and blocked output never runs the recipe twice.

Conveyor components are derived from completed block topology. A cache may
record component membership and a topology generation, but canonical replay can
rebuild it from grids and blocks.

## Intents, commands, and events

### Client intents

`queue_production` contains:

- actor-local `operation_sequence` and diagnostic `operation_id`;
- `machine_block_id`;
- `recipe_id`;
- positive `batches`;
- `source_inventory_id`; and
- `destination_inventory_id`.

The first slice has no cancel, reorder, direct-output, or client-advance intent.
The server may retain the old `refine_ore` and `craft_component` message shapes
for diagnostic compatibility, but protocol-15 canonical worlds reject them as
`physical_machine_required` after the native and end-to-end clients migrate.

### Canonical events

Event schema `13` introduces versioned payloads equivalent to:

- `ProductionQueued`: queue identity, pinned recipe/manifest, cargo endpoints,
  reserved inputs, required ticks, and initial state;
- `ProductionAdvanced`: integer elapsed ticks plus the canonical ordered machine
  and job outcomes for one scheduler quantum;
- `ProductionOutputDelivered`: job, destination, exact output, and terminal job
  state; and
- deterministic block-damage handling that moves a destroyed machine's queue
  escrow into one owner-preserving drop. Grid splits retain the queue on the
  unique fragment that contains the machine and re-evaluate its route.

No event trusts client-supplied yields, duration, progress, loss, connectivity,
power state, or inventory delta. Live preparation and replay derive and validate
the same result from the authoritative prior state.

## Permissions and trust boundaries

Queue admission requires all of the following:

1. The connection is bound to a living canonical player.
2. The player owns the machine's grid.
3. The player owns the source and destination cargo inventories through their
   live cargo blocks.
4. The machine and both cargo blocks are complete and on the same grid.
5. A completed full-face conveyor path connects all three blocks.
6. The recipe is registered for that machine kind and content manifest.
7. Batch count, queue capacity, input quantity, arithmetic, and state size are
   within server bounds.

Knowing a block, inventory, job, or queue ID grants no authority. Queues and
escrow appear only in the matching actor-private projection. A spectator and
another player may see the machine block and an intentionally coarse operating
effect but receive no queue identity, cargo handle, recipe quantity, progress,
or escrow amount.

Owner-wide terminal access is a temporary proof rule. It does not mean cargo is
conveyor-connected across grids or at arbitrary distance; queue admission still
requires one physical same-grid network. Later terminal and signal authority
will replace owner-wide access.

P1.5 narrows presentation without changing the production aggregate. Detailed
cargo, mass, machine queue, endpoints, progress, and escrow are projected only
while the corresponding public grid or machine is in the actor's authorized
active-cell interest view. A machine leaving interest clears its actionable
terminal selection and displays `OUT OF LOCAL VIEW`; it does not cancel, pause,
advance, or destroy the canonical queue. A public machine entering another
player's or spectator's view exposes only coarse public operating state.

## Normal flow

1. The player completes cargo, refinery, conveyor, and assembler blocks.
2. The server derives one or more same-grid conveyor components.
3. The player transfers mined ore into a connected cargo inventory.
4. The player queues a refinery recipe with connected source and destination
   cargo.
5. The enqueue event atomically moves input from cargo into job escrow.
6. Each one-second scheduler quantum advances the head job only while the
   machine, route, and qualifying power are valid.
7. At the exact registered duration, the server commits output and defined loss.
8. Output enters the destination once if capacity permits; otherwise it remains
   in output escrow until a later quantum can deliver it.
9. A connected assembler repeats the process for refined material and produces
   components usable by construction.

## Production scheduler

The scheduler cadence is one canonical second. For manifest `p1.4.0`, one second
equals `fixed_step_hz` integer simulation ticks. The event records elapsed ticks,
not a client timestamp or untrusted wall-clock duration.

Within a quantum, machines are visited in canonical grid-ID then block-ID order.
Only each machine's FIFO head may advance. A job never receives more than its
remaining required ticks. Completion, loss, output escrow, and delivery occur at
the deterministic boundary inside that event.

An active cell accumulates authoritative fixed steps and emits this event. A
future background cell may use a durable schedule to emit the identical event,
but it must supply the same elapsed tick count and validate the same prior state.
This slice does not yet claim offline progress.

## Failure, retry, and recovery

- Rejected queue attempts do not reserve input, create a job, consume an actor
  operation sequence, or change the world hash.
- An exact retry returns the original durable receipt and never reserves inputs
  twice. A changed retry at the same actor sequence is a conflict.
- A full queue rejects before input mutation.
- Power loss changes the head state to `paused_power`; completed ticks remain.
- Route loss changes work or delivery to `paused_route`; escrow remains.
- Insufficient destination capacity changes a completed job to
  `output_blocked`; the recipe is not applied again.
- Recovery rebuilds conveyor components, then validates every queue, owner,
  machine, endpoint, manifest, quantity, progress bound, and escrow balance.
- Malformed canonical queue or escrow state halts authoritative writes rather
  than guessing ownership or quantity.

## Damage, split, and destruction

A grid split assigns a machine and its whole FIFO to the unique fragment
containing the machine. Jobs retain their original source and destination IDs;
missing or separated endpoints pause their route. Queue order, progress, and
escrow cannot be duplicated between fragments.

Destroying a machine atomically removes its queue and creates one dropped
inventory owned by the grid's prior owner. That drop contains every job's
reserved inputs and pending outputs. Amounts already recorded as defined recipe
loss do not return. Ordinary cargo destruction follows the same one-owner,
one-location conservation rules already used by canonical cargo drops.

Destroying a conveyor or endpoint does not destroy job escrow. It only changes
route validity unless the machine itself is destroyed.

## Persistence and migration

P1.4 changes every persisted and public boundary:

| Boundary | P1.4 value | Reason |
| --- | --- | --- |
| Client protocol | `15` | Machine blocks, queue intent, and production receipts |
| Projection schema | `2` | Actor-private queues, progress, endpoints, and escrow |
| World schema | `17` | Machine queues, jobs, and conserved escrow |
| Event schema | `13` | Queue, scheduler, completion, delivery, and destruction outcomes |
| Content schema | `10` | Ports, machine definitions, recipe IDs, power, and duration |
| Content manifest | `p1.4.0` | Jobs pin the exact production rules |
| Operation fingerprint | `1` | Existing typed full-message fingerprint remains sufficient |

P1.3 proof worlds, snapshots, and journals are incompatible and must be
archived/reset. No implicit migration may invent machines, routes, jobs, owners,
progress, or escrow. Rollback restores the prior executable and its archived
world; it does not open a P1.4 world under older schemas.

## Security and abuse cases

- Guessing another actor's machine, cargo, queue, or job ID fails closed.
- Cross-grid, diagonal, corner, incomplete-block, and world-proximity paths are
  rejected.
- Excessive batch counts, queue flooding, arithmetic overflow, and oversized
  canonical records are bounded before reservation.
- Reconnect and concurrent intents cannot reorder FIFO jobs or double-reserve an
  input stack.
- Power toggling cannot reset progress, repeat rewards, or reapply a recipe.
- Capacity toggling cannot deliver pending output more than once.
- Split and destruction cannot clone escrow or restore committed loss.
- Public and other-player projections contain no private queue or cargo fields.
- Interest enter, leave, reset, and re-entry cannot leak, duplicate, cancel, or
  advance a private queue or its escrow.

## Economic and conservation impact

For every job transition:

```text
inventory inputs + job-reserved inputs + authorized source
= inventory outputs + job-pending outputs + defined loss + authorized sink
```

Enqueue changes location but not supply. Completion changes registered resource
kinds and records exact defined loss. Delivery changes location but not supply.
Destruction changes location to a drop but does not authorize a source or sink.

Experience is credited once at canonical recipe completion, not at enqueue,
every scheduler quantum, output delivery, or retry. A blocked output is already
completed work and cannot earn the reward again.

## Native inventory and production experience

The engineering terminal retains its original two-pane inventory interaction,
but it must stop describing all owned cargo as physically connected. It shows:

- the suit inventory and selected authorized cargo;
- whether the selected cargo shares a conveyor component with the selected
  machine;
- authoritative capacity, contents, mass, and rejection reasons;
- a functional Production tab with machine, recipe, source, destination, batch,
  queue position, power state, progress, remaining duration, and blocked state;
  and
- pending receipt/reconnect state without optimistic inventory mutation.

The native client removes R/T pocket conversion after queue controls are usable.
Inventory transfer remains an authoritative action; whether a generic transfer
must also follow conveyors is deferred outside this production slice.

Under P1.5, a machine or cargo enter supplies its structural baseline before
private production details. Unrelated structural updates retain the selected
machine's stable client identity. An `out_of_interest` leave disables queue and
transfer controls without presenting destruction; `destroyed` uses the distinct
authoritative drop outcome. Re-entry installs one fresh machine baseline and
one matching private queue, never a second local job list or optimistic replay.

The interface may use familiar engineering-terminal interaction patterns, but
its visuals, layout, words, assets, and audiovisual feedback must remain an
original Verse design.

## Observability

Operators receive bounded metrics and structured logs for:

- queue admission and rejection code;
- jobs by state and machine kind;
- reserved-input and pending-output totals by resource;
- power-paused, route-paused, and output-blocked duration;
- scheduler drift and processed integer ticks;
- conveyor component rebuild count and duration;
- completion and exactly-once delivery counts;
- queue/drop outcomes from split and destruction; and
- recovery validation and conservation failures.

Logs identify universe, cell, grid, machine, job, actor, operation, event,
manifest, and schema versions without exposing private quantities to public
telemetry.

## Acceptance criteria

1. An unfinished refinery, assembler, conveyor, or cargo block cannot route or
   produce.
2. Same-grid face adjacency connects full-face ports; diagonal, corner,
   proximity-only, and cross-grid layouts do not.
3. Queueing without authority, a registered recipe, complete route, source
   input, or queue capacity rejects without any canonical mutation.
4. Accepted enqueue removes exact input once and creates one FIFO job with the
   manifest-pinned recipe and bounded duration.
5. No output or production experience appears before the exact registered
   duration.
6. Power loss freezes completed ticks, and power restoration resumes from the
   same durable value.
7. A full destination retains output in escrow. Later capacity and route
   restoration deliver it exactly once.
8. An exact intent retry returns the original receipt without another job or
   reservation; a changed retry conflicts.
9. Snapshot and journal restart at enqueue, mid-work, power pause, completion,
   blocked output, and delivery recover the same queue, escrow, event sequence,
   conservation view, and world hash.
10. Grid splitting assigns a queue and escrow to only the machine's fragment.
11. Machine destruction creates one conserved drop containing all reserved and
    pending quantities.
12. Another player cannot inspect or operate the owner's queue or cargo, and a
    spectator receives no private production fields.
13. The native Production tab schedules work and renders running, paused,
    blocked, and completed transitions without client-authored inventory state.
14. The cross-process loop mines ore, routes cargo, refines material, assembles
    a component, and builds a block without accepted direct refine/craft intents.
15. Active and synthetic background advancement produce identical production
    events for the same prior state and elapsed tick count.
16. A machine leaving P1.5 interest disables its terminal controls with an
    out-of-view reason while its canonical queue and escrow remain unchanged;
    re-entry restores one exact private queue after the public machine baseline.

## Test strategy

- **Unit:** Port adjacency, connected components, queue bounds, recipe matching,
  durations, power gate, capacity, state transitions, and delivery.
- **Property/invariant:** Conservation includes reserved inputs, pending outputs,
  committed loss, split, destruction, and drops.
- **Integration:** Two-player authority/privacy and complete mining-to-building
  production loop.
- **Replay/idempotency:** Exact and conflicting retries plus restart at every job
  transition.
- **Load/performance:** Dirty graph rebuild and bounded scheduling on published
  grid, machine, and queue envelopes.
- **Security/fuzz:** Malformed graph links, IDs, quantities, job states,
  manifests, progress bounds, and actor projection.
- **Native smoke:** Functional production controls, progress presentation,
  reconnect reconciliation, interest leave/re-entry, stable machine identity,
  and no pocket conversion shortcut.

## Rollout and rollback

1. Land schemas, content validation, world state, and replay tests.
2. Add conveyor derivation, queue admission, scheduler, escrow, and destruction.
3. Add protocol and actor-private projection.
4. Convert native inventory/production UX and both end-to-end clients.
5. Make legacy direct refine/craft intents reject on protocol 15.
6. Reset the local proof universe and run full cross-process recovery evidence.
7. Publish tested machine, graph, and queue operating envelopes.

Steps 1–6 are implemented and pass the repository's complete local verifier,
including Rust unit/integration/replay coverage, two-player authority and
privacy, browser and native smoke tests, and the cross-process
mining-to-building path. Hosted CI and published operating envelopes remain the
release evidence gate. Sleeping/background-cell equivalence remains attached
to dynamic cell scheduling rather than being claimed by this active-cell slice.

Rollback uses the last protocol-14 executable and an archived P1.3 world. P1.4
events and saves are never interpreted by older code.

## Open questions

No unresolved decision blocks this slice. Battery discharge and allocation,
terminal/signal authority, generic conveyor-bound transfers, job cancellation,
and offline scheduling are explicitly deferred features rather than implicit
P1.4 behavior.
