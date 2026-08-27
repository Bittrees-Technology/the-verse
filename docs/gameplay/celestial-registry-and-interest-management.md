# P1.5 fixed celestial registry and interest-managed visibility

**Feature IDs:** F-003, F-012, F-014, F-038

**Status:** Local proof and independent official-client verification implemented
and cross-process validated; production-scale and current hosted evidence remain
open

**Owner:** Universe, simulation-worker, protocol, native-client, and browser
maintainers

The durable address and registry choices are recorded in
[ADR-0019](../decisions/ADR-0019-fixed-celestial-registry.md). The session,
interest, baseline, delta, view-hash, and recovery choices are recorded in
[ADR-0020](../decisions/ADR-0020-spatial-interest-replication.md).

## Linked requirements

- PLAT-004 — Browser control
- ID-004 — Open viewing
- WORLD-002 — Fixed bodies
- WORLD-003 — Wide separation
- WORLD-004 — Asteroid groups
- WORLD-008 — Partitioned execution
- WORLD-009 — Canonical celestial identity
- SIM-002 — Server authority
- SIM-006 — Finite execution budgets
- SIM-011 — Session-bound player authority
- SIM-012 — Multi-player cell
- SIM-013 — Server-derived spatial interest
- SIM-014 — Interest-view convergence

## Player outcome

The opening worksite reads as one small industrial operation inside a much
larger fixed universe. The mineable asteroid is visibly and canonically
separate from Khepri Prime. The planet, any moon, and every asteroid presented
as a real body come from one authoritative registry instead of client constants
or decorative invention.

Nearby pilots, grids, and voxel chunks appear and update without rebuilding the
whole world. Leaving replication range does not make an object look destroyed,
and returning sends one clean baseline without ghosts or duplicates. A browser
spectator can move between an exact fixed-body map and a bounded public view of
the origin cell without receiving private inventory or production state.

This milestone adopts general space-engineering expectations such as stable
scale, fixed celestial coordinates, smooth entity streaming, and readable
industrial status. It is an original Verse design and does not copy another
game's source, assets, interface, audiovisual presentation, fiction, names, or
distinctive visual expression.

## Current proof-scale boundary

The existing Khepri Prime is a **proof-scale test body** with a 1,200-metre
radius. It validates inverse-square gravity, atmosphere, oxygen, spherical
collision, and a separate orbital asteroid more than 3,000 metres above the
modeled surface. It is not a production-scale planet, a globally streamed voxel
world, or evidence for multi-day interplanetary travel.

P1.5 makes that status explicit in diagnostics and registry metadata. It
removes duplicated presentation constants, but it does not silently relabel the
current test sphere as a production planet. Production planet radii, minimum
separation, cruise speeds, and journey-duration ranges remain governed by
OQ-010. A configurable proof fixture may validate separation mechanics without
claiming that its numeric distance is final.

## Current implementation evidence

The local proof implements the exact registry and manifest bindings, immutable
fixed-body addresses, protocol-16 baselines and deltas, per-kind hysteresis and
cadence, actor-private composition, public spectator projection, browser maps,
and stable native streamed-entity lifecycle. Structural changes bypass a lower
motion cadence, so a mined voxel, block construction/damage transition, death
drop, or life-state change cannot remain stale indefinitely.

The worker lazily builds one immutable, exact-address spatial source per
authoritative revision and shares it across session projections outside the
runtime lock. The local scale harness admits and resynchronizes `2`, `8`, `16`,
`32`, and `64` simultaneous public-origin spectators. Every session receives
the same bounded 25-entity view; the measured 64-session run completed without
failed sessions and explicitly records `production_readiness_claim: false`.
A separate regression adds 2,048 irrelevant far entities and proves they do not
increase intersecting bucket lookups, visited candidates, query identities, or
selected view membership.

