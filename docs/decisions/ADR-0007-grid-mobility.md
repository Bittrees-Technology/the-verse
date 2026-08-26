# ADR-0007: Grid mobility and voxel anchoring

**Status:** Accepted

## Context

Structures should remain movable without an arbitrary size rule while anchored bases need efficient large-scale simulation.

## Decision

A grid not anchored to voxel terrain remains dynamic. An approved voxel/foundation connection permits static or partitioned simulation. Removing the final anchor may return it to dynamic/capital-ship simulation.

## Consequences

- Mobility is a state derived from physical relationships.
- Very large free grids require a capital-ship partitioning model.
- State transitions require checkpointed ownership and collision rebuilding.
