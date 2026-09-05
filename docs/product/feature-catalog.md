# Feature catalog

**Status:** Proposed delivery inventory

Priorities:

- **P0:** required to prove the architecture.
- **P1:** required for a public vertical slice.
- **P2:** required for economic alpha.
- **P3:** required for production breadth.
- **P4:** future expansion.

| ID | Capability | Priority | Depends on |
| --- | --- | --- | --- |
| F-001 | Apple Silicon native client shell | P0 | Engine spike |
| F-002 | Ubuntu authoritative server | P0 | Server runtime |
| F-003 | Local floating-origin coordinates | P0 | Universe coordinate schema |
| F-004 | Sparse voxel asteroid | P0 | Voxel kernel |
| F-005 | Authoritative voxel edits | P0 | F-002, F-004 |
| F-006 | Dynamic block grid | P0 | Grid kernel |
| F-007 | Voxel-anchored static grid | P0 | F-004, F-006 |
| F-008 | Power network | P0 | F-006 |
| F-009 | Inventory ledger and conservation tests | P0 | Event schema |
| F-010 | Damage, block removal, and split grids | P0 | Physics |
| F-011 | Durable snapshot and event recovery | P0 | Persistence |
| F-012 | Multiple players in one cell | P1 | F-002, SIM-011, SIM-012 |
| F-013 | Dynamic cell assignment and handoff | P1 | Universe directory |
| F-014 | Fixed canonical celestial registry | P1 | F-003, WORLD-002/003/009 |
| F-015 | Procedural frontier sector materialization | P4 | F-014, generation rules, universe directory |
| F-016 | Multi-day autopilot travel | P1 | Route service |
| F-017 | Browser journey, asset status, and bounded public-cell spectating | P1 | Public API, F-059 |
| F-018 | Capital hard safe zone | P1 | Policy volumes |
| F-019 | Offline structures and powered turrets | P1 | Cell wake-up |
| F-020 | Death drop, 15-minute recovery grace, and six-hour cleanup | P1 | Lifecycle scheduler |
| F-021 | Unpowered derelict, 24-hour public salvage, and 36-hour cleanup | P1 | Power, scheduler |
| F-022 | Registered-station cleanup exception | P2 | BIT contract |
| F-023 | Physical mining/refining/manufacturing graph and queues | P1 | F-006, F-008–F-012 |
| F-024 | Work and delivery contracts | P2 | Escrow |
| F-025 | Company roles and assets | P2 | Identity |
| F-026 | Company DAO registry | P2 | Chain adapter |
| F-027 | DAO-deployed capital commodity AMM and seed liquidity | P2 | Market custody |
| F-028 | Regional commodity AMMs | P2 | Location receipts |
| F-029 | Unique-item listings and auctions | P2 | Escrow |
| F-030 | Passkey profile | P1 | Identity service |
| F-031 | ERC-4337 smart account | P2 | Wallet provider |
| F-032 | Sponsored routine transactions | P2 | Paymaster policy |
| F-033 | BIT, BTREE, WBTC chain adapters | P2 | Chain registry |
| F-034 | bNOTE purchase integration | P3 | Gameplay/economy proof, ABI/audit |
| F-035 | Asset deposit and withdrawal | P2 | Marketplace contracts |
| F-036 | Lifecycle Merkle batches | P2 | Event log |
| F-037 | Public proof API | P2 | F-036 |
| F-038 | Public GraphQL/REST/WebSocket APIs | P1 | API gateway |
| F-039 | Agent SDK | P2 | Public schemas |
| F-040 | Browser market and management application | P2 | F-038 |
| F-041 | Production delayed, permissioned, and global browser spectating | P3 | F-017, read model/stream |
| F-042 | Cloud-streamed native client | P4 | Operations |
| F-043 | Signed official mod manifests | P2 | Governance |
| F-044 | Mod sandbox and budgets | P2 | Extension API |
| F-045 | Private-server distribution | P3 | Namespace isolation |
| F-046 | Blueprint authoring and sales | P3 | UGC/market |
| F-047 | Skins, clothing, and avatars | P3 | UGC pipeline |
| F-048 | PvE factions and missions | P3 | AI simulation |
| F-049 | PvP crime, salvage, and insurance | P3 | Combat economy |
| F-050 | Partitioned capital ships | P3 | Multi-cell physics |
| F-051 | Static megastructure interiors | P3 | Cell hierarchy |
| F-052 | Jump-drive or gate technology | P4 | Governance/economy |
| F-053 | Security Council pause console | P2 | Governance contracts |
| F-054 | Creative-mode public audit log | P1 | Admin authority |
| F-055 | Economic and supply dashboards | P2 | Analytics |
| F-056 | Direct-download updater for macOS/Linux | P1 | Release signing |
| F-057 | Server-authoritative EVA, landing, and rotation | P0 | F-002, F-004, F-006 |
| F-058 | Server-authoritative grounded and magnetic locomotion | P0 | F-057, F-006, SIM-010 |
| F-059 | Deterministic single-cell spatial interest replication | P1 | F-012, SIM-013/014 |
| F-060 | Fenced single-cell lifecycle and background production | P1 | F-011, F-023, SIM-006/015 |
| F-061 | Durable two-cell assignment and mobile-aggregate handoff | P1 | F-011–F-014, F-059/060, WORLD-010, SIM-016–018 |
| F-062 | Guided first-session engineering worksite and conserved work loop | P1 | F-023, UX-002 |
| F-063 | Movement, camera, interaction, correction, and performance quality gates | P1 | F-057–F-059, UX-001/003, PERF-001/002 |
| F-064 | Programmable engineering and autonomous work automation | P2 | F-023, F-039, F-043/044, IND-008 |
| F-065 | Regional demand, scarcity, and logistics signals | P2 | F-028, F-055, MKT-004/011 |
| F-066 | Company roles, payroll, treasury, permissions, and work operations | P2 | F-024–F-026, IND-005/009 |
| F-067 | Durable survey, discovery, route, and world-history records | P2 | F-014, F-036–F-038, WORLD-011 |
| F-068 | Versioned blueprint, script, and automation-package sharing | P3 | F-043/044, F-046, MOD-005 |
| F-069 | Selectable starter suit tools and short-range block pulse | P1 | F-012, F-062/063, UX-004 |
| F-070 | Seeded scarce workshop ore varieties | P1 | F-012, UX-005 |
| F-071 | Capital arrival, surface outcrops and reliable app entry | P1 | F-062, UX-006 |

