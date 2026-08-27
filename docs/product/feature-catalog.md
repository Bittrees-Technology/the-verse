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
| F-012 | Multiple players in one cell | P1 | F-002 |
| F-013 | Dynamic cell assignment and handoff | P1 | Universe directory |
| F-014 | Fixed celestial registry | P1 | Coordinate schema |
| F-015 | Procedural frontier generation | P1 | Generation rules |
| F-016 | Multi-day autopilot travel | P1 | Route service |
| F-017 | Browser journey and asset status | P1 | Public API |
| F-018 | Capital hard safe zone | P1 | Policy volumes |
| F-019 | Offline structures and powered turrets | P1 | Cell wake-up |
| F-020 | Death drop, 15-minute recovery grace, and six-hour cleanup | P1 | Lifecycle scheduler |
| F-021 | Unpowered derelict, 24-hour public salvage, and 36-hour cleanup | P1 | Power, scheduler |
| F-022 | Registered-station cleanup exception | P2 | BIT contract |
| F-023 | Mining/refining/manufacturing graph | P1 | Inventory ledger |
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
| F-041 | Browser spectating | P3 | Read model/stream |
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
