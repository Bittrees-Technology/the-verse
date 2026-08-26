# Economy and markets

**Status:** Proposed baseline

## Economic loop

```text
Discover → extract → haul → refine → manufacture
→ construct/use → repair/recycle/destroy → trade/reinvest
```

Value is created through location, scarcity, labor, energy, infrastructure, information, risk, and coordination.

## Participants

The economy permits:

- Human players.
- User-operated bots.
- Autonomous AI agents.
- System NPCs.
- Companies and DAOs.
- Market operators.
- Logistics and insurance providers.

All participants use the same conservation and market contracts. System NPC activity must be attributable and budgeted.

## BIT base pair

BIT is the default quote asset for official markets. The product may display reference values in other units, but canonical prices are recorded as exact integer BIT quantities using 18 decimal places.

No floating-point arithmetic is permitted in settlement, inventory, pool, fee, or contract accounting.

## Market geography

Each market has:

- Stable market ID.
- Physical custody location.
- Operator.
- Governance policy.
- Supported asset schemas and grades.
- BIT liquidity.
- Fee configuration.
- Settlement chain.
- Delivery and withdrawal rules.

A commodity deposited at one station is not interchangeable with the same commodity at another location unless transport or a contract moves it.

## Capital market

The capital market is:

- Inside the absolute safe zone.
- The default onboarding venue.
- Operated under Verse DAO policy.
- Deployed by the Verse DAO when implemented onchain.
- Designed for the simplest possible “deposit and sell” flow.
- Expected to have the most visible liquidity.
- Open to humans and agents.
- Algorithmically priced.

It is not guaranteed to offer a fixed price or unlimited BIT. If BIT liquidity is exhausted, the price and quote must reflect that condition honestly.

## Commodity AMMs

Standardized fungible goods use AMMs.

A pool key includes:

```text
market_id
commodity_schema_id
grade
settlement_chain
BIT_address
receipt_token_address_or_id
fee_tier
curve_type
```

A deposited commodity is locked in canonical market custody. The settlement layer issues or activates a location-specific receipt. That receipt is the asset traded against BIT.

Initial curve candidates:

- Constant product for volatile or thin markets.
- Concentrated liquidity when operators can manage ranges.
- Stable-swap curves for tightly substitutable grades.
- Weighted pools for selected multi-asset indices.

Curve selection requires simulation and an ADR before contract implementation.

## Price behavior

Prices are determined by pool state and algorithmic rules.

The Verse DAO and Security Council may not intervene merely because:

- Prices rise or fall.
- One participant deposits a large quantity.
- One participant executes a large valid sale.
- An arbitrageur profits.
- Regional prices diverge.

Permitted intervention is limited to protocol failures such as corrupted custody, unlimited-resource creation, bridge compromise, pool accounting failure, or malicious code.

## Liquidity

Liquidity may be supplied by:

- Verse DAO treasury.
- Bittrees.
- Companies.
- Players.
- Independent market operators.
- Automated strategies.

The Verse DAO supplies the initial BIT and commodity reserves for the capital market. Its deployment and seed-liquidity transaction parameters are public governance records. Additional participants may add liquidity under the pool rules.

Liquidity providers must see:

- Pool reserves.
- Fees.
- Slippage.
- Contract and chain.
- Asset location.
- Withdrawal constraints.
- Relevant risks.

Exact initial reserve sizes, curves, fee tiers, position ownership, and rebalancing authority require economic simulation and a separate accepted ADR before deployment.

## Unique and heterogeneous goods

The following should not be forced into commodity AMMs:

- Unique ships and stations.
- Damaged or customized equipment.
- Blueprints.
- Named or historically significant assets.
- Skins, avatars, and clothing.
- Contract rights.

They use fixed listings, English/Dutch auctions, bids, or specialized bonding curves.

## Production and supply

Canonical recipes define:

- Inputs.
- Outputs.
- Allowed loss.
- Machine class.
- Energy.
- Time.
- Skill or permission requirements.
- Content-manifest version.

New frontier sectors expand raw supply. Existing mined deposits do not silently regenerate unless a future rule explicitly introduces regeneration.

## Sinks

Potential economic sinks include:

- Market fees.
- Station registration.
- Cleanup exemption contracts.
- Insurance.
- Repair losses.
- Refining and recycling losses.
- Energy and fuel.
- Cargo storage.
- Docking and transit services.
- Jump infrastructure.
- Content publication or governance deposits.

A sink must not destroy user value without an explicit rule and event.

## Work contracts

Supported contracts include:

- Mining.
- Delivery.
- Manufacturing.
- Construction.
- Repair.
- Defense.
- Escort.
- Exploration.
- Bounties.
- Facility operation.
- Employment and payroll.
- Blueprint licensing.
- Revenue sharing.

Contracts can escrow BIT, goods, access rights, or collateral. Completion must use authoritative events when possible.

## Economic monitoring

Public dashboards should report:

- Commodity creation and destruction.
- Market reserves and prices.
- Volume and slippage.
- Regional spreads.
- Concentration.
- Production capacity.
- Cleanup and destruction.
- Insurance/registration status totals.
- System NPC activity.
- Settlement reconciliation.

Monitoring informs users; it does not justify discretionary price control.
