# Open questions and blockers

This register contains decisions that remain unresolved after the initial planning reconciliation.

## Critical before economic implementation

### OQ-001 — Real-world operator and jurisdiction

“The open metaverse” describes protocol governance but is not a real-world legal jurisdiction or legal entity. Before operating a real-value market, the project must identify:

- Contracting entity.
- Place of incorporation or recognized organization.
- Governing law and dispute venue.
- Tax and accounting responsibility.
- Consumer and data-protection obligations.
- Treatment of minors and restricted regions.
- Custody and financial-regulation analysis.

**Status:** Blocked; qualified legal counsel required.

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

**Status:** Blocked on Bittrees contract specification.

### OQ-003 — Base BIT and bridge

Confirm:

- Base BIT address.
- Whether it is native, bridged, or independently mintable.
- Current implementation and administrator.
- Canonical Ethereum/Base bridge.
- Supply reconciliation.
- Rate limits and pause authority.

**Status:** Blocked on deployment manifest.

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

### OQ-005 — Capital-market liquidity

Identify who deposits the initial BIT and commodities and what mandate governs that liquidity. An AMM cannot guarantee useful buy quotes without funded reserves.

**Status:** Governance/economic decision.

## Critical before gameplay implementation

### OQ-006 — Death-drop access

Should a death container be:

- Immediately public.
- Owner/team-only for a grace period, then public.
- Private until expiry unless the death occurred in PvP.

Expiration is fixed at six hours.

### OQ-007 — Derelict salvage start

The specification proposes public salvage at 24 hours and deletion at 36 hours. Confirm or replace the 24-hour threshold.

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

Define when a free grid transitions from ordinary single-cell physics into a partitioned capital-ship model and which operations remain possible during transition.

### OQ-010 — Planet separation and travel speed

Set initial numeric ranges for:

- Minimum planet separation.
- Ship cruise speeds.
- Fuel costs.
- Interception windows.
- Journey duration targets.

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

Choose primary direct-download format:

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
