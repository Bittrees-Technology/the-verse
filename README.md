# The Verse

The Verse is an open-source persistent voxel space universe, work-and-economy simulator, and Web3 marketplace.

The repository now contains the **locally cross-process verified P1.5 playable proof**, including authoritative grounded locomotion, EVA, orbital operations, contact physics, survival death, mining, physical production, construction, exact recovery, a fixed celestial registry, and interest-managed multiplayer streaming. A fresh local universe pre-admits two loopback-only development pilots; either native client identity can bind, move independently, aim, mine, and render the other pilot, while ownership prevents one pilot from spending or constructively operating the other's assets. The native client and browser command center now consume the same server-owned celestial identities and bounded public world views. This remains a development multiplayer cell, not yet the production-scale public universe or real-value economy.

## Play it on macOS

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

P0.10 retains durable input-only controls and one atomic Jolt-backed
character/grid physics step. A 1.8 m dynamic capsule owns radial upright
alignment, tangent walk/sprint, buffered jump, slope handling, bounded steps,
ground snap, magnetic attachment, and moving-support velocity inheritance. The
active two-cell transport uses protocol 18: gameplay mutations retain actor-local
idempotency while connections receive a registry-bound interest baseline
followed by acknowledged sparse deltas, bounded recovery baselines, and exact
enter/replace/remove semantics. Actor-private projections hide inventory,
production queues, progression, exact oxygen, operation history, and
cargo-inclusive mass. Before either official client applies a view, one shared
Apache-licensed verifier independently reconstructs it from the raw typed
message, checks the pinned universe/content/registry/manifest trust roots and
BLAKE3 commitment, and alone emits the exact acknowledgement. The browser runs
that core in a same-origin WASM Worker; the native client uses a Godot extension
and preserves protocol `u64` values losslessly even beyond Godot's signed
integer range. Neither client has an unverified fallback.

The canonical proof universe contains fixed Khepri Prime, Sable, an origin
asteroid field, two independently controlled loopback pilots, the salvage
skiff, and a powered industrial platform with physical cargo, conveyor,
refinery, and assembler blocks. Mining, construction, welding, cutting, and
grid operations remain server-authoritative and spatially validated. Refining
and manufacturing reserve cargo input into conserved FIFO job escrow, advance
on one-second integer quanta only with a valid conveyor route and qualifying
power, retain blocked output safely, and recover or drop escrow exactly once.
The native engineering terminal exposes inventory and production tabs and the
browser separates a local operations view from a registry-derived universe
map.

