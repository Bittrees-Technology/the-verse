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
kind, and ordinal. Equal local event sequences in different cells cannot create
the same subject ID. Transfer never renames an existing subject.

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
- All existing EVA, lifecycle, replay, conservation, and client gates remain
  green.

## Non-goals

This decision does not add cross-cell physics, docking, merge/constraint
graphs, cockpit occupancy, anchored structures, voxel transfer, structure
partitioning, arbitrary megastructure support, more proof cells, multi-host
consensus, combat handoff, markets, or public-scale capacity.
