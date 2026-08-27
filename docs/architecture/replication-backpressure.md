# P1 latest-state replication backpressure

**Status:** Implemented local correctness transport; production binary interest management remains required

## Failure being prevented

The original worker used a 64-message broadcast ring for complete and motion snapshots. A slow WebSocket receiver lost its cursor, interpreted the loss as a need for a complete world snapshot, and then continued behind the same high-rate stream while serializing and sending that larger snapshot. Repeated lag therefore amplified disposable motion into repeated full-world work.

Motion snapshots are absolute current player and grid state, not deltas. Sending every intermediate motion state to a receiver that is already behind adds latency and cannot improve convergence. Complete structural snapshots are also absolute. A newer complete snapshot subsumes an unsent older complete snapshot, but the client must receive the newest required structure before any newer motion-only state.

## Worker contract

The worker owns one cell-wide latest-state feed with exactly two optional retained entries:

- the newest complete structural snapshot;
- the newest motion snapshot whose sequence is later than the retained structural snapshot.

Publication occurs while the authoritative runtime lock still orders the mutation. A client cursor separately records its newest state sequence and newest complete-snapshot sequence. On each 60 Hz replication period the connection sends at most one message:

1. Send an unseen complete structural snapshot first.
2. Otherwise send only the newest motion snapshot later than the cursor.
3. Otherwise send nothing.

A slow receiver therefore consumes no per-client state backlog. Missed timer periods use skip semantics. At quiescence it needs at most one complete snapshot and one motion snapshot to reach the current world hash, irrespective of the number of updates produced while it was blocked. Handshake snapshots, direct snapshot requests, and intent receipts remain outside this periodic budget because they are explicit protocol responses.

The normal lock-order invariant prevents a structural snapshot from appearing behind a motion sequence already sent to the same cursor. If that invariant is ever violated, the worker requests a fresh current complete snapshot instead of sending a lower sequence. This fail-safe prevents stale rollback and preserves structural convergence.

## Full-snapshot boundary

A complete snapshot is sent for handshake, an explicit client request, or an accepted/system transition whose state is absent from the motion schema. Current examples include inventory, voxel, construction, suit oxygen, and life-state changes. Character control and physics-only progression publish motion snapshots. A repeated idempotent operation at an already-published sequence does not create another retained update.

Motion congestion alone is never a reason to send a complete snapshot.

## Persistence and idle activity

Replication coalescing does not alter the canonical journal. The simulation runtime appends a physics event only while a grid moves or a living player's velocity, input, pending control, control lease, dampener setting, or jetpack-off locomotion keeps physics active. A client that repeatedly renews an unchanged neutral control lease can therefore cause otherwise idle fixed-step journal growth before replication sees the result. The safe fix is to stop redundant neutral control renewal at the input producer or change the canonical control/physics contract with replay evidence; the worker must not silently discard an authoritative active lease.

Independent one-second oxygen/life-support transitions remain intentional structural journal events.

## Evidence

Worker tests cover:

- 4,096 superseded motion updates collapsing to one retained motion state;
- structural-before-motion ordering even when publication order is adversarial;
- fresh-snapshot recovery instead of a lower-sequence rollback;
- an authoritative burst exceeding the removed 64-message ring converging through at most one retained structural and one retained motion message;
- a per-connection periodic state-send ceiling of 60 Hz;
- existing handshake, spectator, two-player, control, locomotion, and lifecycle WebSocket behavior.

This is a bounded JSON correctness transport. It is not interest management, a binary delta codec, regional subscription, congestion telemetry, or the final thousand-player transport.
