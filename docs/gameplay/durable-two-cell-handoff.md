# P1.7 durable two-cell assignment and mobile-aggregate handoff

**Feature ID:** F-061

**Status:** Accepted implementation contract; implementation in progress and
release evidence pending

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
If a cell has already durably quarantined the destination package when the
directory begins aborting, recovery first adopts that exact retained
quarantine proof without leaving `Aborting`. Destination cleanup must then bind
the adopted receipt. A source cleanup committed before that late proof was
discovered remains valid because it removed no destination reservation.

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

The dormant tuple now has one shared protocol wire declaration, including the
exact `p1.5.0` content-manifest version. A separate manifest-5 codec derives the
immutable registry, content, lifecycle, address, and frontier roots with the
`the-verse/universe-manifest/v5` hash domain and returns a non-serializable
validation capability. It rejects manifest 4, a manifest-4 document built with
world 21 and event 17, noncanonical bytes, and any rehashed tuple or identity
substitution. This does not activate the tuple: the world-21 envelope's nested
gameplay body must still be migrated from its manifest-4 validation path before
directory-v3 or the world-21 Store may accept the capability.

The core world validator now keeps those contexts disjoint. An active
protocol-18 world still requires schema 20 and its exact manifest-4 identity. A
schema-20 gameplay body nested inside the dormant schema-21 envelope instead
requires the non-serializable validated manifest-5 capability; manifest-4 bytes
cannot cross that gate, manifest-5 bodies fail the active validator, and a
capability for another seed or universe cannot authorize the body. Manifest 5
lives at the neutral simulation layer so neither the core world model nor the
future Store depends back on the handoff subsystem.

The outer schema-21 cell state now has a distinct validation gate over that
context-aware body. It rechecks every closure, production-origin, proof,
conservation, state-hash, and byte-budget invariant only after the exact
manifest-5 capability validates the nested body, then returns a borrowed,
non-serializable state capability. A manifest-4 body, the wrong manifest-5
universe, inner schema 21, outer schema 20, or a resealed cell-identity change
cannot mint it. Existing encode/decode and transaction entry points remain on
their isolated test path until the migration receipt and world-21 Store consume
this capability.

Package-v2 documents and directory-v3 grid/cell authority now expose the same
capability-only binding step. Their ordinary codecs still reject malformed or
unsealed material, while the new step additionally compares universe,
manifest, registry, content, package, and receipt identity with the complete
validated manifest-5 document. A directory or package carrying a well-formed
but arbitrary 64-character trust root cannot mint a manifest-bound authority
borrow. Event-17 sealing and the future Store must consume these bound borrows,
not their unbound historical counterparts.

Event 17 now performs that composite gate. A canonical event must advance one
validated world-21 frontier, carry the exact protocol-19 tuple and manifest-5
roots, bind every embedded package or production occurrence to that manifest,
and match a directory-v3 grid or cell authority that was independently bound
to the same capability. The returned event capability borrows the event,
predecessor state, manifest, and validated context; raw serialized authority
claims and self-consistent hash substitutions cannot create it.

The offline protocol-18-to-19 migration bridge now has a dormant canonical
anchor and receipt codec. It commits the exact source and target compatibility
tuples, manifest roots, terminal directory/archive roots, ordered cell set,
fencing and lifecycle evidence, production/identity mapping roots, and equal
global conservation and normalized-gameplay roots. Every target cell retains
its event-16 `(sequence, head hash)` as a typed legacy predecessor, begins with
an empty event-17 journal at that same frontier, and must accept its first event
17 only at `sequence + 1`. Canonical decoding is bounded and hash-sealed but is
explicitly not an install capability; live source locks, terminal-state proof,
staged target reopening, and policy approval remain required.

The receipt's target lifecycle commitment is now concrete rather than an
opaque hash. Each cell derives one canonical lifecycle-v2 genesis in
`staged_unactivated` mode from the receipt's manifest, migration anchor,
directory-v3 genesis, assignment and fence, trusted cut-off, state and
active-world hashes, retained event-16 frontier, empty event-17 frontier,
production cursor, production-origin root, and identity-subset root. Unknown
fields, noncanonical bytes, a changed target frontier, or any resealed
lifecycle field fail closed.

