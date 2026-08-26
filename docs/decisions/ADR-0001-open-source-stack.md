# ADR-0001: Open-source stack

**Status:** Proposed

## Context

The game/server must be open source and support macOS, Linux, dedicated servers, contributors, custom voxel code, and browser-adjacent experiences.

## Decision

Begin P0 with Godot/Jolt for the native client, Rust for authoritative services and simulation experiments, PostgreSQL, NATS JetStream, Redis, and S3-compatible storage.

## Consequences

- The complete stack can be inspected and modified.
- Custom voxel, grid, and large-world work remains substantial.
- Client and server physics may require shared native libraries or explicit reconciliation.
- P0 benchmarks may replace individual components without changing product requirements.
