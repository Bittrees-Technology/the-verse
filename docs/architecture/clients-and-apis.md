# Clients and public APIs

**Status:** Proposed product API baseline; P1.5 replication and P1.7 handoff
contracts accepted

## Native client

The native macOS/Linux client is responsible for:

- Rendering.
- Input.
- Local prediction.
- Audio.
- UI.
- Asset streaming.
- Submitting signed/authenticated gameplay intents.
- Receiving authoritative replication and corrections.

It is not authoritative for inventory, damage, physics results, voxel changes, production, contracts, or market settlement.

Character clients submit bounded controls with a server-owned movement epoch and monotonic input sequence, never transforms. A durable receipt advances the canonical received sequence; it does not claim that physics has consumed the control. The authoritative cell persists a bounded FIFO and consumes at most one transition per fixed substep, advancing a separate processed sequence that the native client uses to discard and replay prediction inputs. Reconnect resumes sequence allocation after the received frontier. The cell owns EVA pose, velocity, gravity, Jolt-backed collision, capsule support, locomotion kind, radial upright alignment, walk/sprint, jump, steps, slope transitions, magnetic attachment, moving-support velocity, and the compatibility `surface_contact` result under [ADR-0013](../decisions/ADR-0013-input-only-authoritative-character-motion.md) and [ADR-0014](../decisions/ADR-0014-authoritative-grounded-and-magnetic-locomotion.md).

## P1.5 gameplay replication

Protocol `16`, projection schema `3`, and interest schema `1` replace the
complete-cell state stream with a server-derived session view. A player anchor
comes only from the immutable authenticated actor binding. A spectator anchor
requires a server-side grant. A camera, query coordinate, client-provided
radius, or requested entity ID cannot widen the view.

The first state message is a complete interest baseline carrying session and
interest epochs, a baseline ID, observer class, cell and derived local-origin
addresses, fixed registry and universe-manifest hashes, canonical event/tick
frontier, global world commitment, and view hash. Contiguous deltas then carry
ordered full enters, absolute component replacements, bounded removals, and an
explicit absolute rebase when the derived local origin changes. The client
applies a delta only when epoch, baseline, sequence, and previous hash match;
otherwise it clears the partial view and requests one new baseline.

The stream's `view_hash` covers only the complete audience-authorized projected
view. It is a convergence check, not authority. Protocol `16` also carries the
canonical event/tick frontier and global world commitment required by existing
authoritative reconciliation. Those global values can signal out-of-view
activity but do not describe hidden entities; subset clients converge with the
view hash and must not infer hidden state from the global commitment.
Actor-private inventory, production, operation, oxygen, drop, control, and
exact owned-mass fields remain in the bound actor's overlay and are never
inferred from spatial proximity. Carried inventory and control reconciliation
remain control critical; ownership alone does not stream detailed remote cargo,
mass, or production when the owning public grid or machine is outside the
authorized active-cell view.

Clients may predict already authorized local motion. An entity leaving
interest is removed from presentation and targeting caches. That removal does
not establish that the canonical entity was destroyed, and a loaded entity
does not establish that an intent against it is valid. The server always
reconstructs range, visibility, collision, ownership, and permissions from
canonical state.

## P1.7 same-session cell handoff

Protocol `18`, projection schema `4`, and interest schema `2` preserve one
authenticated gateway session while a player or piloted grid transfers between
the two proof cells. The client never chooses the destination, package,
transfer closure, assignment generation, or placement generation.

The gateway presents a bounded state sequence:

```text
LIVE -> HANDOFF_PREPARING -> HANDOFF_IMPORTING -> VERIFYING_DESTINATION -> LIVE
```

Controls are neutralized after the source prepare boundary. Directory commit
and destination import increment movement and interest epochs, discard every
source baseline, delta, acknowledgement, predicted input, verification stage,
and private overlay, and install one complete destination baseline. That
baseline binds the transfer ID, destination cell key, current placement
generation, destination cell-scoped frontier, and all existing trust roots.
The official verifier must commit it before the gateway releases new controls.

A timeout or reconnect asks the directory for canonical placement; it never
guesses a cell or restores a stale source route. A source removal marked
`transferred` is valid only when linked to committed transfer evidence and
reveals no private destination or package contents.

## Browser command center

Initial browser capabilities:

- Passkey enrollment and recovery.
- Profile and smart-account status.
- Inventory and provenance.
- Capital and regional markets.
- AMM quotes, slippage, and liquidity.
- Production and power state.
- Employment and company administration.
- Travel routes.
- Maps and alerts.
- Blueprints and content.
- Spectating through delayed or permissioned feeds.

