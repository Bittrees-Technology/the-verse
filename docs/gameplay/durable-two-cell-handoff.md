# P1.7 durable two-cell assignment and mobile-aggregate handoff

**Feature ID:** F-061

**Status:** Accepted implementation contract; implementation and release
evidence pending

**Owner:** Universe-directory, simulation-worker, persistence, protocol,
replication, native-client, and operations maintainers

The durable architecture choices are recorded in
[ADR-0023](../decisions/ADR-0023-durable-two-cell-handoff.md).

## Linked requirements and features

- WORLD-008 — Partitioned execution, as a bounded two-cell prerequisite
- WORLD-009 — Canonical celestial identity
- WORLD-010 — Stable cell routing
- SIM-002 — Server authority
- SIM-005 — No arbitrary design cap
- SIM-006 — Finite execution budgets
- SIM-011 — Session-bound player authority
- SIM-012 — Multi-player cell
- SIM-013/014 — Server-derived convergent spatial interest
- SIM-015 — Durable cell lifecycle and background production
- SIM-016 — Fenced aggregate placement
- SIM-017 — Conservative atomic handoff
- SIM-018 — Handoff session convergence
- F-011 — Durable snapshot and event recovery
- F-012 — Multiple players in one cell
- F-014 — Fixed canonical celestial registry
- F-023 — Physical industry
- F-059 — Deterministic spatial interest replication
- F-060 — Fenced single-cell lifecycle

P1.7 is a correctness prerequisite for WORLD-008 and F-013. It does not
complete arbitrary multi-cell placement, cross-cell physics, large-structure
partitioning, distributed availability, or the thousand-participant envelope.

## Player outcome

A player in EVA, or piloting an ordinary unanchored grid that wholly fits the
supported transfer envelope, may cross from one adjacent simulation cell into
another without choosing a destination server, reconnecting as a different
identity, duplicating cargo, losing queued production, or observing two live
copies. Controls pause for one bounded authority transition. They resume only
after the destination has imported the exact aggregate and the official client
has independently verified a transfer-linked baseline.

The source view removes the aggregate as `transferred`. The destination view
introduces one complete current representation. A crash, duplicate message,
worker replacement, or lost receipt converges to exactly one authoritative
placement and the original accepted operation result.

This is an original clean-room Verse design. It does not copy third-party
source, assets, names, interfaces, fiction, or protected audiovisual
expression.

## Milestone boundary

### Included

- Exactly two pre-materialized adjacent proof cells in one immutable universe.
- `CellKeyV1`, deterministic canonical cell IDs, neighbor derivation, and
  negative-axis/sector-carry golden vectors.
- One durable local universe directory with dynamic assignment for either proof
  cell and one monotonically increasing assignment generation per cell.
- Independent P1.6 lifecycle, lease, fencing, journal, and snapshot roots for
  each cell.
- Directory-authorized activation of a sleeping destination for an accepted
  traversal; public spectators remain non-waking.
- Handoff of one independent EVA player aggregate.
- Handoff of one ordinary unanchored grid aggregate containing:
  - complete block topology, integrity, ownership, and stable IDs;
  - pose, orientation, linear/angular velocity, mass, and control state;
  - cargo and installed component inventories;
  - physical production queues, reserved input, and pending output escrow;
  - power/conveyor state that is wholly internal to the grid; and
  - build, damage, reward, and provenance lineage.
- A player supported by, magnetically bound to, or piloting the transferring
  grid moves in the same closure.
- Actor life, oxygen, inventory, operation history, movement/input frontiers,
  and private projection state move with the player.
- One content-addressed transfer package, one destination quarantine receipt,
  one directory compare-and-swap commit, and idempotent destination import.
- Source `transferred` removal and destination complete enter/baseline derived
  from committed canonical transfer evidence.
- One authenticated gateway session across handoff with a new interest epoch,
  movement epoch, transfer-linked baseline, and bounded handoff state.
- Cross-process hard-crash, stale-writer, replay, conservation, packaging, and
  native-client evidence.

### Excluded

- More than two proof cells, automated placement optimization, or public-scale
  capacity.
- Multi-host consensus, quorum leases, failover availability, or a production
  control-plane claim.
- Cross-cell collision solving, combat, projectiles, damage, docking,
  constraints, conveyors, power networks, or voxel editing.
- Handoff of anchored/static grids, voxel terrain, death drops, derelicts,
  market custody, or arbitrary constrained assemblies.
