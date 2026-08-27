# P0.10 implementation guide

**Status:** Playable simulation proof verified on macOS and hosted Ubuntu

## What this milestone proves

P0.10 connects a first-person Godot client and a browser command center to one Rust authoritative simulation. A player begins in vacuum beside a powered 25-block salvage skiff and an independent orbital asteroid more than three kilometers from Khepri Prime's modeled surface. The player uses input-only six-axis EVA, radial ground movement, jump, and magnetic boots; manages physical inventory through a connected-inventory terminal; controls the helmet seal and oxygen reserve; then mines, manufactures, places oriented frames, welds them through persistent integrity stages, and operates or destroys the resulting grid. One server-owned Jolt scene integrates the capsule and grids against the spherical planet, voxel chunks, and block compounds. Authoritative oxygen failure incapacitates the player, atomically moves carried inventory into a canonical drop, blocks further work, and permits only a free server-selected proof recovery. The server persists received controls and quantized physics outcomes before publishing them and reconstructs the same world after restart.

```mermaid
flowchart LR
    Native["Godot native client"] -->|"intent only"| Worker["Rust simulation worker"]
    Browser["Browser command center"] -->|"spectator hello and reads"| Worker
    Worker --> Rules["Versioned content rules"]
    Worker --> Kernel["Authoritative world kernel"]
    Kernel --> Jolt["Derived Jolt contact scene"]
    Kernel --> Journal["Hash-chained event journal"]
    Kernel --> Snapshot["Atomic world snapshot"]
    Worker -->|"receipts, motion states, snapshots"| Native
    Worker -->|"receipts, motion states, snapshots"| Browser
```

## Authority and conservation

Clients choose targets and request actions; they never choose yields, damage, health, power, production outputs, oxygen outcomes, capacity, grid or character transforms, velocities, contacts, support, or elapsed simulation time. Character clients submit only bounded translation, angular, jump, and suit-mode controls under a server-owned movement epoch. A durable receipt advances the received sequence, while the persisted FIFO consumes at most one transition per 60 Hz substep and advances the processed sequence exposed for prediction reconciliation. The server owns Jolt-backed EVA, gravity, rotation, collision, capsule walking, jump, radial upright alignment, and moving or magnetic support in the same atomic physics event as the grids. A conservation proof runs after every accepted event. If ore, refined material, components, installed blocks, or destroyed blocks do not reconcile, the event is rejected.

The content manifest `p0.10.0` and content schema 8 identify the current contact-physics, survival, and character-motion rule set. Voxel yields, recipes, block health, component costs, construction integrity, power behavior, inventory capacity, resource volume and mass, exact integer block mass, collision chunk edge, grid controls, capsule dimensions and inertia, EVA and ground motion limits, slope/step/snap thresholds, magnetic catch rules, control leases, environment constants, oxygen rates, critical threshold, and proof recovery defaults are server owned. The manifest version is stored in universe manifests and snapshots and included in every canonical event hash. Opening a universe under a different rule version fails explicitly.

The verified P0.10 save schema is version 13 and its canonical event schema is version 8. The active P1 branch intentionally advances those boundaries to world schema 14 and event schema 9: one ordered player map replaces duplicated single-player persistence, and one ordered vector of living-player outcomes replaces the optional single-player field in each physics commit. The cross-platform P0.10 evidence used client protocol 10; P1 uses protocol 11 to authenticate before welcome, bind a socket to the admitted development player, expose a separate read-only spectator role, and replicate deterministic roster snapshots. In addition to construction, grid physics, survival, and death-drop state, the schemas include canonical character orientation, velocity, locomotion mode, support identity/local anchor, magnetic preference, jump state, input frontiers, a bounded persisted control FIFO, the active lease, atomic player physics outcomes, and lightweight motion replication. A receipt proves durable acceptance, not fixed-step consumption; `last_processed_input_sequence` advances only when the control is retired or applied by the authoritative step. A client receives no world state and can submit no intent until a compatible authenticated `hello`; mismatch is fatal and closes the socket. Save and event headers are checked before version-specific payloads are deserialized, so older formats fail explicitly instead of being guessed.

## Local operation

On macOS, run:

