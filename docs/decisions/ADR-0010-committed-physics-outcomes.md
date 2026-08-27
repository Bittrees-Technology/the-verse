# ADR-0010: Commit authoritative physics outcomes

**Status:** Accepted

## Context

The P0 server must provide contact-rich rigid-body behavior while preserving exact restart semantics. Re-running a floating-point physics solver during event replay is unsafe: platform, compiler, threading, insertion order, or solver changes can produce a different contact sequence. Treating the client or an uncommitted solver state as canonical would also violate server authority.

## Decision

Use Jolt Physics behind an isolated Rust adapter for the live authoritative contact step. Jolt bodies and broad-phase data are derived runtime state, never persistence authority.

Before a physics batch becomes visible, the worker sorts stable body and collider identifiers, applies server-approved forces, advances a bounded number of fixed single-thread steps, and validates finite bounded output. It then quantizes and records the step count, fractional 60 Hz phase, ordered body poses, linear and angular velocities, and substep-indexed contacts in one canonical `PhysicsStepCommitted` event. A future version of the same atomic outcome bundle will add collision-derived damage and topology after solved contact data is available.

Event replay applies the committed outcomes exactly and does not run Jolt. After snapshot load and journal replay, the worker reconstructs derived Jolt bodies from canonical state. A content-manifest and schema version pins the mass, collision, material, quantization, and timestep rules used to interpret the event.

The first P0.7 implementation commits body and geometric contact outcomes. The pinned JoltC binding does not expose solved manifolds or impulses, so collision-derived damage and topology are intentionally not claimed yet; adding them requires a versioned outcome schema after that binding gap is closed.

## Consequences

- Exact recovery does not depend on cross-platform floating-point determinism.
- Live contacts remain server authoritative and cannot be selected by a client.
- Physics events are larger than input-only events, so production networking and archival compaction need later work.
- Every new physical property or solver upgrade requires an explicit content/schema migration.
- The adapter can be tested independently for contact behavior while simulation tests verify canonical sorting, conservation, idempotency, and fault recovery.
- An interruption before durable append exposes the prior tick; an interruption after durable append recovers the complete committed tick.