- Static-structure sharding, interior cells, partitioned capital ships, or
  dynamic megastructure conversion.
- Frontier materialization, planet streaming, multi-day routes, offline travel,
  jump drives, or gates.
- Background physics, life support, combat, turrets, cleanup, AI, or markets.
- Global spectator cameras, final binary transport, or thousands-player proof.
- Any permanent maximum grid or structure size.

An aggregate outside the P1.7 containment or isolation envelope remains fully
source-authoritative and reports `partition_required` or another specific
retryable unsupported condition. The implementation must never silently split,
delete, freeze forever, or create a second owner for it.

## Canonical cell identity

`CellKeyV1` is the execution address without a local component:

```text
CellKeyV1 {
  schema_version: 1
  universe_id
  sector: [canonical signed i128 decimal; 3]
  cell: [bounded unsigned index; 3]
}
```

The canonical `cell_id` is a domain-separated hash of the exact canonical
encoding of `CellKeyV1`. Display names such as “origin” and “east proof cell”
are metadata, never identity or routing authority. Neighbor calculation uses
the same Euclidean carry rules as `UniverseAddressV1`; equivalent
non-normalized keys are invalid.

Each world, event, lifecycle record, assignment record, transfer record,
package, projection frontier, and lease binds the exact cell key and cell ID.
A worker cannot reinterpret one cell's snapshot under another cell's path or
lease.

The P1.7 fixture contains:

- the existing origin proof cell; and
- one immediately adjacent positive-X cell with no cloned starter assets,
  deposit, planet surface, or authoritative celestial body.

The empty destination demonstrates routing and transfer without claiming
planet-scale cell partitioning. Fixed celestial identity and distant visual
direction remain registry-derived.

## State authority

### Universe directory

The directory owns:

- canonical cell keys, cell assignment generations, and the immutable mapping
  from every assignment generation to the exact cell-store fencing token it
  acquired;
- desired worker placement and assignment state;
- aggregate placement cell and placement generation;
- transfer ID, phase, source, destination, package hash, and commit record;
- source prepare and destination quarantine event/world proofs;
- destination quarantine receipt binding;
- a destination import proof binding the committed import event frontier,
  live fence, placement generation, and resulting world hash;
- a source finalization proof binding the committed export event frontier,
  live fence, placement generation, and resulting world hash; and
- an explicit `Aborting` phase plus source and destination cleanup proofs; and
- the sole compare-and-swap authority-transfer decision.

The directory cannot create gameplay contents or rewrite a package. Its records
are durable operational/canonical-service state above the cell journals.

### Source cell

Before directory commit, the source owns the resident aggregate. It may finish
an already selected atomic physics or production boundary, compute and lock the
complete transfer closure, move the closure into transfer escrow, append the
prepare event, and publish the package. It rejects ordinary mutation of the
locked closure.

After directory commit, the source can only finalize its export tombstone and
package witness. It can never unlock or simulate the old placement.

### Destination cell

Before directory commit, the destination may validate and durably quarantine a
package but cannot project, simulate, consume, schedule, or mutate it. After
commit, only the committed destination placement generation may import the
package. Import is idempotent and replay-validated.

### Gateway session

The authenticated gateway owns transport continuity and routes one immutable
actor session binding. It cannot select transfer contents or placement. It
pauses controls, changes the cell route only after committed import, increments
the interest epoch, and requires a verified destination baseline before
enabling gameplay messages.

### Clients

Clients submit bounded controls and ordinary intents. They may never select a
destination cell, transfer closure, package, placement generation, assignment
generation, directory commit, import acknowledgement, or `transferred`
removal evidence.

## Durable state machines

### Cell assignment

```text
Unassigned/Sleeping
  -> Claiming
  -> Assigned + Activating
  -> Assigned + Active|Background|Draining
  -> Releasing
  -> Unassigned/Sleeping
```

Assignment generation and the P1.6 cell fencing token are separate monotonic
values. Assignment selects which worker may attempt the cell lease. The lease
still protects every cell append, snapshot, production occurrence, and
lifecycle transition. A successor acquires the cell Store lease and its newer
fence before the directory advances assignment generation; the directory then
records that exact generation-to-fence pair permanently. Losing either
authority stops mutation and publication.

### Aggregate placement and handoff

