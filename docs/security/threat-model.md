# Threat model

**Status:** Initial baseline

## Protected assets

- Canonical inventory and ownership.
- BIT and commodity custody.
- Market receipts and AMM reserves.
- Verse DAO and company treasuries.
- Passkey and recovery authority.
- Content-manifest signing.
- Server leases and fencing tokens.
- Lifecycle proofs.
- Personal information.
- Release-signing keys.
- Administrative creative authority.

## Adversaries

- Modified native clients.
- Malicious browser or Web3 applications.
- Bots attempting resource or market exploitation.
- Compromised account sessions.
- Malicious mod authors.
- Dishonest private servers.
- Compromised simulation workers.
- Relayer or indexer failures.
- Bridge compromise.
- Safe signer compromise or collusion.
- Insider abuse.
- Economic attackers using valid transactions.
- Infrastructure failures mistaken for malicious activity.

Bots and AI participation are allowed. The security objective is not to prove humanity; it is to enforce rules, costs, permissions, and conservation equally.

## Critical threats and controls

| Threat | Primary controls |
| --- | --- |
| Inventory duplication | Authoritative operations, idempotency IDs, conservation invariants, double-entry custody |
| Client physics cheating | Server authority, intent validation, reconciliation, rate/acceleration limits |
| Cross-cell double ownership | Fenced leases, prepare/commit transfer, aggregate versions |
| Private-server import | Namespace validation at every canonical boundary |
| Creative asset laundering | Separate namespace, non-economic flag, provenance checks |
| AMM receipt insolvency | Continuous custody-to-supply reconciliation, paused deposits, invariant alarms |
| Lifecycle proof omission | Published batch ranges, gap detection, independent proof verification |
| Passkey/session theft | Scoped session keys, expiry, limits, fresh confirmation, delayed recovery |
| Malicious third-party app | Explicit scopes, revocation, rate limits, signed intents |
| Mod sandbox escape | No native code initially, capability sandbox, resource budgets, review |
| Bridge compromise | Limits, monitored implementation, route-specific pause, reconciler |
| Governance key compromise | 2-of-3 Safe, hardware-separated signers, transaction simulation, timelocks |
| Infinite resource recipe | Static conservation validation, staging simulation, emergency production pause |
| Cleanup bug | Durable timers, exclusions, warnings, tombstones, outage freeze |
| Release compromise | Signed artifacts, reproducible builds, checksum transparency |

## Economic exploits

Valid-looking activity may still exploit a broken rule.

Examples:

- Cyclic recipes producing net material.
- Energy-free production loops.
- Rounding extraction.
- Flash-liquidity manipulation of a dependent price.
- Wash trading to trigger rewards.
- Infinite salvage recursion.
- Registration contracts preserving trash at negligible cost.
- Artificial sector activation to exhaust server resources.

Controls include property tests, fixed-point arithmetic, explicit oracles, time-weighted references where needed, resource budgets, anomaly alerts, and public economic data.

## Market volatility versus failure

A sharp price move is not itself a security incident.

Eligible incident indicators include:

- Receipt supply exceeding custody.
- Negative or impossible reserves.
- Unauthorized mint.
- Contract implementation change.
- Broken bridge accounting.
- Canonical resource creation outside an authorized source.
- Non-idempotent settlement.
- Invalid state root.
- Loss of withdrawal solvency.

## Administrative authority

Creative and security actions require:

- Separate identities.
- Least-privilege scopes.
- Fresh authentication.
- Immutable audit event.
- Public reason.
- No shared personal accounts.

Creative authority must never hold a code path that silently converts assets to canonical economic value.

## Security gates

Before public alpha:

- Threat-model review.
- Dependency and asset bill of materials.
- Private disclosure channel.
- Backup restoration exercise.
- Client/server protocol fuzzing.
- Mod sandbox tests.
- Economy invariant suite.

Before mainnet:

- Smart-contract audits.
- Stateful fuzzing.
- Safe and signer ceremony.
- Bridge limits.
- Relayer failure drills.
- Pause/unpause drill.
- Public contract manifest.
- Incident communications plan.
