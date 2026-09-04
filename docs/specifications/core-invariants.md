# Core protocol invariants

**Status:** Accepted design constraints

These invariants are higher priority than convenience or performance.

## Authority

- Exactly one active writer owns a simulation aggregate at a time.
- A stale, expired, uncertain, wrong-holder, or wrong-root cell lease cannot
  append events, write snapshots, acknowledge work, publish state, or report
  healthy.
- Every live append and snapshot uses the store's exact current nonzero fencing
  token. Historical event fences are positive and nondecreasing; a replacement
  token is strictly greater than the recovered maximum and cannot wrap.
- Clients and public applications never write canonical state directly.
- Administrative actions identify their authority and reason.
- Spatial interest and client-loaded state never grant intent, ownership,
  collision, targeting, or disclosure authority.
- A mobile aggregate may mutate only under both the resident cell's current
  fencing token and the aggregate's current directory placement generation.

## Universe identity and coordinates

- Every persistent spatial subject has exactly one normalized universe,
  sector, cell, and integer-local address.
- Equivalent non-normalized addresses, numeric overflow, unsafe JSON integers,
  non-finite local physics values, and wrong-universe addresses fail before
  mutation.
- Floating-origin, camera-relative, and cell-local physics transforms are
  derived and cannot mutate a canonical address.
- World schema `18` and event schema `14` bind the exact universe-manifest and
  celestial-registry hashes used to interpret them.
- A worker with the wrong manifest, registry, content, schema, or hash cannot
  load, replay, append, or project the world.
- `CellKeyV1` is normalized and hashes to one deterministic cell ID. Worker
  names, display aliases, storage paths, and noncanonical equivalent keys are
  never routing or persistence authority.
- Subject IDs are universe-unique and do not change when their resident cell
  changes.

## Celestial registry

- A materialized celestial body has one immutable body ID, kind, normalized
  fixed address, gameplay orientation, exclusion radius, and content binding.
- Planets, moons, asteroids, and asteroid fields do not translate or orbit in
  the accepted fixed-body model.
- A moon has exactly one existing planet parent. Missing, self, non-planet, and
  cyclic ancestry is invalid.
- Every physical celestial object rendered by an official client resolves to a
  registry entry; a missing asset uses a labelled proxy rather than hiding an
  authoritative body.
- Every pair in the P1.5 proof registry meets manifest `p1.5.0`'s `3,000 m`
  minimum exclusion-surface gap using deterministic integer validation.
- Voxel edits and mining mutate body-local world state, never registry identity
  or the location of another deposit.
- Generator or content upgrades cannot move or reuse an existing body identity
  without an explicit audited migration.

## Interest and replication

- The server derives player interest from the immutable bound actor and
  spectator interest from a separate bounded server grant.
- Camera state, requested radius, entity IDs, headers, cookies, client names,
  and query coordinates cannot widen an interest or privacy boundary.
- Actor-private projection remains actor-private after spatial filtering;
  proximity to another actor or owned grid never reveals its private overlay.
- Ownership alone cannot stream detailed remote cargo, mass, or production;
  the owning public grid or machine must be in the actor's authorized view.
- Interest membership, baselines, deltas, epochs, acknowledgements, and view
  hashes are derived session state and never canonical events.
- Enter and exit decisions use pinned integer distances, canonical stable
  ordering, and deterministic hysteresis. Control-critical dependencies cannot
  be removed by an ordinary presentation budget.
- An entity receives a complete authorized enter before dependent motion.
  Removal uses only `out_of_interest`, `destroyed`, or `transferred`, and
  re-entry installs a fresh entity baseline.
- Deltas are contiguous within one session epoch, interest epoch, baseline, and
  previous view hash. Any mismatch causes one current baseline, not rollback or
  unbounded replay.
- A view hash commits only to the audience-authorized projected view. The
  separately carried canonical event/tick frontier and global commitment are
  reconciliation values and cannot be interpreted as hidden entity state.
- View hashes use exact canonical projection-schema wire values, never
  renderer floats, non-finite values, or client-reserialized approximations.
- Structural state wins over motion when one authoritative transition contains
  both. Backpressure may coalesce motion but cannot discard required structure,
  removal, receipts, or private reconciliation.
