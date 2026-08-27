# P0.6 orbital operations checkpoint

**Status:** Implemented local proof

## Player promise

The opening salvage site must read as an asteroid operation in planetary orbit, not as a rock sitting on the ground. EVA orientation and connected-inventory work must use the interaction grammar of a serious space-engineering game while retaining original Verse art, code, terminology, and rules.

## Orbital separation

1. The mineable asteroid and starter grid remain around the origin cell.
2. Khepri Prime has a 1,200-meter proof radius and its nearest surface is more than 3,000 meters from the asteroid origin.
3. The starting field has no atmosphere and is not breathable.
4. Khepri still contributes a weak inverse-square gravity vector in orbit. Surface gravity, atmosphere, oxygen, and collision rules remain authoritative near the planet.
5. The server test suite proves both orbital vacuum at genesis and breathable gravity near the modeled surface.

## Flight controls

1. `Q` rolls the character left and `E` rolls right around the local forward axis.
2. Holding either key produces continuous roll; tapping produces a readable orientation step.
3. Mouse pitch and yaw operate in the character's current local frame so they preserve rolled orientation.
4. Construction yaw moves to `[` and `]`, preventing build mode from stealing EVA roll controls while remaining available on compact Mac keyboards.
5. Movement continues to use the camera basis, so thrust follows the complete six-axis orientation.

## Engineering inventory

1. `I` opens a compact two-pane inventory workspace.
2. Each pane identifies its connected owner and container, exposes search and category filtering, and lists items in rows.
3. Rows expose item type, amount, total volume, and total mass rather than oversized presentation cards.
4. The selected item receives an explicit outline. Center arrows transfer one unit or the full source stack in either direction.
5. Capacity bars and aggregate mass remain sourced from authoritative snapshots. The UI never calculates acceptance or changes inventory directly.

## Visual material contract

1. The asteroid is rendered from the authoritative editable isosurface and an original triplanar carbonaceous-regolith albedo map.
2. Ferrite remains visually distinct without replacing canonical voxel material identity.
3. Khepri uses an original equirectangular geological albedo, displaced terrain silhouette, independently moving cloud layer, and view-dependent atmospheric limb.
4. The planet and asteroid are separate objects with separate authoritative coordinates; visual perspective cannot imply physical contact at the genesis camera.
5. Generated reusable textures carry CC BY-SA 4.0 sidecars and source-prompt records in the native asset register.

## Explicit limits

The current planet is a distant rendered and simulated test body rather than a streamed landing destination. The client does not yet change rendering tiers during a multi-kilometer approach, stream editable planetary voxels, generate terrain collision patches, or provide an airtight-room oxygen graph. The compact inventory has one suit and one connected cargo container because the proof world currently owns only those inventories; production queues and conveyor-network enumeration remain later checkpoints.
