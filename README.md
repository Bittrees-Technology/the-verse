# The Verse

The Verse is an open-source persistent voxel space universe, work-and-economy simulator, and Web3 marketplace.

The repository now contains **P0.6: Orbital Operations**, built on the first-person Salvage Frontier, Engineering Hands, and Planetary Logistics loops. It is still a single-player proof—not yet the public multiplayer universe or real-value economy—but realistic orbital separation, six-axis EVA orientation, physical inventory, oxygen, mining, and staged construction now share one authoritative persistent world.

## Play it on macOS

Requirements: Apple Silicon or Intel macOS, Rust, Node.js, `curl`, and `jq`.

```bash
tools/dev/bootstrap-macos.sh
tools/dev/run-local.sh
```

The bootstrap downloads the pinned Godot 4.7.2 editor from the official release and verifies its checksum. The launcher starts the authoritative server and native client. While the server is running, the browser command center is available at <http://127.0.0.1:7777>.

You begin in the Khepri Prime orbital sector beside a powered 25-block salvage skiff and an independent mineable asteroid. The planet surface is more than three kilometers away; the starting field is vacuum with weak distant gravity, not a planetary outcrop. The guided contract asks you to extract three voxels, refine ore, fabricate a component, extend the rig, and anchor it into the asteroid. Actions earn persistent career experience and clearance levels.

Native controls are shown in the client:

| Action | Control |
| --- | --- |
| EVA thrust / ascend / descend | `WASD` / `Space` / `C` |
| Roll character left / right | `Q` / `E` |
| Boost / toggle dampeners | `Shift` / `Z` |
| Toggle helmet work light | `L` |
| Toggle jetpack / helmet seal | `J` / `H` |
| Open engineering inventory terminal | `I` |
| Mine highlighted voxel | Hold left mouse |
| Enter construction / choose block | `B` / `1`–`5` |
| Rotate construction hologram | `[` / `]` |
| Weld construction hologram | Hold left mouse |
| Cut and salvage a block | Hold right mouse |
| Refine / fabricate / transfer cargo | `R` / `T` / `V` (`Shift+V` reverses transfer) |
| Anchor / move / stop targeted grid | `F` / `M` / `X` |

To run only the Linux-compatible headless server:

```bash
tools/dev/run-server.sh
```

Linux native client packaging and signed direct downloads remain scheduled work. Godot 4.7.2 can already open `apps/native-client` on Linux for development.

## Verify the build

```bash
tools/ci/check.sh
```

This runs the Rust tests and lints, browser syntax checks, Godot validation, and an end-to-end scenario that restarts the server and proves exact state recovery. See the [Orbital Operations checkpoint](docs/gameplay/orbital-operations.md), [Planetary Logistics checkpoint](docs/gameplay/planetary-logistics.md), and [P0 implementation guide](docs/architecture/p0-implementation.md) for scope and limitations.

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

P0.6 places the authoritative asteroid field in vacuum more than three kilometers above Khepri Prime, adds continuous and tap-controlled `Q`/`E` character roll, remaps block rotation to `[`/`]`, and rebuilds the logistics screen around compact connected-inventory lists, selectors, search, filters, amount/volume/mass columns, row selection, and central transfer actions. Original high-resolution surface maps, triplanar asteroid material, mapped planetary terrain, moving clouds, and limb atmosphere replace the earlier flat visual treatment. Planet landing, multiplayer, rigid-grid collision physics, global streaming, safe zones, accounts, AMMs, and blockchain settlement remain sequenced in the [delivery roadmap](docs/roadmap/roadmap.md).

## Licensing

- Game client and authoritative server: AGPL-3.0-or-later.
- SDKs and public schemas: Apache-2.0.
- Reusable art assets: CC BY-SA 4.0 unless a file states otherwise.
- The Verse name and official brand assets: reserved pending a public trademark policy.

See [licensing details](LICENSES/README.md).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the applicable architecture documents before proposing implementation. New features that change canonical resources, recipes, markets, blockchain behavior, or governance require a specification change before code is merged.