```text
Resident(source, generation N)
  -> Preparing(source locked)
  -> Prepared(destination quarantined)
  -> InTransit(destination, generation N+1)  [directory commit]
  -> Imported(destination)
  -> Resident(destination, generation N+1)
```

Before directory commit, recovery may abort and restore the exact source
resident state. After commit, recovery is roll-forward only. The destination
must finish import; the source can never regain gameplay authority even if its
snapshot still physically contains locked package bytes.

Pre-commit abort is itself a proved saga:

```text
Preparing|Prepared
  -> Aborting (both cell assignments remain pinned)
  -> source cleanup event + destination cleanup event
  -> Aborted + Resident(source, generation N)
```

Both cleanup events are required even when one cell has no lock or reservation;
the no-op event durably proves absence before the directory unpins either cell.

## Transfer trigger and closure

The authoritative source derives a candidate when the aggregate's canonical
reference point crosses its normalized cell boundary. The client supplies only
movement inputs. Boundary equality belongs to exactly one half-open cell, so a
pose cannot route to both.

The source selects the closure at one stable tick boundary. For an EVA player,
the closure contains the player, carried inventory, life/support state, input
queue/frontiers, operation history, and actor-private state. For a grid, the
closure contains the complete grid and any player whose canonical support,
magnetic anchor, pilot control, or carried relationship depends on it.

Preparation rejects or defers when:

- the grid is anchored/static;
- a selected contact, constraint, dock, conveyor, power edge, or ownership
  relationship crosses outside the closure;
- damage, split, construction, or destruction changes the closure while it is
  being prepared;
- the destination cannot admit the bounded package;
- the aggregate does not fit the P1.7 containment envelope; or
- either cell or directory authority is uncertain.

Rejected preparation changes no placement and cannot strand assets in a
client-selected limbo.

## Transfer package

Transfer schema `1` binds:

```text
transfer_id
aggregate_id
prior_placement_generation
resulting_placement_generation
source_cell_key and source_cell_id
destination_cell_key and destination_cell_id
source_assignment_generation and source_fencing_token
destination_assignment_generation
universe/content/registry/manifest roots
source event sequence, event hash, and world hash
prepared_at_tick and canonical boundary address
ordered subject closure
actor operation and movement/input frontiers
grid topology, physics, inventory, production, ownership, and lineage state
conservation vector
package_schema_version
package_hash
```

The package is canonical, content-addressed, size-bounded, and immutable after
prepare. The same `transfer_id` with different bytes, roots, subjects, or hash
is fatal. Duplicate identical preparation, quarantine, commit, import, or
finalization reconciles without a second mutation.

Canonical subject IDs are universe-unique. New ID derivation includes the
universe ID, creator cell key, canonical event identity or sequence, entity
kind, and ordinal. Moving an entity never changes its ID.

## Atomic handoff flow

1. The source detects an authoritative boundary crossing and stops accepting a
   new mutation for the candidate closure.
2. It finishes only an already selected physics/production atomic boundary.
3. It recomputes the closure, validates isolation, performs an exact closure
   compare-and-swap over the player, carried inventory, operation history, and
   referenced production state, records `locked_for_transfer`, appends
   `TransferPrepared`, and synchronizes the immutable package. Unrelated world
   activity and a newer valid source fence do not invalidate the closure CAS.
4. The directory requests destination activation. A sleeping destination
   performs P1.6 production catch-up through its captured cut-off before it can
   accept a quarantine.
5. The destination validates schemas, roots, identity, placement generation,
   conservation, capacity, and absence of conflicting resident IDs. It writes a
   quarantine receipt without activating the aggregate.
6. The directory compare-and-swap verifies the prior placement, both current
   assignments, source prepare, destination receipt, and package hash. It
   commits placement generation `N+1` to the destination. This is the sole
   linearization point.
7. The destination imports through one canonical `TransferImported` event,
   reconstructs physics, re-arms eligible production from the trusted import
   boundary, validates the world, and snapshots. The directory may mark the
   placement imported only after recording the exact destination event/world
   proof.
8. The source appends `TransferFinalized`, retains an audit tombstone/package
   witness, and removes the locked bytes when retention permits. The directory
   may finalize only after recording the exact source event/world proof.
9. The gateway invalidates the source movement/interest route, increments the
   movement and interest epochs, and sends one transfer-linked destination
   baseline.
10. The verifier commits that baseline. Only then may the gateway release
    queued destination controls and ordinary intents.

