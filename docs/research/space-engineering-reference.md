# Space-engineering reference study

**Status:** Accepted clean-room research boundary

## Source reviewed

- Repository: <https://github.com/KeenSoftwareHouse/SpaceEngineers>
- Repository EULA: <https://github.com/KeenSoftwareHouse/SpaceEngineers/blob/master/EULA.txt>
- Review date: 2026-08-27

The repository describes itself as an archived and outdated version of Space Engineers. Its assets are not included. More importantly, its EULA states that the code is not free software, restricts derivatives to Space Engineers mods, and expressly prohibits using the code in a standalone application or another project.

The Verse must therefore never import, translate, adapt, or use that source code as an implementation dependency. No file from that repository may enter this repository. This review is not legal advice; uncertainty must be resolved conservatively or through qualified counsel.

## Permitted research use

The reference may be used only as a public feature and architecture taxonomy:

- repository-level project and folder names;
- names of broad gameplay subsystems;
- public, observable game behavior;
- general engineering ideas that are independently implemented from first principles; and
- identification of parity gaps and validation scenarios.

Implementation work must be based on The Verse specifications, Godot and Rust documentation, academic or permissively licensed techniques, and independently written tests. Contributors must not consult Keen source while implementing a matching subsystem.

## Architecture lessons from the public module map

The public file tree shows that a mature engineering sandbox treats these as separate systems rather than one monolithic grid script:

| Reference category | Independent Verse counterpart |
| --- | --- |
| Definitions and object builders | Versioned content manifests and canonical schemas |
| Cube grids and slim blocks | Authoritative sparse grid aggregate and block topology |
| Voxel storage and iso-meshers | Sparse voxel chunks, delta journal, and smooth render mesh |
| Character and jetpack components | Predicted local character controller with server correction |
| Grid physics and destruction | Jolt bodies, constraints, fracture, and deterministic ownership |
| Resource source/sink/distribution | Typed power, gas, and logistics graph solvers |
| Conveyors and production blocks | Item-network routing, queues, refineries, and assemblers |
| Cockpit and ship controllers | Possession, input routing, thrust allocation, and gyros |
| Replication state groups | Interest-managed entity deltas and prediction histories |
| Definition-driven ModAPI | Apache-2.0 SDK plus DAO-approved official content packages |
| Dedicated server projects | Headless authoritative cell workers and orchestration |

This module map is a completeness checklist, not a design or code dependency.

## Parity sequence

### R1 — perceptual and tool realism

- Smooth destructible voxel surfaces rather than visible primitive clusters.
- Physically plausible metal, rock, emissive, glass, and damage materials.
- Original industrial machinery silhouettes with readable functional states.
- Deliberate mining, welding, grinding, impact, debris, light, and sound feedback.
- Grounded character motion plus optional six-axis EVA.

### R2 — construction and vehicle simulation

- Small/large grid scales, rotations, mount points, and build stages.
- Cockpit possession, thrusters, gyroscopes, mass, inertia, and collision damage.
- Power/resource distribution, batteries, conveyors, inventories, and production queues.
- Mechanical connections, landing gear, connectors, doors, pistons, and rotors.
- Damage deformation, disconnected topology, salvage, and repair.

### R3 — persistent multiplayer engineering universe

- Server-owned physics with client prediction and interest management.
- Blueprint serialization, projection, ownership, factions, and permissions.
- Mod definitions, scripts, validation, sandboxing, and Verse DAO approval.
- Planets, asteroid sectors, encounters, NPCs, contracts, and safe zones.
- Economy settlement and marketplace deposits without putting simulation ticks on-chain.

## Immediate acceptance target

The next native-client upgrade is accepted when the same authoritative asteroid:

1. renders as a continuous iso-surface;
2. reveals distinct ore-bearing regions without exposing cubic occupancy;
3. remeshes after an accepted mining event;
4. retains integer voxel targeting and anchor contact on the server;
5. uses only original project materials and shaders; and
6. passes the existing mining-to-grid-split recovery scenario unchanged.
