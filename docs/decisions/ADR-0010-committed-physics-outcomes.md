# ADR-0010: Commit authoritative physics outcomes

**Status:** Accepted

## Context

The P0 server must provide contact-rich rigid-body behavior while preserving exact restart semantics. Re-running a floating-point physics solver during event replay is unsafe: platform, compiler, threading, insertion order, or solver changes can produce a different contact sequence. Treating the client or an uncommitted solver state as canonical would also violate server authority.

## Decision

Use Jolt Physics behind an isolated Rust adapter for the live authoritative contact step. Jolt bodies and broad-phase data are derived runtime state, never persistence authority.

Before a physics batch becomes visible, the worker sorts stable body and collider identifiers, applies server-approved forces, advances a bounded number of fixed single-thread steps, and validates finite bounded output. It then quantizes and records the step count, fractional 60 Hz phase, ordered body poses, linear and angular velocities, and substep-indexed contacts in one canonical `PhysicsStepCommitted` event. A future version of the same atomic outcome bundle will add collision-derived damage and topology after solved contact data is available.

Event replay applies the committed outcomes exactly and does not run Jolt. It
still rejects implausible contact evidence before mutation. A player contact
must lie within the bounded swept capsule and counterpart geometry; for
`LinearCast`, the check admits half of at most one fixed-step maximum-velocity
speculative separation because the recorded point is the midpoint between the
two manifold surfaces, then adds only the pinned contact and quantization slop.
After snapshot load and journal replay, the worker reconstructs derived Jolt
bodies from canonical state. Live commit processing rebuilds or reconciles
derived Jolt state at the operation-specific boundary. A content-manifest and
schema version pins the mass, collision, material, quantization, and timestep
rules used to interpret the event.

Event schema 4 commits native Jolt manifold identity, canonical began/persisted lifecycle, integer closing speed, exact integer reduced translational mass, and explicitly named pairwise estimated normal impulse. The reduced mass ignores contact direction, lever arm, and rotational inertia and therefore remains non-damage telemetry. World schema 9 persists the active-pair set so reconciling derived Jolt state for each commit cannot manufacture a second canonical onset. Content `p0.7.2` replaces floating block-mass definitions with exact grams.

Jolt invokes the public contact listener before constraint solving. Its `EstimateCollisionResponse` result is not the final applied solver impulse and can diverge during multi-body contact. Collision-derived damage and topology are therefore intentionally not claimed. That work requires a license-compatible Jolt/JoltC fork with a bounded post-solve applied-impulse callback and a separate winning-CCD path, followed by another versioned outcome schema.

Content schema 5 and manifest `p0.7.3` partition derived voxel collision into stable eight-cell chunk bodies under [ADR-0011](ADR-0011-dirty-voxel-collision-chunks.md). The event and world payload shapes remain unchanged; their explicit content version rejects older single-body contact identities.

## Consequences

- Exact recovery does not depend on cross-platform floating-point determinism.
- Live contacts remain server authoritative and cannot be selected by a client.
- Physics events are larger than input-only events, so production networking and archival compaction need later work.
- Every new physical property or solver upgrade requires an explicit content/schema migration.
- The adapter can be tested independently for contact behavior while simulation tests verify canonical sorting, conservation, idempotency, and fault recovery.
- An interruption before durable append exposes the prior tick; an interruption after durable append recovers the complete committed tick.
