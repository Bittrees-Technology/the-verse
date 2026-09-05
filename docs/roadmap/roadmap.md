# Delivery roadmap

**Status:** Sequencing accepted; P1 in progress; dates uncommitted

Read [current progress](current-progress.md) for the evidence-backed delivery
assessment as of 2026-09-05. Phase lists below describe scope and exit gates;
they are not completion checklists.

## Section 21 — Lessons for The Verse

**Status:** Accepted product direction

The project shall preserve the strengths of a physical engineering sandbox
while extending them into a persistent public industrial society. These are
product guardrails for sequencing, scope review, and milestone acceptance.

### Preserve

- Every manufactured object begins with conserved resources and remains
  traceable through extraction, hauling, refining, manufacturing,
  construction, use, repair, salvage, recycling, and destruction.
- Bases, ships, factories, vehicles, and infrastructure use one interoperable
  functional-block construction grammar.
- Damage changes real subsystems, topology, cargo, power, motion, and productive
  capability instead of acting only as a health-bar reduction.
- Vehicles are tools for work, logistics, exploration, construction, salvage,
  defense, and trade.
- Blueprints, control logic, and automation are programmable and shareable
  through versioned, permissioned, provenance-checked formats.
- The world remembers discoveries, depletion, construction, damage, repair,
  abandonment, ownership, and recovery.

### Improve

- Treat movement, camera behavior, targeting, mining, welding, cutting,
  inventory, and construction feel as release gates before adding more blocks.
- Give a new player safe, guided, server-verified work in the first session.
- Make salvage, hauling, manufacturing, construction, repair, survey, security,
  and logistics durable careers supported by contracts and work records.
- Connect production to regional demand, scarcity, custody, transport cost, and
  risk rather than flattening the economy into one global price.
- Give companies scoped roles, payroll, treasury, permissions, asset custody,
  work assignment, and auditable governance.
- Let NPCs and autonomous agents participate in production, logistics,
  contracts, and markets under the same authoritative rules as humans.
- Design prediction and interpolation for irregular delivery, and publish
  budgets for grids, automation, physics, voxel changes, replication, and
  recovery before claiming scale.
- Make progression reflect useful work and make exploration change economic or
  historical knowledge.

### Build order

1. Make movement, interaction, targeting, mining, welding, cutting, inventory,
   and construction consistently trustworthy.
2. Complete the conserved mining-to-manufacturing loop and guide a new player
   through it at a safe starter worksite.
3. Add work-capable vehicles, power, cargo, conveyors, topology-aware damage,
   salvage, and repair.
4. Add bounded automation, control scripts, and programmable workers.
5. Add contracts, work records, companies, reputation, payroll, and public
   trade with regional demand signals.
6. Expand into regions, planets, stations, routes, surveys, encounters, and
   durable discovery history.
7. Add portable blueprints, scripts, and approved content packages.
8. Deepen combat only after movement, authority, recovery, and economic
   consequences are reliable.

The differentiator is durable economic meaning: what a participant builds,
moves, programs, discovers, damages, repairs, sells, or abandons should matter
to other participants later. Feature reviews shall prefer strengthening that
continuity over reproducing another product's catalog, interface, fiction, or
signature designs.

## Blockchain dependency sequencing

The bNOTE acquisition interface and Base BIT/bridge deployment manifest are intentionally deferred. They do not block P0 gameplay, P1 multiplayer, or P2's internal test-credit economy. They become required inputs before P3 testnet Web3 integration, after voxel mining and the economic lifecycle have been validated.

## Active P1 sequencing

The [P1.4 physical-industry slice](../gameplay/physical-industry.md) is
implemented and verified in the current authoritative cell. Its queues and
escrow remain actor-private under the implemented P1.5 interest projection;
visible machines expose only their permitted public structure and operating
state.

The first P1.4 slice advances production only in an active cell. Dynamic cell
scheduling must reuse the same integer-tick production event before the project
claims sleeping-cell, background, or offline production. This ordering improves
the playable work loop without inventing a second production state machine.

