# Capital frame-pacing evidence

Measured on 2026-09-05, Apple M4 Pro, pinned Godot 4.7.2 OpenGL Compatibility,
capital-start temporary world. Four-second phases after verified entry. Other
Verse app instances were closed. These short developer measurements are not a
cross-device frame-rate guarantee; camera paths diverge under slow input frames.

| Phase | Before median / p95 (ms) | After median / p95 (ms) |
| --- | --- | --- |
| Idle | 83.5 / 87.6 | 12.2 / 22.2 |
| Walking | 86.5 / 141.9 | 9.6 / 23.2 |
| Mouse-look | 67.9 / 68.7 | 9.6 / 22.0 |

The idle scene dropped from 19,225 draw calls and 6,135 nodes to 693 draw calls
and 424 nodes. Changes combine structural instancing, local collision indexing,
less expensive visual fingerprints, interpolated cloud weather and origin-aware
presentation. Celestial meshes now survive ordinary origin shifts. Remaining
replication work still causes frame-time variation; sustained 60 fps is not yet
proven across the universe.

To reproduce, build the release worker and native verifier, then run
`tools/e2e/gameplay-pacing.sh` with the pinned graphical Godot executable available.
Set `GODOT_BIN` on other platforms. The script uses a temporary world on port
17789; override `VERSE_PACING_PORT` if needed. It does not change player saves.
Add `--render-isolation` for additional diagnostic phases that hide the planet
and disable shadows in the test instance only. Output includes frame percentiles
and nested CPU timings; nested timings must not be added together.

`gameplay_structure_smoke.gd` compares 500 deterministic capsule samples with
exhaustive collision checks, including translated and rotated grids. It checks
projection replacement, topology order, damage exclusions and instanced parts.
Run graphically to validate instance transforms: the headless dummy renderer
cannot read MultiMesh transforms back. The motion impairment and interest-stream
suites cover reconciliation, targeting and renderer lifecycle regression.
