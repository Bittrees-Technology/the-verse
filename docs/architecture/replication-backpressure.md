# P1 replication and backpressure

**Status:** P1.5 local and hosted proof transport verified; production active-
player load and independent client hash evidence pending

## Failure being prevented

The original worker used a 64-message broadcast ring for complete and motion
snapshots. A slow receiver lost its cursor, requested another complete world,
and remained behind while serializing that larger response. Disposable motion
therefore amplified into repeated full-world work.

P1.4 fixed that local failure by retaining one complete structural snapshot
and one newer absolute motion snapshot. It proved bounded coalescing and
structural-before-motion ordering, but every session still receives the whole
cell. That transport is not a production scale or privacy boundary.

## P1.5 view contract

[ADR-0020](../decisions/ADR-0020-spatial-interest-replication.md) defines
protocol `16`, projection schema `3`, and interest schema `1`. The server
derives each audience's visible set from its immutable actor binding or
authorized spectator grant, normalized canonical addresses, deterministic
integer distance bands, dependency rules, and hysteresis. A client cannot
select its authority or widen interest.

Interest is derived network state. Coalescing, congestion, disconnect, or view
membership never changes the canonical journal, physics scene, intent
validation, ownership, inventory, production, or economic state.

The implemented worker publishes a cheap immutable world revision after each
canonical mutation and lazily materializes one shared `ProjectionSource` on
the first session demand for that revision. Exact-address 256-metre spatial
buckets bound ordinary candidate discovery to intersecting buckets, prior
hysteresis members, and actor/support-critical identities. Projection and JSON
serialization occur without holding the authoritative runtime lock. A new
canonical revision invalidates the shared source; concurrent sessions never
share their cursors, private overlays, epochs, or serialized messages.

## Baseline and delta frontier

Each connection owns:

- one opaque session epoch;
- one interest epoch;
- one baseline ID and acknowledged view hash;
- one next contiguous delta sequence;
- one coalesced structural target; and
- one newest absolute motion target.

An `InterestBaseline` is complete for the session's authorized view, not for
the cell. An `InterestDelta` references that baseline, its contiguous sequence,
and the previous view hash. It contains ordered complete enters, absolute
versioned component replacements, and removals only for previously visible
entities. Every removal has exactly one reason: `out_of_interest`, `destroyed`,
or `transferred`. It carries no destination, owner, attacker, inventory, cause,
coordinate, or hidden metadata.

The baseline and delta retain the canonical event/tick frontier and global
canonical world commitment used by authoritative reconciliation. Their
separate deterministic `view_hash` covers only the complete resulting
audience-authorized projection. Subset clients use the view hash to converge
their representation and never interpret the global commitment as a listing or
hash of visible entities.

The client applies a delta only when session epoch, interest epoch, baseline
ID, sequence, and previous view hash all match. A mismatch discards the delta
and requests a current baseline. A client acknowledgement is flow-control
evidence only and cannot attest to or mutate canonical gameplay.

## Per-session retention

State publication remains ordered under the runtime's authoritative mutation
lock, but audience projection happens for the exact connection. Shared state
may retain canonical dirty markers or wholly public registry data; it may not
retain an actor-private serialized message under a cell-wide sequence key.

For each connection:

1. Structural changes, enters, and removals are folded into one latest target
   relative to the acknowledged view.
2. Superseded motion is discarded.
3. Required structure is sent before later motion that assumes it.
4. At most one bounded state message is emitted per replication period.
5. Missed timer periods skip rather than queue.

The worker may recompute one cumulative delta from the acknowledged view to
the latest target. It must not retain or concatenate an unbounded series of
intermediate deltas. A removal cannot be dropped merely because a newer motion
state exists.

## Budget and recovery limits

Configuration sets explicit maximums for retained bytes, visible entities,
serialization time, unacknowledged age, delta size, and baseline size. Content
manifest `p1.5.0` pins gameplay-visible interest bands and cadences; an
operator cannot secretly widen an audience through a congestion setting.

