# P0 specification: simulation proof

**Status:** In implementation; P0.2 first-person gameplay slice available

## Purpose

Prove that the proposed open-source stack can support the irreducible technical core before building accounts, markets, planets, or smart contracts.

## Included features

- F-001 Apple Silicon client.
- F-002 Ubuntu headless server.
- F-003 floating-origin local coordinates.
- F-004 sparse voxel asteroid.
- F-005 authoritative voxel edits.
- F-006 movable grid.
- F-007 voxel anchor/static grid.
- F-008 power network.
- F-009 inventory conservation ledger.
- F-010 damage and grid split.
- F-011 snapshot and recovery.

## Demonstration scenario

1. Start an Ubuntu-compatible authoritative server.
2. Connect a macOS native client.
3. Spawn near one procedural asteroid.
4. Mine a voxel deposit into ore.
5. Move ore into a container.
6. Build a small powered grid from registered components.
7. Move and rotate the grid.
8. Anchor it to the asteroid and observe static transition.
9. Remove the final anchor and observe valid dynamic transition.
10. Damage the grid until it separates.
11. Stop the server during an inventory operation.
12. Recover from snapshot and journal.
13. Confirm identical ownership, quantities, voxel deltas, and grid topology.

## P0 block set

Minimal non-production art:

- Structural block.
- Control core.
- Power source.
- Battery.
- Cargo container.
- Drill/mining tool.
- Voxel anchor/foundation.
- Damage test block.

## P0 resource set

- One voxel material.
- One ore.
- One refined material.
- One component.

Recipes must still obey the canonical conservation schema.

## Authority

- Server owns voxel chunks, grids, power, inventory, damage, and events.
- Client submits input and tool intents.
- No blockchain, account abstraction, AMM, DAO, or public-market dependency exists in P0.

## Required schemas

- Universe/local coordinate.
- Voxel chunk identity and delta.
- Grid, block, and connection.
- Inventory stack and operation.
- Power node and edge.
- Damage and split.
- Event envelope.
- Snapshot header and content hash.

## Acceptance criteria

### Voxel

- Replaying the same accepted edit operation does not remove material twice.
- Restart reconstructs identical voxel chunk hashes.
- Edits outside permitted range or capability are rejected.
- Mesh generation does not own canonical volume.

### Grid

- A free grid is dynamic.
- The approved final anchor changes it to static/partitionable state.
- Removing the final anchor restores dynamic eligibility.
- A split creates disjoint grid IDs with conserved blocks and inventories.
- No block belongs to two grids.

### Inventory

- Mining creates only recipe-authorized ore quantity.
- Transfer retry does not duplicate or lose quantity.
- Server interruption at every tested transfer step recovers to one valid state.
- Global conservation test passes after the full scenario.

### Persistence

- Snapshot hashes validate before load.
- Replayed aggregate hashes match the pre-shutdown authoritative hashes.
- Corrupt or incomplete journals are detected and not silently accepted.
- A stale server process cannot write after a new worker acquires authority.

### Networking

- Modified client quantities, positions, or damage outcomes are rejected.
- Client correction does not mutate inventory.
- Disconnect/reconnect restores authoritative state.

## Benchmark report

The proof must publish results for:

- Reference Apple Silicon Mac model.
- Ubuntu server CPU and memory.
- Voxel edit and remesh latency.
- Physics tick duration by block/body count.
- Snapshot size and load time.
- Network bandwidth.
- Split-grid worst case.
- Server recovery time.

Targets are set only after the first reproducible baseline to avoid inventing unsupported performance promises.

The first Apple Silicon kernel baseline is published in [P0.1 benchmark results](../benchmarks/P0.1-apple-silicon.md). Ubuntu, rendering/remesh, multi-body physics, network-bandwidth, and large-grid baselines remain required before the P0 exit decision.

## P0.1 implementation checkpoint

Implemented and continuously verified:

- Deterministic procedural asteroid and authoritative voxel removal.
- Versioned content definitions for yields, recipes, block health, and power.
- Inventory-domain transfers and conservation checks after every event.
- Powered movable grids, voxel-contact anchors, block damage, and deterministic splits.
- Operation idempotency, hash-chained journals, snapshots, recovery, and writer fencing.
- Godot macOS client, browser command center, JSON WebSocket protocol, and public read endpoints.
- End-to-end mining-to-construction scenario with server restart and native-client reconnect.

Still required for the P0 exit gate:

- Collision/contact physics and production-scale Jolt integration.
- Sparse chunk meshing and edit/remesh latency measurements.
- Ubuntu server benchmark and native Linux client package.
- Network bandwidth measurements and larger body/block-count scaling.
- Crash injection at each persistence boundary, beyond the current corruption and restart suite.

## P0.2 gameplay checkpoint

Implemented and continuously verified:

- First-person six-axis EVA movement with boost and toggleable inertial dampeners.
- A physical industrial multi-tool with hold-to-mine, weld, and cut interactions.
- Surface-only irregular asteroid rendering while integer voxels remain canonical.
- A differentiated 25-block starter skiff with control, power, battery, cargo, drill, and work-light silhouettes.
- A five-stage salvage contract spanning extraction, refining, fabrication, construction, and anchoring.
- Event-derived career counters, experience rewards, and clearance levels in the authoritative snapshot.
- Protocol and save-schema rejection for incompatible pre-career clients and worlds.

## Exit decision

P0 ends with one of:

- Accept Godot/Jolt plus Rust architecture.
- Retain Godot client but replace the server physics/grid kernel.
- Replace the client engine.
- Reduce or redesign an unsupported requirement through a new product decision.

No production content work begins before this gate.