Every transfer cell event also appends an event-time boundary containing its
event frontier and replay-derived post-state root. Boundary records form a
domain-separated hash chain whose head is committed in durable lifecycle
control. Recovery truncates incomplete tails, backfills only the exact pending
canonical event, and verifies stored directory proofs against the corresponding
cell boundary before using a phase. Historic terminal verification does not
require retaining the package artifact.

No step relies on a distributed transaction across the directory and two cell
journals. Durable reconciliation follows the pre-commit abort/post-commit
roll-forward rule.

## Production and conservation

Transfer occurs only between atomic one-second production occurrences. A
source occurrence already claimed at the boundary finishes before prepare. A
queue in the package cannot advance while locked, quarantined, or in transit.
After import, the destination schedules its next occurrence exactly one second
after the trusted import boundary, without copying the source cell's unrelated
schedule cursor.

The conservation vector counts every movable asset by schema and domain,
including player inventory, cargo, installed components, reserved production
input, pending output, and dropped/lost definitions inside the closure. Source
export, directory in-transit custody, and destination import form one exact
lineage. At every durable phase, the aggregate is exactly one of resident,
locked, in transit, quarantined-but-not-live, or resident at destination.

Per-cell validation treats a prepared export and committed import as explicit
boundary terms rather than unexplained loss or genesis. A directory audit
reconciles package conservation against both cells' event frontiers. Transfer
cannot award experience, career credit, resource genesis, or production output.

## Operation retry across cells

Operation fingerprint schema `2` excludes the current cell ID. It remains bound
to the universe, immutable authenticated actor, protocol and fingerprint
schemas, positive operation sequence, and exact canonical intent bytes.

The player's complete retained operation suffix, compaction commitment,
committed frontier, pending input queue, and movement/input frontiers move in
the package. An operation committed in the source immediately before handoff
returns its original receipt from the destination after a lost response. It can
never execute again merely because routing changed.

The handoff itself is a system operation and does not consume a client
operation sequence. Controls accepted under the old movement epoch after the
prepare boundary reject without advancing the new destination frontier.

## Session, interest, and presentation

Protocol `18` keeps the authenticated session epoch stable across a successful
handoff owned by the gateway. The destination transition:

- increments the interest epoch;
- increments the actor movement epoch;
- discards every source baseline, delta, acknowledgement, verifier stage,
  predicted input, and private overlay;
- identifies the committed transfer and destination cell in a bounded handoff
  control message; and
- requires one independently verified projection schema `4` baseline before
  returning the client to `LIVE`.

Interest schema `2` makes canonical frontiers cell-scoped. A transfer-linked
baseline carries the destination frontier plus the committed transfer ID and
placement generation; it never compares unrelated source and destination event
sequence values as if they shared one journal.

The source `transferred` removal is emitted only from a committed transfer
tombstone. It carries no hidden destination, owner, inventory, or package data
to ordinary observers. The destination emits one complete enter or baseline.
Backpressure may delay presentation but cannot reverse the directory commit or
restore source control.

Client-visible handoff states are bounded and explicit:

```text
LIVE -> HANDOFF_PREPARING -> HANDOFF_IMPORTING -> VERIFYING_DESTINATION -> LIVE
```

Controls are locally neutralized during the transition. Timeout requests
authoritative status; it never guesses a placement or resumes against the old
cell.

## Failure, retry, and recovery

- Crash before synchronized prepare: recover the original source resident.
- Crash after prepare but before directory commit: reconcile destination
  quarantine; either retry commit or abort/unlock the exact source state.
- Crash after a cell event but before its directory proof: recover the proof
  from the cell's chained event boundary, preserving the historical assignment
  generation and fence even after successor takeover.
- Crash after directory commit: roll forward destination import. Source unlock
  is forbidden.
- Crash after import but before source finalization: destination remains the
  only live placement; source finalization is retried from the directory record.
- Duplicate package or command: reconcile by `transfer_id` and package hash.
- Conflicting package under one ID: fail closed and quarantine both workflows.
- Stale source/destination worker: cell fencing blocks journal, snapshot,
  projection, and import; placement generation blocks aggregate mutation even
  under a valid lease for the wrong cell.
- Destination unavailable before commit: source remains locked for a bounded
  interval, then enters `Aborting`; both cell cleanup proofs finish before
  either assignment is released. After commit, the directory must keep retrying
  destination recovery and report an actionable stuck-transfer incident.
