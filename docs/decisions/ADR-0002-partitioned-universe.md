# ADR-0002: Partitioned universe

**Status:** Accepted

## Context

Thousands of participants, large fixed celestial distances, detailed physics, and persistent structures cannot execute in one simulation process.

## Decision

Expose one logical universe backed by dynamically scheduled simulation cells and shared canonical services.

## Consequences

- Cross-cell handoff and fencing are critical correctness systems.
- Busy regions can scale independently.
- Long travel can use analytical background simulation.
- Very large structures can use multiple cells.
