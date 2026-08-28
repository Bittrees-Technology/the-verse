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
- Universe-manifest and celestial-registry integrity.
- Canonical universe addresses and fixed-body identity.
- Session interest boundaries, actor-private projections, and replication
  epochs.

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
- Clients probing entity IDs, coordinates, or spectator anchors to widen
  spatial disclosure.
- Operators or compromised workers substituting a registry, universe manifest,
  or interest policy.

Bots and AI participation are allowed. The security objective is not to prove humanity; it is to enforce rules, costs, permissions, and conservation equally.

## Critical threats and controls

| Threat | Primary controls |
| --- | --- |
| Inventory duplication | Authoritative operations, idempotency IDs, conservation invariants, double-entry custody |
| Client physics cheating | Server authority, intent validation, reconciliation, rate/acceleration limits |
| Coordinate alias or overflow | Canonical integer normalization, checked arithmetic, bounded cell-local physics, golden cross-platform vectors |
| Celestial registry substitution | Domain-separated content hashes, universe-manifest binding, schema compatibility, fail-before-replay validation |
| Body overlap or movement | Integer exclusion-volume validation, immutable fixed addresses, explicit migration receipts |
| Invalid moon ancestry | Required existing planet parent, self/non-planet/cycle rejection, sorted registry validation |
| Cross-cell double ownership | Fenced leases, prepare/commit transfer, aggregate versions |
| Private-server import | Namespace validation at every canonical boundary |
| Creative asset laundering | Separate namespace, non-economic flag, provenance checks |
| AMM receipt insolvency | Continuous custody-to-supply reconciliation, paused deposits, invariant alarms |
| Lifecycle proof omission | Published batch ranges, gap detection, independent proof verification |
| Passkey/session theft | Scoped session keys, expiry, limits, fresh confirmation, delayed recovery |
| Malicious third-party app | Explicit scopes, revocation, rate limits, signed intents |
| Interest enlargement or remote camera probing | Actor-derived anchor, bounded spectator grants, server-owned radii and hysteresis, generic denials |
| Cross-session private projection leak | Audience-and-epoch cache keys, per-session projection, fail-closed serialization, no canonical fallback |
| Hidden-state leakage through deltas | No unseen IDs/counts/removals/view-hash inputs, bounded removal reasons, actor-private overlay after spatial filter |
| Delta replay, gap, or stale baseline | Session and interest epochs, contiguous sequence, previous/result view hashes, one-baseline recovery |
| Slow-client memory or CPU exhaustion | Bounded bytes/entities/work/age, latest-state coalescing, separate control queue, disconnect on baseline failure |
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

## Spatial privacy and residual disclosure

Interest management reduces unnecessary disclosure but is not the authority
control for gameplay and is not a stealth or radar system. Intent validation
uses canonical state even when a target is loaded. A missing entity means only
that it is absent from the session projection.

Protocol `16` exposes no ID, count, removal, projected-view-hash contribution,
or entity-specific error for an entity that was never visible to the session.
Removal is limited to a previously visible stable ID plus
`out_of_interest`, `destroyed`, or `transferred`; it contains no destination,
owner, attacker, inventory, cause, or hidden coordinates. Projection attaches
private data only after public interest is derived from the server-bound
audience.

The canonical event/tick frontier and global canonical world commitment remain
on the interest stream for authoritative reconciliation. They and observable
packet timing can reveal that out-of-view activity occurred. The separate view
hash commits only to the authorized projected subset. P1.5 therefore promises
field confidentiality and bounded spatial disclosure, not traffic-analysis
secrecy, fog of war, or zero-knowledge state.

## Registry and replication incident controls

- A universe-manifest, registry, content, world, event, projection, protocol,
  or interest version mismatch fails before state delivery or journal replay.
- The P1.5 schema tuple is protocol `16`, projection `3`, world `18`, event
  `14`, content `11`, manifest `p1.5.0`, registry `1`, universe manifest `2`,
  and interest `1`; partial deployment is not allowed.
- Registry hashes and body-parent/separation validation run at build, startup,
  recovery, and migration boundaries.
- Upgrade and rollback drain incompatible sessions. Replication epochs and
  client caches are discarded rather than migrated.
- A compromised or exhausted stream may be closed without affecting canonical
  simulation. Backpressure cannot change ownership, interest authority, or
  canonical destruction.

## P1.6 lifecycle incident controls

- The P1.6 schema tuple is protocol `17`, projection `3`, world `19`, event
  `15`, content `11`, manifest `p1.5.0`, registry `1`, universe manifest `3`,
  interest `1`, lifecycle control `1`, and schedule occurrence `1`; partial
  deployment is not allowed.
- Every event append and snapshot rechecks the exact live holder, nonzero
  fencing token, expiry, universe, cell and trust roots. Renewal uncertainty
  stops mutation, and a successor token must exceed every recovered token.
- Scheduler redelivery uses a stable occurrence key. Duplicate delivery cannot
  repeat production; changed material at the same key, a skipped sequence, a
  clock rollback, arithmetic overflow, or corrupt control record fails closed.
- Background mode has no physics, player, oxygen, combat, AI, cleanup, market,
  or replication authority. Anonymous observation cannot wake or retain the
  fixed proof cell.
- Catch-up is sequential and bounded. Backlog is visible and cannot be hidden
  by skipping, coalescing, or one oversized elapsed-time event.

## Security gates

Before public alpha:

- Threat-model review.
- Dependency and asset bill of materials.
- Private disclosure channel.
- Backup restoration exercise.
- Client/server protocol fuzzing.
- Address normalization, registry hash/parent/separation, and interest
  baseline/delta fuzzing.
- Cross-session privacy and slow-consumer resource-budget tests.
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
