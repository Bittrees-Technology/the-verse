# Implementation checkpoint — 2026-09-05

This preserves the detailed implementation notes from the README at main
revision `18670fe`. These notes describe bounded proofs and compatibility
boundaries, not public deployment. For the delivery assessment and next gates,
read [current progress](current-progress.md).

## Technical checkpoint

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
equality with the local historical results. The dormant source-finalization
transaction now requires those exact directory-retained proofs, advances one
source event without changing gameplay or economy state, and commits both a
compact active tombstone and a full historical proof chain. It rejects
pre-activation, backdated, substituted, missing-local-event, and stale-fence
attempts; exact cell-first and directory-first retries survive restart. The
event-17 Store adapter and protocol-19 lifecycle-v2 coordinator now form a
production-only runtime path. The coordinator durably claims or recovers
directory-v3 authority, records one-second production occurrences, appends
them through the canonical event-17 journal, acknowledges the resulting world
frontier, and releases idle cells without polling. Its recovery tests cover
uncertain lifecycle writes, split directory/lifecycle commits, partial journal
tails, stale logical authority, deleted child history, a universe-level
write-ahead commitment, and read-only all-cell preflight before any cell
recovery write. Ordinary event-17 gameplay admission and external scheduler wake-source
wiring remain disabled. The dormant proof harness also remains bounded and
snapshot-heavy; activation must place occurrence durability in the canonical
event journal and reserve evidence capacity before accepting an import.
The source-bound prepared-install bridge now derives a canonical receipt and
directory-v3 genesis while every legacy lock remains held. It copies the exact
frozen directory and canonical mapping artifacts, persists a
`staged_unactivated` lifecycle-v2 genesis beside each target snapshot, strictly
stages or reopens the complete cell set, and writes one universe commit head
last. Missing, extra, swapped, hybrid, or independently valid material from
another frozen frontier fails closed; without the global head, partial target
files grant no installed authority. The signed activation checkpoint now adds
a canonical 2-of-3 authorization, a universe-root head written last,
forward-only verified restart, protocol-18 startup fencing, an offline
activation/verification tool, and a fail-closed worker readiness boot that
derives the complete target only from that head. Interactive protocol-19
directory history now treats the signed genesis as an immutable prefix while
accepting only validated hash-chained successors. Crate-private claim,
recovery, and release transitions derive every new assignment generation and
fence from the durable tip, reject transition-kind retry aliases, and preserve
transfer pins. Worker gameplay admission still requires ordinary event-17,
projection-5, verifier, and client cutover. Production remains pinned to
protocol 18/directory 2/package 1 until that complete protocol-19 tuple
activates together. Production active-player load, the
production binary codec, general multi-cell execution, safe zones, accounts,
AMMs, and blockchain settlement remain in the
[delivery roadmap](../roadmap/roadmap.md). See [Celestial registry and
interest-managed visibility](../gameplay/celestial-registry-and-interest-management.md),
[Physical refining and manufacturing](../gameplay/physical-industry.md),
[Private player state projection](../gameplay/private-state-projection.md),
and [Operation idempotency and retry
contract](../architecture/operation-idempotency.md) for the exact current
authority, visibility, and recovery contracts.