The browser does not need to load the full physics client.

## Identity flow

1. User creates a profile with email and WebAuthn passkey.
2. Identity service creates or links a smart-account identity.
3. User receives a short-lived application session.
4. Gameplay uses scoped session authority.
5. Market and withdrawal permissions use separate limits.
6. Recovery changes are delayed and auditable.
7. High-value operations require fresh passkey confirmation.

Email alone must not immediately transfer wallet control.

## External agents

Bots and AI agents are first-class clients.

They use:

- OAuth-style application registration.
- Passkey, delegated smart-account authority, or service credentials.
- Explicit scopes.
- Rate and simulation budgets.
- The same authoritative intent gateway as native clients.
- Public event subscriptions.
- Deterministic error codes.

No hidden “human-only” endpoint is required, but system NPCs must be identified as system actors in public economic data.

## API surfaces

### Public reads

- Universe and celestial registry.
- Universe manifest schema/version/hash and celestial registry
  schema/version/hash.
- Market pools, quotes, trades, and liquidity.
- Public companies and governance.
- Public asset provenance.
- Settlement proofs.
- Content manifests.
- Public travel and station data subject to gameplay visibility rules.

### Authenticated reads

- Private inventory.
- Company internal state.
- Production queues.
- Permissions.
- Private contracts.
- Wallet and recovery state.

### Authenticated intents

- Movement and gameplay actions.
- Production scheduling.
- Market deposit and withdrawal.
- AMM swap.
- Contract creation and acceptance.
- Company and governance actions.
- Blueprint publication.
- Mod submission.

## API protocols

- REST for commands and simple resources.
- GraphQL for composed browser queries.
- WebSocket or server-sent events for subscriptions.
- Versioned real-time protocol for native simulation replication; P1.5 pins
  baseline/delta semantics and P1.7 pins transfer-linked cell-scoped
  convergence while the final production binary codec remains later work.
- Webhooks for approved server-to-server notifications.

## Versioning

- Public schemas use semantic versions.
- Breaking changes require a parallel support window.
- Event schemas are immutable after publication; a new version creates a new schema name/version.
- SDK releases identify the compatible API and content-manifest ranges.

The P1.5 compatibility tuple is indivisible:

| Boundary | Version |
| --- | --- |
| Gameplay protocol | `16` |
| Projection schema | `3` |
| World schema | `18` |
| Event schema | `14` |
| Content schema | `11` |
| Content manifest | `p1.5.0` |
| Celestial registry | `1` |
| Universe manifest | `2` |
| Interest schema | `1` |

Handshake rejects any mismatch before state delivery. Protocol `15` may remain
only on an explicitly local diagnostic endpoint and is never an automatic
public downgrade. Reconnect creates a new session epoch and complete baseline;
clients do not reuse old deltas or acknowledgements.

The P1.7 compatibility tuple is also indivisible:

| Boundary | Version |
| --- | --- |
| Gameplay protocol | `18` |
| Projection schema | `4` |
| World schema | `20` |
| Event schema | `16` |
| Content schema | `11` |
| Content manifest | `p1.5.0` |
| Celestial registry | `1` |
| Universe manifest | `4` |
| Interest schema | `2` |
| Operation fingerprint | `2` |
| Lifecycle control | `2` |
| Production occurrence | `1` |
| Cell directory | `2` |
| Transfer/package | `1` |

Upgrade and rollback drain incompatible sessions. Protocol `18` state is never
reinterpreted as a P1.6 stream, and handoff begins only after both cell workers,
the directory, gateway, verifier, and client agree on the complete tuple.

Atomic ordinary-grid closure handoff introduces protocol `19`, projection `5`,
world `21`, event `17`, universe manifest `5`, interest `3`, directory `3`, and
transfer package `2`. The complete boundary is recorded in ADR-0024; v1
transfer artifacts remain independent-EVA artifacts and fail closed under the
grid runtime.

## Permission examples

```text
universe:read
market:read
market:quote
market:trade
inventory:read
inventory:deposit
inventory:withdraw
production:read
production:manage
company:read
company:manage
governance:vote
blueprint:publish
agent:operate
```

Session permissions should be narrow, revocable, time-bound, and optionally amount-limited.

## Distribution

Direct-download releases require:

- Signed macOS applications and notarization.
- Signed Linux artifacts.
- Checksums and a signed update manifest.
- Reproducible build goals.
- Rollback channel.
- Stable, beta, and development release tracks.
- No embedded private RPC credentials.
