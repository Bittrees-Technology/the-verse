# Documentation map

These documents form the initial specification baseline.

- [Glossary](glossary.md)

## Product

- [Vision](product/vision.md)
- [Canonical requirements](product/requirements.md)
- [Reconciled decision register](product/decision-register.md)
- [Feature catalog](product/feature-catalog.md)
- [Visual direction](product/visual-direction.md)

## Architecture

- [System overview](architecture/system-overview.md)
- [P0.1 implementation guide](architecture/p0-implementation.md)
- [Universe simulation](architecture/universe-simulation.md)
- [Data and events](architecture/data-and-events.md)
- [Clients and public APIs](architecture/clients-and-apis.md)
- [Proposed repository layout](architecture/repository-layout.md)

## Economy and blockchain

- [Economy and markets](economy/economy-and-markets.md)
- [Asset lifecycle](economy/asset-lifecycle.md)
- [Chain registry](blockchain/chain-registry.md)
- [Settlement architecture](blockchain/settlement.md)

## Governance, security, and operations

- [Governance and modding](governance/governance-and-modding.md)
- [Open Metaverse governing framework](governance/open-metaverse-framework.md)
- [Threat model](security/threat-model.md)
- [Operations and cleanup](operations/operations-and-cleanup.md)

## Delivery

- [Roadmap](roadmap/roadmap.md)
- [Core protocol invariants](specifications/core-invariants.md)
- [P0 simulation proof](specifications/P0-simulation-proof.md)
- [P0.1 Apple Silicon benchmark](benchmarks/P0.1-apple-silicon.md)
- [Architecture decision records](decisions/README.md)
- [Open questions](open-questions.md)

## Templates

- [Feature specification](templates/feature-spec-template.md)
- [Architecture decision record](templates/adr-template.md)
- [Official mod proposal](templates/mod-proposal-template.md)

## Document status

Documents use the following labels:

- **Accepted:** a confirmed product or architecture decision.
- **Proposed:** recommended but awaiting validation.
- **Blocked:** cannot safely be finalized without missing information.
- **Deferred:** intentionally outside the current milestone.

The requirements document is the canonical product baseline. ADRs explain why durable technical decisions were made. In a conflict, the newest accepted ADR wins for architecture, while explicit product decisions require updating the requirements.
