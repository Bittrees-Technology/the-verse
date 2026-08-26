# ADR-0006: Native/browser capability split

**Status:** Accepted

## Context

Detailed voxel physics and destruction are unsuitable as the minimum browser requirement, while users need broad device access.

## Decision

Provide full gameplay through native macOS/Linux clients. Provide browser identity, markets, management, maps, APIs, spectating, and optional cloud streaming.

## Consequences

- The full simulation can optimize for native hardware.
- Economic and organizational work remains broadly accessible.
- Browser and native clients share identity and public schemas.
