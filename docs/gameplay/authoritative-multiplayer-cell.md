# Authoritative multi-player cell

**Status:** P1.5 local interest-managed multiplayer proof implemented and
verified; production scale gates remain open

This checkpoint targets F-012, SIM-011, and SIM-012. It converts the P0.10 single-pilot proof into a shared authoritative cell without claiming production universe scale. The next delivery contract is [P1.5 fixed celestial registry and interest-managed visibility](celestial-registry-and-interest-management.md); it changes which public entities a session receives without changing actor authority or canonical simulation.

Protocol 12 completes the P1.1 trust-boundary increment: authentication precedes world state; a socket is bound to an admitted player or read-only spectator; and the simulation receives the bound actor separately from the client intent. World schema 15 persists the ordered player map, actor-scoped operation namespaces, grid owners, owner-retaining drops, and one-time anchor eligibility. Event schema 11 stores the player actor on human work, the inventory consumed by construction, exact anchor reward eligibility, explicit lifecycle targets, and an ordered outcome for every living player and grid. Content schema 9 and manifest `p1.1.0` own the corrected non-repeatable reward schedule. Each capsule has independent control and life-support scheduling. Mining and hand-tool grid actions reconstruct the actor's eye ray and accept only the closest visible voxel, block, or build face within nine metres; preparation and replay also resolve inventory and grid authority before mutation. The fresh development worker pre-admits `player-local` and `player-remote`; the starter grid belongs to the local player and the remote player begins without a grid. Separate sockets operate only their own inventory and constructive capabilities, while non-owner closest-hit damage remains possible and reward-free.

## Player-visible contract

1. A signed-in client enters as one server-bound player; a spectator can observe but cannot mutate.
2. The local player and remote players occupy the same authoritative voxel and grid world. P1.4 broadcasts the complete proof cell; P1.5 delivers accepted work and destruction only through each authorized interest view.
3. Walking, EVA, jumping, magnetic support, oxygen, death, inventory, and career state are independent per player.
4. A player cannot move, change the suit mode of, respawn, spend from, or earn credit for another player.
5. Disconnecting does not remove the character or inventory. Controls become neutral after the canonical lease, while gravity, support motion, oxygen, damage, and world time continue.
6. Reconnecting resumes the server-owned movement epoch and received/processed input frontiers rather than resetting them.
7. P1.0 remote characters use an original neutral engineering-suit presentation. Character-to-character collision is disabled until its gameplay and griefing rules are specified.
8. In P1.5, entering replication range creates one stable remote identity, leaving range is distinct from destruction, and re-entry sends one fresh structural baseline without a duplicate actor.

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

Operation ordering and idempotency use `(actor_player_id,
operation_sequence)`. The operation ID is bounded diagnostic metadata and the
server-derived fingerprint detects a changed message at the same sequence.
System events use a separate namespace. A receipt for one player is never
returned to another player even if both choose the same diagnostic operation
ID.

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
| Queue production | Actor ownership of the physical machine grid and source/destination cargo, plus a complete same-grid conveyor route |
| Inventory transfer | Actor access to both source and destination |
| Build / weld | Grid ownership plus closest visible target from the actor eye ray |
| Grid control / anchor | Grid ownership; P1.1 permits remote owner control |
| Damage | Closest visible block; ownership is not required and no experience is awarded |

Generic knowledge of an inventory or grid ID is never authority.

## Replication and budgets

Protocol 15 preserves protocol 14's public deterministic roster and topology plus the bound pilot's exact player, owned inventory, protected-drop, owned-grid-mass, production-queue, and committed-operation-frontier records as an atomic private overlay. Retained receipts, operation IDs, fingerprints, compaction commitments, and every other actor's frontier remain server-private. The shared P1.4 replication feed retains canonical dirty markers rather than session-specific bytes. HTTP and spectator reads cannot select an actor, and dynamic HTTP responses are not cacheable.

P1.5 replaces complete-cell delivery with server-issued session and interest
epochs, a complete authorized baseline, and contiguous deltas containing full
enters, absolute component replacements, and removals. A gameplay observer is
the bound player's canonical position; a public spectator uses only a
server-approved observer cell. Client camera, radius, headers, query parameters,
and payload IDs cannot enlarge authority or disclosure. The control-critical
set always includes the bound player, current locomotion support, controlled
construct, and an accepted interaction awaiting its result. Near players,
grids, drops, and voxel chunks use deterministic spatial queries with a larger
leave radius than enter radius.

Interest state is disposable session state. It cannot alter canonical event
order, simulation, ownership, or world hash. Structural enter precedes motion,
removal distinguishes `out_of_interest`, `destroyed`, and `transferred`, and
re-entry supplies one fresh baseline under the same stable entity ID. Each
subset carries a view hash alongside the canonical event/tick frontier and
global commitment. The global values remain explicit timing/hash side channels,
not proof that an out-of-view entity or field was disclosed. Slow-client
coalescing retains required structural transitions before newer motion. The
complete P1.5 contract and experience evidence are specified in [Fixed
celestial registry and interest-managed
visibility](celestial-registry-and-interest-management.md).

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

P1.5 acceptance additionally requires:

- actor-derived interest with deterministic hysteresis and no client-selected observer authority;
- structural enter before motion, explicit leave reason, and duplicate-free re-entry;
- a support, controlled grid, or accepted pending interaction that cannot be culled by an ordinary view budget;
- out-of-interest public state absent without private-state leakage or canonical mutation;
- unchanged grid and remote-player nodes retained across unrelated structural updates;
- bounded per-session payload as irrelevant cell entities increase; and
- native and browser loading, live, constrained, stale, reconnect, and fatal projection states that fail closed.

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
Protocol-16 sessions now receive bounded interest baselines and deltas rather
than the complete cell. The worker shares one immutable spatial source per
authoritative revision, and local public-spectator distributions through 64
concurrent sessions plus an irrelevant-far-entity regression prove the current
bounded-work slice without claiming active-player or production capacity.

## Not yet included

P1.4 provides physical cargo, conveyors, refinery and assembler queues, power
gating, conserved escrow, and actor-private production state. Grid control is
still owner-authorized remotely without a cockpit, terminal, or signal system.
The implemented P1.5 transport remains JSON and single-cell; independent client
hash verification and the production binary codec remain open. Cell transfer, public passkey
authentication, teams, company permissions, capital safe-zone enforcement,
offline turret behavior, cleanup scheduling, combat sensor rules, and browser
gameplay remain later work.