- Directory unavailable before commit: no authority transfer occurs. Directory
  uncertainty after a commit attempt forbids source unlock until the durable
  result is read.
- Gateway/client disconnect: canonical transfer continues. Reconnect resolves
  placement from the directory and installs a fresh verified baseline.

Every phase and retry has bounded attempts, bytes, work, and age. An operator
may retry or quarantine a stuck transfer but cannot select a conflicting owner
or edit package contents.

## Permissions and abuse cases

- Boundary crossing is derived from canonical motion; spoofed transforms or
  destination IDs have no effect.
- A player cannot transfer another actor, foreign grid, hidden inventory, or
  unrelated nearby object into its closure.
- A malicious worker cannot import without current cell fencing and committed
  placement generation.
- Package decompression, subject count, block count, inventory entries,
  operation history, and production queues have checked implementation budgets.
- Unknown fields, duplicate IDs, invalid ordering, noncanonical addresses,
  wrong roots, arithmetic overflow, nonfinite physics, and lineage mismatch
  reject before mutation.
- Transfer errors are generic to unauthorized observers and never expose hidden
  entity existence or destination activity.
- Repeated boundary oscillation is rate-bounded by canonical hysteresis and
  cannot create resource, reward, or unbounded retained state.

## Compatibility and migration

The implemented independent-EVA checkpoint uses this indivisible boundary:

| Boundary | EVA checkpoint value |
| --- | ---: |
| Gameplay protocol | `18` |
| Projection schema | `4` |
| World schema | `20` |
| Event schema | `16` |
| Content schema | `11` |
| Content manifest | `p1.5.0` |
| Celestial registry schema | `1` |
| Universe manifest schema | `4` |
| Interest schema | `2` |
| Operation fingerprint schema | `2` |
| Lifecycle-control schema | `2` |
| Production-occurrence schema | `1` |
| Cell-directory schema | `2` |
| Transfer/package schema | `1` |

Universe manifest `4` binds cell-key, directory, transfer, placement, operation
fingerprint, projection, and interest policy commitments. Partial combinations
fail before directory claim, cell recovery, package validation, journal replay,
session admission, or state projection.

The ordinary-grid closure completes under the new boundary recorded by
[ADR-0024](../decisions/ADR-0024-versioned-grid-closure-handoff.md):

| Boundary | Grid-closure value |
| --- | ---: |
| Gameplay protocol | `19` |
| Projection schema | `5` |
| World schema | `21` |
| Event schema | `17` |
| Content schema | `11` |
| Content manifest | `p1.5.0` |
| Celestial registry schema | `1` |
| Universe manifest schema | `5` |
| Interest schema | `3` |
| Operation fingerprint schema | `2` |
| Lifecycle-control schema | `2` |
| Production-occurrence schema | `1` |
| Cell-directory schema | `3` |
| Transfer/package schema | `2` |

Protocol-18 package-v1 artifacts are EVA-only and are never reinterpreted as
grid closures. Upgrade refuses nonterminal transfers; incompatible roots stay
archive/read-only unless an explicit offline migration proves the complete
tuple transition.

Implementation is staged behind that boundary. The private directory-v3 draft
already validates ordered grid-and-rider membership, closure and conservation
roots, package and receipt schemas, historical cell fences, phase-specific
cell-event proofs, atomic document identity, exact face adjacency, and terminal
generation history. A separate private package-v2 draft now derives the exact
unanchored-grid closure from authoritative state and binds grid topology,
integrity, motion and controls, cargo, production FIFO and escrow, owner and
supported riders, their inventories and operation histories, and internal
contacts. Its strict canonical codec, full-collider destination containment,
checked conservation, global subject-identity checks, external-edge rejection,
and source/destination transfer-state conflict guards are covered by golden and
adversarial tests. Both drafts are intentionally unreachable from the
production directory-v2/package-v1 paths until every version in the table
above moves in one coordinated activation.

The first P1.7 proof archives and resets P1.6 data. A later offline migration
must derive canonical cell keys, install one placement generation for every
mobile aggregate, replace cell-bound operation fingerprints without weakening
retained conflict detection, issue universe-unique IDs, and prove cross-cell
conservation/replay equality. Rollback restores matching P1.6 binaries, roots,
directory absence, and archived data together.

## Observability

