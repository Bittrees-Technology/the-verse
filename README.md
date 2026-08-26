# The Verse

The Verse is an open-source persistent voxel space universe, work-and-economy simulator, and Web3 marketplace.

The repository now contains the first **playable P0 vertical slice**. It is a local, single-player engineering proof of the authoritative gameplay loop, not yet the public multiplayer universe or real-value economy.

## Play it on macOS

Requirements: Apple Silicon or Intel macOS, Rust, Node.js, `curl`, and `jq`.

```bash
tools/dev/bootstrap-macos.sh
tools/dev/run-local.sh
```

The bootstrap downloads the pinned Godot 4.7.2 editor from the official release and verifies its checksum. The launcher starts the authoritative server and native client. While the server is running, the browser command center is available at <http://127.0.0.1:7777>.

Native controls are shown in the client. The main loop supports flying, voxel mining, ore refining, component crafting, cargo transfers, block construction, anchoring, grid motion, block damage, and grid splitting.

To run only the Linux-compatible headless server:

```bash
tools/dev/run-server.sh
```

Linux native client packaging and signed direct downloads remain scheduled work. Godot 4.7.2 can already open `apps/native-client` on Linux for development.

## Verify the build

```bash
tools/ci/check.sh
```

This runs the Rust tests and lints, browser syntax checks, Godot validation, and an end-to-end scenario that restarts the server and proves exact state recovery. See the [P0 implementation guide](docs/architecture/p0-implementation.md) for scope and limitations.

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

P0.1 validates the server-authoritative resource loop, deterministic rules, hash-chained persistence, idempotent operations, grid connectivity, and native/browser client integration. Multiplayer, collision physics, planets, safe zones, accounts, AMMs, and blockchain settlement are not in this slice. Those systems remain sequenced in the [delivery roadmap](docs/roadmap/roadmap.md).

## Licensing

- Game client and authoritative server: AGPL-3.0-or-later.
- SDKs and public schemas: Apache-2.0.
- Reusable art assets: CC BY-SA 4.0 unless a file states otherwise.
- The Verse name and official brand assets: reserved pending a public trademark policy.

See [licensing details](LICENSES/README.md).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the applicable architecture documents before proposing implementation. New features that change canonical resources, recipes, markets, blockchain behavior, or governance require a specification change before code is merged.
