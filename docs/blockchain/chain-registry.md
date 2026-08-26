# Chain and contract registry

**Status:** Partially verified; deployment entries marked pending

All addresses must be checksum-normalized and keyed by chain ID. Configuration must never infer a chain from an address.

## Networks

| Network | Chain ID | Role |
| --- | ---: | --- |
| Ethereum mainnet | 1 | Canonical BIT, treasury, governance, high-value settlement |
| Sepolia | 11155111 | Ethereum test integration |
| Base | 8453 | Proposed lower-cost markets and settlement |
| Base Sepolia | 84532 | Proposed Base test integration |
| Local Anvil/devnet | project-defined | Automated contract tests only |

## Ethereum mainnet

### BIT

- Address: `0x57A447E4d5e18A9423408C365963A73F08B9d18C`
- Type: ERC-20-compatible upgradeable proxy.
- Name/symbol: BIT.
- Decimals: 18.
- Supply observed 2026-08-26: 350,000 BIT.
- Implementation observed 2026-08-26: `0xa27b118c0770939295f052ae1b003366e5ef806f`.
- Upgrade authority: Bittrees; exact administrator and monitoring policy must be documented.

### BTREE

- Address: `0x6bDdE71Cf0C751EB6d5EdB8418e43D3d9427e436`
- Symbol: BTREE.
- Decimals: 18.
- Supply observed 2026-08-26: 21,000,000 BTREE.

### WBTC

- Address: `0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599`
- Symbol: WBTC.
- Decimals: 8.

### Bittrees bNOTE application

- Proxy address: `0xf1AAfFc982B5F553a730a9eC134715a547f1fe80`.
- Type: upgradeable application proxy; it is not an ERC-20 token.
- Implementation observed 2026-08-26: `0x0358631d5b844a06f001946ceb88112530046cbf`.
- Role: existing Bittrees mechanism from which BIT may be sourced.
- Required before P3 testnet integration: verified ABI, supported purchase routes, quote behavior, slippage rules, events, failure modes, upgrade authority, and audit status.
- Sequencing: deferred until gameplay, voxel mining, and the internal economic proof have passed; it does not block P0, P1, or internal P2 work.

### Verse DAO Safe

- Address: `0x4E7cf530B84DAE10c4500737C3408761a9385051`.
- Type: Safe v1.4.1 proxy.
- Owners observed 2026-08-26: three.
- Threshold observed 2026-08-26: two.
- Singleton observed 2026-08-26: `0x41675C099F32341bf84BFc5382aF534df5C7461a`.
- Role: Verse DAO executor and initial treasury authority.

## Sepolia

### BIT

- Address: `0x57A447E4d5e18A9423408C365963A73F08B9d18C`.
- Decimals: 18.
- Supply observed 2026-08-26: 331,000 BIT.
- Implementation observed 2026-08-26: `0xa27b118c0770939295f052ae1b003366e5ef806f`.

### Verse DAO Safe

- Intended address: `0x4E7cf530B84DAE10c4500737C3408761a9385051`.
- Status observed 2026-08-26: not yet deployed.
- Deployment owner: project founder.
- Expected threshold: two of three.

## Base

- BIT deployment: reported to exist with no minted supply.
- Exact address and implementation: pending registry confirmation.
- Canonical bridge: not yet specified.
- Verse DAO Safe intended address: `0x4E7cf530B84DAE10c4500737C3408761a9385051`, pending deployment.
- AMM and market contracts: not deployed.
- Deployment authority: Verse DAO after pool design and economic simulation are accepted.

## Base Sepolia

- Verse DAO Safe intended address: `0x4E7cf530B84DAE10c4500737C3408761a9385051`, pending deployment.
- BIT, bridge, AMM, market, and settlement contracts: pending.

## Registry requirements

Every production entry must include:

- Chain ID and finality policy.
- Address.
- Contract kind.
- Code hash.
- Proxy kind.
- Implementation and administrator.
- Deployment transaction.
- Verified source/ABI location.
- Audit report.
- Pause authority.
- Monitoring alerts.
- Deprecation and migration plan.

An unexpected code, implementation, or administrator change automatically disables new deposits until reviewed.