## Current implementation readiness

See [current progress](../roadmap/current-progress.md) for the dated delivery
assessment and latest main-branch verification. Priority labels above express
feature priority, not completion or permission to bypass roadmap phase gates.

F-023 is implemented and locally verified for the P1.4 active-cell milestone.
Its accepted [gameplay specification](../gameplay/physical-industry.md) defines
the player outcome, state authority, trust boundary, queue and escrow lifecycle,
failure and recovery behavior, persistence versions, observability, acceptance
criteria, and rollout. [ADR-0018](../decisions/ADR-0018-authoritative-physical-industry.md)
records the durable conveyor, scheduler, power, privacy, split, and destruction
choices. Subsequent [main-branch CI](https://github.com/Bittrees-Technology/the-verse/actions/runs/33838538429)
passes the inherited verification and Linux/Mac packaging gates.

Bounded single-cell background production is implemented in F-060; general
dynamic scheduling remains incomplete. F-059 implements bounded interest
management, while active-player load and public-scale replication evidence
remain open. Both retain the P1.4 canonical production-event contract.

F-014 and F-059 form the bounded P1.5 correctness slice. It introduces an
immutable, content-addressed celestial registry and deterministic interest
baselines/deltas inside the current authoritative cell. The server derives a
player's view from canonical position; a browser spectator receives only a
server-approved public view. Coarse visible machine operation may replicate,
but inventory, queue, job, recipe, progress, cargo-handle, and escrow fields
remain actor-private.

P1.5 does not materialize new frontier sectors, schedule or hand off cells,
simulate multi-day routes, stream planetary terrain, provide arbitrary remote
spectator cameras, or establish a thousands-of-players capacity claim. Its
encoding-independent correctness contract may continue over the inspectable
transport while the production binary codec remains a later P1 exit item.

F-060 is implemented and verified for the bounded P1.6 lifecycle slice at
revision `0664130`. The complete local gate and
[hosted CI run 33137371577](https://github.com/Bittrees-Technology/the-verse/actions/runs/33137371577)
prove one fixed cell can drain, sleep, wake for a durable production
occurrence, process a bounded backlog through the same whole-cell quantum used
while active, and reject stale writers through a renewable fenced lease. It
does not complete F-013 or WORLD-008: multi-cell assignment, handoff,
distributed control-plane availability, and background physics remain separate
milestones.

The protocol-19 lifecycle-v2 checkpoint carries F-060's production-only
semantics across the signed migration boundary. Its per-cell runtime history
is anchored to immutable migration genesis and the global active head;
directory-v3 claim, recovery, and release are coordinated as recoverable
split-store transactions; and due work uses the event-17 Store transaction.
This checkpoint deliberately keeps gameplay admission closed and does not
claim external wake scheduling, multi-host availability, or capacity.

F-061 has an implemented independent-EVA checkpoint under protocol `18`. Its
[accepted gameplay contract](../gameplay/durable-two-cell-handoff.md) and
[ADR-0023](../decisions/ADR-0023-durable-two-cell-handoff.md) define two
adjacent proof cells, stable cell keys, durable assignment generations, a
separate aggregate placement fence, content-addressed prepare/quarantine,
directory compare-and-swap commit, exact transfer conservation, and a
same-session verified destination baseline. The promised ordinary-grid closure
remains an implementation gate under the versioned boundary in
[ADR-0024](../decisions/ADR-0024-versioned-grid-closure-handoff.md). P1.7 does
not complete arbitrary
multi-cell placement, cross-cell physics, multi-host availability, static
megastructure partitioning, or the public-scale concurrency envelope.

F-062 and F-063 are cross-cutting release gates, not optional polish. A
playable milestone may add depth behind them, but shall not claim broader
construction or economy readiness while the first verified work loop or
ordinary movement and interaction remain unreliable on supported hardware.

## Definition of specification-ready

A feature is ready for implementation when it has:

- Linked requirement IDs.
- User-visible behavior.
- State ownership.
- Trust boundary.
- Failure and recovery behavior.
- Persistence model.
- Observability requirements.
- Testable acceptance criteria.
- License-compatible dependencies.
- No unresolved blocker that changes its architecture.