P1.5 is the published fixed-celestial and interest-management correctness slice. It pins
the current planet and asteroid to immutable hierarchical universe addresses,
validates a versioned proof separation threshold, and replaces whole-cell
fanout with deterministic server-derived interest baselines and deltas. Player
views follow authoritative position. The browser receives a bounded public
spectator view. Visible machines may expose only coarse operating state; actor
inventories, production queues, job details, progress, cargo handles, and
escrow remain private.

The local implementation now builds immutable public projection material once
on first demand for each authoritative revision, uses exact-address spatial
buckets for per-session candidate queries, and projects outside the runtime
lock. The local distribution harness has admitted and resynchronized `2`, `8`,
`16`, `32`, and `64` simultaneous public spectators with 25 visible entities
per session. A synthetic regression adds 2,048 irrelevant far entities and
proves the queried bucket count, visited candidate count, selected identities,
and resulting view remain unchanged. These are local correctness and bounded-
work results, not a production-capacity claim. The current revision
independently reconstructs and hashes raw native and browser frames through one
shared verifier, pins all four connection trust roots, preserves exact protocol
integers, and proves a shipped browser page applies and acknowledges an
untouched view while rejecting an in-flight tamper without either action.
[CI run 33128613104](https://github.com/Bittrees-Technology/the-verse/actions/runs/33128613104)
passes the complete Linux replay, Linux container probe, those independent
verifier suites, and Linux/Apple Silicon packaging for implementation revision
`71e955c`. Active-player load and the partitioned thousand-participant envelope
remain open gates.

P1.5 deliberately remains inside one active authoritative cell. It does not
materialize frontier sectors, allocate workers dynamically, hand entities
between cells, simulate multi-day routes, stream editable planets, provide
arbitrary remote spectator cameras, or claim thousands-player capacity. Its
correctness semantics are encoding-independent; the production binary codec,
cross-cell execution, and public-scale soak evidence remain later P1 work.

P1.5 is published and fully green at implementation revision `71e955c`; the
documentation-only evidence update is `e4cb385`.

[P1.6 durable single-cell lifecycle and background production](../gameplay/durable-single-cell-lifecycle.md)
is implemented and fully green at implementation revision `0664130`.
[CI run 33137371577](https://github.com/Bittrees-Technology/the-verse/actions/runs/33137371577)
passes the hosted verifier, isolated Linux-container verification, and Linux
and Apple Silicon native package jobs. It proves one fixed cell can drain,
sleep, wake, reconcile and resume exact physical production under renewable
single-host fencing. Active and Background share one atomic occurrence-bound
whole-cell quantum, catch-up is finite, and activation creates a fresh verified
baseline only after due work commits.

P1.6 intentionally advances no background physics, oxygen, damage, combat,
turrets, AI, travel, cleanup, or markets. It does not complete dynamic
multi-cell assignment, cross-cell handoff, WORLD-008, F-013, or the
thousand-participant production envelope.

P1.6 acceptance evidence covers:

- exact Active/Background production parity and one atomic multi-machine event;
- stable at-least-once occurrence identity and crash reconciliation;
- strict append/snapshot fencing across a two-process replacement race;
- durable partial-second scheduling, bounded long-downtime catch-up, and
  controlled-clock discontinuity tests;
- recoverable Sleeping, Background, Activating, Active, and Draining
  transitions with no writer overlap;
- non-waking public spectator behavior and fresh verified activation baselines;
  and
- hosted Linux and Apple Silicon package evidence without a multi-host or
  public-scale claim.

[P1.7 durable two-cell assignment and mobile-aggregate handoff](../gameplay/durable-two-cell-handoff.md)
is partially implemented: independent-EVA handoff runs under protocol 18,
while ordinary grid-and-rider closure remains gated by the protocol-19 cutover.
Its accepted contract adds exactly one empty adjacent
proof cell, canonical cell keys, durable directory assignment, per-cell P1.6
lifecycle roots, a separate aggregate placement generation, and atomic handoff
of an EVA actor or isolated ordinary unanchored grid. One directory
compare-and-swap is the sole authority-transfer point; pre-commit failures may
abort exactly, while post-commit failures only roll forward to destination
import.

P1.7 must preserve cargo, production queues and escrow, physics state,
ownership, lineage, actor operation history, and movement/interest frontiers.
The existing gateway session pauses controls and installs one transfer-linked,
independently verified destination baseline before play resumes. Its crash
matrix must prove exactly one mutable placement under stale workers, lost
receipts, duplicate delivery, and restart.

P1.7 remains a two-cell local correctness proof. It does not claim multi-host
availability, cross-cell collision/combat, static or oversized structure
partitioning, planet streaming, frontier expansion, routes, or thousands-player
capacity.

P1.8 persistence migration and install is the active correctness bridge. It
must transform the protocol-18/world-20/event-16 proof state into the isolated
protocol-19/world-21/event-17 store through canonical migration receipts,
immutable per-cell genesis records, exact legacy-frontier preservation,
single-writer installation, and process-crash recovery. It may activate no
hybrid world and may not invent, duplicate, omit, or reseal conserved state.
After this bridge is green, the next playable slice prioritizes F-062 and
F-063 before broader construction breadth.

The first P1.8 source-validation slice acquires only existing directory-v2 and
cell-store locks, in directory-then-ordered-cell order, and returns a
non-serializable frozen-source capability. It requires both proof cells and
their directory assignments to be released sleeping, all transfers terminal,
and every event-16 and transfer-boundary record to be exact, canonical, fully
replayable, and backed by issued fencing history. It never creates, truncates,
backfills, recovers, advances, or rewrites a source artifact. Identity and
production-origin transformation is now a second write-free capability that
borrows those locks. It derives subject creation from replay, validates the
manifest-5/world-21 targets, emits canonical bounded mapping blobs, and proves
independently equal conservation plus inverse-normalized gameplay roots.
The source-bound anchor and receipt issuer now consumes that locked transform
plus a deterministically derived directory-v3 genesis. The dormant prepared
installer copies the exact directory archive and canonical mapping artifacts,
stages or strictly reopens both world-21 cells, and writes one universe head
last. Absence of that head grants no installed authority; presence requires the
exact receipt, directory, artifact set, cell set, immutable lifecycle records,
and retained event-16 frontiers. Process-failure tests cover every universe
boundary, and foreign, missing, extra, swapped, or independently valid but
source-mismatched material fails closed. The signed activation gate now
requires two of three externally anchored Ed25519 signers to bind the exact
prepared receipt and roots, commits one global head last, fences updated legacy
startup, and reopens only the head-selected directory and cells. The offline
operator tool and worker readiness mode exercise the same verified boot while
keeping gameplay admission closed. The activated directory-v3 store now
anchors recovery to the signed genesis prefix and durably supports
directory-issued claim, recovery, and transfer-safe release transitions with
exact retry semantics. The next dependency is now implemented as a
production-only lifecycle-v2 coordinator: per-cell runtime history binds the
signed activation head, current directory authority, world frontier, and one
exact production cursor; bounded dispatch uses the canonical event-17 Store
path and recovers split directory, lifecycle, and event commits without
duplicating work. Quiescent migrated cells remain Sleeping without polling.
Ordinary event-17 gameplay and the coordinated projection/verifier/client
cutover remain subsequent P1.8 gates.

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
- Published movement, camera, interaction, correction, and performance gates.
- Safe guided first-session worksite and one conserved engineering work loop.
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
- Company payroll, treasury, permissions, custody, and work operations.
- Useful-work reputation and occupational progression.
- Bounded programmable workers and engineering automation.
- Internal test-credit AMMs.
- Capital and regional custody.
- Regional demand, scarcity, transport, and logistics signals.
- Unique-item listings.
- Durable survey, route, discovery, and world-history records.
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
