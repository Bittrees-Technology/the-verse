# Operations, persistence, and cleanup

**Status:** Proposed baseline

## Service objectives

Initial objectives must be measured before promises are made. Production targets should eventually cover:

- Account and market API availability.
- Simulation cell availability.
- Persistence recovery point and time.
- Cross-cell handoff success.
- Market reconciliation delay.
- Settlement proof publication delay.
- Client crash-free sessions.
- Direct-download update success.

## Cell operations

Each simulation worker exposes:

- Tick duration and backlog.
- Entity/grid/block counts.
- Active and sleeping physics bodies.
- Voxel mesh queue.
- Network replication load.
- Event journal lag.
- Snapshot age.
- Lease/fencing token.
- Power, production, combat, and cleanup queues.

A cell may be drained and replaced without losing canonical state.

## Unpowered cleanup

### Qualifying power

A structure is powered for cleanup purposes when an eligible control core or beacon receives the required continuous power heartbeat. The required power may scale with registered construct mass or complexity.

### Canonical timeline

- 0 hours: unpowered timer begins.
- 24 hours: public salvage eligibility begins.
- 30 hours: urgent derelict warning.
- 35 hours: final warning.
- 36 hours: remaining eligible object is destroyed and removed.

### Exclusions

- Active market custody.
- Valid registered-station cleanup exemption.
- Explicit capital/admin infrastructure.
- Objects affected by a verified service outage.
- Objects already inside an atomic transfer.

### Cleanup event

Cleanup writes:

- Subject ID.
- Last owner/company.
- Location.
- Power-loss time.
- Warning history.
- Registration status.
- Salvage activity.
- Conserved outputs or defined sinks.
- Final tombstone.
- Settlement eligibility.

## Station registration/insurance

A claimed station may purchase a BIT-denominated contract with:

- Station ID.
- Owner/company.
- Coverage start and expiration.
- Price paid.
- Covered cleanup behavior.
- Renewal rule.
- Chain and transaction.
- Revocation conditions.

It exempts eligible unpowered cleanup while valid. It does not provide combat immunity, safe-zone status, repayment for attack, or unlimited server resources.

A future policy must decide whether very large registrations require size-based pricing or operational review.

## Death drops

- Inventory leaves the character atomically at death.
- Owner/team-only recovery lasts 15 minutes.
- Public salvage begins at 15 minutes.
- Dropped container remains six hours.
- Expiration uses a durable scheduler.
- Verified outages pause expiration.
- Recovery and salvage actions are server-authoritative.
- Final cleanup records a tombstone and settlement leaf.

## Background travel and production

Long-running actions store next-event times rather than ticking continuously.

Examples:

- Route checkpoint.
- Fuel exhaustion.
- Production completion.
- Insurance expiration.
- Power depletion.
- Cleanup threshold.
- Contract deadline.

The scheduler delivers events at least once; handlers must be idempotent.

## Backups

Required:

- Encrypted database backups.
- Snapshot and event-log replication.
- Content-store replication.
- Restore testing.
- Chain index rebuild procedure.
- Safe/contract manifest backup.
- No wallet private keys in ordinary application backups.

## Direct downloads

Release operations require:

- macOS signing and notarization.
- Linux signing.
- HTTPS distribution.
- Signed manifest.
- Hash verification.
- Delta or complete update.
- Rollback.
- Revocation of compromised releases.
- Public source tag matching distributed binaries.

## Incident scopes

Runbooks must exist for:

- Cell crash.
- Database failover.
- Event-stream outage.
- Inventory invariant failure.
- Market custody mismatch.
- Chain reorganization.
- RPC/provider outage.
- BIT or bNOTE upgrade.
- Bridge pause.
- Compromised release.
- Malicious mod.
- Cleanup malfunction.
