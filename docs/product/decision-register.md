# Reconciled decision register

**Status:** Accepted record of founder direction as of 2026-08-28

This register reconciles the planning conversations into explicit project decisions. “Qualified” means the intent is accepted with a technical or legal boundary documented elsewhere.

| ID | Decision | Disposition | Canonical location |
| --- | --- | --- | --- |
| D-001 | Build an original Space Engineers-like voxel industrial universe | Qualified: mechanics and genre goals accepted; code/assets/distinctive expression cannot be copied | Vision, OSS-004/005 |
| D-002 | Native macOS first | Accepted | PLAT-001 |
| D-003 | Native Linux and Linux servers | Accepted; Ubuntu 26.04 LTS initial target | PLAT-002/003 |
| D-004 | Browser management, spectating, Web3 apps, optional cloud streaming | Accepted; P1 provides bounded public-cell viewing, while production delayed/permissioned/global spectating remains later | PLAT-004/005/007, F-017/041 |
| D-005 | One public universe with fixed planets and asteroids | Accepted | WORLD-001/002 |
| D-006 | Planets widely separated; asteroids may cluster | Accepted; P1.5 uses a versioned proof threshold, while production separation and travel tuning remain unresolved | WORLD-003/004, OQ-010 |
| D-007 | Thousands of concurrent participants | Accepted through partitioned simulation | WORLD-008, ADR-0002 |
| D-008 | Real-world days of travel without jumping | Accepted | WORLD-006/007 |
| D-009 | Detailed physics, destruction, PvP, and PvE | Accepted | SIM-007, PVP-001, PVE-001 |
| D-010 | No arbitrary maximum structure size | Qualified: spatial partitioning and finite work budgets are required | SIM-005/006 |
| D-011 | A free structure remains movable until voxel/foundation anchoring | Accepted | SIM-003/004, ADR-0007 |
| D-012 | Capital market and settlement are absolutely safe | Accepted | CAP-001/002 |
| D-013 | Offline ships and structures may be destroyed | Accepted | LIFE-001/002 |
| D-014 | Powered automated defenses operate offline | Accepted | LIFE-002 |
| D-015 | Unpowered eligible objects are deleted at 36 hours | Accepted | LIFE-008 |
| D-016 | Derelicts become publicly salvageable at 24 hours and are deleted at 36 hours | Accepted | LIFE-008/009 |
| D-017 | Valuable claimed stations may buy a BIT cleanup exception | Accepted | LIFE-010/011 |
| D-018 | Character death has no currency/experience charge | Accepted | LIFE-003 |
| D-019 | Carried inventory drops, is owner/team-only for 15 minutes, becomes public, and expires after six hours | Accepted | LIFE-006/007/013 |
| D-020 | Capital empty-state respawn always available | Accepted | LIFE-005 |
| D-021 | Universe expands through new asteroid fields and frontier sectors | Accepted; P1.5 establishes the address and immutable registry, while frontier materialization begins in P4 | WORLD-005/009, F-014/015 |
| D-022 | No reachable practical boundary in any direction | Qualified through hierarchical 128-bit procedural space | Universe simulation |
| D-023 | BIT is the primary base pair | Accepted | MKT-001 |
| D-024 | Only deposited/exported assets require direct on-chain representation | Accepted | CHAIN-001/002 |
| D-025 | Consumption/destruction are represented on-chain | Accepted through batched Merkle commitments | CHAIN-003, ADR-0003 |
| D-026 | Hide routine chain transactions | Accepted through passkeys, smart accounts, sessions, relayers, and paymasters | ID-001/003, CHAIN-004/005 |
| D-027 | BIT can bridge to approved chains | Accepted in principle; canonical Base bridge unresolved | OQ-003 |
| D-028 | bNOTE is the existing BIT source application | Accepted; proxy verified and integration details deferred until after gameplay and internal economic validation | Chain registry, OQ-002 |
| D-029 | Verse market purchase/swap contracts do not yet exist | Accepted | OQ-002/004 |
| D-030 | Each standardized commodity market uses an AMM | Accepted; exact protocol/curve unresolved | MKT-005, ADR-0005 |
| D-031 | Market prices are entirely algorithmic | Accepted | Economy and markets |
| D-032 | Users may create additional markets | Accepted | MKT-003 |
| D-033 | Unique/heterogeneous goods use listings, auctions, or specialized mechanisms | Qualified because ordinary AMMs require fungibility | MKT-006 |
| D-034 | Any participant may play and use markets | Accepted under published protocol rules, the Open Metaverse framework, and applicable nonwaivable law | ID-004/005, MKT-009 |
| D-035 | Bots, NPCs, and AI agents are permitted | Accepted | ID-005/006 |
| D-036 | Companies are configurable DAOs with contracts and ranks | Accepted | IND-003/005 and governance spec |
| D-037 | Verse DAO executor is the Ethereum two-of-three Safe | Accepted and live-state verified | GOV-001 and chain registry |
| D-038 | Equivalent-address Safes will be deployed on Sepolia, Base, and Base Sepolia | Accepted founder deployment plan; pending verification | OQ-011 |
| D-039 | Verse DAO may delegate a Security Council | Accepted | GOV-002 |
| D-040 | Pause only for integrity/security failures, not volatility | Accepted | GOV-004 and ADR-0008 |
| D-041 | Pause has no automatic expiry | Accepted | GOV-003 and ADR-0008 |
| D-042 | Official servers accept only DAO-approved mods | Accepted | MOD-001 |
| D-043 | Private servers may run anything | Accepted subject to applicable law | MOD-002 |
| D-044 | Private assets never enter the official economy | Accepted | MOD-003 and ADR-0004 |
| D-045 | Blueprints, skins, clothing, and avatars can be sold | Accepted subject to rights and moderation | MOD-004 |
| D-046 | Admin creative mode is available | Qualified: creations are permanently non-economic | CAP-003/004 |
| D-047 | Game/server AGPL; SDKs Apache 2.0; reusable assets CC BY-SA | Accepted | OSS-001/002/003 |
| D-048 | Public repository is Bittrees-Technology/the-verse | Accepted and created | Repository |
| D-049 | Reconcile/specify before feature implementation | Accepted | Roadmap S0/S1 |
| D-050 | Direct-download macOS and Linux distribution | Accepted | PLAT-006 |
| D-051 | Visual mood blends industrial space horror, monumental portals, and expeditionary military science fiction | Qualified into an original visual language | Visual direction |
| D-052 | Founder and AI tools begin the build | Accepted for specification/prototype; production still requires human-controlled legal, signing, security, and operations duties | Roadmap |
| D-053 | Funding and launch date are deferred | Accepted | Open questions |
| D-054 | The Open Metaverse is the governing framework | Accepted with the meaning and mandatory-rights boundary in Section 13 of the Bittrees Bounties Terms effective 2026-08-12 | GOV-006/007, governance framework |
| D-055 | The Verse DAO deploys official AMMs and provides initial capital-market liquidity | Accepted; exact pools, curves, fees, and reserve sizes require simulation and public governance parameters | MKT-010, OQ-004 |
| D-056 | bNOTE and Base BIT/bridge details wait until gameplay, voxel mining, and the internal economy are validated | Accepted sequencing decision; required before P3 testnet integration | CHAIN-008, OQ-002/003 |
| D-057 | P1.5 combines a fixed canonical celestial registry with deterministic single-cell spatial interest | Accepted as a bounded correctness slice; dynamic cells, handoff, frontier materialization, and production scale remain later | WORLD-009, SIM-013/014, F-014/059 |
| D-058 | P1.6 proves a fenced durable lifecycle and production-only background execution for one fixed cell | Accepted as the next bounded dependency; it must reuse one atomic canonical production quantum and does not complete multi-cell scheduling or handoff | SIM-006/015, F-023/060, ADR-0022 |
| D-059 | P1.7 proves durable assignment and atomic mobile-aggregate handoff between two adjacent cells | Accepted as a bounded correctness slice: one directory compare-and-swap is the authority-transfer point; cell fencing and aggregate placement generations are both required; general cross-cell physics and megastructure partitioning remain later | WORLD-010, SIM-016/017/018, F-061, ADR-0023 |

## Amendment rule

A decision changes only through:

1. A documented founder/governance decision.
2. Updated affected requirements.
3. A new or superseding ADR when architecture changes.
4. Updated feature and test scope.
