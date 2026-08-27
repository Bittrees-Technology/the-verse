# ADR-0015: Bind sessions to deterministic multi-player cell actors

**Status:** Proposed

## Context

P0.10 proves one durable player in one authoritative cell. The worker can host several WebSocket connections, but those connections currently address the same player and share one global operation-id map. That is observation fan-out, not multiplayer, and allowing it on a public server would let any connected client move the pilot or spend the pilot's inventory.

F-012 requires multiple players to share voxel, grid, physics, and event state without weakening the input-only authority established by ADR-0013 and ADR-0014. The later passkey identity service must be able to replace development authentication without changing simulation semantics. Replay must not depend on connection order, socket lifetime, hash-map iteration, or an authentication provider being online.

## Decision

P1 introduces an authenticated `ActorContext` at the worker boundary. After a compatible protocol handshake, the worker validates a short-lived opaque session credential through an authority interface and binds that socket to one immutable `player_id`. The worker passes the bound actor separately from the decoded client intent. A gameplay payload cannot choose or override its actor. Public spectator sessions have no actor and cannot submit mutations.

The first development authority reads explicitly configured, hashed bearer credentials and is disabled unless selected by server configuration. Credentials never enter canonical events, snapshots, logs, receipts, URLs, or repository fixtures. The production identity service will issue equivalent short-lived actor bindings after passkey authentication. Plaintext gameplay credentials require a loopback connection or TLS-terminating trusted gateway; the worker rejects an unsafe public development-auth configuration.

Protocol 11 makes the actor boundary explicit. `hello` carries authentication material in a dedicated envelope, `welcome` identifies the bound local player or spectator role, and every full or motion snapshot contains a deterministic player roster. The local-player identity is connection metadata rather than world state and is never included in the world hash. Clients select their controlled player using the `welcome` binding; they render other roster members as remote actors and never infer local authority from roster order.

World schema 14 replaces the single player with a player map keyed by canonical ID. Each player owns an independent movement epoch, received and processed input frontier, bounded control FIFO, inventory, career, oxygen, life state, and respawn state. Inventory domains identify their owner. Canonical iteration uses ordered player IDs. Player rigid-body and collider IDs are derived from an unambiguous encoded player ID and cannot collide with grid or voxel IDs.

Event schema 9 records `actor_player_id` on every player-generated event and stores a vector of player physics outcomes in canonical player-ID order. System events have no actor. Idempotency is scoped by `(actor_player_id, operation_id)`, preventing one player's chosen operation ID from replaying another player's receipt. Replay verifies that an actor could affect only their own controls, suit, respawn, carried inventory, range origin, and career credit at the prior event state.

All living players occupy the same Jolt scene and are advanced in the same fixed-step event as grids. P1.0 deliberately disables player-to-player collision using collision layers while retaining player collision against planets, voxels, and grids; character collision and pushing remain a later evidence checkpoint. Each player's locomotion probes ignore only that player's own body. Work actions measure range from the bound player's capsule. Shared-grid commands require a separate permission decision and are initially denied except for the existing development owner.

Complete JSON rosters remain the P1.0 correctness transport. The worker applies a bounded connection send queue, rate limit, and slow-consumer disconnect policy. Interest-managed binary deltas are the next P1 transport slice and must preserve the same actor, ordering, and snapshot semantics.

Joining creates a player only through a separate server-authorized admission operation; sending `hello` cannot mint a canonical player. Disconnecting neutralizes that player's controls through one canonical system event after the configured lease and leaves their body, inventory, and life state durable. Reconnect receives the persisted epoch and input frontiers. Concurrent sessions for one player are rejected unless a future explicit session-takeover policy revokes the earlier binding first.

## Consequences

- Socket fan-out can no longer grant shared control of one pilot.
- Authentication providers remain outside deterministic replay while their resolved actor identity is durable and auditable.
- Global event order remains simple, but active-player physics cost grows approximately linearly until interest and cell budgets are introduced.
- Existing P0 worlds require an explicit one-player migration or a fresh P1 universe; silent schema coercion is forbidden.
- Protocol 10 clients fail compatibility negotiation instead of accidentally receiving or controlling a protocol 11 roster.

## Required evidence

- Two authenticated clients receive distinct local bindings and the same ordered roster/world hash.
- Each client can advance only its own movement epoch and input sequence; spoofed actor fields and another player's epoch fail closed.
- Duplicate operation IDs are idempotent per actor and independent across actors.
- Mining, refining, crafting, transfer, build, suit, death, and respawn actions use the bound actor's range, inventory, life state, and career.
- Delayed, duplicated, and out-of-order controls for one player do not alter another player's frontier.
- Disconnect neutralizes only the disconnected player; reconnect resumes the exact durable frontier.
- Journal replay, snapshot recovery, graceful restart, and injected append failure preserve every player and the exact world hash.
- One accepted fixed step commits every living player and grid atomically in canonical order.
- A two-client WebSocket test proves isolation, shared-world observation, and exact recovery.
- A published active-player benchmark defines the safe per-cell full-rate envelope before public load testing.

## Deliberate limits

P1.0 does not complete passkey enrollment, smart accounts, character-to-character collision, rollback or lag compensation, binary replication, interest management, cell handoff, social permissions, teams, company roles, combat ownership, or public deployment authentication. Those systems build on the actor binding and roster contract rather than bypassing them.
