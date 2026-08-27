# P1 actor authority architecture

**Status:** Implemented and verified local P1.1 proof

## Boundary

The worker authenticates a connection once and passes its immutable player ID
beside each decoded intent. The client message does not contain an actor field.
The simulation resolves every inventory and grid capability from that binding
and the prior canonical world.

```text
session-bound actor + intent + prior world
  -> classify human or system event
  -> resolve inventory and grid capabilities
  -> validate range, recipe, power, integrity, and capacity
  -> prepare actor-labelled event
  -> repeat the same authorization during replay
  -> apply to a candidate world
  -> prove conservation
  -> persist before receipt and replication
```

Preparation is not a replay trust boundary. A correctly serialized or
re-hashed payload must still be rejected when its actor cannot perform the
mutation from the replayed prior state.

## Capability resolution

Inventory resolution has no permissive fallback:

- a player inventory is accessible only to that exact player;
- live cargo is accessible only through one complete cargo block on a grid
  owned by the actor;
- dropped inventory is sealed from generic refine, craft, and transfer paths;
- missing, orphaned, multiply linked, incomplete, or ownerless inventory is
  inaccessible.

P1.1 requires actor access to both transfer endpoints. Gifts, company access,
market custody, salvage, and delegated deposits need distinct future events
rather than exceptions in the generic transfer path.

Grid resolution distinguishes constructive authority from hostile interaction.
Build, weld, control, anchor, and cargo capabilities require the grid owner.
Hand-tool damage is intentionally available to a non-owner when closest-hit,
range, life-state, and future spatial combat policy permit it.

## Event exactness

Protocol 12, world schema 15, event schema 11, content schema 9, and manifest
`p1.1.0` form one reset boundary. Event 11 records the component inventory used
by a placed frame and whether an anchor engagement consumed its one-time
reward. World 15 persists grid ownership, anchor eligibility, and the owner of
dropped cargo. Content 9 makes reward-free transfers, repairs, and damage part
of the opened universe's immutable rules.

Every client gameplay payload requires a present human actor and operation ID.
Only explicitly enumerated lifecycle and physics payloads use the system
envelope. Replay rechecks:

- exact actor inventory and grid ownership;
- checked recipe input and output arithmetic;
- source quantity and destination capacity;
- construction component source and cost;
- finite normalized controls, power, and anchor state;
- exact anchor toggle, contact, and reward eligibility;
- exact registered hand-tool damage and closest visible target; and
- actor-only career credit after the mutation is valid.

Grid splitting copies ownership, neutralizes controls, preserves all blocks and
inventories exactly once, and assigns unused anchor eligibility only to the
deterministic primary fragment. Generated fragment identities are checked
before insertion so a split cannot overwrite a live grid.

## Replication scope

Owner IDs are public structural state in protocol 12. Complete cell snapshots
remain a local proof transport and still include all inventories. That is not a
privacy boundary. Public deployment requires actor-specific projection and
authorization without changing the canonical hash or mutation rules.

See [Actor-owned industry and engineering](../gameplay/actor-owned-industry.md)
for the player-visible authority matrix, reward contract, acceptance evidence,
and deliberate gameplay limits.
