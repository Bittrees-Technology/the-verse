# Playable engineering starter kit

**Status:** Implemented local playtest; gameplay loop, input boundaries, and live UI verified.

**Features:** F-062, F-063, F-069. **Requirements:** UX-001, UX-002, UX-004,
SIM-002, IND-001, IND-002.

## Playable outcome and delivery order

1. Preserve the verified voxel mining, cargo transfer, physical refining,
   component production, frame placement, and completed welding loop.
2. Expose four distinct starter suit tools in inventory and the gameplay hotbar.
3. Explain the cargo-to-machine workflow inside the game using authoritative
   career progress; never instruct the player to use pocket crafting.
4. Launch optimized physics, package the client/server together, and verify a
   fresh and restarted world before delivering a playable build.

This user-requested local playtest advances the existing interactive protocol-18
path. It does not activate protocol 19 or declare the broader P1 roadmap complete.

## Starter equipment

Each existing development suit exposes its always-available work capabilities
as four permanent, non-tradable equipment slots. They are not resource stacks,
newly minted conserved assets, collectible weapons, or droppable cargo. Like the
existing helmet work light, selected tool presentation is local; reconnect may
return to the drill. Server-owned actions and permissions remain authoritative.

| Slot | Tool | Primary action |
| --- | --- | --- |
| 1 | Mining drill | Hold to mine the closest visible voxel |
| 2 | Salvage grinder | Hold to damage a block through the existing cut/salvage path |
| 3 | Welding torch | Hold to finish or repair an owned block |
| 4 | Pulse tool | Click to fire one short-range pulse at a block |

`I` opens inventory; its Tools tab lists the kit, descriptions, and Equip
buttons. `B` enters construction with the welder; only while construction is
active do `1`–`8` select block kinds. `B` or right-click exits construction.
Outside construction `1`–`4` always select tools. Switching tools cancels any
unfinished charge. Closing a menu, restoring mouse capture, or changing tools
while primary fire is held requires release before another action.

Tools have distinct original primitive-based models, colors, and effect timing.
A pulse produces recoil and a brief beam, but hit effects never imply confirmed
damage before authoritative state arrives. Empty-space pulses are cosmetic.

## Authority and intentional limits

Mining, build, weld, and damage still use the exact existing intent and event
shapes. The server derives the eye ray, closest hit, inclusive nine-metre reach,
ownership checks where applicable, damage of 35, destruction, and salvage. A
pulse and grinder intentionally share the same block-damage capability in this
first kit. The client does not supply a ray, damage amount, ammo state, or yield.

No new equipment authority is granted by selecting an icon. Controls disable
while disconnected, unverified, incapacitated, transferring, or using a menu.
Tool work waits for any previous mutation to resolve rather than accumulating a
backlog of destructive actions. Pulse fire is single-click with a local cooldown;
this is interaction pacing, not a new server-enforced weapon-rate contract.

The kit has no ammunition economy, tradable equipment, tool durability, player
damage, long-range shots, projectile physics, or safe-zone combat rule changes.
Those require separately versioned authoritative gameplay and inventory work.
The existing survival clock and recovery rules remain in force.

## Acceptance

- Tool selection changes no inventory quantity, canonical state, or protocol.
- Drill cannot cut blocks; grinder cannot mine; welder cannot damage; pulse
  submits at most one existing block-damage intent per press.
- Full blocks do not become new frames unless construction is explicitly open.
- Tool switches, menus, mouse release, disconnect, death, and pending receipts
  cannot carry an old charge into a different target or cause accidental fire.
- Views remain stable; switching reuses the four prepared viewmodels.
- Guidance requires cargo deposit before refining, authoritative completion
  before components, and moving components back before frame construction.
- Native tests cover the action matrix and input boundaries. The existing
  cross-process mining-to-building, authority, conservation, and exact restart
  scenario passes. The packaged client loads its verifier and connects.

## Compatibility and rollback

No network, world, content, event, or verifier version changes. Existing saves
retain their exact ledger. Reverting removes local tool selection and guidance
without migrating inventory. A dedicated engineering launcher uses a separate
orbital ore-workshop save with seeded ferrite, cuprite, and cobaltite deposits; the existing Earthlike surface launcher remains available.

## Verification checkpoint — 2026-09-05

- `tools/e2e/verify-local.sh` passes the complete input-only industry loop,
  two-player permissions/privacy, both native pilot connections, and exact
  restart recovery. The main scenario reports conservation true, level 2,
  165 experience, and successful physical production and construction.
- `motion_impairment_smoke.gd` includes the tool action matrix, inventory
  non-mutation, explicit construction, switch cancellation, single-click pulse,
  receipt backpressure, menu/disconnect/handoff gates, and cosmetic empty shots.
- `tools/e2e/starter-kit-ui-smoke.sh` uses a fresh temporary world, the real
  verifier, and actual mouse events at the window's scale. It verifies the Tools
  and Production tabs, all four Equip buttons, and exclusive viewmodels, then
  saves screenshots under `artifacts/starter-kit-review`.
- Godot editor import, Markdown lint, and shell syntax checks pass. Existing
  native test shutdown diagnostics include retained resources; the GPU UI
  harness also reports a particle-shader cleanup diagnostic after its pass.

The GPU UI smoke requires a real display. It does not run under a headless
renderer. Packaging runs the existing shipped-client connection/verifier smoke
and records the exact source revision in `VERSION.txt`.

## Remaining gameplay work

This closes the starter-kit presentation slice, not F-062/F-063 in full. The
next beginner-play improvements are a reachable life-support/resupply loop and
stronger worksite navigation. Durable collectible/craftable tools, ammunition,
long-range combat, and tool-specific authoritative cooldowns require versioned
inventory and combat specifications. Protocol-19 cutover and public-scale
universe work retain their existing roadmap gates.
