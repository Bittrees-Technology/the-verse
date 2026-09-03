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

### Protocol-19 activation and verified boot

Protocol-19 activation is an offline, two-of-three signing ceremony. First
drain every protocol-18 proof cell to Sleeping, finish or abort all transfers,
stop old workers, and run `verse-world-activation prepare` to emit the exact
prepared-world summary. Each signer must independently compare its universe
ID, seed, complete compatibility tuple, receipt, prepared head, manifest,
directory, cell set, conservation, gameplay, identity, and production roots
before signing the canonical bounded authorization. Private keys never enter
the universe directory or ordinary worker configuration.

The signed envelope includes the signers' authorized activation timestamp
inside its validity window. The tool also requires the host's current trusted
time to be inside that window, but local mutable storage cannot prove the
selector was first written then. Until a one-use nonce or timestamp is
externally anchored, protect every prepared copy, destroy unused signed
envelopes after the ceremony, and treat an envelope plus its exact prepared
world as a durable capability.

`verse-world-activation activate` receives canonical policy bytes, an expected
policy hash from separate operator configuration, and the signed authorization.
It samples trusted time once, verifies exactly two distinct signatures and every
prepared binding, persists content-addressed authorization and head records,
and writes `active-protocol-head-v1.json` last. A process failure before that
final rename leaves the old world authoritative and permits cleanup of only
known activation debris. A failure after rename is a committed activation and
must recover forward.

An activated universe is inspected with `--protocol19-verified-boot`,
`--protocol19-activation-policy`, and
`--protocol19-activation-policy-hash`. The worker opens every route from the
global head, holds all writers, and exposes `/healthz` plus the bounded
`/api/v1/protocol19/activation` evidence. The response deliberately reports
`gameplay_session_admission: false` until the complete protocol-19 runtime and
client tuple ships. Do not route gameplay traffic to this readiness service.

`verse-world-activation verify` performs the same exact-head verification
without starting a readiness listener. If verified boot fails after
activation, preserve all files and capture the exact error. Do not delete a
head, copy a per-cell file, edit JSON, or start a protocol-18 worker. Roll back
only to a protocol-19-compatible binary that verifies the same global head.
Returning to protocol 18 requires a separately authorized reverse migration;
no such path exists in this checkpoint.

### P1.7 two-cell directory and handoff proof

For a fresh local proof universe, run the worker with
`--two-cell-universe --data-directory data/two-cell-universe`. This mode owns
both adjacent proof-cell roots and their directory; it rejects a standalone
cell key and paused single-cell startup. It is a local correctness/test mode,
not a production topology or capacity claim.

Run `tools/e2e/verify-two-cell-handoff.sh` for the isolated assembled-binary
gate. It generates a temporary near-boundary universe, drives one live
same-session EVA handoff, proves the destination acknowledgement barrier and
public-origin isolation, gracefully restarts the worker, and verifies the
destination route, carried inventory, movement epoch, and actor operation
frontier from the recovered roots. The script removes only its own `mktemp`
directory and accepts an alternate loopback port through
`VERSE_TWO_CELL_E2E_PORT`.

The coordinator fences the whole local authority after directory, transfer,
artifact, persistence, or canonical-invariant failure. Ordinary rejected
player intent and an explicitly stale session route remain bounded client
errors. This classification also covers bootstrap/route reads, explicit
snapshot persistence, lease renewal, and drain persistence; they cannot leave
the public authority status active after a fatal coordinator failure. Physics
that would leave the hosted two-cell topology, including every grid crossing
until grid handoff is implemented, is rejected before journal commit and
rebuilt from the last canonical state.

The bounded P1.7 operator view covers both proof cells and the durable local
directory. It reports:

- canonical cell key and ID, assignment state/generation, desired worker,
  lifecycle mode, current holder, lease fence, and renewal margin;
- aggregate placement cell/generation and transfer phase;
- source/destination cells, immutable transfer ID, package hash/size, subject
  count, quarantine receipt, and age;
- prepare, quarantine, directory commit, import, finalization, abort, and retry
  latency;
- stale assignment, cell-fence, placement-generation, route, frame, and control
  rejections;
- source/package/destination conservation reconciliation and event frontiers;
- sleeping-destination activation, catch-up, import, production re-arm, and
  verified-baseline timing; and
- bounded actionable alerts for stuck pre-commit and post-commit transfers.

Package subjects, inventories, production queues, actor IDs, and destinations
are not exposed in public health. An operator may request exact pre-commit
abort, retry quarantine/import/finalization, or quarantine an integrity
conflict. An operator cannot manually rewrite a package, decrement a placement
generation, invent a commit, or choose a second authoritative owner.

Recovery always reads the directory before taking action. A transfer without a
directory commit may reconcile or abort to the source. A committed transfer is
roll-forward only to the recorded destination, even if the source still holds
locked recovery bytes. Each cell independently follows the P1.6 lifecycle and
lease recovery rules.

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

### P1.6 production-only proof

The first durable scheduler handles one fixed cell and physical production
only. Its lifecycle record stores the desired/observed mode, monotonic revision,
holder, fencing token, lease times, verified world frontier, next production
occurrence and acknowledged occurrence. One exclusive local file lock plus a
renewed durable lease proves safe same-host replacement; it is not a
distributed-availability claim.

Each production delivery carries a stable occurrence key and trusted due time.
The handler validates the exact next sequence, plans one atomic whole-cell
quantum, appends and syncs it, and only then acknowledges it. Redelivery after a
crash reconciles against the canonical frontier. A conflicting, skipped,
future, wrong-root, wrong-cell or wrong-fence delivery fails closed.

Catch-up processes at most 60 exact quanta, at most 256 queue-bearing machines,
and at most 250 milliseconds of coordinator work per dispatch before yielding
after the current atomic quantum. Backlog is visible and never silently skipped
or combined. Paused and empty queues sleep until a relevant canonical mutation
re-arms evaluation. Clock rollback outside tolerance, forward-time overflow,
lease uncertainty or inability to finish inside the lease margin halts mutation
with an actionable reason.

P1.6 explicitly excludes background travel, power depletion, cleanup,
insurance, market deadlines, physics, oxygen, combat, turrets and AI. Those
examples remain future handlers with separate outage and conservation rules.

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
- Cell assignment conflict or stale holder.
- Stuck cross-cell transfer before and after directory commit.
- Transfer package or conservation mismatch.
- Gateway handoff or destination-baseline verification failure.
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
