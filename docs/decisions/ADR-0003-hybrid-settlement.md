# ADR-0003: Hybrid settlement

**Status:** Accepted

## Context

Real-time mining, physics, and production cannot wait for Ethereum transactions, while market custody and lifecycle history should be publicly verifiable.

## Decision

Keep ordinary inventory and simulation off-chain. Directly represent deposited market assets and BIT settlement on-chain. Commit ordinary lifecycle events in Merkle batches.

## Consequences

- Gameplay remains responsive.
- Users can verify committed history.
- Reconciliation and proof availability become core services.
- The operator must publish complete batch data, not only roots.
