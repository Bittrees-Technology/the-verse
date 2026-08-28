# P0.5 planetary logistics checkpoint

**Status:** Implemented local proof

P0.6 supersedes the original near-surface spawn with a physically separate orbital asteroid field. The inventory capacity and suit contracts below remain active; current spatial and interface behavior is specified in [Orbital Operations](orbital-operations.md).

## Player promise

The engineering loop must read as physical survival work rather than counters over disconnected cubes. Inventory has volume, mass, ownership, and deliberate transfers. Adjacent completed blocks share a sealed one-meter envelope. The nearby celestial body supplies visible terrain, local gravity, atmospheric density, and oxygen rules that affect suit operation.

## Inventory terminal

1. `I` opens a two-sided logistics terminal and releases the mouse.
2. The left side represents the player's suit inventory; the right side represents the connected starter-grid cargo inventory.
3. Ore, registered alloy, and construction parts show stack quantity, unit volume, and unit mass.
4. One-arrow controls transfer one unit. Double-arrow controls request the complete source stack.
5. The server rejects any transfer, mining yield, or fabrication output that would exceed destination capacity.
6. Used volume, maximum volume, and total mass come from authoritative snapshots rather than client estimates.
7. `I`, `Escape`, or the close control returns to first-person input without submitting a gameplay action.

## Grid presentation

1. Canonical block coordinates remain spaced at exactly one meter.
2. Completed block bodies render at a 1.01-meter envelope so adjacent faces meet without visible space caused by presentation tolerances.
3. Construction frames retain open skeletons until welding reaches full integrity.
4. Cargo, reactor, battery, drill, anchor, control, structural, and breach blocks have different silhouettes, service faces, lights, or labels.
5. Visual overlap is presentation-only; canonical occupancy remains one block per integer coordinate.

## Planetary environment

1. The authoritative snapshot identifies Khepri Prime, its center and surface radius, player altitude, gravity vector and magnitude, atmospheric density, oxygen fraction, and breathability.
2. The server rejects movement beneath the modeled planetary surface.
3. `J` toggles the authoritative jetpack state. The server owns inverse-square gravity, EVA motion, rotation, ballistic landing/contact, and P0.10 capsule walking under [ADR-0013](../decisions/ADR-0013-input-only-authoritative-character-motion.md) and [ADR-0014](../decisions/ADR-0014-authoritative-grounded-and-magnetic-locomotion.md); the client predicts those values for presentation and never submits a transform. Jetpack drift accumulates gravity with dampeners off, powered dampeners compensate while idle, and jetpack-off input drives only a server-classified ground or magnetic motor.
4. `H` toggles the authoritative helmet seal.
5. An open helmet replenishes suit oxygen in breathable atmosphere and rapidly loses oxygen in vacuum. A sealed helmet preserves oxygen in breathable atmosphere and consumes reserve oxygen in vacuum. P0.8 makes terminal depletion canonically incapacitating under the separate survival-death contract.
6. Suit oxygen and equipment modes persist through snapshots, events, restart, and reconnect.
7. The surface shader, terrain ridges, boulder field, atmosphere shell, and horizon are disposable visual state; the Rust environment model remains authoritative.

## Explicit limits

Khepri Prime is a bounded planetary-environment and spherical-collision proof around the origin cell. Its visible surface is not yet a globally streamed editable voxel sphere, and its decorative terrain scatter has no canonical resource yield. P0.10 supplies server-authoritative input-based EVA plus capsule walking, jump, slopes, bounded steps, radial upright alignment, magnetic boots, and translating-platform velocity inheritance; native prediction remains presentation only. Full planet streaming, pole-traversal release evidence, weather, pressurized rooms, air vents, airtightness graphs, oxygen tanks, general combat/impact health, ladders, and grid gravity generators remain later checkpoints.
