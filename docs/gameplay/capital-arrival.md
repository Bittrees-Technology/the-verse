# Capital arrival playtest (F-071 / UX-006)

The packaged app shall start its own loopback server when opened without an
explicit server URL. It shall show connection progress, failures and controls,
and require an explicit Enter action before capturing gameplay input. A stopped
owned worker may reopen its durable save with bounded automatic retries. Lease
fencing remains enforced; recovery must never renew an expired writer in place.

A new `capital-start` development profile creates a grounded arrival hall on
Khepri Prime with a clear entrance, industrial machines, and nearby mineable
surface outcrops. All pre-admitted development pilots start in the hall. Recovery
in this profile returns to its clear arrival corridor. The capital is the intended
future public admission point; public account registration and cross-cell new-user
admission are not implemented by this fixture.

Use a separate Capital Playtest save. Existing saves and default orbital genesis
remain unchanged. Surface outcrops use the same authoritative voxel storage,
mining, collision and depletion as the asteroid. They are above the current solid
planet collider: excavating a whole planet or underground tunnels remains future
terrain work. Other registry planets are not yet active mineable environments.

Deposit rendering should use restrained mineral tones and subtle veins instead
of fluorescent patches. Varieties still share existing refinery feedstock;
separate metal balances and recipes need an economy migration.

## Local verification

The capital fixture tests cover grounded local/remote admission, planetary mining,
conservation, depletion across restart, and recovery into the arrival hall. The
live GPU test clicks Enter and the inventory controls, walks the player, checks
floor support, and verifies all three deposit labels. The packaged owned-worker
test suspends the real server for 18 seconds, checks that stale input is blocked,
then resumes it and verifies lease fencing and recovery in a fresh process.
