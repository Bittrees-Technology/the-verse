# Core protocol invariants

**Status:** Accepted design constraints

These invariants are higher priority than convenience or performance.

## Authority

- Exactly one active writer owns a simulation aggregate at a time.
- A stale cell lease cannot commit events.
- Clients and public applications never write canonical state directly.
- Administrative actions identify their authority and reason.

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