The full local playable scenario passes two-player mining and control,
refining, manufacturing, inventory transfer, construction, welding, damage,
grid splitting, death-drop, oxygen, respawn, exact recovery, and both native
client identities. The native and browser clients now run the same independent
verifier over the raw typed message, require pinned universe/content/registry/
manifest roots, recompute the complete authorized view commitment, stage
presentation before commit, and emit only verifier-owned acknowledgements. A
real shipped browser-page test proves an in-flight tamper produces no applied
state and no acknowledgement, while native tests cover exact unsigned values
beyond Godot's signed integer range and missing-extension failure. [Hosted CI
run 33112815767](https://github.com/Bittrees-Technology/the-verse/actions/runs/33112815767)
is the published Linux replay and Linux/Apple Silicon package result for
implementation revision `bb4ab4e`; hosted evidence for the verifier revision
is pending. Still open are active-player rather than spectator load
distributions, WAN/failure/soak testing, partitioned thousand-participant
evidence, and the production binary codec.

## Scope

### Included

- An immutable, versioned registry for fixed planets, moons, asteroid fields,
  and materialized voxel bodies.
- A body-relative address for the origin asteroid and a registry-derived Khepri
  environment and render transform.
- Configurable planet-separation validation and deterministic asteroid-field
  membership.
- Player-centered interest sets for public players, grids, drops, and voxel
  chunks inside one active cell.
- Explicit interest baseline, enter, update, leave, and re-entry semantics with
  stable entity identity and hysteresis.
- Actor-private projection applied to the authorized interest view.
- Native celestial rendering tiers, infinite-sky presentation, connection and
  stale-state feedback, and streamed-entity lifecycle handling.
- Read-only browser universe and local-cell maps with exact coordinates,
  selection, pan, zoom, loading, failure, and staleness states.
- Deterministic recovery, privacy, bandwidth, and native/browser UX evidence.

### Excluded

- Frontier materialization, surveying, procedural resource expansion, and
  generator-governance workflows.
- Dynamic cell scheduling, sleeping/background execution, and cross-cell
  handoff.
- Ship autopilot, multi-day routes, interception, or interplanetary travel.
- Streamed editable planetary terrain, terrain collision patches, and landing
  gameplay.
- Radar, stealth, combat sensors, weapons, or a gameplay detection model.
- Cockpit possession, thrusters, gyroscopes, and vehicle flight.
- Global live player tracking, unrestricted spectator cameras, or observation
  that wakes arbitrary sleeping cells.
- Browser physics gameplay, accounts, markets, contracts, or blockchain state.

## Authority and canonical registry

The universe service owns registry materialization, fixed addresses, generator
and content versions, and the minimum-separation rule. A simulation cell owns
the active body-local voxel, grid, player, and environment state within its
lease. Clients receive descriptions and relative transforms; they never create,
move, resize, or select the gravity source of a canonical body.

A materialized registry entry contains the equivalent of:

```text
CelestialBody
  body_id
  display_name_definition_id
  body_kind: planet | moon | asteroid | asteroid_field
  parent_body_id?
  field_id?
  normalized_center_address
  exclusion_radius_um
  fixed_gameplay_orientation
  geometry_definition_id
  voxel_definition_id?
  material_definition_id
  gravity_definition_id?
  atmosphere_definition_id?
  resource_definition_id?
  visual_descriptor_id
  scale_class: proof | production
  generation_rule_version
  content_manifest_version
  content_hash
  materialized_registry_version
```

Global address components use canonical signed integer or fixed-point encodings,
not unsafe JSON numbers. Physics receives a bounded body-relative local frame.
Changing the local origin changes only derived coordinates; it cannot move a
body, change relative separation, or mutate the canonical world hash.

Once materialized, a body keeps its ID and fixed address through generator
upgrades. An authorized migration may change content only through an explicit
versioned record. Planets and asteroids do not orbit in this model.

Every body that looks physical in the native client must resolve to a registry
entry. The current decorative moon must therefore become a registered moon
kind or be removed. If celestial registry schema 1 does not include a moon kind,
P1.5 removes that geometry rather than encoding a moon under a false kind. A
missing visual asset produces a neutral labelled proxy and an asset error; it
must never make an authoritative collider or gravity source invisible.

## Environment selection

The cell derives the player's current gravity and atmosphere source from the
registry and authoritative position. The environment projection distinguishes:

- the body currently supplying gravity or atmosphere;
- the nearest known registered body;
- distance to body center and nearest surface;
- altitude when the current body defines a surface;
- gravity magnitude and vector;
- atmosphere density, oxygen fraction, and breathability; and
- proof-scale versus production-scale classification.

The native and browser clients render these values. They do not recompute which
body owns the environment. If no body qualifies, the current environment is
deep space while the nearest known body may still be reported separately.

## Celestial presentation and scale truth

The native client renders a body from its registry-relative direction, radius,
and visual descriptor. A body beyond the local camera range may use a
camera-relative proxy, but the proxy must scale its distance and radius by the
same factor so its angular diameter remains:

```text
angular_diameter = 2 * asin(radius / distance_to_center)
```

Distance labels always report canonical distance, never proxy distance. Near
and far tiers overlap or cross-fade so a tier transition does not visibly move,
resize, or blink the body. A rendered planet cannot imply contact with an
independent asteroid at the genesis camera.

Stars are an effectively infinite camera-centered sky. Translating through the
local worksite cannot produce nearby star parallax. Sparse dust may provide
local motion cues, but it must read as nearby particulate matter rather than a
star, moon, asteroid, navigation target, or ore signal.

One-metre blocks, labelled body diameter and range, consistent light direction,
atmospheric limb, cloud altitude, terrain frequency, and asteroid metre scale
provide coherent size cues. Presentation never invents a larger canonical
collision surface or editable volume.

## Interest authority

An interest view is an ephemeral delivery decision, not canonical simulation
state. The worker derives a gameplay observer from the server-bound player's
authoritative position. Client camera direction, requested radius, and supplied
IDs may be bounded quality hints but never grant authority, disclosure, tool
range, collision, or wake-up rights.

The dynamic view has three priority classes:

1. **Control critical:** the bound player, exact actor-private player state,
   current locomotion support, actively controlled construct, and an accepted
   interaction awaiting its result.
2. **Near physical:** public players, grids, drops, and voxel chunks within the
   configured actor-centered active-cell range.
3. **Selected context:** an already visible selected entity retained through a
   bounded hysteresis margin long enough to present a clean leave reason.

Each entity type has a configured enter radius and a larger leave radius. The
server uses deterministic spatial queries and stable ID ordering. Rapid motion
inside the hysteresis band cannot cause enter/leave flicker. The control-critical
set cannot be removed to satisfy an ordinary presentation budget.

Public fixed-body and field summaries use the separate bounded celestial
registry read. Receiving registry metadata does not entitle a session to any
dynamic player, grid, drop, or voxel state near that body.

Interest management is not radar or occlusion gameplay. The absence of an
entity means only that the current projection does not include it. It is not
proof that the object does not exist, is destroyed, or is undetectable under a
future sensor system.

## View protocol and recovery

Every connection receives a fresh opaque `session_epoch` after authentication
or reconnect. Its `interest_epoch` changes when the authorized anchor, role,
policy, registry binding, or discontinuous view boundary changes; ordinary
hysteretic entry and exit do not change it. A complete `InterestBaseline`
identifies both epochs, a baseline ID, registry and universe-manifest hashes,
canonical event and simulation-tick frontiers, global commitment, ordered
complete visible view, and view hash.

A contiguous `InterestDelta` identifies the same epochs and baseline, delta
sequence, canonical event and simulation-tick frontiers, global commitment,
prior view hash, ordered entity enters, absolute component replacements,
ordered removals, and result view hash. These wire operations map to the
user-visible lifecycle as follows:

- an enter supplies the complete allowed public baseline before motion;
- a component replacement updates an already entered entity or chunk;
- a removal supplies the stable previously visible ID and the bounded UX reason
  `out_of_interest`, `destroyed`, or `transferred`;
- an origin change uses absolute replacement data or a new interest epoch and
  never claims canonical body motion; and
- a sequence, epoch, baseline, or previous-hash mismatch discards pending state
  and requests one fresh current baseline.

An entity cannot receive motion before its structural baseline. Re-entry after
leave sends one fresh baseline and does not reuse stale topology. An unchanged
grid keeps one client node across unrelated structural events. Voxel streaming
uses body ID, chunk ID, and revision rather than total visible voxel count.

The canonical event/tick frontier and global commitment remain explicit on the
interest stream so clients can relate a subset to the authoritative universe.
They are timing and hash side channels: a change may reveal that out-of-view
state changed, but the commitment is not proof that a particular entity or
field is visible. The deterministic view hash separately commits to the complete
resulting authorized subset, schemas, manifest and registry binding, epochs,
and delivery frontier. Interest never changes the server's canonical event
sequence or world hash. A missing entity is never interpreted as a canonical
deletion without an explicit removal.

Slow receivers retain bounded latest state. Structural enter/leave state is
ordered before newer motion and cannot be discarded merely because intermediate
motion was coalesced. Exceeding a non-critical stream budget reduces detail or
update frequency and exposes `stream_constrained`; it cannot drop control,
private reconciliation, accepted receipts, or required structural transitions.

## Privacy composition

Interest and privacy are separate filters. The worker first determines the
authorized public view, then attaches actor-private data resolved from the
immutable session binding. A spectator has no actor-private overlay. Another
player's actor-private records never become visible because the corresponding
public entity entered interest.

The bound player's carried inventory, control reconciliation, life state, and
operation frontier remain control-critical. Detailed cargo, grid mass, and
production queues are delivered only when their owning public machine or grid
is in the authorized active-cell view. Browser asset-status summaries and
signal-based remote operation remain later features; P1.5 does not turn
ownership into unlimited-distance visual or terminal access.

Public spectator observation uses a server-approved origin-cell observer. Query
parameters, cookies, headers, client names, and payload IDs cannot select a
player, enlarge a view, or wake an arbitrary cell. Public reads remain
non-cacheable when they contain live cell state.

## Native experience

Connection state is explicit:

```text
CONNECTING -> REGISTRY -> CELL BASELINE -> LIVE
                          |               |
                          v               v
                    PROJECTION ERROR   STREAM CONSTRAINED
                                          |
                                          v
                                        STALE
```

Movement, tools, inventory mutation, and production commands remain disabled
until the compatible registry, interest baseline, and matching actor-private
player state are installed atomically. A socket being open is not sufficient to
show `LIVE`.

Disconnect freezes authoritative presentation, marks it with last-update age,
disables commands, and begins bounded reconnect attempts while retaining a
manual retry. Reconnect clears the old interest epoch, transient selections,
pending presentation nodes, and authority-sensitive private state before
installing the new baseline. Fatal schema, projection, or registry errors remain
visible and fail closed rather than showing an empty universe.

Entity enter and leave effects are restrained and original. A leave transition
may briefly show `OUT OF LOCAL VIEW`; it cannot retain an actionable stale
transform or masquerade as destruction. A selected entity that leaves clears
its action controls and explains why. Destruction uses a distinct authoritative
effect and tombstone reason.

## Browser universe and local maps

The browser command center remains read-only and does not load the physics
client. It provides:

### Universe map

- Exact fixed-body registry positions and IDs.
- Body kind, radius or field extent, scale class, and registry version.
- Configurable logarithmic or hierarchical navigation that preserves exact
  coordinate strings outside rendering calculations.
- Clear separation between materialized bodies and unavailable frontier.
- No live player markers outside an authorized public observer view.

### Local-cell map

- The approved observer cell, local origin, view radius, and stream status.
- Pan, zoom, fit-to-visible, layer controls, and a distance scale.
- Direct nearest-marker selection for visible players, grids, voxel bodies, and
  chunks; clicking cannot cycle an unrelated object.
- Selected-object identity, position, public condition, power, last update, and
  `out_of_interest`, `destroyed`, or `transferred` departure reason.
- A fitted default view that includes the starter skiff, origin asteroid, and
  industrial platform rather than placing legitimate objects off-canvas.

`CONNECTING`, `LOADING REGISTRY`, `LOADING CELL`, `LIVE`, `STALE`, `RECONNECTING`,
`PROJECTION UNAVAILABLE`, and a genuinely empty authorized view are distinct
states. Previously rendered values are visibly stale after disconnect and are
never presented as current.

## Observability and budgets

Operators receive bounded metrics and structured logs for:

- registry materialization, validation, version, and body counts;
- planet-separation failures and invalid body references;
- active interest views and visible entities/chunks by priority class;
- enter, update, leave, re-entry, rebase, and reset rates;
- structural and motion bytes per session and entity type;
- coalesced motion, stream-constrained time, and baseline latency;
- projection, privacy, asset, and view-hash failures; and
- client connection-phase, stale-duration, and reconnect distributions.

Logs identify universe, registry, cell, interest epoch, observer class, entity,
event, and schema versions without publishing private inventory or queue data.

## Automated acceptance

### Registry, coordinates, and environment

1. The same universe seed, generation rules, and content manifest produce the
   same sorted registry bytes and hash across restart and replay.
2. Body IDs and addresses are unique; a materialized body cannot move after a
   generator upgrade without an explicit migration.
3. Every P1.5 fixed-body exclusion volume satisfies the manifest-pinned
   3,000-metre proof surface gap, equality passes, one micrometre below fails,
   and asteroid-field membership is deterministic. This proof value is not the
   production planet-separation decision.
4. Global address serialization round-trips signed values without JavaScript
   precision loss. Local rebase preserves relative transforms within the
   published tolerance and sends no non-finite or out-of-bounds physics value.
5. Gravity source, nearest body, altitude, atmosphere, oxygen, and native render
   transform derive from the same registry entry.
6. Every visible physical planet, moon, or asteroid resolves to a registry
   body. A missing asset uses the labelled fallback and never hides authority.

### Interest, ordering, and privacy

1. The gameplay view derives from the bound actor, and spoofed position, actor,
   observer, radius, headers, or query parameters cannot enlarge it.
2. An entity enters at the inner boundary, remains throughout hysteresis, and
   leaves only beyond the outer boundary in stable ID order.
3. The bound player, current support, controlled grid, and accepted pending
   interaction cannot be culled by an ordinary view budget.
4. Structural enter precedes motion. Leave is explicit, destruction is
    distinct, and re-entry sends one baseline without duplicate identity.
5. A voxel chunk change is detected from body/chunk revision even when the
    total visible voxel count does not change.
6. Interest computation and view reset do not change canonical event sequence,
    canonical world hash, ownership, inventory, or physics.
7. Delayed and slow clients converge through bounded structural state plus the
    newest motion without losing required enter/leave transitions.
8. Spectator and foreign-player views contain no private inventory, drop,
    exact mass, control, progression, operation, or production-queue field.
9. Increasing irrelevant entities does not increase one session's payload
    beyond its configured view budget. Published 2-, 8-, 16-, 32-, and 64-player
    distributions and a partitioned 1,000-participant synthetic run define the
    tested envelope; this does not claim 1,000 full-rate players in one cell.

### Native and browser clients

1. Native celestial position and radius are registry-derived, apparent angular
    diameter remains within the visual tolerance at multiple distances and
    after rebase, and local translation produces no star parallax.
2. Unchanged grids retain node identity across unrelated structural updates;
    enter, leave, and re-entry produce no ghost or duplicate nodes.
3. Native controls remain disabled until `LIVE`; disconnect marks state stale,
    reconnect installs a new epoch, and schema/projection failure remains
    visible and fail-closed.
4. Browser universe and local maps fit their data, preserve exact global
    coordinates, select the clicked nearest marker, and remove out-of-interest
    objects from state and the document.
5. Browser reconnect replaces the prior view epoch, marks old data stale, and
    never represents projection failure as an empty universe.

## Manual UX acceptance

1. From the genesis camera, the mineable asteroid unmistakably reads as a
   separate orbital body and does not intersect or appear attached to Khepri.
2. One-metre blocks, the local asteroid, Khepri's labelled proof radius, its
   range, atmosphere, cloud height, and light direction form coherent scale
   cues without claiming a production-sized planet.
3. Roll, turn, and translate around the worksite. Khepri retains stable
   direction and angular size for the movement, while the star sky has no local
   parallax and nearby dust remains recognizable as dust.
4. Move two clients repeatedly across an interest boundary. The remote pilot
   does not flicker, duplicate, look destroyed, or retain an actionable stale
   transform.
5. Mine while another client observes. Only affected asteroid chunks change,
   tool and impact feedback remain continuous, and neither client rebuilds an
   unrelated construct.
6. Disconnect during movement and production. Native and browser views clearly
   become stale, commands stop, and reconnect recovers without ghosts, private
   leakage, or optimistic inventory changes.
7. In the browser, move from the universe registry to the origin-cell view,
   fit all visible objects, and directly select the asteroid, starter skiff,
   industrial platform, and pilots.
8. Capture macOS and Linux evidence for planet limb, asteroid separation,
   celestial tier continuity, entity enter/leave/re-entry, loading, stale,
   constrained, and fatal projection states.
9. Originality review confirms that maps, HUD, transitions, celestial assets,
   terminology, silhouettes, colors, audio, and interaction layout do not copy
   protected expression from Space Engineers or another franchise.

## Rollout and rollback

1. Land the registry and address schemas with deterministic validation fixtures.
2. Derive the current Khepri environment and origin asteroid identity from the
   registry without changing proof gameplay quantities.
3. Add session interest epochs, spatial queries, view projection, and fault and
   privacy tests.
4. Convert native scene updates to stable entity/chunk diffs and registry-driven
   celestial tiers.
5. Convert the browser into exact universe and fitted local-cell maps.
6. Publish cross-platform UX, bandwidth, and active-player evidence.

Rollback restores the prior protocol executable and archived compatible proof
world. A newer registry, world, projection, or view schema is never interpreted
by an older executable. Interest state is disposable and rebuilt from canonical
state after restart; it is never recovered by trusting a client cache.

## Open numeric gate

OQ-010 must set production planet separation, ship cruise speed, fuel cost,
interception windows, and journey-duration targets before The Verse claims
production-scale interplanetary geography. That gate does not block the fixed
registry, exact coordinates, proof-scale classification, body-relative local
frames, or active-cell interest-management architecture specified here.
