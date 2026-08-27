# Verse physics adapter

`verse-physics` is the single project-owned FFI boundary around Jolt Physics.
Its public API contains no Jolt handles and accepts only validated body,
collider, control, and scene descriptions.

The adapter pins `rolt` and `joltc-sys` git revision
`72ac0cb1acc2037c72dc29865da6f52a5483dadc` with double-precision world
coordinates and embedded Jolt Physics 5.3 source. Jolt owns integration and
collision response. A native contact listener captures Jolt manifolds, stable
leaf-collider identity, onset/persistence, contact points, closing speed, and
`EstimateCollisionResponse` normal impulse estimates.

The estimate is pre-solver and pairwise. It is not the final applied solver
impulse, may diverge during multi-body contact, and is therefore telemetry—not
production collision-damage evidence. Dynamic bodies use bounded discrete
motion in this checkpoint so discarded LinearCast CCD candidates cannot become
canonical contact telemetry. The focused 2,048-collider probe remains
available with:

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
- uses raw JoltC callbacks that read only initialized manifold entries instead
  of forming a Rust reference to fixed native tail storage;
- writes only numeric records into a preallocated contact buffer sized for
  every configured internal collision substep, resolves strings after the
  update, and fails the step closed on overflow or any native invariant breach;
- chooses one complete manifold record per collider pair with a total
  deterministic ordering rather than combining fields from different native
  callbacks;
- marks a scene as requiring an explicit rebuild after an update or contact
  extraction error, preventing reuse of partially advanced native state;
- detaches and releases the callback after the physics system stops using it;
- permits a scene to move between threads but requires exclusive mutable access
  for rebuild and step operations; and
- does not implement `Sync` or expose native pointers.

Jolt's process-global factory intentionally remains registered until process
exit. Deleting it while independently owned scenes or tests still exist would
be unsafe.
