# ADR-0025: Signed universe activation and authoritative boot

**Status:** Accepted

**Requirement linkage:** F-061, WORLD-010, SIM-016, SIM-017, SIM-018

## Context

The protocol-19 migration bridge can freeze an exact protocol-18 universe,
derive a canonical migration receipt, and install the complete dormant
world-21 target. The prepared-install head proves that all target artifacts
belong to one migration, but deliberately grants no authority to run them.
Starting a worker from a per-cell file, scanning for a plausible target, or
falling back to protocol 18 after a partial activation could create two live
histories from one conserved world.

Activation is an operational safety decision rather than an in-world market or
governance transaction. It must be independently authorized, deterministic,
recoverable after process failure, and verifiable without a network service.

## Decision

### Externally anchored 2-of-3 policy

Protocol-19 activation uses Ed25519 strict verification under a canonical
policy with three distinct public signers and threshold two. Each canonical
authorization envelope contains exactly two valid signatures; surplus
signatures are rejected so an accepted envelope cannot be resealed by removing
one valid signature. The policy is not
trusted because it is stored beside the world. A worker or activation tool is
given the expected policy hash through its separately managed configuration
and accepts canonical policy bytes only when their self-hash equals that
expected value.

No production private key, seed, recovery phrase, or signing service belongs
in this repository or the universe directory. Test keys are deterministic and
compiled only in tests. A future Safe or on-chain attestation may authorize a
policy update, but it does not change the local activation signature domain in
place.

Signer IDs are BLAKE3 commitments to the domain and raw 32-byte Ed25519 public
key. Signers and detached signatures are strictly sorted by signer ID and may
not repeat. Public keys, signatures, hashes, and nonces use fixed-width
lowercase hexadecimal encoding. Unknown fields, noncanonical JSON, duplicate
signers, unknown signers, invalid signatures, and a signature count below the
threshold fail closed.

### Fully bound authorization

The signed payload binds the authorization kind and signature scheme; the
complete protocol-19 compatibility tuple; universe ID and seed; exact
prepared-install head, migration receipt and migration anchor; manifest,
directory, cell-set, conservation, gameplay, identity, and production roots;
cell count; policy hash and generation; activation generation; a unique nonce;
a signer-chosen authorized activation timestamp; an inclusive not-before time;
an exclusive expiry; and the previous active head hash.

Signatures cover the exact canonical JSON payload prefixed by
`the-verse/protocol-19-world-activation-authorization/v1` and a terminating
zero byte. The activation process samples trusted time once while all relevant
locks are held. The migration cut-off must not be in the future, and the sample
must satisfy `not_before <= now < expires_at`. Authorization validity is
bounded by policy. The cooperative activation tool rejects a new attempt after
expiry; expiry does not deactivate an already committed universe.

The authorized activation timestamp is signed and must fall inside the same
window. The active head copies that signed value; a worker cannot invent or
edit it and reseal an equivalent head. This is durable evidence of the time
the signers authorized, not proof that an ordinary local filesystem first
persisted the selector at that wall-clock instant. Without an external
append-only timestamp or one-use nonce anchor, an unused authorization copied
with its exact prepared world remains replayable after expiry by a host that
can bypass the activation tool or roll back its clock. Therefore the validity
window is a cooperative activation-tool control, while the signatures are a
permanent authorization of only the exact bound world. Operators must destroy
unused signed envelopes and protect staged world copies. A future externally
anchored activation generation can remove this residual replay assumption.

The first activation has generation one and an empty previous-head hash. Later
generations must be a separately specified, signed policy rotation or forward
migration. Replaying the same authorization for different prepared material,
generation, policy, or history is impossible because those values are signed.

### One global commit point

Activation holds the frozen source, prepared-install, directory-v3, and every
world-21 cell writer lock while it verifies authorization. It persists the
canonical signed authorization and content-addressed active-head history in an
isolated activation namespace. It then atomically replaces and synchronizes
the universe-root `active-protocol-head-v1.json` last. That global head is the
only activation linearization point.

Before the global head exists, known activation staging files and their
temporary files grant no authority and may be removed under the activation
lock. The prepared world and legacy source remain byte-identical. Once the
global head exists, recovery is forward-only: every referenced authorization,
receipt, prepared head, manifest, directory, and cell must validate exactly.
Missing, extra, swapped, malformed, hybrid, or altered material is an incident;
startup never repairs it and never falls back to protocol 18.

### Authoritative runtime boot

An updated process checks the universe-root active head before and after
acquiring legacy writer authority. A canonical protocol-19 head permanently
fences protocol-18 startup for that universe. Truly older executables that do
not know this marker must be stopped operationally before activation.

A protocol-19 boot starts with the global active head. It derives the prepared
head, receipt, manifest, directory route, and ordered cell set exclusively from
that signed root; it never scans namespaces or accepts command-line cell roots
as authority. The returned in-memory capability owns the install, activation,
directory, and cell locks and exposes only verified summary data until the
protocol-19 scheduler and gameplay runtime consume it.

The worker may run this verified boot as a fail-closed readiness gate before
session admission. It must not serve protocol-18 messages over a protocol-19
world, reinterpret event-16 as event-17, or claim interactive protocol-19
gameplay before the event-17 runtime, projection, verifier, and client tuple is
complete.

### Rollback

Before the global-head rename, a failed activation removes only the known
activation staging material and can retry against the same prepared world.
After the rename, process recovery completes verification of the committed
head. Binary rollback is allowed only to a protocol-19-compatible binary that
reads and verifies that same head.

Returning to protocol 18 is not a pointer edit. It requires a separately
authorized reverse migration that proves no event-17 work can be lost. No such
reverse migration is implemented by this checkpoint.

## Consequences

- One signed, content-addressed root selects the authoritative universe.
- Authorization can be audited and verified offline without blockchain or
  signer availability during normal restart.
- A stale or compromised single signer cannot activate a world.
- Operators must protect the separately configured policy hash and complete a
  deliberate signing ceremony, and must treat an unused signed envelope plus
  its exact prepared world as a durable activation capability.
- Interactive protocol-19 service remains gated on its lifecycle, scheduler,
  event, projection, and client adapters.

## Validation

- Canonical policy, payload, envelope, signer ID, and active-head vectors.
- Exactly two distinct valid signatures pass; one, three, duplicates, unknown
  signers, and altered signatures fail.
- Mutating every prepared-world or policy binding fails verification.
- Not-before, exclusive-expiry, bounded-validity, cut-off, generation, and
  replay boundaries are controlled-clock tested.
- Failure at every persistence boundary either leaves no active head or leaves
  one exact recoverable active head.
- Tampered committed material fails without cleanup, rewrite, or fallback.
- Restart opens only the receipt/head-selected directory and cells, including
  after legacy source archives are removed from the runtime copy.
- Updated protocol-18 startup rejects an active protocol-19 selector both
  before and after acquiring its legacy lock.
- Worker readiness is reached only while the verified activated-world
  capability and all of its locks remain alive.

## Non-goals

This decision does not add directory-v3 lease transitions, lifecycle-v2
runtime records, event-17 ordinary gameplay, production wake-up, projection-5
messages, interest-3 verification, client cutover, Safe signature validation,
key custody, or a reverse migration.
