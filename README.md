# The Verse

The Verse is an open-source persistent voxel space universe, work-and-economy simulator, and Web3 marketplace.

The repository now contains the **playable P1 development proof**, including authoritative grounded locomotion, EVA, orbital operations, contact physics, survival death, mining, physical production, construction, exact recovery, a fixed celestial registry, and interest-managed multiplayer streaming. A fresh local universe pre-admits two loopback-only development pilots; either native client identity can bind, move independently, aim, mine, and render the other pilot, while ownership prevents one pilot from spending or constructively operating the other's assets. The native client and browser command center now consume the same server-owned celestial identities and bounded public world views. This remains a development multiplayer cell, not yet the production-scale public universe or real-value economy.

## Play it on macOS

Open **The Verse.app** in the latest packaged build. It starts its own local
server and shows **Enter the Verse** when your player is ready. New saves begin
in Khepri Capital on the starting planet, with an arrival hall, industrial
machines, and nearby mineable outcrops. See [capital arrival](docs/gameplay/capital-arrival.md).

WASD moves, the mouse looks, Space jumps, 1–4 selects tools, I opens inventory
and production, B opens construction, and Esc returns to the entry menu.
The [engineering starter kit](docs/gameplay/starter-tool-kit.md) and older
orbital workshop launcher remain available with separate saves.

Requirements: Apple Silicon macOS, Rust, Node.js, `curl`, and `jq`.

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
| Select drill / grinder / welder / pulse tool | `1` / `2` / `3` / `4` outside construction |
| Mine / grind / weld with selected tool | Hold left mouse |
| Fire short-range block pulse | Click left mouse with pulse tool |
| Enter construction / choose block | `B` / `1`–`8` |
| Rotate construction hologram | `[` / `]` |
| Weld construction hologram | Hold left mouse |
| Exit construction | `B` or right mouse |
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

**P1 multiplayer vertical slice is in progress.** The playable development
build includes the mining-to-manufacturing loop, two independent pilots,
private inventories, verified native/browser views, exact recovery, sleeping
cell production, and a bounded two-cell independent-EVA handoff.

The latest merged checkpoint adds protocol-19 production lifecycle scheduling.
Interactive play still uses protocol 18; protocol-19 gameplay admission remains
closed pending the event, projection, verifier, and client cutover. Ordinary
grid-and-rider handoff is not yet a completed playable milestone.

[Main revision `18670fe` passed hosted verification and Linux/Mac packaging](https://github.com/Bittrees-Technology/the-verse/actions/runs/33838538429).
That evidence supports the development proof, not a public-scale universe.
Safe zones, offline defense and cleanup, public accounts, signed updates,
regional markets, and blockchain settlement remain ahead.

Read [current progress and next gates](docs/roadmap/current-progress.md) for
what is built, partial, and planned. The
[detailed implementation checkpoint](docs/roadmap/implementation-checkpoint-2026-09-05.md)
preserves the technical authority and migration notes.

## Licensing

- Game client and authoritative server: AGPL-3.0-or-later.
- SDKs and public schemas: Apache-2.0.
- Reusable art assets: CC BY-SA 4.0 unless a file states otherwise.
- The Verse name and official brand assets: reserved pending a public trademark policy.

See [licensing details](LICENSES/README.md).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the applicable architecture documents before proposing implementation. New features that change canonical resources, recipes, markets, blockchain behavior, or governance require a specification change before code is merged.
