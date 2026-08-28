# Open questions, blockers, and resolved gates

This register preserves resolved gates for traceability and lists the decisions that still remain after the initial planning reconciliation.

## Resolved in S0 approval

### OQ-001 — Open Metaverse governing framework

The Verse adopts the Open Metaverse contractual governing framework defined by Section 13 of the Bittrees Bounties Terms of Use effective 2026-08-12. The framework is not a nation, sovereign, court, territorial jurisdiction, or immunity from law; applicable nonwaivable rights and law remain controlling to the extent of conflict.

**Status:** Resolved for the product baseline. Deployment-specific operator disclosures, policies, consent, and mandatory obligations remain a production release check under the accepted framework.

### OQ-005 — Capital-market liquidity

The Verse DAO deploys the official AMMs and deposits the capital market's initial BIT and commodity liquidity. Exact pool parameters, reserve sizes, fees, and position ownership remain part of OQ-004 and the economic simulation.

**Status:** Resolved as to authority and source.

### OQ-006 — Death-drop access

Death drops are owner/team-only for 15 minutes, then publicly salvageable until their six-hour expiry.

**Status:** Resolved.

### OQ-007 — Derelict salvage start

Eligible unpowered structures become publicly salvageable at 24 hours and are deleted at 36 hours.

**Status:** Resolved.

## Critical before Web3 implementation

### OQ-002 — BIT acquisition interface

The existing bNOTE proxy is known, but the Verse-specific purchase/swap contracts are not deployed. Required:

- Verified bNOTE ABI and source.
- Supported BTREE/WBTC routes.
- Quote functions.
- Mint/purchase functions.
- Slippage and deadline parameters.
- Events.
- Upgrade administrator.
- Audit status.

**Status:** Deferred until gameplay, voxel mining, and the internal economic proof pass; required before P3 testnet integration.

### OQ-003 — Base BIT and bridge

Confirm:

- Base BIT address.
- Whether it is native, bridged, or independently mintable.
- Current implementation and administrator.
- Canonical Ethereum/Base bridge.
- Supply reconciliation.
- Rate limits and pause authority.

**Status:** Deferred until gameplay, voxel mining, and the internal economic proof pass; required before P3 testnet integration.

### OQ-004 — Market contracts and curves

Choose:

- Uniswap protocol/version or compatible implementation.
- Receipt-token standard.
- Pool deployment model.
- Curve by commodity type.
- Oracle usage.
- Fee tiers.
- Liquidity-position ownership.

**Status:** Requires economic simulation and ADR.

## Critical before later gameplay and economy rollout

### OQ-008 — Registration pricing

Determine whether cleanup-exemption contracts are priced by:

- Flat term.
- Grid mass.
- Block count.
- Spatial footprint.
- Server cost.
- Market auction.
- Combination.

### OQ-009 — Dynamic megastructures

Define when a free grid transitions from ordinary single-cell physics into a
partitioned capital-ship model and which operations remain possible during
transition. The later design must resolve:

- canonical ownership of static topology spanning cell boundaries;
- external contacts, projectiles, docking, and mechanical constraints;
- conveyor, power, atmosphere, damage, and control systems crossing cells;
- interior-cell creation and occupants moving between exterior/interior space;
- partitioned capital-ship motion, split, merge, anchoring, and recovery; and
- finite work and transfer budgets without creating a permanent product size
  cap.

**Status:** Does not block the bounded P1.7 two-cell handoff. P1.7 transfers
only an isolated EVA actor or ordinary unanchored grid whose complete closure
fits one package; unsupported structures remain source-authoritative with an
explicit `partition_required` result. OQ-009 must be resolved before F-050,
F-051, arbitrary cross-cell grids, or production megastructure claims.

### OQ-010 — Planet separation and travel speed

P1.5 resolves the architecture needed for fixed bodies: a universe manifest
selects a positive minimum-separation threshold, registry validation applies it
deterministically, and the proof threshold is versioned without being presented
as production geography or route balance.

Before multi-day routes or frontier materialization, set production ranges for:

- Minimum planet separation.
- Ship cruise speeds.
- Fuel costs.
- Interception windows.
- Journey duration targets.

**Status:** Partially resolved. The configurable validation rule and P1.5 proof
threshold do not block the fixed-registry implementation. Production numbers
remain required before F-015 frontier materialization and F-016 route rollout.

## Governance and deployment

### OQ-011 — Test Safe deployments

Record deployment transactions, owners, threshold, singleton, and code hashes for Sepolia, Base, and Base Sepolia after the founder deploys the same-address Safes.

### OQ-012 — Security Council composition

Determine members, threshold, chain deployments, scopes, replacement procedure, and public communications duty.

### OQ-013 — Community governance

Decide whether the Safe remains the sole policy authority initially or executes proposals from a membership, reputation, token, or bicameral governance process.

## Experience and content

### OQ-014 — Original faction and fiction

Create original:

- Setting history.
- Capital planet identity.
- Factions and companies.
- Technology language.
- Architecture and interface motifs.
- Conflict and exploration premise.

### OQ-015 — Linux packaging

P0 uses a portable archive containing the client, authoritative server, launcher,
version record, licenses, and checksums. Before a signed public release, choose
whether that archive remains primary or is supplemented/replaced by:

- AppImage.
- Flatpak.
- Portable archive plus updater.
- Multiple formats.

### OQ-016 — Account recovery provider

Select passkey smart-account, recovery, relayer, and paymaster implementations after threat modeling and cost testing.

## Deferred planning inputs

- Infrastructure budget.
- Calendar launch target.
- Final production staffing model.

These are intentionally deferred, but public-alpha and production estimates remain non-binding until they are supplied.
