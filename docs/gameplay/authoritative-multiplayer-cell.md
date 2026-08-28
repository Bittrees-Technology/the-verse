# Authoritative multi-player cell

**Status:** P1.2 local proof verified; public identity and scale remain deferred

This checkpoint targets F-012, SIM-011, and SIM-012. It converts the P0.10 single-pilot proof into a shared authoritative cell without claiming production universe scale.

Protocol 12 completes the P1.1 trust-boundary increment: authentication precedes world state; a socket is bound to an admitted player or read-only spectator; and the simulation receives the bound actor separately from the client intent. World schema 15 persists the ordered player map, actor-scoped operation namespaces, grid owners, owner-retaining drops, and one-time anchor eligibility. Event schema 11 stores the player actor on human work, the inventory consumed by construction, exact anchor reward eligibility, explicit lifecycle targets, and an ordered outcome for every living player and grid. Content schema 9 and manifest `p1.1.0` own the corrected non-repeatable reward schedule. Each capsule has independent control and life-support scheduling. Mining and hand-tool grid actions reconstruct the actor's eye ray and accept only the closest visible voxel, block, or build face within nine metres; preparation and replay also resolve inventory and grid authority before mutation. The fresh development worker pre-admits `player-local` and `player-remote`; the starter grid belongs to the local player and the remote player begins without a grid. Separate sockets operate only their own inventory and constructive capabilities, while non-owner closest-hit damage remains possible and reward-free.

## Player-visible contract

1. A signed-in client enters as one server-bound player; a spectator can observe but cannot mutate.
2. The local player and every remote player occupy the same voxel and grid world and see the same accepted work and destruction.
3. Walking, EVA, jumping, magnetic support, oxygen, death, inventory, and career state are independent per player.
4. A player cannot move, change the suit mode of, respawn, spend from, or earn credit for another player.
5. Disconnecting does not remove the character or inventory. Controls become neutral after the canonical lease, while gravity, support motion, oxygen, damage, and world time continue.
6. Reconnecting resumes the server-owned movement epoch and received/processed input frontiers rather than resetting them.
7. P1.0 remote characters use an original neutral engineering-suit presentation. Character-to-character collision is disabled until its gameplay and griefing rules are specified.

## State and trust boundary

The identity/session service authenticates a credential and returns an immutable actor binding. The simulation receives only the actor ID, role, scopes, expiry, and audit correlation ID. It never persists credentials. The cell owns the ordered player roster and all player state. Client messages carry requested actions, not actor identity or outcomes.

Mutating execution is conceptually:

```text
execute(actor_context, intent)
  -> authorize actor and scope
  -> validate against actor-specific prior state
  -> prepare actor-labelled canonical event
  -> apply to a cloned world
  -> append and synchronize event
  -> publish world and actor-scoped receipt
```

An operation key is `(actor_player_id, operation_id)`. System events use a separate namespace. A receipt for one player is never returned to another player even if both choose the same operation ID.

## Deterministic ordering

- Player maps and replicated rosters are ordered lexicographically by canonical player ID.
- The server queues controls independently per player.
- Within one fixed substep it consumes at most one queued transition per player, ordered by player ID, stages every player and grid result, and commits one physics event.
- Physics outcome vectors, contacts, inventories, death drops, and snapshots use stable ordering before hashing or serialization.
- Life support advances in canonical player-ID rounds. Simultaneous deaths receive unique sequence-derived IDs, and one player's death cannot clear another player's controls, contacts, inventory, or shared-grid input.
- Join, disconnect neutralization, and session revocation are explicit canonical events, never inferred during replay from socket timing.

## Admission and reconnect

Player creation is an authenticated universe-service command with a unique canonical ID, starter inventory, spawn selection, and audit record. A gameplay `hello` can bind only to an existing admitted player. P1.0 test fixtures may pre-admit two deterministic players, but the public worker must not auto-create a player from a client-supplied string.

Only one gameplay session controls a player at a time. A second session receives `player_already_connected`. An orderly disconnect or expired session schedules control neutralization; an unclean network loss relies on the existing bounded input lease. A replacement session can bind after the old lease is revoked or expires and receives the persisted input frontiers.

## Authorization rules

| Intent | Actor-owned validation |
| --- | --- |
| Character control | Actor movement epoch and actor FIFO |
| Suit mode / respawn | Actor life and suit state |
| Mine | Closest visible target from the living actor's eye ray; yield enters actor inventory |
| Refine / craft | Actor access to the addressed player or owner-derived cargo inventory |
| Inventory transfer | Actor access to both source and destination |
| Build / weld | Grid ownership plus closest visible target from the actor eye ray |
| Grid control / anchor | Grid ownership; P1.1 permits remote owner control |
| Damage | Closest visible block; ownership is not required and no experience is awarded |

Generic knowledge of an inventory or grid ID is never authority.

## Replication and budgets

Protocol 14 preserves protocol 13's public deterministic rosters and topology, then adds only the bound pilot's exact player, owned inventory, protected-drop, owned-grid-mass, and committed-operation-frontier records as an atomic private overlay. Retained receipts, operation IDs, fingerprints, compaction commitments, and every other actor's frontier remain server-private. The shared replication feed retains canonical dirty markers rather than session-specific bytes. HTTP and spectator reads cannot select an actor, and dynamic HTTP responses are not cacheable. Spatial interest sets and binary deltas remain later scale work and must not change canonical state.

The serialized P0.10 Linux baseline measured a 0.0298 ms median three-cast grounded query set but a 29.8 ms worst-path naive loop for 1,000 characters. Therefore P1.0 must publish 2, 8, 16, 32, and 64 active-player distributions and set a conservative cell budget; thousands of universe participants require multiple cells and reduced-frequency/background modes.

## Acceptance

- Two clients walk independently and observe both authoritative poses.
- The same sequence and exact message return one actor's original receipt; a changed message conflicts, while another actor has an independent frontier.
- Cross-player control and inventory attempts fail without changing event sequence or world hash.
- Shared mining removes one voxel once and credits only the accepted actor.
- Non-owner production, transfer, construction, welding, grid control, and anchoring fail closed.
- A non-owner can damage a closest-visible block without receiving experience or ownership.
- Disconnect/reconnect, snapshot recovery, and event replay preserve both players exactly.
- The native client identifies the local actor from connection metadata and renders at least one remote actor.
- Automated two-client impairment tests cover delay, duplication, reordering, lag recovery, and stale epochs.
- The active-player benchmark and tested cell envelope are published for Apple Silicon and hosted Linux.

Current evidence additionally runs the native visual/control smoke once as each
development identity and verifies identical shared-state recovery after the
mining, refining, manufacturing, construction, destruction, death, and respawn
scenario. The browser unit harness proves deterministic roster ordering,
identity-based motion merging, and selected-pilot environment fallback. A
concurrent two-socket scenario submits the
same operation IDs from both actors, verifies independent processed frontiers,
aims and mines one shared voxel as the remote actor, proves actor-only ore and
career credit, retries idempotently, and converges both clients on the same
hash. The full scenario derives construction coordinates from exact hit faces
and proves that an intentionally occluded damage request cannot mutate state.

## Not yet included

Refining and crafting remain immediate recipe proofs without machines, power,
queues, duration, or conveyor paths. Grid control is owner-authorized remotely
without a cockpit, terminal, or signal system. Complete snapshots expose the
proof cell's inventories and do not claim confidentiality. This checkpoint
also excludes interest-managed scale, cell transfer, public passkey
authentication, teams, company permissions, capital safe-zone enforcement,
offline turret behavior, cleanup scheduling, and browser gameplay.