- Per-session retained messages, bytes, serialization work, and recovery
  baseline frequency have explicit tested bounds.

## P1.5 compatibility

- P1.5 admits only protocol `16`, projection schema `3`, world schema `18`,
  event schema `14`, content schema `11`, content manifest `p1.5.0`, celestial
  registry schema `1`, universe manifest schema `2`, and interest schema `1` as
  one coordinated set.
- Upgrade and rollback drain incompatible sessions. Older executables never
  reinterpret newer worlds, journals, registries, baselines, or deltas.

## P1.6 lifecycle and compatibility

- The production-clock generation survives ordinary lifecycle transitions,
  process restart, lease renewal and worker replacement. Only an explicit
  reset or audited migration may increment it and restart its contiguous
  occurrence sequence.
- One due production occurrence commits at most one atomic whole-cell quantum.
  Duplicate delivery cannot repeat progress, output, loss, ledger credit, or
  experience; missing, reordered, future or conflicting delivery cannot mutate.
- Active and Background recompute the same canonically ordered production
  outcomes from the same prior state. Background advances no physics tick,
  pose, controls, contacts, oxygen, damage, combat, AI, cleanup or interest.
- Catch-up is exact, sequential and bounded. It never skips or coalesces overdue
  quanta, and paused/empty production does not create a one-second busy poll.
- P1.6 admits only protocol `17`, projection schema `3`, world schema `19`,
  event schema `15`, content schema `11`, content manifest `p1.5.0`, registry
  schema `1`, universe manifest schema `3`, interest schema `1`, operation
  fingerprint schema `1`, lifecycle-control schema `1`, and schedule-occurrence
  schema `1` as one coordinated set.
- Activation admits no gameplay state before occurrence catch-up through its
  wake cut-off, invariant validation, snapshot, and a fresh session/interest
  baseline. Public spectators cannot wake or retain a sleeping proof cell.

## P1.7 placement, transfer, and compatibility

- Every transferable mobile aggregate has exactly one resident cell and one
  monotonically increasing placement generation.
- Source prepare locks one complete server-derived dependency closure at an
  atomic tick/production boundary. Destination quarantine is durable but has
  no physics, production, projection, or mutation authority.
- A cell assignment generation permanently maps to the exact nonzero store
  fence held when it was issued. A successor must acquire the store before the
  directory advances, and cannot relabel an older cell event as its own proof.
- One directory compare-and-swap from source placement generation `N` to
  destination generation `N+1` is the sole handoff linearization point.
- Before directory commit, recovery may abort to the exact source state. After
  commit, source unlock is forbidden and recovery only rolls forward through
  idempotent destination import and source finalization.
- Every transfer phase transition is backed by the exact canonical cell event,
  store fence, and resulting world hash in a lifecycle-anchored boundary chain.
  Pre-commit abort pins both cells until source and destination cleanup proofs
  exist; a no-op cleanup is still a canonical proof-of-absence event.
- One transfer ID identifies one immutable content-addressed package. The same
  ID with changed bytes, roots, subjects, generations, or conservation vector
  fails closed.
- Cargo, installed components, production queues, reserved input, pending
  output, ownership, rewards, physics state, actor history, and lineage are
  conserved across source export, in-transit custody, and destination import.
- Operation fingerprint schema `2` is cell-independent; retained receipts and
  compaction commitments move with the actor so route changes cannot repeat an
  accepted mutation.
- A successful same-session handoff increments movement and interest epochs,
  rejects every stale source control or frame, and resumes only after one
  independently verified transfer-linked destination baseline.
- P1.7 admits only protocol `18`, projection schema `4`, world schema `20`,
  event schema `16`, content schema `11`, content manifest `p1.5.0`, registry
  schema `1`, universe manifest schema `4`, interest schema `2`, operation
  fingerprint schema `2`, lifecycle-control schema `2`, production-occurrence
  schema `1`, cell-directory schema `2`, and transfer/package schema `1` as one
  coordinated set.
- An anchored, externally connected, boundary-spanning, or unsupported
  aggregate remains source-authoritative and is never silently split, deleted,
  capped, or assigned two writers.

## P1.8 activation authority