World-21 snapshots now have a separate bounded canonical encode/decode path
that requires manifest 5 before serialization and after pose hydration on
reopen. The active manifest-4 decoder rejects those bytes, while the world-21
decoder rejects active snapshots, wrong manifests, whitespace aliases, and
resealed schema/identity changes. This is the snapshot boundary the isolated
protocol-19 Store will use; it does not modify the active Store.

The first recovery-only world-21 Store slice now uses a separate
`protocol-19-world-v21` namespace and an exclusive writer lock. Its canonical
identity binds the complete protocol-19 tuple, externally expected cell key,
manifest 5, cell and universe identity, migration anchor and receipt,
lifecycle-v2 genesis, snapshot and active-world hashes, per-cell production and
identity roots, and retained event-16 frontier. A sealed initialization head is
written last after the identity, manifest, lifecycle, snapshot, and empty event
journal. The
caller supplies an already durable per-cell root; the new namespace entry is
then synchronized through that root. A missing head exposes no authority and
initialization may replace only that known precommit debris, while uncertain
head metadata fails closed without cleanup.

The production-compiled staging seam accepts only a non-Serde target
capability minted by matching canonical receipt bytes to an already validated
manifest-5/world-21 state. It rejects a changed route, cell, fence, snapshot,
active-world hash, or legacy event frontier before writing. It remains dormant:
there is no worker entry point or global install head yet, and receipt source
roots are not treated as proven merely because their JSON is internally
consistent. The committed target directory hash also does not prove the
directory document bytes until the offline installer validates that artifact.

Recovery performs bounded reads and revalidates the exact routed cell,
manifest, and world-21 snapshot. The recovery-only constructor still refuses a
nonempty event-17 frontier so no caller can accidentally skip replay. The
history-aware constructor instead streams the pinned event and boundary
journals, resolves the exact historical directory revision and document for
each event, rebinds only an issued successor fence, re-runs the manifest-5
composite gate and trusted dispatcher, and requires the resulting state and
proof boundary to equal the committed head.

The Store now has a test-only live append transaction. It accepts only a
current grid or cell capability borrowed from the locked directory head and
holds that borrow through committed-head synchronization. A pending event head
is durable before journal mutation; the canonical event is synchronized before
its replay-derived boundary, and the committed head is installed last. The
boundary hash chain retains event kind/frontier, exact directory authority,
cell and optional transfer/package identity, proof hash, and resulting state
hash. Any uncertain append poisons that writer until reopen.

