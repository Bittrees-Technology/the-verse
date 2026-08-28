# ADR-0016: Session-private state projection

**Status:** Implemented and verified in P1.2

## Context

The canonical P1.1 snapshot is intentionally complete. Sending that object to
an unauthenticated HTTP reader, a spectator, or every authenticated player
reveals carried inventory, cargo contents, protected death drops, economic
totals, and command handles. Mutation authorization does not make disclosure
safe.

Filtering only the inventory vector is insufficient. Player and cargo records
carry inventory IDs, grid mass includes cargo mass, conservation totals reveal
economic quantities, and motion frames disclose every player's input
frontiers and current controls. The worker also retains one structural
`ServerMessage` for every subscriber, so actor-private bytes cannot be placed
in the shared replication feed.

## Decision

Protocol 13 introduces projection schema 1 and distinct network projection
types. Canonical world schema 15, event schema 11, content schema 9, and the
canonical world hash remain unchanged.

Every full network update is one atomic pair:

```text
ProjectedWorldSnapshot       public cell state
ActorPrivateSnapshot?        present only for its bound player
```

Every motion update uses the same split:

```text
ProjectedMotionSnapshot      public poses and grid motion
ActorPrivateMotionSnapshot?  the bound player's input/control acknowledgement
```

The audience is derived only from the completed server-side session binding.
HTTP world reads and spectator sessions have no actor. Query parameters,
cookies, origins, bearer strings, headers, client names, roster order, and
payload IDs cannot select or upgrade an audience.

## Public projection

The public full projection contains cell identity, canonical event/tick/hash,
environment, voxels, public player pose and visible life/locomotion state,
owner-labelled grid topology, public block identity/integrity, and a boolean
conservation-valid signal.

It omits:

- all inventory records and contents;
- carried and cargo inventory IDs;
- protected death-drop records and inventory links;
- player experience and career totals;
- exact suit oxygen and critical thresholds;
- input sequences, leases, and current control vectors;
- cargo-inclusive grid mass; and
- conservation source, live, consumed, or destroyed totals.

The public motion projection contains poses, velocities, visible locomotion,
life state, and grid transforms. It contains no private operation or control
frontiers.

## Actor-private projection

A player overlay is valid only when its player ID equals the immutable session
binding. It is serialized atomically inside the accompanying full projection,
so the outer event sequence commits both views. It contains:

- the actor's complete player snapshot, including their carried inventory ID,
  progression, suit state, and control acknowledgements;
- only inventories for which the canonical strict owner resolver returns that
  actor: carried inventory, cargo on actor-owned grids, and owner-retaining
  dropped inventory;
- only that actor's protected death drops; and
- exact mass only for grids that actor owns.

Collections use canonical stable ordering. Missing, orphaned, duplicated, or
inconsistently linked ownership fails projection. The worker closes the
connection with one generic error and never falls back to serializing the
canonical snapshot.

Spectator JSON omits the private overlay field rather than serializing a null
or an empty actor-shaped object.

## Delivery and caching

Initial handshake, explicit resnapshot, coalesced structural update, refresh
after a replication gap, reconnect, and post-restart delivery call the same
projector while holding one runtime read lock. Shared replication state may
retain only canonical dirty markers or data that is already entirely public.
Actor-private bytes are generated for the exact connection immediately before
delivery and are never stored under a global event-sequence-only cache key.

Dynamic HTTP responses use `Cache-Control: no-store`. Static application
assets remain public and cacheable.

## Hash and timing residual

Protocol 13 retains the canonical world hash so all projections still converge
on the same authoritative sequence and commitment. That hash, event timing,
and observable ship acceleration can reveal that private economic state
changed and may permit low-entropy inference. P1.2 therefore promises private
record and content confidentiality, not traffic-analysis secrecy or a
zero-knowledge commitment. Removing those side channels requires a later
commitment and traffic-shaping design.

## Consequences

- Native and browser clients can no longer read canonical `WorldSnapshot`
  inventory fields from the network.
- Native clears private UI state on disconnect, absent/mismatched overlay, or
  failed resync; a public-only motion frame may preserve a previously valid
  overlay because motion cannot mutate inventory or drops.
- The spectator command center reports `PRIVATE TO PILOT` instead of displaying
  the primary pilot's balances as though they were public.
- Public authentication remains a separate dependency. Local-development
  player binding proves projection mechanics only and is not a production
  privacy claim.

## Rejected alternatives

- **Send the canonical snapshot and trust the UI not to display it.** The data
  has already crossed the authorization boundary.
- **Filter only inventory contents.** IDs, mass, totals, drops, and controls
  remain disclosure channels.
- **Cache one projected snapshot globally.** The first actor's private overlay
  could be replayed to another actor or spectator.
- **Recompute an audience-specific authoritative hash.** That forks the
  convergence commitment and obscures whether projections refer to the same
  canonical state.
