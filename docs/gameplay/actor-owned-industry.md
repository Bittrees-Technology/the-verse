# Actor-owned industry and engineering

**Status:** Implemented and verified local P1.1 proof

## Player promise

Two players can work in the same simulation cell without either player being
able to spend the other's carried materials, operate the other's cargo, or
change the other's ship by guessing an inventory, block, or grid ID. Ownership
is checked from the session-bound actor and the authoritative prior world. It
is never selected by a client payload.

P1.1 completes the actor-authority conversion for the existing proof industry
loop. It does not yet claim production machinery, company permissions, private
inventory replication, safe-zone enforcement, or the public-universe identity
service.

## Version boundary

This authority contract is intentionally incompatible with the P1.0 save and
wire formats:

| Boundary | P1.1 value | Reason |
| --- | --- | --- |
| Client protocol | `12` | Grid snapshots expose their canonical owner |
| World schema | `15` | Grids, cargo drops, and anchor rewards retain ownership |
| Event schema | `11` | Construction names its consumed inventory and anchoring records reward eligibility |
| Content schema | `9` | Actor-safe, non-repeatable career rewards are content rules |
| Content manifest | `p1.1.0` | Existing universes cannot silently adopt the new economy rules |

A local protocol-11, world-14, event-10, or `p0.10.0` universe must be archived
and reset. No implicit migration guesses a grid owner, reinterprets an old
reward, or converts an ownerless cargo drop.

## Canonical ownership

1. Every live grid has one `owner_player_id` in canonical state.
2. The fresh starter grid belongs to `player-local`. The development
   `player-remote` suit begins without a grid.
3. A player inventory belongs only to the player named by its domain.
4. A cargo inventory derives its owner from its unique live cargo block and
   the grid containing that block. Missing, duplicate, or inconsistent links
   fail closed.
5. A destroyed cargo inventory retains the grid owner's player ID when it
   becomes dropped inventory. Generic industry and transfer actions cannot use
   a dropped inventory.
6. Damage and grid separation never transfer ownership. Every fragment inherits
   the original owner.
7. A structural split neutralizes retained controls. Only the deterministic
   primary fragment retains unused first-anchor reward eligibility; new
   fragments cannot multiply that reward.

Ownership is deliberately player-only in this checkpoint. Company, team,
lease, delegated-operator, and capture permissions require explicit later
principal and policy schemas.

## Authority matrix

All checks run during live preparation and event replay against the same prior
canonical state.

| Intent | P1.1 authority | Additional authoritative checks |
| --- | --- | --- |
| Character control, suit, respawn | Bound actor only | Actor epoch, input frontier, life and suit state |
| Mine voxel | Any living actor | Closest visible voxel within 9 m; yield enters actor's carried inventory |
| Refine ore | Actor-accessible inventory only | Positive recipe quantity, exact inputs and capacity |
| Craft component | Actor-accessible inventory only | Positive recipe quantity, exact inputs, outputs, and capacity |
| Transfer inventory | Actor access to both endpoints | Distinct live inventories, source quantity, destination capacity |
| Place block frame | Grid owner only | Closest visible face, actor carried component cost, free connected coordinate |
| Weld or repair block | Grid owner only | Closest visible block and exact integrity transition |
| Grid control | Grid owner only | Finite bounded control, online power, grid not anchored |
| Toggle anchor | Grid owner only | Exact toggle; engagement also requires power and voxel contact |
| Damage block | Any living actor | Closest visible block within 9 m and exact registered tool damage |

Knowledge of an ID grants no capability. Cross-player inventory attempts return
`inventory_access_denied`; constructive or control attempts against another
player's grid return `grid_access_denied`. Replay uses equally fail-closed
rejection paths before inventory, topology, career, receipt, or event sequence
can change.

Non-owner damage remains allowed by design. PvP and offline vulnerability are
core product rules, and ownership is not raid immunity. P1.1 has no capital
safe-zone policy volume, so the local proof sector must not be represented as
protected. A later safe-zone ruleset will prohibit damage by spatial policy,
not by silently turning ownership into invulnerability.

## Proof production

Refining and component crafting remain immediate proof transformations inside
an actor-authorized inventory. They do not yet require a refinery, assembler,
conveyor path, power draw, queue, production duration, operator terminal, or
physical proximity. These commands prove actor isolation, exact recipes,
capacity, conservation, idempotency, persistence, and recovery only.

Grid control is likewise owner-authorized remote control for this checkpoint.
The owner does not yet need to occupy a cockpit or stand at a control terminal.
Cockpit possession, delegated pilots, antenna range, terminal access, and
signal loss are later control-authority layers.

## Career rewards

Rewards are derived only after an accepted event and are credited only to its
authenticated human actor.

| Accepted work | Experience |
| --- | ---: |
| Mine voxel | `ore yield x 5` |
| Refine batch | `12` |
| Fabricate component | `18` |
| Place a component-backed frame | `5` |
| Complete that frame for the first time | `20` |
| First eligible anchor engagement | `40` |
| Inventory transfer | `0` |
| Intermediate weld or ordinary repair | `0` |
| Damage | `0` |

The construction total is therefore exactly 25, not a reward for every weld.
Transfer ping-pong, damage-and-repair loops, anchor cycling, idempotent retries,
and system events cannot farm experience. Anchor eligibility is durable and is
consumed by the first accepted engagement.

## Acceptance evidence

- Two authenticated sockets receive the same owner-labelled grid and world
  hash.
- The secondary player can mine, refine, and craft only through its own
  inventory and receives only its own career credit.
- The secondary player cannot use either endpoint of the primary player's
  inventory or cargo transfer.
- Even with a valid visible target, the secondary player cannot place or weld
  on, control, or anchor the primary player's grid.
- The secondary player can damage the primary grid from a valid visible target
  without receiving experience.
- Both clients converge after every accepted or rejected attempt. Rejected
  attempts leave event sequence, world hash, inventory, topology, and career
  unchanged.
- Journal and snapshot restart recover the same ownership, fragments,
  inventory contents, career state, anchor eligibility, and world hash.

## Deliberate limits

Complete snapshots still expose the proof cell's inventory records to every
local player and spectator. P1.1 enforces mutation authority but does not claim
inventory confidentiality. A later protocol must separate a public world
projection from actor-specific private inventory and cargo projections before
public deployment.

The checkpoint also defers company and team ownership, gifts, market custody,
death-drop recovery and salvage, production machines and queues, cockpits,
delegated grid operators, safe-zone enforcement, combat attribution, insurance,
and public authentication.
