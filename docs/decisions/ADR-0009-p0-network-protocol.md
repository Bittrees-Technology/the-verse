# ADR-0009: P0 network protocol

**Status:** Accepted

## Context

P0 needs one understandable protocol shared by the native client, browser client, automated scenario, and authoritative server. The proof favors observability and deterministic verification over production bandwidth efficiency.

## Decision

Use a versioned, tagged JSON protocol over WebSocket for P0. Every mutating client intent carries a caller-generated operation ID. The server returns a durable receipt and publishes a complete authoritative snapshot after accepted operations and simulation ticks.

The server may announce its protocol version in `welcome`, but it does not disclose world state, subscribe the connection to updates, or accept any intent until the client sends one compatible `hello`. A missing or incompatible handshake receives a fatal response and the connection closes.

The server rejects unknown or invalid messages, incompatible protocol versions, non-finite motion, movement outside the allowed step, mining outside tool range, and client-supplied outcomes. Content-manifest versions are included in snapshots and canonical event hashes.

HTTP provides readiness, status, and a public read-only world snapshot. The same server hosts the zero-build browser command center.

## Consequences

- Packet captures, tests, and hand-authored clients are easy to inspect.
- Operation retries are safe because the server persists receipts by operation ID.
- Full snapshots avoid delta-ordering ambiguity during the proof and make reconnect simple.
- The current format is too bandwidth-heavy for thousands of players or large worlds.
- P1 must introduce binary spatial deltas, interest management, authentication, backpressure budgets, and compatibility negotiation without weakening server authority.
