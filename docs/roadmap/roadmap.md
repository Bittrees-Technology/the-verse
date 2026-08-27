# Delivery roadmap

**Status:** Proposed; sequencing accepted, dates uncommitted

The project will reconcile and specify systems before building them one by one.

## Blockchain dependency sequencing

The bNOTE acquisition interface and Base BIT/bridge deployment manifest are intentionally deferred. They do not block P0 gameplay, P1 multiplayer, or P2's internal test-credit economy. They become required inputs before P3 testnet Web3 integration, after voxel mining and the economic lifecycle have been validated.

## Active P1 sequencing

The [P1.4 physical-industry slice](../gameplay/physical-industry.md) is
implemented and locally verified in the current authoritative cell before full
interest management. Its queues and escrow are actor-private and do not depend
on public-scale replication. Hosted CI remains its evidence gate. Interest
management must next include visible machine state without exposing private
queue or inventory fields.

The first P1.4 slice advances production only in an active cell. Dynamic cell
scheduling must reuse the same integer-tick production event before the project
claims sleeping-cell, background, or offline production. This ordering improves
the playable work loop without inventing a second production state machine.

P1.5 is the fixed-celestial and interest-management correctness slice. It pins
the current planet and asteroid to immutable hierarchical universe addresses,
validates a versioned proof separation threshold, and replaces whole-cell
fanout with deterministic server-derived interest baselines and deltas. Player
views follow authoritative position. The browser receives a bounded public
spectator view. Visible machines may expose only coarse operating state; actor
inventories, production queues, job details, progress, cargo handles, and
escrow remain private.

P1.5 deliberately remains inside one active authoritative cell. It does not
materialize frontier sectors, allocate workers dynamically, hand entities
between cells, simulate multi-day routes, stream editable planets, provide
arbitrary remote spectator cameras, or claim thousands-player capacity. Its
correctness semantics are encoding-independent; the production binary codec,
cross-cell execution, and public-scale soak evidence remain later P1 work.

P1.5 acceptance requires:

- Deterministic registry identity, normalized addresses, minimum-separation
  validation, persistence binding, and exact recovery.
- Explicit rejection of duplicate IDs, malformed or overflowing addresses,
  registry-hash mismatch, and silent body relocation.
- Deterministic interest ordering, hysteretic enter/leave behavior, complete
  entity entry, explicit removal, and structural-before-motion delivery.
- Exact baseline convergence after delayed, duplicated, reordered, stale-epoch,
  disconnected, or backpressured delivery.
- Proof that interest projection cannot mutate canonical state, authority,
  conservation, event order, or the world hash.
- Raw spectator and other-player messages containing no private inventory or
  production fields while coarse visible machine state remains available.
- Scaling evidence showing that out-of-view decoy entities do not make a
  connection's payload proportional to the total cell population.
- Existing multiplayer, industry, recovery, native, and browser evidence
  remaining green.

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

- Server-issued session authority bound to one durable player identity.
- Multiple users in a cell with independent input, inventory, life, and recovery state.
- Deterministic shared-player physics and replication budgets.
- Deterministic single-cell interest management.
- Fixed canonical celestial registry.
- Dynamic cell scheduling.
- Cross-cell handoff.
- Capital safe zone.
- Offline assets and powered defense.
- Death drops.
- Derelict cleanup.
- Physical refining, conveyor logistics, manufacturing, and production queues.
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

- Procedural frontier-sector materialization and continued resource expansion.
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
