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

## Grand capital follow-up

The expanded 25 by 23 metre hall was measured separately with the same probe.
Connected median/p95 frame times were 23.6/24.5 ms idle, 12.0/26.5 ms walking,
and 11.1/24.4 ms mouse-look on the same M4 Pro. All three phases retained a
verified, ready gameplay connection. The probe now fails when that connection
is lost, so disconnected rendering cannot be mistaken for a performance gain.

A longer authoritative test walks toward the relocated outcrops for ten seconds
and reopens the saved world. The deposits leave an apron around the expanded
foundation. Local smoke checks also cover the new architectural batches and
keep non-colliding ornamental meshes above the player corridor.

## Movement contact repair

A disposable copy of the failed capital save reproduced
`replay_player_contact_spatially_invalid` at a back-wall leaf. Disabling manifold
reduction allowed that same copy to advance 120 calls of 16 ms and persist.
The automatic adjacent-wall test also fails under the old setting. The full
simulation suite passed 467 tests; physics, impairment, interest, and structure
checks passed. The graphical UI test walked 6.19 m and exercised all four tools.

The revised probe records origin-adjusted camera steps, correction magnitudes,
maximum frame times, and explicit camera snaps, and requires actual world entry.
After the contact, deck-prediction, camera-reset, and targeting-cache fixes, one
confirmation run measured median/p95 frame times of 23.5/24.5 ms idle, 9.6/26.1 ms
walking, and 9.2/23.8 ms mouse-look. Mouse-look's maximum camera translation was
0.12 m and maximum turn 1.05 degrees per frame. Targeting CPU cost was about
1.94 ms per mouse-look frame, versus 4.02 ms before caching in this session.

This is improved, not a claim of hitch-free rendering: walking still exhibited
an occasional 90–103 ms frame stall and up to 0.58 m camera steps. Instrumented
movement/network/presentation handlers did not show a matching >40 ms individual
stall. Driver/render synchronization remains a possible cause, not established
by these measurements. Long sessions and other hardware still need testing.
