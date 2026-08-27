# Private player state projection

**Status:** P1.5 local interest/private composition and independent
official-client hash verification implemented and verified

## Player promise

Watching the universe does not grant access to another pilot's inventory.
Knowing or guessing an inventory ID does not make it visible or usable. A
player receives their own suit inventory, cargo on grids they own, and their
protected death drops through a private session overlay. Another player and a
spectator receive none of those records.

Protocol-16 sessions receive an authorized public interest view of player and
ship positions, visible life state, grid owners, blocks, voxels, and canonical
frontiers. The bound player receives a separately derived private overlay tied
to that same view revision. Privacy and interest change presentation, not
canonical simulation or mutation authority.

## Visibility matrix

| State | Spectator | Bound player |
| --- | --- | --- |
| Public world, voxel and grid topology | Authorized observer view | Actor interest view |
| Public player pose and life state | Authorized observer view | Actor interest view |
| Grid owner and block integrity | Authorized observer view | Actor interest view |
| Exact self suit oxygen and controls | Hidden | Self only |
| Experience and career | Hidden | Self only |
| Carried inventory record and contents | None | Self only |
| Cargo inventory record and contents | None | Owned grids in authorized active-cell view |
| Protected death drop and contents | None | Self only |
| Cargo-inclusive grid mass | Hidden | Owned grids in authorized active-cell view |
| Production queue, progress, endpoints, and escrow | Hidden | Owned machines in authorized active-cell view |
| Economic conservation totals | Hidden | Hidden |
| Conservation validity | Visible | Visible |
| Intent receipt or rejection | None | Requesting connection only |

The generic transfer and physical queue-production authority rules remain
unchanged. The private overlay is knowledge, not a capability grant; the server
re-resolves ownership during preparation and replay.

## P1.5 interest composition

Interest and privacy are independent server-side decisions. The worker first
derives the public entity and voxel-chunk view from the immutable session role:
the canonical bound-player position for gameplay or a server-approved public
observer for a spectator. It then resolves the actor-private overlay from the
same immutable binding. A public entity entering the view can never attach that
entity's owner's private fields.

The bound player's exact player record, carried inventory, movement and
operation frontiers, life state, and command reconciliation remain
control-critical. Detailed owned cargo, cargo-inclusive mass, and production
queues are included only when their public grid or machine is also present in
the authorized active-cell view. This prevents an orphaned private machine or
inventory handle from becoming an unlimited-distance visual or terminal path.
Future browser asset summaries and signal-authorized remote management require
separate specifications.

Every private overlay is bound to the same `session_epoch`, `interest_epoch`,
canonical event/tick frontier, global commitment, and view hash as its public
baseline or delta. A reconnect, interest reset, or incompatible registry/view
schema clears the old overlay before a new one is installed. Public motion for
an already entered entity may preserve a valid private baseline, but an
out-of-interest leave clears private state whose authority link depended on
that public object.

Absence from an interest view is not a privacy claim or proof of destruction.
The client acts only on explicit `out_of_interest`, `destroyed`, or
`transferred` leave reasons and never exposes or infers a hidden object's last
exact state after its authorized view ends.

## Client fail-closed behavior

The native client accepts a private overlay only after the shared verifier has
validated its player ID against the welcomed player and reconstructed the same
P1.5 interest baseline/delta as the public projection. It then stages the
sanitized presentation candidate and resolves cargo by the private cargo
domain's globally unique block ID and the public completed block on an
actor-owned grid. Presentation commit and the verifier-owned acknowledgement
are one ordered fail-closed boundary.

The client immediately clears inventory data, selected cargo, protected drop
data, and industry controls when:

- the socket disconnects;
- a new handshake starts;
- a full snapshot omits the expected overlay;
- a P1.5 baseline or delta omits its required overlay;
- the actor does not match;
- ownership linkage is missing or ambiguous;
- the registry, session epoch, interest epoch, or view hash is incompatible; or
- a public grid or machine leaves the view and its private record depends on
  that public authority link.

Public motion frames do not modify inventory and may retain the last valid
overlay. Actor-private motion updates only the bound player's input
acknowledgements and cannot alter another player's state.

The browser command center remains a spectator through P1.4. It shows that
economic state is private and exposes no production buttons or inferred zero
balances.

In P1.5 the browser receives a fixed-body registry plus one bounded public
observer view. Query parameters, cookies, headers, client names, and map clicks
cannot select a player, increase the radius, attach an actor-private overlay, or
wake an arbitrary cell. Out-of-interest entities are removed from browser state
and the document; stale markers cannot retain private or actionable data. A
same-origin Worker runs the shared verifier before the page receives sanitized
state. Worker/WASM failure, a trust-root mismatch, or a hash alteration closes
the stream with no unverified fallback and no acknowledgement.

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
- Different actor and spectator interest views share the canonical commitment
  while carrying distinct deterministic view revisions.
- Interest enter, leave, re-entry, reset, and reconnect never attach a foreign
  inventory, queue, mass, operation frontier, or protected drop.
- A public machine entering view exposes only coarse public operating state;
  another actor and a spectator receive no recipe, endpoint, queue, progress,
  escrow, or quantity field.
- A grid leaving view clears dependent private cargo, mass, and queue records
  without clearing the bound player's control-critical carried state.

## Deliberate limits

The canonical hash, event timing, and physical acceleration can still reveal
that state changed. Private projection does not hide public ownership, combat,
ship motion, or future publicly salvageable drops. It also does not turn the
loopback development identity into production authentication.

P1.5 interest management does not provide traffic-analysis secrecy, radar,
stealth, team visibility, global live tracking, or signal authority. It also
does not implement the accepted 15-minute owner/team death-drop protection
timer, six-hour expiry, team/company access, market custody, cockpit/signal
authority, or capital safe-zone policy.