```bash
tools/dev/bootstrap-macos.sh
tools/dev/run-local.sh
```

The persistent local world is stored under the ignored `data/local-universe` directory. To start a fresh world without deleting the old one, run `tools/dev/reset-local-world.sh`; it moves the existing world into an ignored backup directory.

The headless service can run independently on macOS or Linux:

```bash
tools/dev/run-server.sh
```

Its local interfaces are:

| Interface | Address | Purpose |
| --- | --- | --- |
| Browser UI | `http://127.0.0.1:7777/` | Read-only public cell spectating |
| Status | `http://127.0.0.1:7777/api/v1/status` | Health, authority, hash, and conservation |
| World read | `http://127.0.0.1:7777/api/v1/world` | Complete public P0 snapshot |
| WebSocket | `ws://127.0.0.1:7777/ws` | Versioned client protocol |

## Verification

`tools/ci/check.sh` runs formatting, unit/property/fault tests, static analysis, browser parsing, documentation linting, Godot project validation, and the complete cross-process scenario.

The scenario proves:

1. Input-only authoritative EVA and tapped roll, environment, suit mode, oxygen failure, death-drop conservation, proof respawn, inventory capacity, mining, refining, crafting, transfer, sealed unfinished cargo, durable construction completion, repair, anchoring, force/torque motion, character/grid/voxel collision, tool damage, tool-driven split, experience, and contract progression.
2. Conservation after every mutation.
3. Exact world-hash recovery after graceful restart.
4. A higher writer-fencing token after authority changes.
5. Godot WebSocket connection, separate received/processed control acknowledgement, lightweight motion reconciliation, and snapshot rendering against the recovered server.
6. Deterministic native impairment coverage for delayed or skipped motion states, correction smoothing and snaps, menu-open gravity, lifecycle gating, bounded prediction history, and motion-only updates without structural rebuilds.
7. Capsule walking, sprint, braking, jump, slopes, bounded steps, ground snap, six-axis radial upright alignment, pole traversal, moving/rotating supports, magnetic eligibility, deterministic detach, and grid-split rebinding.
8. Portable Apple Silicon and Linux archives whose exported clients connect to their bundled authoritative servers and pass the native smoke scenario.

## Deliberate limits

This slice admits one playable local actor, although the active P1 canonical state and physics outcome can represent multiple ordered players. It has one orbital asteroid, one 25-block salvage skiff, one bounded spherical planet/contact proof, and one five-stage contract. Individual block placement orientation is limited to four local yaw rotations, while grid bodies and each character have full three-axis orientation. Native Jolt manifolds replace geometric fallback telemetry, but the public callback exposes only a pre-solver pairwise impulse estimate, not the applied solver impulse required for production collision damage. Khepri is not yet a globally streamed editable voxel planet or practical multi-kilometer travel destination. Ladders, crouch, player-to-player collision, ragdolls, artificial gravity, cockpit possession, and suit-energy consumption remain later locomotion work. The death drop has no recovery, permission, or expiry scheduler yet, and the proof recovery point is neither a powered spawn facility nor the capital. Complete JSON snapshots and local lightweight motion broadcasts are not production multiplayer replication. Accounts, safe zones, offline cleanup, multiplayer partitioning, pressurized-room graphs, real markets, token custody, and smart contracts remain roadmap work, not implied capabilities of P0.10.

## Migration and rollback

P0.10 has no deployed predecessor to migrate. Its implementation rejects save schemas 1 through 12 and event schemas before 8 because their player, inventory, environment, block, physics, life-state, support, or queued-control contracts differ. The active P1 branch likewise requires world schema 14 and event schema 9; it does not silently coerce a schema-13 single-player save. A universe records content manifest `p0.10.0` and refuses to open under another rule set, including `p0.9.0` worlds without canonical capsule support. The active P1 branch rejects protocols before 11 at the handshake; the P0.10 release evidence used protocol 10. Local testers can use `tools/dev/reset-local-world.sh` to move an incompatible world into a recoverable backup before creating a new one.

Rollback is operational only: stop the worker, preserve its data directory, and return to the prior repository revision. No P0.10 action reaches a blockchain or changes external custody. A future content migration must be explicit, versioned, tested against a copy, and documented before the runtime will accept it.
