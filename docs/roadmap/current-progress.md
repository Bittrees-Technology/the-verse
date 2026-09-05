# The Verse — current progress

**Reviewed:** 2026-09-05 against main revision `18670fe`.

The Verse is in **P1: multiplayer vertical slice**. A playable local engineering
prototype and substantial persistence infrastructure exist. The public,
persistent industrial universe remains the destination. A completion percentage
would obscure the difference between a bounded proof and production readiness.

## Delivery position

| Area | Evidence-backed position | Remaining boundary |
| --- | --- | --- |
| Specification and engineering baseline | Requirements, feature IDs, ADRs, toolchains, and CI exist | Later open questions and acceptance gates still apply |
| P0 simulation proof | Implemented gameplay foundation with published Mac/Linux benchmark checkpoints | Continue preserving conservation, authority, and recovery |
| P1.0–P1.5 multiplayer and industry | Two local pilots, mining, physical refining/manufacturing, private state, fixed celestial registry, verified native/browser interest views | Development scope; spectator measurements do not establish active-player capacity |
| P1.6 background production | Bounded sleeping-cell production and fenced recovery implemented | No background physics, combat, travel, or general distributed scheduling |
| P1.7 two-cell handoff | Independent-EVA handoff implemented under protocol 18 | Ordinary grid-and-rider closure is not yet a completed playable feature |
| P1.8 compatibility migration | Source validation, prepared install, signed activation, directory-v3 authority, and production-only lifecycle-v2 scheduling merged | Ordinary event-17 gameplay, projection, verifier, and client cutover; external scheduler wake sources |
| First-session experience | Guided contract and engineering loop exist; recent movement/interpolation fixes merged | F-062/F-063 acceptance evidence remains required before broader readiness claims |
| Public P1 release | Native development packages and local browser operations available | Safe zone, offline defense/cleanup, public identity/API scope, signed updater, multi-day soak and active-player load |
| P2 economy | Design baseline exists | Companies, contracts, test-credit markets, automation, regional demand, agent SDK and economic simulation gates |
| P3 and later | Planned | Testnet settlement, subsequent production release gates, frontier expansion and advanced structure partitioning |

The current universe has fixed celestial identities and a small authored proof
worksite. It does not yet provide streamed editable planets, an expanding
frontier, multi-day routes, or thousands-player capacity. The local development
pilots and progression are not public account or occupational economy systems.

## Verified baseline

[CI run 33838538429](https://github.com/Bittrees-Technology/the-verse/actions/runs/33838538429)
passed on 2026-09-04 for main `18670fe`, including the verification job and
Linux x86_64 / Apple Silicon native package jobs. The verification job includes
the repository check script and isolated Linux container probe.

The latest feature merge is [PR #19](https://github.com/Bittrees-Technology/the-verse/pull/19),
protocol-19 lifecycle-v2 scheduling. Recent merged fixes address local movement
reconciliation, remote-player interpolation, grounded obstacle presentation,
and lease renewal during long world loads. These fixes support ongoing quality
work; they do not independently close F-063.

The interactive development path remains protocol 18 / directory 2 / package 1.
The signed protocol-19 path is a verified readiness and production-only
infrastructure checkpoint with gameplay admission closed. Do not interpret
signed universe activation as a released interactive universe or a signed
public client distribution.

## Next gates in accepted order

1. Complete P1.8 ordinary event-17 gameplay and the coordinated projection,
   shared verifier, native client, and browser cutover. Preserve exact migration,
   authority, conservation, restart, and legacy-startup fencing evidence.
2. Close the ordinary grid-and-rider handoff acceptance boundary. Require the
   complete closure, cargo, queues, escrow, ownership, controls, destination
   verification, and crash/retry matrix before calling F-061 complete.
3. Prioritize F-062 and F-063: a safe first-session engineering worksite and
   measured movement, camera, interaction, correction, and performance gates.
   The next playable slice should demonstrate one trustworthy conserved work
   loop before expanding construction breadth.
4. Complete the remaining public P1 scope and publish multi-day soak,
   active-player load, recovery, and packaging evidence. Connect external wake
   sources and validate scheduling without making a multi-host claim from a
   local proof.
5. Advance P2's internal test-credit economy after the gameplay gates. Keep
   bNOTE and the Base bridge deployment inputs deferred until P3 testnet work,
   as required by the roadmap.

The lifecycle coordinator's append-before-ack crash test becomes a release gate
when ordinary event-17 gameplay can create a runnable queue through the
activated Store; migration currently imports a quiescent frontier. See
[ADR-0027 required validation](../decisions/ADR-0027-protocol-19-lifecycle-v2-scheduling.md#required-validation).

## Planning decisions still ahead

The [open-question register](../open-questions.md) remains authoritative for
unresolved decisions. Market curves require economic simulation and an ADR;
production route distances and speeds precede travel rollout; megastructure
partitioning precedes arbitrary cross-cell structures. These do not justify
blocking the already specified bounded gameplay work on Web3 deployment.

Use the [roadmap](roadmap.md) for full phase scope, the
[feature catalog](../product/feature-catalog.md) for stable feature IDs, and the
[detailed implementation checkpoint](implementation-checkpoint-2026-09-05.md)
for the preserved migration and authority notes. Update this page's reviewed
revision and evidence when the next gate merges; keep accepted scope separate
from demonstrated implementation.
