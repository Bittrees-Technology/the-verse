# Private player state projection

**Status:** P1.2 local proof implemented and verified

## Player promise

Watching the universe does not grant access to another pilot's inventory.
Knowing or guessing an inventory ID does not make it visible or usable. A
player receives their own suit inventory, cargo on grids they own, and their
protected death drops through a private session overlay. Another player and a
spectator receive none of those records.

All sessions still receive the same public universe: player and ship
positions, visible life state, grid owners, blocks, voxels, and canonical event
sequence. Privacy changes presentation, not canonical simulation or mutation
authority.

## Visibility matrix

| State | Spectator | Bound player |
| --- | --- | --- |
| Public world, voxel and grid topology | Visible | Visible |
| Public player pose and life state | Visible | Visible |
| Grid owner and block integrity | Visible | Visible |
| Exact self suit oxygen and controls | Hidden | Self only |
| Experience and career | Hidden | Self only |
| Carried inventory record and contents | None | Self only |
| Cargo inventory record and contents | None | Owned grids only |
| Protected death drop and contents | None | Self only |
| Cargo-inclusive grid mass | Hidden | Owned grids only |
| Economic conservation totals | Hidden | Hidden |
| Conservation validity | Visible | Visible |
| Intent receipt or rejection | None | Requesting connection only |

The generic transfer, refine, and craft authority rules remain unchanged. The
private overlay is knowledge, not a capability grant; the server re-resolves
ownership during preparation and replay.

## Client fail-closed behavior

The native client accepts a private overlay only when its player ID matches the
welcomed player and it is nested in the same full snapshot as the public
projection. It then resolves cargo by the private cargo domain's globally
unique block ID and the public completed block on an actor-owned grid.

The client immediately clears inventory data, selected cargo, protected drop
data, and industry controls when:

- the socket disconnects;
- a new handshake starts;
- a full snapshot omits the expected overlay;
- the actor does not match; or
- ownership linkage is missing or ambiguous.

Public motion frames do not modify inventory and may retain the last valid
overlay. Actor-private motion updates only the bound player's input
acknowledgements and cannot alter another player's state.

The browser command center is a spectator in P1.2. It shows that economic state
is private and exposes no production buttons or inferred zero balances.

## Acceptance evidence

- Unauthenticated `/api/v1/world` contains no inventory/drop record, inventory
  handle, cargo-inclusive mass, progression, suit oxygen, control frontier, or
  conservation total.
- Spoofed HTTP query, cookie, origin, authorization, and player headers do not
  upgrade the public projection.
- A spectator and two players share canonical event sequence and world hash,
  while each player overlay contains only that actor's carried, owned-cargo,
  and protected-drop records.
- Initial, requested, live, coalesced-refresh, reconnect, and post-restart paths
  apply the same projection.
- Inventory changes for one actor never appear in the other actor's or
  spectator's raw JSON.
- Missing or malformed authority links close fail-closed without a canonical
  snapshot fallback.
- Projection does not mutate canonical state or change its authoritative hash.

## Deliberate limits

The canonical hash, event timing, and physical acceleration can still reveal
that state changed. Private projection does not hide public ownership, combat,
ship motion, or future publicly salvageable drops. It also does not turn the
loopback development identity into production authentication.

P1.2 does not yet implement the accepted 15-minute owner/team death-drop
protection timer, six-hour expiry, team/company access, market custody,
cockpit/signal authority, or capital safe-zone policy.
