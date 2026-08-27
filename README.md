# The Verse

The Verse is an open-source persistent voxel space universe, work-and-economy simulator, and Web3 marketplace.

The repository now contains the **cross-platform verified P0.10 simulation proof**, including authoritative grounded locomotion, EVA, orbital operations, contact physics, survival death, mining, production, construction, and exact recovery. P1.0 work has begun with a protocol 11 session boundary, a deterministic persisted player roster, and one atomic physics outcome for every living roster member and grid. Only one development pilot is admitted through the playable client today, so this is not yet the public multiplayer universe or real-value economy.

## Play it on macOS

Requirements: Apple Silicon or Intel macOS, Rust, Node.js, `curl`, and `jq`.

```bash
tools/dev/bootstrap-macos.sh
tools/dev/run-local.sh
```

The bootstrap downloads the pinned Godot 4.7.2 editor from the official release and verifies its checksum. The launcher starts the authoritative server and native client. While the server is running, the read-only browser command center is available at <http://127.0.0.1:7777>.

To create the portable Apple Silicon development package used by release testing:

```bash
tools/release/install-godot-export-templates.sh
tools/release/package-native.sh
```

The generated archive under `artifacts/release` contains the native app, authoritative server, one-click launcher, license notices, exact version record, and checksums. It is an unsigned development build; public signing, notarization, and an automatic updater remain later release work.

You begin in the Khepri Prime orbital sector beside a powered 25-block salvage skiff and an independent mineable asteroid. The planet surface is more than three kilometers away; the starting field is vacuum with weak distant gravity, not a planetary outcrop. The authoritative server consumes sequenced movement controls and owns the character's pose, gravity, collision, and landing contact. The guided contract asks you to extract three voxels, refine ore, fabricate a component, extend the rig, and anchor it into the asteroid. Actions earn persistent career experience and clearance levels.

Native controls are shown in the client:

| Action | Control |
| --- | --- |
| Walk or EVA thrust | `WASD` |
| Jump / EVA ascend / EVA descend | `Space` / `Space` / `C` |
| EVA roll left / right | `Q` / `E` |
| Sprint or boost / toggle dampeners | `Shift` / `Z` |
| Toggle helmet work light | `L` |
| Toggle jetpack / helmet seal | `J` / `H` |
| Arm or release magnetic boots | `K` |
| Open engineering inventory terminal | `I` |
| Mine highlighted voxel | Hold left mouse |
| Enter construction / choose block | `B` / `1`–`5` |
| Rotate construction hologram | `[` / `]` |
| Weld construction hologram | Hold left mouse |
| Cut and salvage a block | Hold right mouse |
| Refine / fabricate / transfer cargo | `R` / `T` / `V` (`Shift+V` reverses transfer) |
| Anchor / move / stop targeted grid | `F` / `M` / `X` |
| Request recovery when incapacitated | `Enter` |

To run only the Linux-compatible headless server:

```bash
tools/dev/run-server.sh
```

The same packaging command runs on x86_64 Ubuntu and produces a portable Linux archive. Hosted automation builds and smoke-tests both development packages. Signed public downloads remain scheduled work.

## Verify the build

```bash
tools/ci/check.sh
```

This runs the Rust tests and lints, browser syntax checks, Godot validation, native motion-impairment coverage, and an input-only end-to-end scenario that restarts the server and proves exact state recovery. See the [Grounded and magnetic locomotion checkpoint](docs/gameplay/authoritative-grounded-locomotion.md), [Authoritative EVA checkpoint](docs/gameplay/authoritative-character-motion.md), [Survival Death checkpoint](docs/gameplay/survival-death.md), [Contact Physics checkpoint](docs/gameplay/contact-physics.md), and [P0 implementation guide](docs/architecture/p0-implementation.md) for scope and limitations.

## Product pillars

- A single public universe containing fixed planets, asteroid fields, frontier sectors, and deep-space routes.
- Native macOS and Linux gameplay with browser management, spectating, APIs, and optional cloud streaming.
- Server-authoritative voxel mining, construction, physics, damage, destruction, logistics, and production.
- A location-aware economy using BIT as its primary base pair.
- Passkey accounts that hide routine blockchain operations.
- DAO companies, formal work contracts, first-class bots and AI agents, and approved user-created content.
- Official servers using only Verse DAO-approved mods; economically isolated private servers may run anything.
- The Open Metaverse contractual governing framework, with nonwaivable rights and laws preserved.
- Open development and community contribution.

## Specification index

Start with [the documentation map](docs/README.md), then read:

1. [Vision and non-negotiable principles](docs/product/vision.md)
2. [Canonical requirements](docs/product/requirements.md)
3. [Feature catalog](docs/product/feature-catalog.md)
4. [System architecture](docs/architecture/system-overview.md)
5. [Roadmap](docs/roadmap/roadmap.md)
6. [Open questions and blockers](docs/open-questions.md)

## Current status

P0.10 retains durable input-only controls and one atomic Jolt-backed character/grid physics step. A 1.8 m dynamic capsule owns radial upright alignment, tangent walk/sprint, buffered jump, 50° slope entry with 2° exit hysteresis, bounded 45 cm steps, 18 cm ground snap, magnetic attachment to completed grid blocks, and moving-support velocity inheritance. Protocol 11 authenticates before publishing a playable session, binds gameplay mutations to the admitted local pilot outside the JSON intent, prevents concurrent pilot claims, and gives the browser an explicit read-only spectator role. The active P1 state uses a canonical ordered roster; its shared physics step advances every living capsule and grid atomically, while collision layers prevent character pushing without disabling world collision. Second-player admission, actor-scoped work and lifecycle state, native remote-character rendering, drop recovery/expiry, global streaming, safe zones, accounts, AMMs, and blockchain settlement remain sequenced in the [delivery roadmap](docs/roadmap/roadmap.md). Local and hosted P0.10 Rust, Godot, protocol, impairment, exact-recovery, package, container, and serialized performance gates are green in [run 33078109914](https://github.com/Bittrees-Technology/the-verse/actions/runs/33078109914).

## Licensing

- Game client and authoritative server: AGPL-3.0-or-later.
- SDKs and public schemas: Apache-2.0.
- Reusable art assets: CC BY-SA 4.0 unless a file states otherwise.
- The Verse name and official brand assets: reserved pending a public trademark policy.

See [licensing details](LICENSES/README.md).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the applicable architecture documents before proposing implementation. New features that change canonical resources, recipes, markets, blockchain behavior, or governance require a specification change before code is merged.
