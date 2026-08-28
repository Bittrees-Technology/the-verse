# The Verse

The Verse is an open-source persistent voxel space universe, work-and-economy simulator, and Web3 marketplace.

The repository now contains the **cross-platform verified P0.10 simulation proof**, including authoritative grounded locomotion, EVA, orbital operations, contact physics, survival death, mining, production, construction, and exact recovery. P1 work adds a session-bound deterministic player roster, one atomic physics outcome for every living roster member and grid, server-reconstructed closest-visible hand-tool targeting, and a P1.1 actor-owned industry boundary. A fresh local universe pre-admits two loopback-only development pilots; either native client identity can bind, move independently, aim, mine, and render the other pilot, while ownership prevents one pilot from spending or constructively operating the other's assets. This remains a development multiplayer cell, not yet the public universe or real-value economy.

## Play it on macOS

Requirements: Apple Silicon or Intel macOS, Rust, Node.js, `curl`, and `jq`.

```bash
tools/dev/bootstrap-macos.sh
tools/dev/run-local.sh
```

The bootstrap downloads the pinned Godot 4.7.2 editor from the official release and verifies its checksum. The launcher starts the authoritative server and native client. While the server is running, the read-only browser command center is available at <http://127.0.0.1:7777>.

To test the second pilot, leave the first client and server running, open a
second terminal, and launch:

```bash
tools/dev/run-client.sh player-remote
```

Each window controls only the pilot named in its connection binding and renders
the other pilot as a remote engineering suit. A previous single-pilot test
world cannot be silently rewritten; if startup reports an incompatible world,
archive it with `tools/dev/reset-local-world.sh` and launch again.

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
| Enter construction / choose block | `B` / `1`–`8` |
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

P0.10 retains durable input-only controls and one atomic Jolt-backed character/grid physics step. A 1.8 m dynamic capsule owns radial upright alignment, tangent walk/sprint, buffered jump, slope handling, bounded steps, ground snap, magnetic attachment, and moving-support velocity inheritance. The active P1.4 transport uses protocol 15: every gameplay mutation carries a contiguous actor-local operation sequence plus a typed, server-derived intent fingerprint, while actor-private projections hide inventory, production queues, progression, exact oxygen, operation history, and cargo-inclusive mass. The canonical universe contains two independently controlled loopback pilots, the original salvage skiff, and a separate powered industrial platform with physical cargo, conveyor, refinery, and assembler blocks. Mining, construction, welding, cutting, and grid operations remain server-authoritative and spatially validated. Refining and manufacturing now reserve cargo input into conserved FIFO job escrow, advance on one-second integer quanta only when the machine has a valid conveyor route and qualifying power, retain blocked output safely, and recover or drop escrow exactly once. Legacy pocket refine/craft inputs fail closed. The native engineering terminal exposes inventory and production tabs, queues refinery/assembler work, shows authoritative progress and pause reasons, and offers the three new industrial blocks on construction keys `6`–`8`. The complete local cross-process loop mines ore, transfers it to the industrial grid, refines material, assembles a component, transfers it back, builds a block, and proves restart convergence without accepted shortcuts. Hosted P1.4 evidence and sleeping-cell production remain pending. Drop recovery/expiry, global streaming, safe zones, accounts, AMMs, and blockchain settlement remain in the [delivery roadmap](docs/roadmap/roadmap.md). See [Physical refining and manufacturing](docs/gameplay/physical-industry.md), [Actor-owned industry and engineering](docs/gameplay/actor-owned-industry.md), [Private player state projection](docs/gameplay/private-state-projection.md), and [Operation idempotency and retry contract](docs/architecture/operation-idempotency.md) for the exact current authority, visibility, and recovery contracts.

## Licensing

- Game client and authoritative server: AGPL-3.0-or-later.
- SDKs and public schemas: Apache-2.0.
- Reusable art assets: CC BY-SA 4.0 unless a file states otherwise.
- The Verse name and official brand assets: reserved pending a public trademark policy.

See [licensing details](LICENSES/README.md).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the applicable architecture documents before proposing implementation. New features that change canonical resources, recipes, markets, blockchain behavior, or governance require a specification change before code is merged.