When a bound is exceeded, a hash or epoch mismatches, the policy or anchor
changes discontinuously, or the receiver falls behind the retained frontier,
the worker:

1. discards pending state for the old frontier;
2. increments the applicable interest epoch when required;
3. projects one current audience-safe baseline; and
4. resumes deltas only after that baseline is acknowledged.

It never replays historical baselines in a loop. If the receiver cannot accept
the bounded baseline, the worker closes the connection. Receipts, handshake,
fatal errors, and session revocation use separate bounded control queues so a
motion flood cannot starve them.

Backpressure may lower motion frequency or a versioned presentation-detail
class. It cannot omit authority-relevant state, expose hidden state, expand
interest, alter a canonical tick, or tell the simulation to destroy an entity.

## Structural boundary

The following require a structural component replacement, enter or removal,
or fresh baseline before dependent motion:

- inventory, production queue, escrow, voxel, construction, damage, split, and
  destruction state;
- suit oxygen, life-state, spawn, support, docking, and constraint transitions;
- interest membership and audience-overlay changes; and
- registry, universe-manifest, policy, or epoch changes.

Pure pose and velocity advancement may use motion replacement. A transition
that contains both motion and structural state is structural. An idempotent
retry at an already represented result creates no new state publication.

## Privacy and caching

The P1.5 view hash excludes the global world hash, global event sequence,
hidden entity counts and IDs, and every other actor's private state. The global
frontier and commitment remain separately visible and are a documented
aggregate-activity side channel; P1.5 does not promise traffic-analysis or
zero-knowledge secrecy. An unseen entity creates no ID, removal, count, or
per-entity rejection. Newly visible entities receive only their current
permitted projection, not hidden history.

Dynamic HTTP and WebSocket state is non-cacheable outside the correctly keyed
session pipeline. Session-projected cache keys include audience, session epoch,
interest epoch, baseline, projection version, registry hash, and universe
manifest hash. Projection failure closes with a generic error and never falls
back to canonical state.

## Persistence and upgrade

Replication state is not canonical and is never stored in world schema `18`
or event schema `14`. Restart and reconnect create a new session epoch and
baseline. Upgrade drains protocol `15` sessions before protocol `16` state is
enabled. Rollback drains protocol `16`; packets, baselines, and acknowledgements
are never reinterpreted across versions.

The coordinated P1.5 boundary is protocol `16`, projection schema `3`, world
schema `18`, event schema `14`, content schema `11`, content manifest
`p1.5.0`, registry schema `1`, universe manifest schema `2`, and interest
schema `1`.

## Evidence gates

Existing tests cover 4,096-motion coalescing, structural ordering,
fresh-snapshot recovery, bounded bursts, a 60 Hz send ceiling, actor-private
projection, exact spatial membership, irrelevant-far-entity query independence,
and a local `2`/`8`/`16`/`32`/`64` public-spectator distribution. P1.5 release
acceptance additionally requires:

- exact enter/exit/hysteresis and negative-coordinate membership vectors;
- delay, loss, duplicate, reorder, stale epoch, and hash mismatch recovery;
- no hidden IDs, counts, projected hashes, tombstones, or private overlays
  across two players and one spectator, apart from the documented global
  commitment side channel;
- bounded messages, bytes, work, and baseline rate for a slow consumer;
- structural enter/removal before dependent motion; and
- published Mac and hosted-Linux distributions for `2`, `8`, `16`, `32`, and
  `64` active players plus synthetic nearby entities. The current spectator
  harness does not satisfy this active-player gate.

[Hosted CI run 33112815767](https://github.com/Bittrees-Technology/the-verse/actions/runs/33112815767)
passes the complete Linux replay and packages Linux and Apple Silicon clients
for implementation revision `bb4ab4e`; it does not widen the spectator harness
into an active-player or production-capacity claim.

This is still a local-cell scale slice. A final binary codec, multi-process
cell scheduler, cross-cell handoff, and thousand-participant production result
remain separate evidence gates.