Operators receive bounded metrics and logs for:

- cell assignment state, generation, holder, lease, and transition duration;
- aggregate placement cell/generation and transfer phase;
- source/destination cell IDs, package hash/size, subject counts, and age;
- prepare, quarantine, commit, import, finalization, abort, and retry latency;
- pre-commit abort and post-commit recovery reason;
- stale cell-fence and placement-generation rejection counts;
- in-transit conservation vector and directory/cell frontier reconciliation;
- destination activation and production re-arm timing;
- gateway handoff, baseline verification, and stale-control rejection; and
- stuck-transfer alerts with no actor-private quantities in public status.

Public APIs may expose generic cell availability and transfer health. They do
not expose private package subjects, inventories, queues, routes, or actor IDs.

### Implemented local gateway checkpoint

The bounded worker can be started in explicit two-cell coordinator mode. A
resident EVA session is pinned to an exact cell key and placement generation;
ordinary projection and mutation never follow a newer directory placement
implicitly. After durable terminal transfer proof, the gateway emits the three
ordered handoff presentation phases and one transfer-linked destination
baseline in the existing session and a new interest epoch. Only an exact
acknowledgement installs the destination route permit.

The live socket regression crosses from the origin proof cell to its eastern
neighbor, proves that snapshot recovery and stale acknowledgement cannot
overtake the linked baseline, rejects a queued old-route control without
advancing the operation frontier, and accepts the identical control after the
destination acknowledgement. A crossing before the initial source baseline
is acknowledged fails closed and requires reconnect. This checkpoint does not
satisfy the grid, opposing-transfer, multi-process, load, packaging, or
retention acceptance gates below.

`tools/e2e/verify-two-cell-handoff.sh` now exercises that flow through the
assembled worker rather than an in-process fixture. It creates a canonical
pilot 25 centimeters inside the east boundary, crosses through ordinary
authenticated thrust, verifies spectator isolation and origin-pinned public
status, stops the process, then reopens the same two-cell roots. Reconnect must
resolve directly to the destination with the same movement epoch, operation
frontier, and carried inventory and without replaying the one-time transfer
link. The durable event decoder has a regression for retained numeric
operation-history keys inside tagged transfer events, so a successful live
handoff that cannot be indexed after restart fails this gate.

Player and public-origin spectator sockets consume independent, cell-scoped
update frontiers. After the crossing, the player follows only the verified
destination baseline while an already connected spectator remains on the
origin feed. The source projection carries worker-owned `transferred` evidence
for a previously visible player; it does not infer destruction and never
exposes the destination or actor-private state. A fresh connection whose exact
directory route was already produced by a retained completion treats that
route as its initial state instead of replaying the completion. Bootstrap
captures route, immutable world, and retained completion in one coordinator
cut, while the origin world and removal evidence travel as one projection
bundle. Timeout recovery, requested snapshots, invalid acknowledgements, and
ordinary replication all defer a typed stale route until its handoff marker.

The local coordinator passes its complete hosted-cell set into authoritative
physics before commit. A player outcome outside that set, or any grid outcome
outside its source cell while grid handoff remains unimplemented, is rejected
before the physics journal append and native physics is rebuilt from canonical
state. The actor therefore remains at the last valid pose. A polished boundary
collision/feedback response and more than the eastern proof neighbor remain
future work; this guard proves containment, not seamless arbitrary traversal.

## Acceptance criteria

1. `CellKeyV1`, cell ID, neighbor, and normalization vectors match on macOS and
   Linux across positive/negative cell and sector boundaries.
2. Two workers racing for one cell produce one assignment/lease winner; stale
   holders cannot append, snapshot, project, prepare, or import.
3. At every durable step, an aggregate has exactly one mutable placement and
   one increasing placement generation.
4. Hard-kill at prepare, package sync, quarantine, directory commit, import,
   snapshot, and finalization recovers exactly one placement.
5. Duplicate, reordered, missing, wrong-root, wrong-cell, wrong-fence,
   wrong-generation, changed-package, and conflicting-ID inputs fail before a
   second mutation.
6. Player/grid resources, installed components, cargo, queues, reserved inputs,
   pending outputs, ownership, rewards, and lineage conserve exactly across
   transfer and replay.
7. Concurrent creation at equal local event sequences in two cells cannot
   create the same canonical subject ID.
8. A lost operation receipt immediately before handoff returns the original
   receipt afterward and never repeats the mutation.