- A prepared protocol-19 world is dormant until two distinct trusted signers
  authorize its exact receipt, prepared head, compatibility tuple, roots,
  generation, nonce, and bounded time window under an externally anchored
  policy hash.
- The universe-root active-protocol head is written and synchronized last. It
  is the sole activation commit point; per-cell heads and staged authorization
  files grant no authority.
- Protocol-19 restart derives every manifest, directory, cell, and receipt from
  that global head. Namespace discovery, scalar substitution, hybrid tuples,
  and silent repair or fallback are forbidden.
- Before the global head exists, recovery may delete only known activation
  debris and must preserve the prepared target and frozen source byte for byte.
  After it exists, recovery is forward-only under protocol 19.
- An active protocol-19 selector fences protocol-18 startup. Returning to
  protocol 18 requires a separately authorized reverse migration proving that
  no protocol-19 work is discarded.
- Mutable lifecycle-v2 history is a separate append-only successor of the
  immutable per-cell migration genesis. It binds the signed active head,
  directory-v3 authority, trusted-time frontier, exact world frontier, and
  production cursor; it cannot rewrite or reseal prepared receipt material.
- An activation-lock-owned universe lifecycle head authorizes each exact child
  successor before cell materialization. Missing, rolled-back, or unauthorized
  child lifecycle state never becomes a new local genesis.
- Directory v3 is the only issuer of cell generations and fencing tokens. A
  lifecycle claim, recovery, or release records its exact request before the
  directory transition and accepts only that request's predecessor or one
  exact successor during recovery.
- A production occurrence is pending in lifecycle history before event-17
  append and acknowledged only after the event journal proves its exact world
  successor. Quiescent cells have no occurrence and receive no periodic poll.
- Protocol-19 background dispatch starts at most 60 sequential one-second
  occurrences and starts no new quantum after 250 milliseconds. Hitting either
  bound preserves
  the next exact cursor and never skips or coalesces conserved work.

## Assets

- Every live canonical asset has exactly one owner and one location domain.
- Terminal assets cannot return to life without an explicit authorized genesis or recovery event.
- Split quantities equal the original quantity.
- Merge quantity equals the sum of inputs.
- A market receipt cannot exist without matching custody or a recorded pending operation.
- Private and creative assets cannot cross into the canonical namespace.

## Production

- Each transformation balances registered inputs, outputs, loss, sources, and sinks.
- A recipe graph cannot contain an unpriced positive-output cycle.
- Energy and machine-time requirements cannot be bypassed by retries or crashes.
- Content-manifest version determines the recipe applied.

## Transfers

- Cross-cell transfers never produce two active copies or zero conserved
  ownership domains.
- Retrying an operation or transfer phase returns the same result.
- An incomplete transfer is recoverable to exactly one authoritative side
  according to the durable directory commit.
- A market-deposited asset cannot be simultaneously installed, consumed, or transferred in-world.

## Markets

- AMM reserve updates use exact integer arithmetic.
- Fees and rounding directions are explicit.
- A quote cannot promise more BIT or commodity than the settled pool can deliver.
- Location receipts redeem only at their registered custody market.
- Ordinary price changes never trigger privileged balance mutation.

## Blockchain

- Chain ID is explicit in every address reference and signature domain.
- Testnet and mainnet state cannot share a configuration namespace.
- Unexpected proxy implementation changes quarantine new deposits.
- Chain reorganization cannot duplicate a deposit, mint, withdrawal, or swap.
- Settlement batch ranges are non-overlapping and gap-detectable.

## Lifecycle

- Death moves inventory atomically before respawn.
- Drop cleanup happens no earlier than the six-hour rule.
- Unpowered cleanup happens no earlier than the 36-hour rule.
- Valid registration prevents ordinary cleanup but not combat destruction.
- Verified service outages do not advance destructive timers.
- Cleanup creates a tombstone and an auditable event.

## Safe zone

- Damage, weapon discharge, destructive collision, and theft are impossible inside the capital safe-zone policy volume.
- Objects cannot exploit a boundary crossing to apply delayed damage inside the safe zone.
- Creative assets remain non-economic throughout their descendants.

## Tests

Each invariant must have at least one automated property, state-machine, fuzz, or fault-injection test before its subsystem reaches public testing.
