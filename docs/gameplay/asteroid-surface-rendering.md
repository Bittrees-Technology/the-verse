# Asteroid surface rendering checkpoint

**Status:** Implemented local rendering improvement; public performance gate open.

**Scope:** F-004, F-063; existing human-scale readability and celestial scale
visual direction. No protocol, persistence, economic, or collision changes.

## Player-visible change

Rock and ferrite now use restrained light-reactive surface relief. Broad rock
variation, the existing regolith texture, and fine grain perturb the shading
normal without offsetting mesh vertices. Removing the old decorative vertex
displacement also avoids opening cracks where neighboring chunks have different
surface normals. The underlying marching-tetrahedra presentation mesh is
unchanged; this is not a new collision-accuracy claim.

Texture projection and geological variation use the shared asteroid-local
coordinates already emitted by the chunk mesher. Moving the asteroid or its
render origin therefore does not slide the material through the surface.
Unresolved procedural octaves and grain fade toward their mean based on pixel
footprint; the existing texture retains mipmapped anisotropic filtering.

The material keeps the existing Compatibility renderer, texture, rock/ferrite
color classification, and three triplanar texture samples. Surface derivatives
supply relief without adding normal-map assets or mesh tangent requirements.
The slope is bounded, and `relief_strength=0` provides a comparison with relief
disabled. No new dependency or third-party asset is introduced.

## Validation

Run the GPU regression with the pinned Godot executable and a real display:

```bash
"$GODOT_BIN" --path apps/native-client \
  --rendering-method gl_compatibility --resolution 640x480 \
  --script res://tests/asteroid_render_smoke.gd
```

This is a separate rendered-image test: `--headless` uses a dummy renderer and
is explicitly rejected. It builds an eight-chunk fixture with the actual client
voxel mesher, captures the material under fixed lighting, translates the scene
and camera together, and compares images. It also checks that disabling relief
changes the lighting, catching an inactive or failed shader. A PNG preview is
saved to the Godot user-data directory by default; pass `-- --output=/absolute/path.png`
to choose a destination. Optional `--baseline=/absolute/previous.gdshader`
captures a before image using the same camera and fixture.

Local evidence on 2026-09-05, Apple M4 Pro, Godot 4.7.2, Compatibility OpenGL:

- Actual GPU shader compilation and the rendered regression pass.
- Mean normalized RGB translation difference: `0.000004` (limit `0.002`).
- Mean relief-on/off difference: `0.001322` (minimum `0.0002`).
- Before/after fixture images inspected for surface depth and chunk continuity.
- Native movement impairment and planet visual smoke assertions pass. The
  movement harness emits resource-cleanup warnings at shutdown.

Linux GPU validation, full-scene frame-time measurements, and an in-game camera
motion review remain necessary before closing F-063 or claiming a performance
increase. The change filters fine detail but does not establish a measured
reduction in temporal aliasing. Headless CI alone is not visual evidence.
