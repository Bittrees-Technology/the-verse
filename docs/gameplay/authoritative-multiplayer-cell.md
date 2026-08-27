# Authoritative multi-player cell

**Status:** P1.0 in progress; session boundary and deterministic roster physics verified

This checkpoint targets F-012, SIM-011, and SIM-012. It converts the P0.10 single-pilot proof into a shared authoritative cell without claiming production universe scale.

Protocol 11 completes the first trust-boundary increment: authentication precedes the welcome and world snapshot; a socket is bound to an admitted player or to a read-only spectator role; unknown and concurrently claimed players fail closed; and the simulation receives the bound actor separately from the client intent. World schema 14 now persists one canonically ordered player map and actor-scoped operation namespaces. Event schema 10 stores a canonical player actor on human events, explicit target IDs on automatic life-support events, no impersonated actor on system events, and an ordered outcome for every living roster member in the same fixed-step event as all grids. Each capsule has independent control and life-support scheduling and collides with planets, voxels, and grids while character-to-character collision and locomotion-query occlusion are disabled. Full and motion snapshots include the environment evaluated at every player's own position. Mining and hand-tool grid actions reconstruct the bound actor's canonical eye ray and accept only the closest visible voxel, block, or exact build face within nine metres; the same rule is replay-validated before mutation. Unit tests cover roster ownership and kinematics, atomic two-player live stepping and exact restart recovery, per-player idempotency and input frontiers, canonical outcome order, collision-layer/query behavior, actor-isolated oxygen/death/drop/respawn, closest-hit targeting, and rejection of forged character, lifecycle, or tool outcomes. A fresh loopback development worker now pre-admits `player-local` and `player-remote`; separate sockets bind, advance independent control frontiers, mine using only their own pose, aim, inventory, experience, and career state, and operate only their own suit and recovery state. The native client selects its actor and location-specific gravity from welcome metadata and renders the other engineering suit, while the browser merges and displays the complete roster. Secondary-player refining, manufacturing, transfers, construction, and grid actions fail closed until their ownership rules are converted, so F-012 and SIM-012 are not yet complete.

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
| Mine / build / weld | Closest visible target from actor eye ray; actor must be alive |
| Refine / craft | Actor must be allowed to operate the addressed inventory |
| Inventory transfer | Actor must have withdraw permission on source and deposit permission on destination |
| Grid control / anchor / damage | Explicit grid permission; hand damage also requires the closest visible block; P1.0 development ownership only |

Generic knowledge of an inventory or grid ID is never authority.

## Replication and budgets

Protocol 11 initially sends complete deterministic rosters so correctness is inspectable. The worker limits message size, accepted input rate, queued outbound messages, and consecutive lag recoveries. A client that cannot consume within budget is disconnected and must resume from a new snapshot. The next transport slice introduces spatial interest sets and binary deltas without changing canonical state.

The serialized P0.10 Linux baseline measured a 0.0298 ms median three-cast grounded query set but a 29.8 ms worst-path naive loop for 1,000 characters. Therefore P1.0 must publish 2, 8, 16, 32, and 64 active-player distributions and set a conservative cell budget; thousands of universe participants require multiple cells and reduced-frequency/background modes.

## Acceptance

- Two clients walk independently and observe both authoritative poses.
- The same operation ID can succeed once for each player but duplicate only within that player's namespace.
- Cross-player control and inventory attempts fail without changing event sequence or world hash.
- Shared mining removes one voxel once and credits only the accepted actor.
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

This checkpoint does not claim interest-managed scale, cell transfer, public passkey authentication, PvP hit validation, teams, company permissions, capital safety, offline turret behavior, cleanup scheduling, or browser gameplay.
