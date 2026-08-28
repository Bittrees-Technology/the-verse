# Earthlike surface playtest

## Player promise

The packaged Desktop playtest can open a fresh universe at a small powered
surface outpost on Khepri Prime. The player arrives on foot in breathable air,
facing the outpost, with the starter salvage skiff parked nearby. Khepri reads
as an Earthlike ocean-and-continent world with clouds and a blue atmosphere,
and its rendered surface agrees with the authoritative spherical collider.

## Authority and persistence

1. `earth-start` is a server-selected development genesis profile. It may
   change a world only while `event_sequence == 0`; it cannot rewrite an
   active or previously played universe.
2. The profile changes canonical player and grid poses, suit defaults, and the
   starter industry grid. The authoritative worker persists the result before
   admitting development players or accepting gameplay.
3. Every added outpost block has a canonical content cost. The genesis ledger
   increases by exactly the added installed-component total so conservation
   remains valid.
4. The outpost is a dynamic completed grid resting on the planet collider. It
   is not falsely marked as voxel-anchored, and the client never invents its
   position or collision.
5. The ordinary `orbital` profile remains the default for tests, existing
   commands, and established saves.

## Movement and packaging gate

The macOS playtest must launch the client as arm64 because the mandatory native
interest verifier is arm64. Packaging fails if the client architecture can
select a slice for which no verifier library is shipped. The launcher also
passes `--genesis-profile earth-start` and stores this playtest in a distinct
application-support directory so prior orbital saves remain untouched.

## Acceptance

- A fresh `earth-start` world validates its authority graph and conservation.
- Reopening the world preserves the surface poses and outpost.
- Applying the profile after canonical history begins is rejected.
- The packaged native smoke test loads the verifier without a missing-library
  error.
- The player can walk with `WASD`, sprint with `Shift`, jump with `Space`, and
  toggle EVA with `J`; the server remains the movement authority throughout.
