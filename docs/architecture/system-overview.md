# System architecture

**Status:** Proposed production baseline with published P1.5 and accepted P1.6
single-cell contracts

## Architectural goals

- Present one persistent universe to thousands of concurrent participants.
- Preserve authoritative physics and economic conservation.
- Permit a practically unbounded generated frontier.
- Keep routine gameplay independent of blockchain latency.
- Make lifecycle history publicly verifiable.
- Support humans, bots, NPCs, AI agents, browser applications, and native clients.
- Keep official and private-server economies cryptographically and operationally isolated.
- Permit individual services and simulation cells to fail without corrupting canonical ownership.
- Bind every worker to one immutable universe manifest and celestial registry.
- Bound replication work by a server-derived session view without weakening
  canonical intent validation.

## Context

```mermaid
flowchart LR
    N["Native macOS/Linux client"] --> G["Intent gateway"]
    B["Browser command center"] --> G
    A["Bots, AI agents, and Web3 apps"] --> G
    G --> I["Identity and policy"]
    G --> U["Universe directory"]
    U --> CR["Universe manifest and fixed celestial registry"]
    U --> S1["Active simulation cells"]
    U --> S2["Sleeping/background cells"]
    CR --> S1
    CR --> S2
    S1 --> E["Canonical event stream"]
    S2 --> E
    E --> P["Persistence and read models"]
    E --> M["Markets, contracts, and companies"]
    E --> SB["Settlement batcher"]
    M --> C["Marketplace contracts"]
    SB --> C
    C --> L1["Ethereum / approved L2"]
```

## Authority hierarchy

1. **Smart contracts** own deposited BIT, tokenized market receipts, settlement commitments, and on-chain governance authority.
2. **Canonical universe services** own identity links, asset ownership, organizations, contracts, and cross-cell transfer state.
3. **Simulation cells** own active voxel, physics, machine, damage, and local inventory state under one registry-bound fenced lease.
4. **Clients and external applications** own presentation and submit authenticated intents.
5. **Private servers** own a separate namespace with no canonical asset authority.

No lower layer may unilaterally create state owned by a higher layer.
Session interest is a projection below canonical state: loading or hiding an
entity never grants authority and never changes simulation ownership.

## Proposed components

### Edge and identity

- API gateway.
- Passkey/WebAuthn service.
- Smart-account association and recovery coordinator.
- Session-key and authorization service.
- Rate limiting and abuse controls.
- Public read-only API cache.

### Universe control plane

- Celestial and sector registry.
- Content-addressed universe manifest and canonical address normalization.
- Dynamic cell scheduler.
- Cell lease and fencing service.
- Player/grid transfer coordinator.
- Route and autopilot service.
- Content-manifest registry.
- Configuration and feature-policy service.

### Simulation plane

- Headless authoritative workers.
- Voxel chunk service.
- Grid/physics kernel.
- Power, conveyor, inventory, production, damage, combat, and AI systems.
- Interest management and client replication.
- Snapshot writer and event journal.
- Offline/background simulation.

### Economic plane

- Canonical asset registry.
- Company and permissions service.
- Contracts and escrow coordinator.
- Market custody service.
- AMM indexer and quote service.
- Unique-asset listing service.
- Insurance/registration service.
- Economic invariants and anomaly detector.

### Blockchain plane

- Chain registry.
- BIT/bNOTE/BTREE/WBTC adapters.
- Passkey smart-account and paymaster integration.
- Deposit/withdrawal reconciler.
- Market contract indexer.
- Settlement Merkle batcher.
- Upgrade and Safe monitor.
- Cross-chain bridge adapter.

### Experience plane

- Native Godot client.
- Browser command center.
- Spectator service.
- Public TypeScript and Rust SDKs.
- Direct-download release and update service.

## Deployment principles

- Simulation workers are disposable; canonical events and snapshots are durable.
- A cell worker must hold a renewable lease with a monotonically increasing
  fencing token before writing. Every append and snapshot revalidates the live
  holder, unexpired token and exact embedded fence.
- A P1.6 worker must verify universe manifest schema `3`, celestial registry
  schema `1`, lifecycle-control schema `1`, schedule-occurrence schema `1`, and
  their hashes before opening world schema `19` or event schema `15`.
- Cross-cell transfers use an idempotent prepare/commit protocol.
- Economic writes use stable operation IDs and double-entry accounting.
- Blockchain consumers wait for chain-specific confirmation policy and tolerate reorganization.
- Mainnet is never a dependency for player movement, mining, combat, or machine ticks.
- A degraded blockchain plane may pause deposits or withdrawals without stopping the physical universe.
- Replication derives an audience-specific view from the bound session and
  canonical state. It never supplies inputs to authoritative validation.

## P1.5 local scale slice

The accepted P1.5 slice adds:

- normalized universe/sector/cell/local addresses;
- fixed registry-bound celestial bodies;
- deterministic actor- or grant-anchored interest with hysteresis;
- audience-safe baselines, absolute component deltas, removals, epochs, and
  view hashes; and
- bounded per-session coalescing and one-baseline recovery.

The coordinated versions are protocol `16`, projection schema `3`, world
schema `18`, event schema `14`, content schema `11`, content manifest
`p1.5.0`, registry schema `1`, universe manifest schema `2`, and interest
schema `1`.

This slice is an architecture and local proof boundary. It does not establish
multi-process cell scheduling, cross-cell handoff, a final binary codec, or a
thousand-player production envelope. Universe-scale concurrency still depends
on the partitioning in ADR-0002 plus published scheduler and handoff evidence.

## P1.6 durable one-cell lifecycle slice

The accepted next slice hosts one already generated fixed cell through
`Sleeping`, `Background`, `Activating`, `Active`, and `Draining`. A local
coordinator owns desired mode, due-occurrence dispatch, lease renewal and
acknowledgement; the simulation aggregate remains the only authority for
production state and its committed occurrence frontier.

Active and Background call one deterministic whole-cell production planner and
commit one ordered event per one-second occurrence. Background constructs no
physics scene and advances no gravity, contact, oxygen, damage, combat, AI,
cleanup, travel, or market timer. Catch-up is sequential and bounded. Public
spectators cannot wake or retain the cell, and gameplay receives a fresh
verified baseline only after activation catch-up and snapshot complete.

This local proof does not implement a universe directory, multiple-cell
placement, handoff, multi-host lease availability, frontier expansion, or a
thousand-player capacity envelope.

## Initial implementation choice

- Godot/Jolt native client prototype.
- Rust server and simulation kernel prototype.
- PostgreSQL, NATS JetStream, Redis, and S3-compatible object storage.
- Protocol Buffers or an equivalently versioned binary schema for internal events.
- JSON/GraphQL representations generated from canonical public schemas.

This stack remains subject to the P0 benchmark gate.
