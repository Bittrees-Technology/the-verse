# Clients and public APIs

**Status:** Proposed baseline

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
- Binary real-time protocol for native simulation replication.
- Webhooks for approved server-to-server notifications.

## Versioning

- Public schemas use semantic versions.
- Breaking changes require a parallel support window.
- Event schemas are immutable after publication; a new version creates a new schema name/version.
- SDK releases identify the compatible API and content-manifest ranges.

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
