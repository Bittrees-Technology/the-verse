# P0.6 orbital operations checkpoint

**Status:** Implemented P0.6 local proof; P1.5 registry presentation specified

## Player promise

The opening salvage site must read as an asteroid operation in planetary orbit, not as a rock sitting on the ground. EVA orientation and connected-inventory work must use the interaction grammar of a serious space-engineering game while retaining original Verse art, code, terminology, and rules.

## Orbital separation

1. The mineable asteroid and starter grid remain around the origin cell.
2. Khepri Prime has a 1,200-meter **proof radius** and its nearest surface is more than 3,000 meters from the asteroid origin. This test scale validates mechanics and is not a production-scale planet or multi-day-travel claim.
3. The starting field has no atmosphere and is not breathable.
4. Khepri still contributes a weak inverse-square gravity vector in orbit. Surface gravity, atmosphere, oxygen, and collision rules remain authoritative near the planet.
5. The server test suite proves both orbital vacuum at genesis and breathable gravity near the modeled surface.

## Flight controls

1. `Q` rolls the character left and `E` rolls right around the local forward axis.
2. Holding either key produces continuous roll; tapping produces a readable orientation step.
3. Mouse pitch and yaw operate in the character's current local frame so they preserve rolled orientation.
4. Construction yaw moves to `[` and `]`, preventing build mode from stealing EVA roll controls while remaining available on compact Mac keyboards.
5. Movement uses the camera basis, so thrust follows the complete six-axis orientation. P0.9 converts these local translation and angular controls into one server-owned EVA, landing/contact, and rotation step under [ADR-0013](../decisions/ADR-0013-input-only-authoritative-character-motion.md). A durably queued press and release are consumed on successive fixed substeps, preserving tapped `Q`/`E` roll. P0.10 adds server-classified capsule walking and jump under [ADR-0014](../decisions/ADR-0014-authoritative-grounded-and-magnetic-locomotion.md), while `Q`/`E` remains EVA-only.

## Engineering inventory

1. `I` opens a compact two-pane inventory workspace.
2. Each pane identifies its connected owner and container, exposes search and category filtering, and lists items in rows.
3. Rows expose item type, amount, total volume, and total mass rather than oversized presentation cards.
4. The selected item receives an explicit outline. Center arrows transfer one unit or the full source stack in either direction.
5. Capacity bars and aggregate mass remain sourced from authoritative snapshots. The UI never calculates acceptance or changes inventory directly.
6. Focused search fields own printable keys, including `I`; `Escape` always closes the terminal, while `I` closes it only when text entry does not own focus.

## Visual material contract

1. The asteroid is rendered from the authoritative editable isosurface and an original triplanar carbonaceous-regolith albedo map.
2. Ferrite remains visually distinct without replacing canonical voxel material identity.
3. Khepri uses an original equirectangular geological albedo, displaced terrain silhouette, independently moving cloud layer, and view-dependent atmospheric limb.
4. The planet and asteroid are separate objects with separate authoritative coordinates; visual perspective cannot imply physical contact at the genesis camera.
5. Generated reusable textures carry CC BY-SA 4.0 sidecars and source-prompt records in the native asset register.

## P1.5 celestial continuation

[Fixed celestial registry and interest-managed visibility](celestial-registry-and-interest-management.md)
supersedes only the hard-coded presentation boundary, not the verified P0.6
gravity, atmosphere, oxygen, collision, inventory, or EVA behavior.

1. Khepri's center, proof radius, gravity/atmosphere model, scale class, visual
   descriptor, and name come from one authoritative registry entry.
2. The origin asteroid has an independent registered body ID, fixed address,
   body-local voxel chunks, and render transform. It is never positioned as an
   unregistered child of the planet.
3. Any visible moon resolves to a registry entry or is removed. Decorative
   celestial geometry cannot imply a body that the universe does not know.
4. A distant proxy preserves canonical direction and angular diameter by
   scaling body distance and radius together. Range labels remain canonical.
5. Near/far tiers overlap or cross-fade without a visible shift in direction,
   size, or light response.
6. Stars use an effectively infinite camera-centered sky and do not parallax
   during local EVA translation. Nearby dust remains visibly local particulate.
7. The HUD distinguishes current gravity source, nearest known body, distance
   to center and surface, altitude, gravity, atmosphere, breathability, and
   proof-versus-production scale.
8. From the genesis camera, automated geometry checks and macOS/Linux visual
   evidence confirm that the asteroid and Khepri silhouettes do not imply
   physical contact.

## Explicit limits

The current planet is a distant rendered and simulated proof-scale body rather
than a production-scale planet or streamed landing destination. P1.5 adds
registry-driven rendering tiers but does not stream editable planetary voxels,
generate terrain collision patches, add landing gameplay, or provide an
airtight-room oxygen graph. Physical cargo, conveyors, and production queues
are now covered by the P1.4 industry slice. Production planet dimensions,
separation, cruise speeds, and multi-day journey targets remain an OQ-010 gate.
