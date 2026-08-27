# P0.10 authoritative grounded and magnetic locomotion

**Status:** Local implementation verified; remaining acceptance and hosted release evidence in progress

## Player promise

Turning off the jetpack near a planet, asteroid, station, or ship produces deliberate boots-on-deck movement instead of a floating sphere. Walking, sprinting, jumping, slope and step traversal, and motion inherited from a ship remain responsive, while the universe—not the client—decides whether a surface is valid and where the player is.

The controls and presentation are original to The Verse. The interaction vocabulary may feel familiar to space-engineering players, but no third-party source, assets, interface, names, or protected visual design are copied.

## Canonical locomotion states

| State | Entry | Translation behavior | Exit |
| --- | --- | --- | --- |
| `EVA` | Jetpack enabled | Six-axis thrust, boost, dampeners, pitch/yaw, `Q`/`E` roll | Jetpack disabled |
| `FREEFALL` | Jetpack disabled with no valid support | Gravity and inherited velocity; no air steering | Walkable support, magnetic lock, or jetpack |
| `GROUND` | Jetpack disabled on gravity-walkable support | Tangent walk/sprint, grounded yaw, jump | Support loss, jump, jetpack, or steep surface |
| `MAG-LOCK` | Boots armed and valid completed-grid support | Support-tangent walk/sprint with bounded adhesion | Release, jump, jetpack, support loss, or excessive separation |

`BOOTS ARMED` is a preference indicator while not locked, not a fifth physics state. Generic `surface_contact` remains diagnostic compatibility data and never grants movement permissions.

## Controls

| Input | Ground or magnetic | EVA |
| --- | --- | --- |
| `WASD` | Walk tangent to support | Local horizontal thrust |
| `Shift` | Sprint | Boost |
| `Space` | Buffered rising-edge jump | Up thrust |
| `C` | Reserved in P0.10 | Down thrust |
| Mouse | Yaw about canonical up; separate view pitch | Local pitch/yaw |
| `Q` / `E` | No body roll | Local-forward body roll |
| `Z` | Retains the EVA dampener preference | Toggle dampeners |
| `J` | Enable jetpack and detach | Disable jetpack |
| `K` | Arm or release magnetic boots | Arm boots for a later valid contact |

Opening inventory or another modal view sends a neutral transition, but authoritative gravity, support motion, oxygen, and local reconciliation continue. Menus cannot freeze the universe.

## Initial content values

The first implementation uses a 1.80 m standing capsule with 0.34 m radius and a 1.62 m eye height. Walk speed is 4.5 m/s, sprint speed 7.5 m/s, ground acceleration 18 m/s², braking 24 m/s², upright-alignment acceleration 28 rad/s², and jump launch speed 5.0 m/s. A 50-degree slope is walkable with two degrees of exit hysteresis. Step-up is limited to 0.45 m and ground snap to 0.18 m. Jump buffering and coyote time each last six 60 Hz ticks.

All authoritative values are server-owned content. The native client mirrors the same checked-in values only for prediction and presentation; it cannot use them to choose a canonical outcome. These are tuning baselines and may change only through a new content manifest.

## Support and movement rules

1. Canonical up is opposite nonzero gravity for ordinary ground, or the bound grid-support normal for magnetic movement.
2. View-forward is projected onto the support tangent plane. Looking down cannot drive the capsule into the floor, and diagonal input is normalized.
3. The ground motor targets velocity relative to the support point. It does not erase inherited ship motion or tangential gravity on a steep surface.
4. A step succeeds only when the forward obstruction, raised-capsule clearance, and walkable landing probes all agree within the content limits.
5. Jump consumes one false-to-true edge, inherits support-point velocity, and cannot repeat until a false state has been processed.
6. A moving support is named by stable body and collider identity plus a local anchor and normal. Translation and rotation preserve that local relationship while the player stands still.
7. Magnetic boots attach only to a completed grid block at low relative normal speed. Voxels and planets use ordinary gravity support and cannot become magnetic support.
8. Support loss never teleports the player. The existing world velocity carries into `FREEFALL` or `EVA`.

## Authority, persistence, and recovery

Protocol 10 adds jump input and magnetic preference but no client transform. World schema 13, event schema 8, content schema 8, and manifest `p0.10.0` version the new locomotion contract. Player and grid results remain one atomic `PhysicsStepCommitted` outcome at 60 Hz.

The server persists received controls before acknowledging them, consumes at most one transition per substep, and records the locomotion/support result before publishing it. Replay validates committed support-aware envelopes rather than rerunning historic floating-point shape casts. Death, respawn, movement-epoch changes, and incompatible reconnects clear jump latches and support bindings. A same-support small correction may smooth; a state, support, epoch, lifecycle, or large correction snaps.

## Acceptance matrix

- Flat ground: two seconds of input reaches the content speed without radial sinking or diagonal gain.
- Sprint/release: sprint reaches its tier; release brakes relative to support without reversal.
- Jump: a queued tap produces exactly one launch; holding cannot bunny-hop.
- Slopes: the walkable boundary and hysteresis are deterministic.
- Steps: the exact-height fixture climbs with clearance; the over-height fixture blocks without penetration.
- Radial gravity: all six planet axes align upright and pole traversal stays continuous.
- Magnetic boots: only completed grid contact under the catch limit locks; every release condition detaches once.
- Moving support: translating and rotating deck fixtures retain a stable local anchor and correct support-point velocity.
- Destruction: split/rebind and destroyed support transition without teleportation.
- Impairment: delay, loss, reordering, reconnect, and menu state cannot roll locomotion backward or duplicate jump.
- Recovery: pending jump, support, magnetic preference, and moving-deck attachment recover with the exact canonical hash.

## Implementation state

Implemented and locally gated: capsule bodies and stable casts; versioned locomotion state and input-only intents, including cross-process jump and magnetic-preference delivery; walk/sprint/brake; jump; radial upright alignment on all six planet axes and continuous pole-neighborhood traversal; slope hysteresis; bounded steps and snap; completed-grid-only magnetic support; translating and rotating support retention; deterministic detach when a bound block is destroyed; collider-identity rebind after a grid split; complete-capsule construction and respawn exclusion; native controls, prediction, camera, and HUD; impaired-network behavior; real client/server playability; and exact restart recovery.

Still required before publication: hosted Ubuntu evidence for this version, Linux packaging, and the P0 performance envelope.

## Explicit limits

Ladders, crouching, ragdolls, player-to-player collision, impact damage, suit energy, artificial gravity, cockpit possession, and production-scale multiplayer remain later checkpoints. The lack of those systems does not weaken the P0.10 authority boundary, but they remain required for the production universe.
