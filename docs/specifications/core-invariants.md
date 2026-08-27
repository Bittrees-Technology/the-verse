# Core protocol invariants

**Status:** Accepted design constraints

These invariants are higher priority than convenience or performance.

## Authority

- Exactly one active writer owns a simulation aggregate at a time.
- A stale cell lease cannot commit events.
- Clients and public applications never write canonical state directly.
- Administrative actions identify their authority and reason.
- Spatial interest and client-loaded state never grant intent, ownership,
  collision, targeting, or disclosure authority.

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

- Cross-cell transfers never produce two active copies.
- Retrying an operation returns the same result.
- An incomplete transfer is recoverable to exactly one authoritative side.
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
