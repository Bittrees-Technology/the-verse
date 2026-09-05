# ADR-0029: Capital start and packaged worker supervision

- Status: Accepted
- Date: 2026-09-05
- Requirements: F-071, UX-006

The native development app owns a bundled loopback worker only when no explicit
server URL is supplied. Explicit server connections remain externally managed.
Show a blocking connection/entry panel while verified gameplay is unavailable.
An owned worker exits after supervisor failure, releasing its process locks.
The app can start a fresh process against the same journal with bounded retries;
it cannot override lease fencing or repair corrupt state.

Add an opt-in event-zero capital profile, identified by a durable arrival-floor
block. It uses existing grids, voxel grades and ledger accounting. Derive its
spawn/recovery corridor from the marker and planet geometry. Ordinary profiles
keep their existing policy. No event, content or network schema changes occur.

The capital fixture is a local first-session implementation, not public user
admission. Only the active starting planet receives surface resource outcrops.
Preserve older worlds and expose the new start in a separate save directory.
