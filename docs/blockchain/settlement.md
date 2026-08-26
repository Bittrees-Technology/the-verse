# Blockchain settlement architecture

**Status:** Proposed; contract design not yet approved

## Design objective

Expose verifiable custody, trades, and lifecycle history while keeping blockchain latency, signatures, gas, and reorganization out of the real-time simulation loop.

## Settlement layers

### Gameplay layer

- Immediate server-authoritative actions.
- Personal and world inventory.
- Physics, damage, mining, and production.
- Durable event journal.

### Market settlement layer

- Deposited commodity receipts.
- AMM positions and swaps.
- Unique-asset escrow.
- Company treasury operations.
- Station registration/insurance.
- Gas-sponsored smart-account operations.

### Ethereum canonical layer

- BIT and bNOTE integration.
- Verse DAO Safe.
- Canonical bridge custody.
- High-value settlement.
- Periodic lifecycle commitments.

Base is the leading proposed market-settlement network after its BIT deployment, bridge, and governance are validated.

## Hidden transaction experience

Routine flow:

1. User authenticates with a passkey.
2. Application builds a typed intent.
3. User's scoped session authority signs within limits.
4. Relayer submits a UserOperation or batched transaction.
5. Paymaster sponsors eligible gas.
6. Reconciler waits for required confirmations.
7. UI presents a gameplay outcome rather than raw transaction mechanics.

Explicit fresh confirmation is required for:

- Large-value swaps.
- Withdrawals.
- Recovery changes.
- New delegated permissions.
- Governance or Safe actions.
- Cross-chain transfers above policy thresholds.

## Commodity receipt model

A receipt identifies:

- Market/custody location.
- Commodity schema and grade.
- Quantity.
- Content-manifest version.
- Canonical custody contract.
- Source universe.
- Settlement chain.

ERC-1155 is a candidate for standardized location receipts because it supports many fungible IDs and batches. It is not accepted until gas, wallet, AMM, and bridge behavior are benchmarked.

## AMM architecture

Potential contracts:

- `MarketFactory`
- `CommodityReceipt`
- `CustodyEscrow`
- `PoolFactory` or approved Uniswap-compatible integration
- `UniqueAssetMarket`
- `StationRegistration`
- `LifecycleRootRegistry`
- `ChainRegistry`
- `EmergencyController`

The system should prefer audited protocols and minimal adapters over inventing a new AMM.

The Verse DAO deploys canonical AMM contracts and supplies the capital market's initial BIT and commodity liquidity. Pool parameters and seed transactions must be published as governance records. This work begins only after gameplay, voxel mining, and the internal economic proof validate the underlying asset lifecycle.

## Deposits and reconciliation

A deposit spans game and chain state. It uses a saga with compensating transitions, never a distributed lock held across chain confirmation.

```text
world asset
→ local lock
→ custody accepted
→ receipt mint submitted
→ confirmation
→ market enabled
```

Failures before confirmation leave the asset locked or safely reversible. Reconciliation continuously checks that custody quantities equal outstanding receipts plus pending operations.

## Lifecycle commitments

The batcher:

1. Selects finalized canonical events.
2. Normalizes them under a versioned settlement schema.
3. Constructs a Merkle tree.
4. Stores the event bundle in content-addressed storage.
5. Posts root and content hash.
6. Waits for confirmation.
7. Publishes proof availability.

The root contract does not interpret game physics. It proves that the operator committed to a specific lifecycle history.

## Consumption

- Off-chain inventory: server consumes immediately; event is batched.
- Market receipt: must be redeemed/burned before the underlying asset returns to the world.
- Destroyed receipt-backed custody: requires an explicit recovery/governance path and public reconciliation event.

## Bridges

A bridge must define:

- Canonical BIT origin.
- Lock/mint or burn/release behavior.
- Upgrade and pause authority.
- Rate limits.
- Confirmation assumptions.
- Replay protection.
- Liquidity versus canonical supply.
- Failure and recovery.
- Treatment of chain reorganizations.

The Verse shall not independently designate a wrapped BIT as canonical without Bittrees coordination.

## Pausing

The Verse DAO or delegated Security Council may pause narrowly scoped functions.

Potential scopes:

- New deposits.
- Receipt minting.
- New listings.
- AMM swaps.
- Withdrawals.
- Bridge route.
- Settlement-root publication.
- Specific mod manifest.

A pause has no automatic expiration. It remains until authorized unpause.

Every pause records:

- Scope.
- Reason.
- Authority.
- Evidence/reference.
- Time.
- User impact.
- Recovery state.

Ordinary market volatility is not an eligible reason.

## Testing requirements

Before mainnet:

- Unit and invariant tests.
- Stateful fuzzing.
- Fork tests.
- Reorganization tests.
- Failed-relayer and duplicate-event tests.
- Bridge accounting tests.
- Emergency pause/unpause exercise.
- Independent audit.
- Public deployment manifest.
- Treasury and Safe operational rehearsal.
