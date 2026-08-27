# ADR-0012: Make oxygen death and proof respawn canonical

**Status:** Accepted

## Context

The authoritative simulation already owns suit oxygen, helmet state, and environmental breathability, but oxygen currently stops at zero without changing what the player may do. A client can continue moving, mining, manufacturing, building, or damaging blocks. A visual-only failure screen would conceal that authority defect.

The accepted product rules require death to be free, move carried inventory atomically to the death location, and eventually offer server-selected powered or capital respawn. They also require a 15-minute private recovery period, six-hour expiry, outage-aware scheduling, and auditable cleanup. The current single-player proof has no accounts, team permissions, capital, powered respawn facilities, or durable lifecycle scheduler, so it cannot honestly claim the complete death-drop feature.

## Decision

Content schema 6 and manifest `p0.8.0` move oxygen capacity, per-second atmosphere/helmet rates, the critical threshold, respawn suit defaults, and one temporary proof recovery origin into validated server-owned content. The server tries that origin first, then searches a fixed two-meter, positive-Y corridor for the first collision-free point. The search is deterministic and bounded to 2,048 steps, beyond the P0 grid-span bound. The proof corridor is not called the capital and does not satisfy powered-facility spawn selection.

World schema 10 stores an explicit player life state and canonical death-drop metadata. Protocol 7 exposes that state, death drops, and `RespawnPlayer { operation_id }`. Event schema 5 adds `PlayerIncapacitated` and `PlayerRespawned`. Older protocol, world, event, or content versions are rejected rather than implicitly migrated.

An alive player whose authoritative oxygen calculation would reach zero transitions through one `PlayerIncapacitated` system event. No durable `alive + zero oxygen` intermediate state is permitted. The event records the cause, canonical position, previous oxygen, deterministic death identity, and the exact carried-inventory move. It sets oxygen to zero, disables the jetpack, clears player-issued grid control inputs into dampening, and changes life state to incapacitated.

P0.8 advances the single canonical player's life support continuously while the simulation worker is running. WebSocket presence and client-provided names cannot start, stop, or reset the oxygen clock. Account-bound online/offline character presence is deferred until authenticated player sessions exist.

If the carried inventory is nonempty, the same event creates one new dropped inventory and one metadata record. IDs derive from player identity and event sequence. The original suit inventory retains its stable ID and capacity but becomes empty. Empty carried inventory creates no empty drop. The ledger does not change, and conservation totals before and after the event are identical.

An incapacitated player may request only respawn. Every other player mutation is rejected before event preparation. World-owned simulation may continue. Repeated life-support advances cannot create another death or drop.

`RespawnPlayer` contains no client-selected location or outcome. The server prepares one `PlayerRespawned` event from validated content, restores the alive state at the first clear point in the proof recovery corridor, refills suit oxygen, applies the configured helmet and jetpack defaults, and preserves experience, career, ledger, existing drops, and the empty suit inventory. The operation ID makes retries return the original receipt without a second respawn.

Dropped inventories are sealed from generic refine, craft, and transfer paths. Recovery and salvage require later explicit permission-aware events; knowing an inventory ID never grants access.

Automatic death uses the existing append-before-publication transaction boundary. A failure before durable append retains the complete alive state and inventory. A failure after durable sync halts the writer; restart replays the complete incapacitated state and exact drop. No hybrid state is published or recovered.

## Consequences

- Zero oxygen gains a server-enforced gameplay consequence and persistent client-visible state.
- Death and respawn neither charge nor award BIT, experience, career progress, or economic resources.
- The proof closes the inventory-duplication boundary without prematurely introducing a real-time cleanup scheduler.
- The existing absolute-position character protocol remains a known authority gap while the player is alive. Incapacitation still rejects its movement messages. Input-only server character motion is a separate required milestone.
- Combat death, health, corpse physics, drop recovery, team access, public salvage, expiry, outage pausing, powered spawn selection, and capital fallback remain unimplemented and unclaimed.

## Required evidence

- Crossing the oxygen boundary emits one incapacitation event directly, while repeated advances emit none.
- Mixed carried contents move into exactly one drop at the canonical death position; empty inventory creates no drop; conservation and career totals do not change.
- Every non-respawn mutation is rejected while incapacitated, and generic inventory actions cannot address a dropped inventory.
- Replay rejects tampered death identity, owner, position, oxygen, inventory ID, capacity, or contents before mutation.
- Respawn position and suit outcome are server-selected, obstruction-safe within the bounded P0 world, free, deterministic, and idempotent.
- Before-write and after-sync failpoints recover either the complete alive state or the complete incapacitated/drop state, never a hybrid.
- Snapshot and journal restart preserve incapacitated and post-respawn states exactly.
- Protocol 6, world schema 9, event schema 4, and content `p0.7.3` fail explicitly at their version boundaries.
- The native client disables movement and work from canonical life state, displays critical and incapacitated feedback, and offers a respawn action that submits no location.

## Deferred lifecycle contract

LIFE-004, LIFE-005, LIFE-007, and LIFE-013 remain binding future requirements. Their implementation must add powered personal/company/allied spawn selection, a real capital fallback, owner/team recovery, public salvage, six-hour expiry, verified-outage pausing, cleanup tombstones, and settlement evidence before F-020 can be marked complete.
