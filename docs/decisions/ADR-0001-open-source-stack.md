# ADR-0001: Open-source stack

**Status:** Accepted

## Context

The game/server must be open source and support macOS, Linux, dedicated servers, contributors, custom voxel code, and browser-adjacent experiences.

## Decision

Begin P0 with Godot for the native client and Rust for authoritative services and simulation experiments. Adopt Jolt-backed collision physics when the P0 physics spike begins. PostgreSQL, NATS JetStream, Redis, and S3-compatible storage remain the production service baseline but are not dependencies of the self-contained P0.1 local universe.

## Consequences

- The complete stack can be inspected and modified.
- Custom voxel, grid, and large-world work remains substantial.
- Client and server physics may require shared native libraries or explicit reconciliation.
- P0 benchmarks may replace individual components without changing product requirements.
- The local proof uses file-backed snapshots and a journal to keep testing reproducible before distributed infrastructure is introduced.
