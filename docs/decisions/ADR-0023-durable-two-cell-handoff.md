# ADR-0023: Two-cell assignment and atomic mobile-aggregate handoff

**Status:** Accepted

## Context

P1.5 establishes normalized universe addresses, fixed celestial identities,
and independently verified interest views. P1.6 establishes durable cell
lifecycle, exact background production, renewable same-host leases, and strict
cell fencing for one fixed cell. The accepted product architecture still needs
many dynamically managed cells and cross-cell movement without duplicate
assets or authority.

Moving directly to arbitrary cells, cross-cell collision solving, multi-host
consensus, planet streaming, and partitioned megastructures would combine too
many independent correctness boundaries. The smallest proof that genuinely
advances WORLD-008 and F-013 is one authority transition for an exact mobile
aggregate between two adjacent cells.

The complete behavior and evidence contract is the
[P1.7 durable two-cell handoff specification](../gameplay/durable-two-cell-handoff.md).

## Decision

### Bounded milestone

P1.7 proves two pre-materialized adjacent cells under one local durable
directory. It supports an independent EVA player and an ordinary unanchored
grid whose complete dependency closure fits the tested transfer envelope.

It does not complete general multi-cell placement, multi-host availability,
cross-cell physics/combat, static-structure sharding, planet streaming,
megastructures, routes, frontier expansion, or production scale. Unsupported
large or connected aggregates remain source-authoritative and are never
silently capped, split, deleted, or duplicated.

### Canonical cell key

`CellKeyV1` is the normalized universe/sector/cell portion of
`UniverseAddressV1`. A domain-separated canonical hash derives `cell_id`.
Aliases and worker paths are not identity. Every directory, lifecycle, lease,
world, event, package, and projection frontier binds the exact key and ID.

The proof cells are the existing origin cell and one empty positive-X neighbor.
This fixture proves routing and authority transfer without cloning origin
resources or claiming planet-scale partitioning.

### Directory above cell authority

The durable universe directory owns cell assignments and mobile-aggregate
placements. Each cell retains its independent P1.6 lease and fencing token.

Two fences are required:

1. the cell fencing token protects mutation of one cell journal/snapshot; and
2. the aggregate placement generation protects one mobile aggregate across
   otherwise valid leases for different cells.

A worker must hold both applicable authorities. Cell fencing alone cannot stop
two valid cells from each believing they own the same transferred grid.

On assignment or recovery, the successor opens and exclusively fences the cell
store before the directory advances. The directory permanently records the
mapping from assignment generation to that exact store fence. Transfer proofs
therefore retain the authority generation that actually wrote each cell event,
even after a later holder takes over the cell.

### Durable saga and linearization point

Handoff is a durable saga, not a distributed transaction across the directory
and two cell journals:

```text
Resident(source, N)
  -> Preparing(source locked)
  -> Prepared(destination quarantined)
  -> InTransit(destination, N+1)  [directory CAS commit]
  -> Imported(destination)
  -> Resident(destination, N+1)
```

A pre-commit abort uses a separate proved branch:

```text
Preparing|Prepared
  -> Aborting(source and destination assignments pinned)
  -> Aborted(source restored after both cell cleanup proofs)
```

The directory compare-and-swap commit is the only linearization point. Before
it, recovery may abort and restore source residency. After it, recovery is
roll-forward only to destination import. A source snapshot may retain locked
package bytes for recovery, but placement generation prevents them from being
live assets.

### Complete transfer closure

The source derives traversal from canonical boundary crossing and freezes the
closure at an atomic tick/production boundary. Clients do not select the
destination or package.

An EVA closure includes the player, inventory, life/support state, input and
operation frontiers, and private actor state. A grid closure includes every
block, inventory, installed component, production queue and escrow, ownership,
physics state, lineage, and any player whose support, magnetic attachment, or
pilot authority depends on it.

Anchored grids, cross-closure contacts/constraints/docking, cross-cell systems,
and aggregates outside the proof envelope defer or reject before prepare.

### Content-addressed package and conservation

Transfer schema `1` binds the immutable transfer ID; universe, manifest and
registry roots; source/destination keys and assignment/fencing generations;
prior/resulting placement generation; source event/hash frontier; complete
ordered closure; physics and actor frontiers; conservation vector; and package
hash.

Destination quarantine is durable but not live. The same transfer ID with
different material is fatal. Duplicate identical prepare, quarantine, commit,
import, and finalize operations reconcile idempotently.

Each prepare, quarantine, import, finalization, and abort-cleanup mutation is
also represented by an event-time transfer boundary. Boundaries form a hash
chain anchored by the cell lifecycle record and bind the canonical event
sequence/hash, live store fence, and resulting world hash. The directory may
advance a phase only after validating the matching proof from the cell that
performed it. Recovery may complete the one exact event-to-directory gap; it
cannot invent a proof from the cell's current state.

Prepared export, directory in-transit custody, and committed import are
explicit conservation domains. Transfer cannot create output, loss, reward, or
experience. Canonical IDs are universe-unique and remain unchanged after
movement.

### Production at atomic boundaries

