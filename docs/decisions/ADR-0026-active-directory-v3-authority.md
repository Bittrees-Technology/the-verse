# ADR-0026: Active directory-v3 cell authority transitions

**Status:** Accepted

## Context

ADR-0025 makes one signed global head the exclusive protocol-19 boot root, but
its first checkpoint treats the selected directory-v3 genesis as immutable.
Lifecycle-v2 scheduling and event-17 gameplay require the activated universe to
advance cell ownership without accepting a substituted genesis, reviving a
stale writer, or confusing two different transition requests after an
uncertain commit.

## Decision

The signed active head permanently anchors the exact first directory-v3
history record. Opening the activated directory acquires its exclusive writer
lock, validates the durable head identity and byte-for-byte genesis prefix,
and only then performs journal recovery. Recovery may adopt a complete valid
successor record or truncate an unterminated suffix outside the pinned head; it
must not modify artifacts before the signed prefix is proved. After that proof,
recovery may remove a bounded set of regular head-temporary files only when
every directory entry is either a canonical authority artifact or has the
exact process-ID and UUID temporary-name shape. Any unknown artifact prevents
all cleanup.

Verified boot first acquires and read-only validates the prepared-install
capability, then acquires the activation lock and verifies the stable selector,
authorization signatures, active-head history, binding set, and exact
activation file set. Only after those checks pass may it consume the prepared
capability to open any recovery-capable directory or cell store.

The recovered hash chain, rather than the signed genesis revision, is the live
directory tip. The activated-world capability retains the directory writer and
ordered cell writers for its lifetime. Its authority mutators remain
crate-private until lifecycle-v2 can coordinate cell state with them.

Directory-v3 supports three cell-authority transitions:

1. Claim changes the exact current sleeping generation with no holder into an
   assigned successor. The directory derives generation `N + 1` and fence
   `F + 1`.
2. Recovery replaces the holder of the exact current assigned generation. It
   also derives generation `N + 1` and fence `F + 1`, permanently fencing the
   predecessor.
3. Release changes the exact current assigned generation and holder to
   sleeping without changing generation or fence. Release is forbidden while
   a nonterminal transfer names the cell.

Callers supply only the expected generation and stable holder identity. They
cannot select the resulting generation, fence, revision, document hash, or
history hash. Every mutation is one validated directory document
compare-and-swap followed by the existing journal/head atomic commit.

An exact redelivery is a no-op only when history proves both the requested
predecessor state and the resulting authority. This distinguishes a sleeping
claim from an assigned recovery even if expected generation and resulting
holder are otherwise identical. Stale generations, stale holders, transition
kind aliases, overflow, and pinned releases fail closed.

## Consequences

- Activated restart accepts valid directory successors while continuing to
  reject a foreign but internally consistent genesis.
- Crash recovery exposes only the prior authority or its exact successor.
- Assignment generations and fences remain monotonic and directory-issued.
- Lifecycle state is not yet advanced by these transitions; no worker or
  public gameplay entry point may invoke them in this checkpoint.

## Validation

- Signed-genesis successor history reopens through the activated-world path.
- Every journal/head failpoint around a claim recovers to the prior state or
  the exact successor, and retry commits exactly once; the same matrix covers
  recovery and release.
- A persisted synced head temporary plus a complete successor journal record
  recovers exactly, while an unknown neighbor prevents any cleanup.
- Claim and recovery retries cannot alias each other.
- Stale holders and generations cannot release or replace current authority.
- A nonterminal transfer pins release without advancing the directory.
- A self-consistent foreign genesis is rejected before recovery changes its
  history or head bytes.
- Missing or tampered activation material rejects without repairing an
  otherwise recoverable directory suffix.

## Non-goals

This decision does not define the mutable lifecycle-v2 record, scheduling or
wake policy, event-17 ordinary gameplay append, projection-5, interest-3,
client cutover, multi-host consensus, or distributed lease service.
