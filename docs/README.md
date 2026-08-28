# Documentation map

These documents form the initial specification baseline.

- [Glossary](glossary.md)

## Product

- [Vision](product/vision.md)
- [Canonical requirements](product/requirements.md)
- [Reconciled decision register](product/decision-register.md)
- [Feature catalog](product/feature-catalog.md)
- [Visual direction](product/visual-direction.md)

## Gameplay

- [P0.2 Salvage Frontier](gameplay/salvage-frontier.md)
- [P0.3 visual engineering checkpoint](gameplay/engineering-realism.md)
- [P0.4 Engineering Hands](gameplay/engineering-hands.md)
- [P0.5 Planetary Logistics](gameplay/planetary-logistics.md)
- [P0.6 Orbital Operations](gameplay/orbital-operations.md)
- [P0.7 Server-Authoritative Contact Physics](gameplay/contact-physics.md)
- [P0.8 Survival Death Foundation](gameplay/survival-death.md)
- [P0.9 Authoritative EVA Physics](gameplay/authoritative-character-motion.md)
- [P0.10 Authoritative Grounded and Magnetic Locomotion](gameplay/authoritative-grounded-locomotion.md)
- [P1.0 Authoritative Multi-player Cell](gameplay/authoritative-multiplayer-cell.md)
- [P1.1 Actor-owned Industry and Engineering](gameplay/actor-owned-industry.md)
- [P1.2 Private Player State Projection](gameplay/private-state-projection.md)
- [P1.4 Physical Refining and Manufacturing](gameplay/physical-industry.md)
- [P1.5 Celestial Registry and Interest-managed Visibility](gameplay/celestial-registry-and-interest-management.md)
- [P1.6 Durable Single-cell Lifecycle](gameplay/durable-single-cell-lifecycle.md)
- [P1.7 Durable Two-cell Handoff](gameplay/durable-two-cell-handoff.md)
- [Authoritative Hand-tool Targeting](gameplay/authoritative-hand-tool-targeting.md)

## Architecture

- [System overview](architecture/system-overview.md)
- [Operation idempotency and retry contract](architecture/operation-idempotency.md)
- [Current P0 implementation guide](architecture/p0-implementation.md)
- [Universe simulation](architecture/universe-simulation.md)
- [Data and events](architecture/data-and-events.md)
- [Clients and public APIs](architecture/clients-and-apis.md)
- [P1 latest-state replication backpressure](architecture/replication-backpressure.md)
- [P1 actor authority architecture](architecture/p1-actor-authority.md)
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
- [P0.7 Apple Silicon contact-physics benchmark](benchmarks/P0.7-contact-physics-apple-silicon.md)
- [P0.10 Apple Silicon grounded-locomotion benchmark](benchmarks/P0.10-grounded-locomotion-apple-silicon.md)
- [P0.10 hosted Linux grounded-locomotion benchmark](benchmarks/P0.10-grounded-locomotion-linux.md)
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
