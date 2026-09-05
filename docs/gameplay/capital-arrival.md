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

## Frame pacing

Complete, undamaged structural blocks may share instanced render batches per
moving grid. Damage, construction, ownership, targeting and collision continue
to use verified block records. Batch transforms include each block's orientation
and move with the grid. Damaged blocks and unfinished frames retain their
individual visuals. Verify idle, walking and mouse-look frame times in the actual
capital scene; synthetic camera tests alone do not establish playability.

Collision broad-phase queries must match the existing capsule overlap checks.
Cloud weather may interpolate samples from the dense shell to keep mouse-look
and walking responsive. These presentation changes require no save migration.

Coordinate-origin changes must preserve camera and remote-avatar continuity.
Walking validation must measure world travel, including origin translation.

## Grand capital revision

New capital worlds use a 25 by 23 metre foundation and an eight-metre atrium,
with an open entrance and skylight, colonnades and a clear central arrival route.
Use pale stone, dark stone inlays and brass trim for the capital structure.
Industry remains reachable on the arrival floor. Decorative lighting and planting
must not create invisible obstacles or obscure the mining exits.

The packaged grand-capital revision uses its own Grand Capital Playtest save.
Existing Capital Playtest saves remain intact; their layout is not rewritten.
All physical expansion uses authoritative structural blocks and exact genesis
component accounting. Healthy architectural finishes remain instanced; damage
and construction states must stay readable. Verify restart, walking, mouse-look,
industrial tools and frame pacing in the larger scene.

The grand-capital outcrops leave a clear apron around the larger foundation;
legacy capital deposit coordinates remain available for older worlds. A continuous
walk from the hall toward the deposits must remain authoritative and reopen from
its saved journal. Render fingerprints may reuse unchanged verified block arrays,
while changes to damage, construction or topology still invalidate the visual.

If suspension expires a writer lease between an operation's initial check and
its lifecycle publication, reject that publication without writing a post-expiry
live timestamp. Keep the prior durable record valid for a fresh fenced reopen;
never extend an already expired lease in place.

Player contacts must retain exact leaf-block identity. Native contact reduction
must not merge neighboring collider manifolds into a point attributed to only
one block. Reproduce wall traversal from a copied saved world and verify that
contact validation stays enabled and the recovered world continues advancing.

Voxel collision prediction, tool rays, and highlighting must use the same body-to-render origin translation as deposit meshes, including after a local origin rebase.

Client movement prediction permits a 1 cm contact skin so ordinary resting solver penetration does not block tangential floor movement. Deeper overlaps and walls still block prediction; authoritative physics and replay validation remain unchanged.

On grid supports, project predicted walking displacement onto the actual deck normal, not the radial gravity tangent. A bounded prediction-history reset must preserve the rendered camera pose and converge smoothly; respawn, reconnect, and large corrections retain explicit snaps.
