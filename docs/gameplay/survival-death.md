# P0.8 survival death foundation

**Status:** Implemented local proof

## Player promise

Suit oxygen must be a survival system rather than decorative telemetry. If life support fails, the authoritative universe stops the character's work, preserves their economic assets in one canonical death drop, and offers a clear free recovery action without asking the client to choose the outcome.

## Acceptance behavior

1. Suit oxygen, helmet state, breathability, and all oxygen rates remain server-owned.
   The P0 worker advances life support continuously; connecting, disconnecting, or changing a client name cannot reset it.
2. Below the content-defined critical threshold, the native client shows a persistent critical warning and the action needed to preserve oxygen.
3. Reaching zero oxygen atomically changes the canonical player state from alive to incapacitated. There is no playable zero-oxygen state.
4. Movement, suit toggles, mining, refining, fabrication, inventory transfer, construction, welding, grid control, anchoring, and damage intents are rejected while incapacitated.
5. Any carried ore, refined material, and components move into one canonical drop at the death coordinate. The suit inventory becomes empty without changing its stable ID.
6. An empty suit creates no empty drop. Death does not change experience, career progress, or the material ledger.
7. The incapacitated state and drop survive disconnect, process restart, snapshot, and journal replay.
8. The client presents a centered life-support failure state, disables local movement and tools, and sends a location-free respawn request.
9. Respawn is free and server-selected. P0.8 uses a named proof recovery origin plus a deterministic clear-point fallback corridor, restores configured suit oxygen and modes, preserves progression and the existing drop, and remains idempotent on retry.
10. Dropped inventory is visible in authoritative snapshots but cannot be refined, crafted, or remotely transferred through generic inventory actions.

## Authority and failure boundary

The client renders and predicts feedback only. It cannot declare death, choose a drop, change its contents, select a spawn, refill oxygen, or restore control. Death and respawn are canonical events validated before mutation and appended before publication. Conservation runs after both.

## Explicit limits

This checkpoint is the creation and recovery foundation, not the complete P1 death-drop lifecycle. It does not implement recovery or salvage actions, team permissions, the 15-minute private window, public access, six-hour cleanup, verified-outage pausing, tombstones, physical loot-container collision, powered spawn facilities, or the capital. The separate absolute-position character-motion gap is governed by the P0.9 contract in [ADR-0013](../decisions/ADR-0013-input-only-authoritative-character-motion.md).
