# ADR-0024: Versioned atomic grid-closure handoff

**Status:** Accepted

## Context

ADR-0023 defines the complete P1.7 mobile-aggregate outcome. The first durable
implementation checkpoint proves only an independent EVA player transfer. Its
serialized package, receipt, world locks, journal variants, directory record,
and client transfer link are concretely player-specific even though the product
contract also requires an ordinary unanchored grid and its dependent riders.

Changing those published version-1 structures in place would allow two
incompatible meanings to claim the same compatibility tuple. A grid is also
not one isolated world record: its closure includes blocks, cargo, production
queues and escrow, active internal contacts, and every player whose canonical
support depends on the grid.

## Decision

### New compatibility boundary

The atomic grid-closure slice uses protocol `19`, projection schema `5`, world
schema `21`, event schema `17`, universe-manifest schema `5`, interest schema
`3`, cell-directory schema `3`, and aggregate-transfer package schema `2`.
Content schema `11`, content manifest `p1.5.0`, registry schema `1`, operation
fingerprint schema `2`, lifecycle-control schema `2`, production-occurrence
schema `1`, and cell-key schema `1` remain unchanged.

Protocol-18 transfer-package-v1 artifacts remain independent-EVA artifacts.
A protocol-19 runtime never reinterprets them as grid closures.

### One server-derived closure

The first supported grid closure contains exactly:

- one unanchored grid wholly contained by the destination cell;
- every block, including topology, integrity, installed component count,
  ownership, controls, motion, and lineage state;
- every block-linked cargo inventory;
- every production queue whose machine is on the grid, including FIFO order,
  reserved input and pending output;
- the grid owner and every grounded or magnetic player supported by the grid,
  including each carried inventory and operation/movement frontier; and
- only active contact identities whose bodies are both closure members.

Power and conveyor graphs remain derived from complete blocks. Native physics
bodies, broad-phase caches, and solver impulses remain disposable and are
rebuilt at the destination.

The bounded first slice requires the owner to be a supported rider when the
grid has queued production. Cockpit/pilot occupancy, docking, constraints,
merge blocks, external power or conveyors, anchored grids, and cross-cell
collisions remain unsupported because those relationships are not yet
canonical model records.

### Bundled placement compare-and-swap

The grid and every rider retain separate durable placement generations, but
the directory advances all of them in one document compare-and-swap. The
transfer record binds the ordered member set and every prior/resulting
generation. Any missing, added, stale, duplicated, or differently ordered
member rejects before the commit point.

The directory record also binds package schema, aggregate kind, closure root,
conservation root, package hash, and receipt schema. Cell fencing and bundled
placement fencing are both required for prepare, quarantine, import, export,
projection, and recovery.

### Closure lock and production boundary

The source captures the closure after any already selected physics and
production quantum. One generic aggregate lock freezes grid control, physics,
build, weld, damage, split, cargo transfer, production enqueue/advance, and all
rider mutation. A changed closure causes deterministic retry or pre-commit
abort; it cannot produce a partial package.

Imported queues do not inherit or reset the destination cell's production
clock. Each imported queue receives a durable eligibility boundary no earlier
than one second after the trusted import time. Unrelated destination work keeps
its existing schedule.

### Universe-unique creation identity

New block, cargo, production-job, split-grid, and transfer-member identities
bind at least universe ID, creator cell ID, canonical event sequence, entity
kind, and ordinal. The ordinal is assigned independently within each entity
kind, in sorted legacy-ID order, across every creation of that kind in the
event before terminal-state filtering. Equal local event sequences in different
cells cannot create the same subject ID. Transfer never renames an existing
subject, and a disappeared legacy identity cannot be reused.

### Containment and rejection

Containment evaluates every rotated block-collider corner and every rider
collider against the destination's canonical half-open cell interval. Checking
only the grid reference point or center of mass is insufficient.

Anchoring, partial overlap, an external contact/support/system edge, an absent
owner dependency, conflicting destination ID, stale generation, changed
closure, or resource-bound excess returns a typed unsupported or retryable
result before prepare. The source remains authoritative and unchanged.

### Conservation

The v2 package and receipt bind checked totals for cargo, rider inventories,
installed components, reserved production input, pending production output,
capacity, volume, and mass, plus topology and subject counts. Transfer changes
no reward, experience, career, production, or destruction ledger.

### Client convergence and privacy

The v3 transfer link binds aggregate ID, aggregate kind, closure root, package
hash, destination cell key, and resulting placement generation. The official
verifier must commit that exact destination baseline before rider controls
resume.

Public source observers receive canonical transferred removals without cargo,
queue, escrow, exact mass, package, closure, destination, or private rider
material. Destination observers receive one complete current public grid and
public riders. Actor-private state remains visible only to its authorized
player.

### Dormant world-21 Store initialization

Protocol-19 cell state uses the isolated `protocol-19-world-v21` namespace;
the active protocol-18 Store never reads it. Initialization holds one exclusive
writer lock and persists a canonical identity, manifest-5 document, world-21
snapshot, immutable lifecycle-v2 genesis, empty event-17 journal, and a sealed
initialization head. The lifecycle genesis is derived from the canonical
migration receipt. It binds staged-unactivated mode, the exact directory-v3
genesis, assignment and fence, trusted cut-off, target state and active-world
hashes, retained event-16 predecessor, empty event-17 frontier, production
cursor root, production-origin root, and identity-subset root. Its hash must
equal the receipt's target lifecycle commitment. The initialization head is the
per-cell commit marker and is written last. It binds the complete protocol
tuple, identity, manifest, migration anchor and receipt, lifecycle genesis,
snapshot, and zero-length event frontier.
The caller must supply an already durable per-cell root. The namespace
directory is committed by synchronizing that root, and each authority
replacement is synchronized before the head is installed.

