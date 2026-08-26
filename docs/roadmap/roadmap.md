# Delivery roadmap

**Status:** Proposed; sequencing accepted, dates uncommitted

The project will reconcile and specify systems before building them one by one.

## Phase S0 — Canonical specification

Deliverables:

- Requirements with stable IDs.
- Feature catalog.
- System architecture.
- Event and asset schemas.
- Economy and AMM design.
- Chain and contract registry.
- Governance and mod policy.
- Security threat model.
- Operations model.
- ADRs and open-question register.
- Public contribution and licensing policy.

Exit gate:

- No unresolved contradiction in P0 requirements.
- Every P0 feature has acceptance criteria and state authority.
- Founder approves the baseline.

## Phase S1 — Engineering design

Deliverables:

- Repository/monorepo layout.
- Toolchain versions.
- Network protocol.
- Persistence schema.
- Voxel format.
- Grid and block schemas.
- Benchmark harness.
- CI and reproducible development environment.
- Local simulation and contract devnet.

Exit gate:

- P0 technical spikes can be implemented without product ambiguity.

## Phase P0 — Simulation proof

Build:

- Apple Silicon Godot client.
- Ubuntu Rust headless server.
- Sparse voxel asteroid.
- Server-authoritative mining.
- Movable grid.
- Voxel anchor/static transition.
- Power network.
- Inventory ledger.
- Damage and grid split.
- Snapshot, replay, and crash recovery.

Exit gate:

- Conservation suite passes.
- State replay is stable.
- Target Mac and Linux server benchmarks are published.
- No client-authoritative economic path.

## Phase P1 — Multiplayer vertical slice

Build:

- Multiple users in a cell.
- Interest management.
- Fixed celestial registry.
- Dynamic cell scheduling.
- Cross-cell handoff.
- Capital safe zone.
- Offline assets and powered defense.
- Death drops.
- Derelict cleanup.
- Refining and manufacturing.
- Browser status application.
- Public read API.
- Signed direct-download updater.

Exit gate:

- Continuous multi-day soak test.
- Crash and handoff tests show no duplicate assets.
- Offline raid and cleanup rules operate deterministically.

## Phase P2 — Economic alpha

Build:

- Passkey profiles.
- Companies, roles, and formal contracts.
- Internal test-credit AMMs.
- Capital and regional custody.
- Unique-item listings.
- Registered-station cleanup exception.
- Economic dashboards.
- Agent SDK.
- Official content manifest and mod staging.

Exit gate:

- Simulated economy runs without conservation or solvency failure.
- AMM behavior, fees, and liquidity risks are accepted.
- Bots and humans can operate through the same APIs.

## Phase P3 — Testnet Web3 beta

Build:

- Sepolia and Base Sepolia registries.
- Matching Verse DAO Safes.
- Passkey smart accounts.
- Paymaster and session policies.
- BIT/bNOTE test integration.
- Commodity receipts.
- Market contracts.
- Lifecycle root registry.
- Proof API.
- Security Council controls.

Exit gate:

- Independent contract audit.
- Stateful fuzz suite.
- Chain reorganization and relayer failure drills.
- Full custody/receipt reconciliation.
- Successful pause/unpause exercise.

## Phase P4 — Public universe alpha

Build:

- Expanding frontier.
- Multi-day routes.
- Regional markets.
- PvP/PvE.
- Blueprints and UGC.
- Approved mod pipeline.
- Private-server distribution.
- Larger static structures and partitioned capital ships.
- Community operations.

Exit gate:

- Hundreds of concurrent testers.
- Stable multi-week economy.
- Operational security and support process.
- Legal and deployment approvals for real-value markets.

## Phase P5 — Production economy

Build:

- Base/mainnet settlement.
- Canonical BIT bridge integration.
- Production AMMs.
- Mainnet receipts and lifecycle roots.
- DAO treasury operations.
- Thousands of concurrent universe participants.
- Mature live operations.

Exit gate:

- Mainnet audits and deployment manifest.
- Treasury and signer readiness.
- Solvency and monitoring.
- Applicable legal/compliance approvals.
- Incident-response readiness.

## Phase P6 — Expansion

Potential:

- Jump drives and gates.
- Larger mobile worlds.
- More planets and frontier generation rules.
- Browser world client.
- Cloud-streamed full client.
- New industries, factions, and creator tools.
- Broader community governance.

## Staffing reality

AI-assisted development can produce specifications, scaffolding, code, tests, and documentation quickly. Full production still requires human control of signatures and deployments and eventually benefits from specialized security, legal, art, community, QA, and operations contributors.