An already claimed P1.6 production occurrence finishes before prepare. Queued
work cannot advance while locked, quarantined, or in transit. The destination
does not copy the source cell's scheduler cursor; after import it re-arms
eligible work one second after the trusted import boundary. This preserves
discrete production state without coupling unrelated cell clocks.

### Cell-independent operation retries

Operation fingerprint schema `2` removes current `cell_id` from the
fingerprint. It remains bound to universe, authenticated actor, schemas,
positive sequence, and exact canonical message bytes. The player's retained
receipt history, compaction commitment, and frontiers move in the package.

An accepted source operation whose receipt is lost before handoff returns the
same receipt afterward; cell routing cannot turn it into a new mutation.

### Transfer-linked session convergence

The authenticated gateway keeps one session epoch across a successful handoff
but pauses routing. Commit/import increments movement and interest epochs,
discards all source view/control state, and sends one destination baseline that
binds the transfer ID, placement generation, destination key, and destination
cell-scoped frontier.

Projection schema `4` and interest schema `2` are required because independent
cell event/tick values are not globally comparable. The official verifier must
commit the destination baseline before controls resume. Source
`transferred` removal derives only from committed transfer evidence.

### Failure direction

- Before directory commit: source recovery or exact abort is legal.
- Abort remains nonterminal until both source and destination cleanup events
  are durable and proved; both assignments stay pinned during that interval.
- After directory commit: only destination roll-forward is legal.
- Uncertain commit result: source unlock is forbidden until the directory is
  read.
- Stale cell fence: all journal, snapshot, projection, and import work stops.
- Stale placement generation: aggregate mutation stops even under an otherwise
  valid cell lease.
- Reconnect: directory placement determines the route; the client never guesses.

### Compatibility

P1.7 uses protocol `18`, projection schema `4`, world schema `20`, event schema
`16`, content schema `11`, content manifest `p1.5.0`, celestial registry schema
`1`, universe manifest schema `4`, interest schema `2`, operation fingerprint
schema `2`, lifecycle-control schema `2`, production-occurrence schema `1`,
cell-directory schema `2`, and transfer/package schema `1`. Directory schema
`2` makes prepare, quarantine, destination import, source finalization, and
both abort-cleanup event/world proofs mandatory before their placement phases
can advance, and retains immutable assignment-generation-to-fence history.

The first proof archives and resets P1.6 data. A later migration must derive
cell keys, install aggregate placement generations, preserve retry conflict
semantics, introduce universe-unique IDs, and prove cross-cell conservation and
replay equality.

## Alternatives considered

### Copy then delete

Rejected. A crash between copy and delete produces two live assets, and a crash
in the opposite order can lose the aggregate.

### Cell leases alone

Rejected. Source and destination can each hold a valid lease for different
cells. Aggregate placement needs its own monotonic generation.

### Distributed transaction across both journals and the directory

Rejected for the proof. It adds blocking coordinator and partial-availability
failure modes without removing the need for durable reconciliation.

### Client-selected destination and acknowledgement

Rejected. Routing derives from canonical position, and clients cannot attest
to authority or conservation.

### Disconnect and create a new player session

Rejected as the target experience. It weakens retry and control continuity and
does not prove transfer-linked view convergence. Reconnect remains a recovery
path, not the ordinary handoff.

### Split every boundary-spanning structure in P1.7

Rejected. Static/dynamic megastructure partitioning has unresolved topology,
contact, conveyor, power, damage, and interior-cell semantics. Unsupported
aggregates remain source-owned without creating a product size cap.

## Consequences

### Positive

- Exactly one explicit authority-transfer point.
- Cell and aggregate fencing address different races.
- Crash direction is deterministic and independently testable.
- Cargo, production escrow, ownership, and operation retries retain exact
  continuity.
- P1.5 view verification and P1.6 lifecycle remain composable.
- The proof advances partitioning without pretending to solve cross-cell
  physics or megastructures.

### Negative

- Protocol, projection, interest, world, event, universe-manifest, operation
  fingerprint, directory, and transfer schemas change together.
- Gateway routing becomes stateful across cell workers.
- Source/destination/directory recovery requires a larger fault matrix.
- The first proof supports a deliberately narrow aggregate envelope.
- P1.6 proof data requires archive/reset unless an audited migration is built.

## Validation

- Cross-platform cell-key and neighbor golden vectors.
- Two-worker assignment and stale-fence races.
- Property tests for exactly-one placement and universe-unique identities.
- Full prepare/quarantine/commit/import/finalize hard-crash matrix.
- Transfer conservation of grids, cargo, queues, escrow, ownership, and lineage.
- Lost operation receipt immediately before handoff.
- Sleeping destination activation and production re-arm.
- Transfer-linked verifier baseline with stale source frame/control rejection.
- Native Mac/Linux handoff presentation and packaged-client gates.
- Explicit unsupported evidence for anchored, externally constrained, and
  oversized structures.

## Supersedes

This ADR refines the six-step handoff outline in ADR-0002 and the universe
simulation document for the bounded P1.7 proof. It does not supersede their
general partitioned-universe direction.