9. A sleeping destination performs bounded P1.6 catch-up before quarantine or
   import; anonymous observation cannot trigger that wake.
10. A transferred production job advances neither twice nor zero times and
    re-arms from the trusted import boundary.
11. Same-session handoff increments interest and movement epochs, accepts one
    transfer-linked destination baseline, and rejects every stale source delta,
    acknowledgement, verifier stage, control, and private overlay.
12. EVA pose, velocity, orientation, oxygen, inventory, support, input queue,
    operation frontier, and life state survive exactly.
13. An unanchored grid and supported rider transfer as one closure with
    identical topology and no artificial impulse.
14. Anchored state, external contact/constraint, damage, split, or construction
    during prepare deterministically defers or aborts without partial transfer.
15. Source observers receive one canonical `transferred` removal and
    destination observers receive one complete current enter, with no ghost or
    false destruction.
16. An oversized or boundary-spanning structure receives `partition_required`
    and is neither deleted, implicitly split, nor assigned two writers.
17. Two cells can perform opposing sequential and independent concurrent
    transfers while directory, journal, projection, and package retention stay
    within published bounds.
18. Native macOS and Linux clients show the bounded handoff state, neutralize
    control during transition, preserve celestial direction/address, and return
    to `LIVE` only after verified destination state.
19. Existing mining, physics, industry, lifecycle, verifier, browser,
    packaging, conservation, and hard-crash suites remain green.
20. Evidence explicitly states that P1.7 proves two-cell local correctness, not
    general multi-cell physics, multi-host availability, megastructure support,
    or production scale.

Criterion 17 remains a release gate rather than a claim of this local proof:
terminal directory records, assignment-fence history, transfer-boundary
journals, and retained world witnesses are not yet compacted. A later
hash-chained archive/compaction milestone must bound them without weakening
historic proof verification.

## Test and evidence strategy

- **Unit:** Cell-key normalization, cell-ID hashing, neighbor carries, placement
  transitions, package ordering, retry classification, and epoch changes.
- **Property/invariant:** Exactly-one placement, universe-unique IDs, transfer
  conservation, operation idempotency, and package hash stability.
- **Negative/replay:** Tamper every package, directory, fence, generation,
  frontier, identity, physics, inventory, production, and lineage field.
- **Fault injection:** Fail before/after every directory and journal sync,
  package/quarantine write, commit, import, snapshot, and finalization.
- **Cross-process:** SIGSTOP/SIGKILL source and destination workers at every
  handoff phase, resume stale workers, and prove roll-forward/abort direction.
- **Client:** Same-session native transfer, lost/duplicate/reordered frames,
  stale controls and ACKs, reconnect during every phase, and tamper rejection.
- **Load/budget:** Opposing transfers, maximum proof-envelope grid/package,
  repeated boundary traversal, stuck transfers, and bounded cleanup.
- **Release:** Full local gate, isolated hosted Linux container, Linux and Apple
  Silicon packages, and two-process crash matrix from assembled artifacts.

## Rollout

1. Freeze ADR-0023, feature/requirements, schema tuple, cell-key vectors,
   directory commit point, and transfer non-goals.
2. Parameterize world, store, lifecycle, and worker startup by canonical cell
   key while preserving the origin fixture.
3. Add the empty adjacent proof cell, local directory, assignment generations,
   and per-cell roots.
4. Add universe-unique subject IDs and operation fingerprint schema `2`.
5. Implement EVA extraction/quarantine/commit/import/finalization and crash
   reconciliation.
6. Extend closure and conservation to ordinary unanchored grids, cargo,
   production escrow, and supported riders.
7. Add protocol `18`, projection `4`, interest `2`, gateway session continuity,
   verifier updates, and native handoff presentation.
8. Run the full crash matrix, packaging, local/hosted gates, and publish bounded
   evidence.

No implementation may claim P1.7 because an object can be serialized from one
world and inserted into another. The claim requires the directory
linearization point, two independent fences, exact closure/conservation,
cell-independent retries, transfer-linked view convergence, crash recovery, and
cross-platform evidence together.

## Open questions

No product decision blocks the bounded two-cell proof. General cross-cell
static topology, external contacts, docking/constraints, cross-cell damage,
interior cells, partitioned capital ships, production multi-host directory
availability, and permanent capacity policy remain later decisions under
OQ-009 and future ADRs.
