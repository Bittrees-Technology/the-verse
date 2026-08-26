# P0.3 visual engineering checkpoint

**Status:** Implemented local presentation checkpoint

## Player-facing goal

P0.3 makes the existing Salvage Frontier loop read as a grounded space-engineering game. It improves the material, lighting, voxel, machinery, and work-feedback layers without weakening the authoritative simulation beneath them.

This is an original clean-room implementation. The project does not include or adapt Space Engineers code, assets, audiovisual material, interface designs, or proprietary content.

## Implemented presentation layer

- The authoritative integer asteroid is converted into a continuous marching-tetrahedra render mesh.
- A density filter rounds exposed occupancy while server targeting, mining, persistence, and anchor contact remain integer and deterministic.
- Seeded fixed-point shape noise gives the canonical asteroid an irregular silhouette and clustered ferrite deposits.
- An original CC BY-SA modular armor texture, triplanar mapping, metallic materials, glass, emissive details, and a procedural rock shader replace flat primitive shading.
- The suit has a shadow-casting helmet work light, and mining, welding, and cutting emit a beam, flare, and short-lived impact sparks.
- The starter skiff has additional panel rails and a framed control canopy so its block functions read from first-person range.

## Invariants

The render mesh never owns canonical volume. Clients may interpolate a surface, but only the Rust worker decides whether a voxel exists, what it yields, whether an anchor contacts it, and whether an edit is accepted. Remeshing is therefore a disposable view of replicated state.

The generated armor image and shader are original project assets. Their source, prompt, and licenses are recorded in the native-client asset register.

## Acceptance checks

1. The asteroid appears continuous rather than as a pile of spheres or cubes.
2. Ferrite appears as restrained, clustered iron-bearing material rather than glowing decoration.
3. Accepted mining edits rebuild the same client-side surface from the updated authoritative snapshot.
4. The existing mining-to-grid-split recovery scenario passes without new client authority.
5. A fresh world preserves the voxel contact used by the anchor scenario.
6. Godot can parse and launch the client on the pinned version without shader errors.

## Next realism gates

Visual similarity alone does not deliver the target engineering sandbox. The next gates are functional:

1. grounded character-body movement, walking surfaces, gravity, collision, and EVA transitions;
2. Jolt-backed rigid grids with mass, inertia, impact damage, and disconnected topology;
3. staged construction, rotations, mount points, deformation, repair, and salvage;
4. cockpit possession, thrusters, gyroscopes, dampeners, and ship cameras; and
5. power graphs, conveyors, refineries, assemblers, inventories, and production queues.

These systems must remain server authoritative and independently implemented from The Verse specifications.
