# Verse physics adapter

`verse-physics` is the single project-owned FFI boundary around Jolt Physics.
Its public API contains no Jolt handles and accepts only validated body,
collider, control, and scene descriptions.

The adapter pins `rolt` and `joltc-sys` `0.3.1+Jolt-5.0.0` with double-precision
world coordinates. Jolt owns integration and collision response. The pinned
JoltC surface does not expose Jolt's contact listener or solved manifolds, so
this checkpoint reconstructs sorted contact telemetry from the same compound
box geometry before and after each fixed step. The records provide stable body
and collider IDs, an approximate point and normal, penetration/proximity, and
closing speed. They are suitable for testing and conservative impact inputs,
but are not solver impulses. A future binding upgrade must replace this
fallback before impulse-derived production damage is enabled.

The fallback first rejects non-overlapping collider pairs with conservative
swept world bounds, including translation and intermediate rotation. The
focused 2,048-collider probe remains available with:

```console
cargo test -p verse-physics --test compound_grid_benchmark -- --ignored --nocapture
```

## Safety boundary

All project-owned unsafe operations are isolated in `src/ffi.rs`. The adapter:

- initializes Jolt's process-global allocator, factory, and registered types
  exactly once;
- owns every physics system, temporary allocator, zero-worker job system, body,
  and retained shape for their complete native lifetimes;
- releases bodies before their system and releases the system before its
  allocators;
- keeps callback implementations owned by `rolt::PhysicsSystem`;
- permits a scene to move between threads but requires exclusive mutable access
  for rebuild and step operations; and
- does not implement `Sync` or expose native pointers.

Jolt's process-global factory intentionally remains registered until process
exit. Deleting it while independently owned scenes or tests still exist would
be unsafe.