Recovery requires an externally routed cell key and verifies its canonical
cell ID against both identity and snapshot. Moving a complete, otherwise valid
cell namespace onto another cell's route fails closed. A missing head grants no
authority; while holding the writer lock, initialization may remove only the
known precommit authority files and their temporary files, synchronize that
cleanup, and retry. An I/O or metadata error while checking the head is not
absence and fails closed without cleanup. Once a head exists, initialization
never overwrites it and ordinary recovery must validate every bounded
canonical file. A non-Serde staging capability is minted only when a canonical
receipt identifies the exact validated manifest-5/world-21 state, cell route,
active-world root, fence, and legacy frontier. The Store stages through that
capability rather than accepting an arbitrary anchor hash. This proves the
target cell binding, not the source archives or authority to activate it.

After initialization, a separate atomically replaced event head owns the
mutable event-17 frontier. It binds the initialization head, identity,
manifest, cell, retained event-16 predecessor, committed event count and hash,
resulting state hash, event-journal byte boundary, and an equally counted
event-boundary journal. Each event-boundary record is replay-derived and binds
the exact directory revision and document, event and payload hashes, cell,
transfer/package identity when applicable, proof hash, resulting state hash,
and prior boundary hash.

Append accepts only a non-serializable current authority borrowed from the
locked directory head. It validates and applies the event against the same
manifest-5 capability held by the Store, persists a pending head first, then
synchronizes the canonical event and replay-derived boundary before replacing
the committed head. Any uncertain write poisons the open writer until recovery;
a failure before the first head write remains retryable.

Recovery streams bounded records and reconstructs each committed successor
through the exact historical directory revision, manifest-5 gate, and trusted
dispatcher. A pending head with no event or a strictly partial event rolls back
to the committed frontier. One complete pending event deterministically
backfills an absent or strictly partial boundary and commits. Tampered prefixes,
wrong history, unpinned or excess suffixes, and data at or beyond a sealed
pending range fail closed without truncation. The receipt-bound staging path is
compiled as dormant code but has no worker, runtime, or public entry point.
The source-side validator now acquires the existing directory-v2 lock followed
by existing cell writer locks in canonical cell-ID order and mints one
non-Serde frozen-source capability. It requires sleeping assignments and
lifecycles, terminal transfers, exact full event replay, canonical bounded
archives, matching lifecycle/snapshot/production frontiers, issued historical
fences, and exact directory-to-cell transfer proofs. Validation is read-only:
it does not create, truncate, backfill, heal, advance a fence, or sample trusted
time. A second non-Serde capability now borrows that source and performs the
write-free identity and production-origin transform. Creation provenance comes
from event replay, every typed live reference is rewritten, each target is
validated under manifest 5, and canonical mapping blobs plus independently
equal conservation and inverse-normalized gameplay roots are derived. The
source-bound receipt and prepared installer now copy the frozen directory and
canonical mapping artifacts, derive directory-v3 genesis, stage or strictly
reopen the complete world-21 cell set, and write one universe commit head last.
Without that head, partial target files grant no installed authority. With it,
foreign, hybrid, swapped, missing, extra, or source-mismatched material fails
closed. The runtime scheduler/wake path, signatures, and coordinated
protocol-19 activation remain required.

Rollback removes the entire unactivated protocol-19 namespace or restores it
as one matching compatibility set. Copying individual identity, manifest,
snapshot, journal, or head files between cells or protocol versions is not a
supported rollback.

## Compatibility and migration

Upgrade is refused while either proof cell contains a nonterminal protocol-18
transfer. Operators must complete or abort it with the matching protocol-18
binary first.

Earlier worlds, manifests, journals, directories, verifier vectors, and
transfer artifacts fail closed and remain archive/read-only unless an explicit
offline migration proves the complete transformation. A terminal v1 player
transfer may be migrated only with a receipt binding the old and new hashes and
tuples. An in-flight transfer is never transformed.

Rollback requires matching binaries, manifest, directory, both cell
checkpoints, journals, and retained artifacts as one compatibility set. After
the placement commit, restoring only the old source cell is forbidden.

## Consequences

### Positive

- Player-only artifacts keep one immutable meaning.
- Grid, cargo, production, and riders share one authority transition.
- Two cells cannot create colliding subjects at equal local event sequences.
- Production and conservation have explicit handoff boundaries.
- Client evidence identifies the transferred aggregate, not only a session.

### Negative

- The version tuple and portable verifier vectors move together again.
- Directory commit and recovery must handle a bounded member set atomically.
- Every mutation and simulation path needs aggregate-lock awareness.
- The first ship envelope remains intentionally narrower than general
  multi-cell capital-ship simulation.

## Validation

- Canonical v2 package/hash vectors on macOS and Linux.
- Exact grid, blocks, cargo, queues, escrow, contacts, owner, riders, and
  operation-frontier round trip.
- Bundled placement races and every prepare-to-finalize crash boundary.
- Anchored, partially contained, externally connected, stale, conflicting,
  oversized, and tampered closures reject before mutation.
- Production advances exactly once around prepare/import/restart.
- Rebuilt destination physics adds no artificial impulse.
- Source/destination spectator privacy and verified rider-session convergence.
- Event-17 append/replay under manifest 5, including a valid successor fence,
  every pending/event/boundary/head crash point, poisoned-writer recovery,
  exact boundary backfill, overlong-tail preservation, and tamper rejection.
- All existing EVA, lifecycle, replay, conservation, and client gates remain
  green.

## Non-goals

This decision does not add cross-cell physics, docking, merge/constraint
graphs, cockpit occupancy, anchored structures, voxel transfer, structure
partitioning, arbitrary megastructure support, more proof cells, multi-host
consensus, combat handoff, markets, or public-scale capacity.
