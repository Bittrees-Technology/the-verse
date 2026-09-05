# ADR-0030: Native capital frame pacing

- Status: Accepted
- Date: 2026-09-05
- Requirements: F-071, UX-006

Batch complete, healthy structural mesh parts per grid with MultiMesh. Preserve
part materials, orientation, shadows and moving-grid transforms. Damaged blocks
and construction frames retain individual visuals. Verified records remain the
source for targeting and collision; rendering never owns gameplay state.

Index integer block coordinates for capsule collision. Cache entries are tied to
the verified projection's block-array identity and are replaced when that array
changes. Grid motion reuses local coordinates. Query only cells within capsule
sample reach, then apply the existing sphere-versus-box overlap calculation.
Projection arrays must remain immutable after installation, as required by the
verification boundary. Removed grids release their cached entries.

Sort serialized visual records once per fingerprint instead of repeatedly
formatting sort keys and deep-copying entire blocks. Fingerprints remain
independent of record order and include damage and construction state.

Evaluate cloud weather noise at vertices on the existing dense cloud shell and
interpolate it for lighting. This trades the finest procedural cloud detail for
lower per-pixel cost while preserving coverage, animation and opacity.

When the verified local origin changes, translate camera, interpolation endpoints
and remote presentation poses into the new frame before reconciliation. Install
current grid collision poses before replay. Reposition existing celestial shells
when registry and visible membership match, preserving their meshes and cloud
rotation. An origin change must not become a movement correction.

No protocol, economic state or save migration changes. Reverting these client
changes restores the prior presentation without modifying saved worlds.
