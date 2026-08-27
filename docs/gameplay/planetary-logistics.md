# P0.5 planetary logistics checkpoint

**Status:** Implemented local proof

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
3. `J` toggles the authoritative jetpack state. With the jetpack offline, the client predicts gravity-aware walking, jumping, and surface contact while the server continues to validate accepted positions.
4. `H` toggles the authoritative helmet seal.
5. An open helmet replenishes suit oxygen in breathable atmosphere and rapidly loses oxygen in vacuum. A sealed helmet preserves oxygen in breathable atmosphere and consumes reserve oxygen in vacuum.
6. Suit oxygen and equipment modes persist through snapshots, events, restart, and reconnect.
7. The surface shader, terrain ridges, boulder field, atmosphere shell, and horizon are disposable visual state; the Rust environment model remains authoritative.

## Explicit limits

Khepri Prime is a bounded planetary-environment proof around the origin cell. Its visible surface is not yet a globally streamed editable voxel sphere, its decorative terrain scatter has no canonical resource yield, and the player controller is predicted rather than a production character-body solver. Full planet streaming, weather, pressurized rooms, air vents, airtightness graphs, oxygen tanks, health damage, grid gravity generators, and rigid-body ground collisions remain later checkpoints.
