# ADR-0031: Exact leaf contacts and supported movement prediction

- Status: Accepted
- Date: 2026-09-05
- Requirements: F-071, UX-006

Disable Jolt manifold reduction when creating physics bodies. Reduction combines
contacts from different compound leaves but retains one leaf ID. Averaging those
points can place the reported point outside that leaf, violating authoritative
replay validation. Preserve exact leaf identity instead of relaxing validation.
The adjacent-wall regression fails with reduction enabled and passes disabled.
The copied capital wall save also failed deterministically before this change
and advances and persists afterward. More contact manifolds can increase physics
cost; keep the full adapter/simulation suites and graphical pacing probe.

Client prediction uses a 1 cm contact skin and projects supported walking onto
the verified grid deck normal. This avoids floor sticking from resting penetration
and from differences between radial gravity and a flat deck. Deep penetration
and walls remain blocked. Bounded history resets reset prediction but preserve
the rendered camera pose; lifecycle transitions and large corrections still snap.

Voxel queries, highlights, and fragments use the same body-to-render translation
as their meshes. Storage coordinates remain unchanged in mining intents. Cache
sorted targeting blocks with the existing immutable projection identity cache.

No protocol or save schema changes are required. Existing events replay with the
same validation. Reverting the server setting can reproduce the wall halt;
reverting client changes restores the earlier presentation only. No saves are
regenerated or reset by this repair.