The complete local cross-process loop mines ore, transfers it to the industrial
grid, refines material, assembles a component, transfers it back, builds a
block, exercises two-player visibility and privacy, independently verifies
native and browser views, rejects an in-flight tamper without applying or
acknowledging it, recovers from an invalid frontier, and proves restart
convergence without accepted shortcuts. [Hosted CI run
33128613104](https://github.com/Bittrees-Technology/the-verse/actions/runs/33128613104)
passes the complete Linux replay, Linux container probe, independent browser
and native verifier suites, and Linux/Apple Silicon packages for implementation
revision `71e955c`. The later P1.6 checkpoint adds fenced sleeping-cell
production, and the current P1.7 checkpoint adds a durable two-cell directory,
same-session independent-EVA handoff, verified destination routing, and exact
restart recovery. Ordinary grid closure handoff is the next compatibility
boundary. Its audited atomic grid-and-rider placement primitive and dormant,
strict directory-v3 codec and package-v2 closure extractor are implemented.
The private package draft captures exact grid topology and motion, cargo,
production FIFO/escrow, owner and supported riders, operation histories, and
internal contacts; it independently checks containment, conservation,
identity conflicts, and external edges. A private draft-world-21 envelope now
freezes every closure subject, reserves every destination identity, issues an
exact quarantine receipt, survives successor-worker fencing, and persists
source and destination precommit-abort witnesses, all through a validated
directory-v3 authority view. Directory abort cleanup now accepts only the
matching witness hash, side, nonzero cleanup frontier, and resulting
draft-world commitment. The package now requires an exact canonical creation
origin for every production job, preventing equal local event numbers in two
cells from aliasing one job. The draft-world envelope persists the exact origin
map, derives package capture from that state instead of caller metadata, and
accepts an origin-qualified package through a quieter intermediate cell. A
private import-eligibility map is derived exactly from every packaged machine
FIFO and binds each queue to its transfer, destination fence, typed import
authority, production-clock generation, and checked one-second re-arm
boundary. Its pure planner rejects a substituted current queue or foreign
occurrence, pauses pre-boundary work, and releases the queue for normal
power/route evaluation only at or after that boundary. Raw import-authority
construction is test-only; production construction remains unavailable until
the import transaction can derive it from validated directory, canonical cell
event, live-fence, and trusted-clock evidence.
The dormant world-21 source-export transaction now removes the exact frozen
grid, riders, inventories, queues, provenance, and contacts in one cloned-state
mutation. Its checked conservation witness includes carried and escrowed
resources plus installed block components, advances one draft event frontier,
and produces separate acyclic mutation, event, resulting-world, and final proof
commitments. Exact retries survive later directory phases only when the
directory retains the matching final proof hash. The directory also persists
the exact mutation witness, conservation vector, and trusted export time so a
restart can reconstruct and validate that typed proof instead of trusting an
opaque hash. Directory v3 now requires
that durable source-export proof before import and a destination-activation
proof before source finalization. The draft-world envelope now also reserves
three destination-import persistence families: a live pending-activation lock,
live per-machine production holds, and a historical import record excluded
from the active-world hash. The typed import proof binds the nested validated
source-export proof, original quarantine evidence, monotonic trusted times,
destination event and fence, exact ledger vector, production lifecycle/root,
resulting active world, and separate acyclic mutation and final proof hashes.
The pure destination-import transaction now validates the live successor fence
and every world/draft identity, consumes the exact reservation, rebases only
derived local poses, inserts the complete grid/rider/inventory/contact/queue
closure and production provenance, records one checked import witness, and
derives the machine holds from its committed event without ticking production.
It then seals the pending-activation lock, resulting active world, typed proof,
and historical record in one cloned-state mutation. Exact retry returns that
durable result. Restart validation retains the complete pending lock and
machine-hold set until authenticated activation and later eligibility-release
witnesses replace them; an unrelated cell event cannot silently discard either
authority. The pure destination-activation transaction removes only the exact
pending gameplay lock, advances one event, and records an acyclic historical
proof without moving assets, ticking production, or changing conservation.
Restart validation reconstructs the exact pre-activation active world at that
event. Later gameplay may change the activated grid, and historical evidence
does not blacklist its root from a future transfer ID. Per-machine holds retain
the full packaged queue hash and sealed import boundary until a whole-cell
production occurrence consumes them. That dormant occurrence now derives one
ordered decision for every queue-bearing machine: pre-boundary imports emit an
explicit transfer pause without inspecting power, route, or capacity, while
due imports release and run the ordinary one-second outcome in the same
cloned-state mutation as unrelated machines. It validates trusted due time,
advances the production clock exactly once, retains complete occurrence and
release evidence outside the active-world payload while committing its compact
append-only head and count inside that hash, and reconstructs and replays the
exact predecessor at the release frontier. Live and released eligibility
records form an exclusive partition of every original import root, so silent
disappearance, resurrection, queue substitution, historical deletion, and
another handoff while a cell-bound hold remains fail closed. Release may
precede the independent gameplay activation because its boundary is derived
from import time. Dormant directory v3 persists and reconstructs both typed
import and activation proofs; late Imported/Finalized retries require exact
equality with the local historical results. Source finalization,
event-17/runtime scheduler and durable wake-up wiring, and the persistence
failpoint crash matrix remain disabled until their atomic paths are
implemented. The dormant proof harness also remains bounded and
snapshot-heavy; activation must place occurrence durability in the canonical
event journal and reserve evidence capacity before accepting an import.
Production remains
pinned to protocol 18/directory 2/package 1 until the complete protocol-19
tuple activates together. Production active-player load, the
production binary codec, general multi-cell execution, safe zones, accounts,
AMMs, and blockchain settlement remain in the
[delivery roadmap](docs/roadmap/roadmap.md). See [Celestial registry and
interest-managed visibility](docs/gameplay/celestial-registry-and-interest-management.md),
[Physical refining and manufacturing](docs/gameplay/physical-industry.md),
[Private player state projection](docs/gameplay/private-state-projection.md),
and [Operation idempotency and retry
contract](docs/architecture/operation-idempotency.md) for the exact current
authority, visibility, and recovery contracts.

## Licensing

- Game client and authoritative server: AGPL-3.0-or-later.
- SDKs and public schemas: Apache-2.0.
- Reusable art assets: CC BY-SA 4.0 unless a file states otherwise.
- The Verse name and official brand assets: reserved pending a public trademark policy.

See [licensing details](LICENSES/README.md).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the applicable architecture documents before proposing implementation. New features that change canonical resources, recipes, markets, blockchain behavior, or governance require a specification change before code is merged.