Crash recovery yields only the prior or exact successor frontier. A pending
head with no event or a strictly partial event rolls back; one complete event
with no or a strictly partial boundary gets the one deterministic boundary
backfill before commit. Complete mismatches, extra records, unpinned suffixes,
tampered committed bytes, wrong manifests/history, and unterminated evidence
at or beyond its sealed pending range fail closed without cleanup. Test-only
constructors remain separate from the dormant receipt-bound staging seam. A
staged cell is not migration-install authority and does not activate the
protocol-19 tuple.

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
adversarial tests. A private draft-world-21 envelope now persists the complete
closure lock, destination identity reservation, content-addressed quarantine
receipt, and transfer-keyed source or destination abort witness. Its pure
transactions require a read-only authority view derived from a fully validated
directory-v3 document, accept successor worker fences without accepting stale
callers, reconcile quarantine retries after the directory advances, and retain
cleanup evidence for crash recovery even when a side had no lock or
reservation. The directory accepts cleanup only through a dedicated proof that
binds the exact witness hash, source or destination role, live assignment and
fence, nonzero cleanup frontier, package/member roots, quarantine receipt, and
resulting draft-world commitment. Directory v3 now also persists a typed
source-export proof before allowing destination import and requires a distinct
destination-activation proof before source finalization. The persisted export
evidence includes the mutation witness, exact ledger vector, resulting world,
final proof hash, and trusted export time; directory decode reconstructs the
typed cell proof and rejects substituted roots or missing evidence. The package draft now
requires every production job to carry its exact
canonical creation material: universe, creator cell, event sequence, entity
kind, and ordinal. This makes the source-local queue frontier unambiguous after
movement and rejects missing, substituted, or cross-cell-colliding identities.
The draft-world envelope persists the exact origin map and its authoritative
capture path replaces caller-supplied origin material before extraction. A
separate private eligibility map is derived from the exact packaged machine
set and FIFO order. Each record binds its transfer and package, destination
assignment and fence, typed import authority, pinned roots,
production-clock generation, and checked import-time-plus-one-second boundary.
Its pure occurrence decision rejects a changed live queue, pauses work before
that time, and releases the boundary for normal power, route, and capacity
evaluation at the exact eligible time. Raw import-authority construction is
test-only; no production caller can choose an event, fence, or trusted time
until the import transaction derives them from validated directory and cell
evidence. A private draft-world-21 source-export transaction now atomically
removes every frozen closure family, records one checked transfer witness that
includes installed components, advances a draft cell-event frontier, and seals
acyclic mutation, event, resulting-world, and final proof hashes. It rejects
partial state, ledger or witness tampering, proof substitution, overflow, and
later-phase retries without the exact directory-retained final proof hash.
World 21 now also has strict dormant state models for the destination side. A
pending import lock and its per-machine production eligibility records are
part of the active-world commitment, while the completed import record remains
historical and therefore cannot self-commit its resulting active-world hash.
The pending record nests the exact validated quarantine reservation and typed
source-export proof, matches their package, member, receipt, ledger and time
evidence, and binds destination fence, event, lifecycle, conservation,
eligibility root, and mutation witness. The typed import proof then adds the
resulting world and a separate final proof hash. Canonical decode rejects
unknown fields, substituted source evidence, non-monotonic import time, changed
event material, and tampered historical records. The pure materialization
transaction consumes the exact reservation only after validating the live
successor fence, full package-derived ledger vector, authenticated source proof,
world and draft identity conflicts, and monotonic trusted time. It inserts the
complete grid, rider, inventory, operation-history, contact, production FIFO,
and job-origin closure; rebases only derived destination poses; records the
checked import witness; advances one event; and derives the exact machine holds
without advancing the production clock or any job. It then seals the pending
lock, active-world result, typed proof, and historical record atomically, with
an exact no-op retry. Before activation, restart validation requires the exact
pending lock and complete machine-hold set. The pure activation transaction
removes only that pending gameplay lock, advances one event, and seals the
prior/resulting active worlds, exact import proof, trusted time, mutation, and
final proof in historical evidence. At the activation frontier, validation
reconstructs the pre-removal active world; after later events it permits
ordinary motion, damage, rider changes, and a new transfer ID for the same grid.
Production remains independently held: every eligibility binds the exact
import boundary and full packaged queue hash, so even a same-job-ID queue-body
change fails until an authenticated whole-cell occurrence exists. The dormant
production occurrence now derives a canonical decision for every ordered
queue-bearing machine. A schedule before an imported queue's boundary records
an explicit transfer pause, does not evaluate ordinary machine conditions, and
changes no job work state. The first occurrence at or after the boundary removes
every due hold and evaluates its ordinary power, route, capacity, progress,
output, ledger, and reward result in the same atomic vector as unrelated
machines. Accepted trusted time must have reached the schedule, the production
clock advances once, and exact redelivery returns the historical result.
Complete release batches are excluded from the active-world hash but retain
the predecessor snapshots needed to reconstruct and replay the release
frontier. A compact append-only occurrence head and count remain inside that
hash, so an older pause or release cannot be silently deleted after the frontier
moves. Validation partitions each import's original eligibility root between
live records and archived released records, rejecting loss, duplication,
resurrection, queue-body substitution, contradictory lifecycle/quantum identity,
and another handoff while a destination-bound hold remains. Because the boundary
is import time plus one second, background production can release while the
gameplay activation lock is still pending; later activation neither recreates
nor delays it. Dormant directory v3 persists and reconstructs both typed
destination import and
activation proofs, and Imported/Finalized retries must match the local
historical results exactly. A separate dormant source-finalization transaction
now consumes that authenticated chain only after destination activation. It
advances one source event, changes no gameplay or economy family, retains the
export conservation witness, writes a compact active tombstone, and archives
the full import/activation-linked finalization record outside the active-world
projection. Restart validation reconstructs the exact predecessor at the
finalization frontier. Cell-first and directory-first retries are exact, while
a directory-finalized transfer with no local finalization event fails closed.
The private event-17 envelope now binds the complete protocol-19 compatibility
tuple and has distinct typed payloads for prepare, quarantine, export, import,
activation, finalization, side-specific abort, and production occurrence.
All eight draft operations now apply through that canonical envelope. Every
retained proof binds the canonical event and payload hashes; source export also
retains its exact predecessor event and draft-world commitment. Handoff proofs
round-trip through directory v3 with its document identity and complete
assignment-generation-to-cell-fence histories. Validation requires every proof
to use an authority pair the directory actually issued, and rejects a resealed
cross-pair even when its individual numbers are in range. Successor workers use
separate event-free reconciliation transactions, so an already committed event
is never invented again during recovery. The dormant proof-only replay
dispatcher now resolves the exact historical directory revision and compares it
with the event payload; serialized authority is not itself a trust root. The
dormant directory-v3 path now retains every full sealed revision in a canonical
NDJSON hash chain under an isolated protocol-19 namespace. An atomically replaced head
pins its exact entry count, record boundary, revision, document hash, and chain
hash, so deleting a durable suffix fails closed while a valid journal record
that reached disk before its head is safely adopted on restart. Recovery
truncates only an unterminated final record outside the pinned prefix. Exact
historical transfer and assigned-cell capabilities resolve only by revision plus
document hash; stale CAS attempts, revision forks, rewritten predecessors,
complete garbage suffixes, and live directory-v2 filename collisions reject.
The history remains dormant because its head advertises the indivisible
protocol-19 tuple and no active runtime calls it. Its constructors are compiled
only for tests until a validated manifest-5 capability exists; a merely
well-formed manifest hash cannot activate the store.
The dispatcher total-matches all eight event-17 operation kinds, rebinds only a
directory-proven successor lease fence while preserving the gameplay and event
frontiers, and calls only exact-predecessor transactions—never reconciliation.
Grid claims must equal the independently reconstructed transfer capability.
Production events now commit directory revision, document hash, assignment
generation, and fence, and their retained release record and proof bind the same
four values. Canonical-but-fabricated claims, generation/fence cross-pairs,
wrong capability kinds, second application, and successor-document substitution
reject before mutation. A round-tripped old event still resolves its precise
historical authority after later directory revisions. Historical authority is
replay evidence only. Live event sealing and append use the distinct
current-authority borrow from the locked directory head; there is no
historical-to-live capability conversion.
The first live-authority seam is now non-serializable and borrows the locked
directory's exact current head, including its holder, generation, and fence.
That borrow prevents a successor directory commit while an event is being
sealed and durably appended; historical revision lookup cannot construct the
capability. The Store-owned manifest-5 identity gate is exercised again on
replay, including the successor-fence path.
The active event-17 runtime adapter, scheduler and durable wake-up path, the
source-evidence validators and universe-wide migration install head, and
whole-world process-crash integration
remain to be implemented before activation. All drafts are
intentionally unreachable from the production directory-v2/package-v1 paths
until every version in the table above moves in one coordinated activation.
The dormant proof harness retains bounded predecessor projections for replay;
the activated path must persist occurrences through the canonical event journal
and reserve evidence capacity before an import can consume the cell envelope.

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
