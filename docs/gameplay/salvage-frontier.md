# P0.2 gameplay specification: Salvage Frontier

**Status:** Implemented local slice

## Player promise

The opening should feel like the first shift of a space-industry career, not an engineering dashboard. The player wakes in an EVA suit beside the Khepri relay salvage skiff, reads the nearby asteroid as material, reads the rig as machinery, and uses one physical tool to change both.

This is clean-room visual and mechanical direction. It does not use third-party code, assets, names, story, or copied interface designs.

## Opening contract

The persistent career aggregate drives one ordered contract:

1. **Cut a path:** extract three asteroid voxels.
2. **Smelt feedstock:** refine at least one batch of ore.
3. **Fabricate a part:** manufacture at least one component.
4. **Expand the relay:** weld two blocks onto the skiff.
5. **Lock the rig:** build an anchor against the asteroid and energize it.

The next incomplete stage is derived from authoritative counters. It is not trusted client quest state, so reconnect and restart cannot skip or roll it back independently of the work that completed it.

## Moment-to-moment loop

- Fly with six-axis EVA thrust; dampeners brake drift when enabled.
- Aim at visible asteroid material or a constructed block.
- Hold the appropriate tool input while a progress indicator, tool motion, light, beam, and impact flare communicate work.
- Receive only the server-authorized yield or structural result.
- Turn ore into refined material, material into components, and components into blocks.
- Change a live grid between free and voxel-anchored states, then test movement and destructive separation.

## Career progression

The simulation derives experience from accepted events:

| Accepted work | Experience |
| --- | ---: |
| Mine voxel | `ore yield × 5` |
| Refine batch | `12` |
| Fabricate component | `18` |
| Transfer inventory | `0` |
| Place a component-backed frame | `5` |
| Complete that frame for the first time | `20` |
| First eligible anchor engagement | `40` |
| Intermediate weld or ordinary repair | `0` |
| Damage block | `0` |

Movement, inventory shuffling, intermediate welds, ordinary repair, damage,
release or repeated engagement of an anchor, grid-motion settings, and
simulation ticks award no experience. A placed and completed frame awards 25
total exactly once. Level 2 begins at 100 total experience; each later
threshold adds another `level × 100`. The current implementation caps the
derived level at 100.

## Presentation rules

- Show an industrial suit HUD, current cargo, power, rank, objective, target, and interaction charge without exposing raw debug structures.
- Give machinery distinct silhouettes and emissive state cues even when canonical blocks still occupy one-meter cells.
- Render only surface voxels as irregular overlapping mineral forms; integer occupancy remains authoritative for mining, anchoring, and persistence.
- Keep the tool in the first-person view and make work require a deliberate hold rather than a single abstract click.
- Use original dark-industrial art direction with cyan work lights and amber hazard accents.

## Acceptance scenario

The automated cross-process scenario must complete the contract path and continue through cargo transfer, anchor release, grid motion, block damage, and deterministic grid splitting. At the end it must prove:

- at least three mined voxels;
- one refining batch and one fabricated component;
- three constructed blocks and one anchor engagement;
- clearance level 2 or higher;
- exact conservation of ore, refined material, components, live blocks, and destroyed blocks;
- identical world recovery after server restart; and
- successful native-client reconnect against the recovered world.

## Explicitly deferred

This milestone does not claim multiplayer replication, character bodies, interiors, walking gravity, collision physics, conveyor networks, production queues, ship cockpits, weapons, NPCs, planets, survival needs, safe zones, markets, or blockchain settlement. Those features must build on the same authoritative intent, persistence, and conservation boundaries rather than bypass them.
