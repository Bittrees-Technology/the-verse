# ADR-0021: Independent interest-view verification before client apply

**Status:** Implemented, packaged, and hosted verified for P1.5

## Context

ADR-0020 defines an audience-specific view hash and requires clients to apply a
delta only from the acknowledged frontier. The first P1.5 clients validate the
advertised frontier and hash shape, but they do not independently reconstruct
the complete resulting view or recompute the digest. Browser JSON numbers also
cannot preserve every protocol `u64` or distinguish an integer from a typed
floating-point value. Godot's generic JSON representation has the same trust
boundary problem.

A client that parses into a presentation object first can therefore echo a
server-advertised hash without proving that the bytes it applied match that
hash. It can also partially mutate presentation state before discovering a
late private-overlay or frontier error.

## Decision

### Apache verifier boundary

P1.5 introduces `verse-interest-verifier`, an Apache-2.0 SDK crate that depends
only on the Apache protocol crate and permissively licensed serialization and
hash dependencies. It has no dependency on the AGPL simulation or worker
crates. Its normative version-one encoding is published with the crate in
`SPECIFICATION.md` and frozen by portable golden vectors.

The authoritative server and verifier remain separate implementations during
parity evidence. They may share protocol types, the published specification,
and immutable test vectors, but not private simulation implementation code.

### Raw typed input

Official clients pass the original UTF-8 WebSocket text bytes to the verifier
before generic JavaScript or Godot parsing. The verifier rejects invalid UTF-8,
duplicate object keys, unknown fields, schema/type/range errors, non-finite
numbers, noncanonical entity and operation ordering, identity mismatches, and
configured byte, collection, or nesting bounds.

Typed parsing happens before canonicalization. In particular, exact `u64`
values never cross a JavaScript-number boundary and a protocol `f32` or `f64`
is canonicalized according to its declared type rather than its input spelling.

### Staged state machine

The verifier state proceeds through:

```text
await_welcome -> await_registry -> await_baseline -> current
```

The compatible welcome tuple and registry/manifest binding are immutable for
one connection. Protocol-15 complete snapshots and motion messages are rejected
on a protocol-16 connection. Receipts and fatal messages may be relayed only in
the phases allowed by the protocol; they never change verified view state.
Verifier construction also requires trusted expected universe, content,
celestial-registry, and universe-manifest commitments. A self-consistent
replacement registry cannot choose its own trust root. The SDK validates
schema-1 definition ID shape and body-kind structure; exact pinned content,
registry, and manifest roots remain the definition allowlist authority because
the Apache verifier does not import the AGPL content catalog.

For a baseline, the verifier stages the complete ordered entity set,
environment, conservation flag, optional actor-private state, and delivery
frontier. It validates duplicated outer projection arrays against the interest
entity set, reconstructs the version-one hash material, and compares the digest
without changing committed state.

For a delta, the verifier first requires the exact committed session epoch,
interest epoch, baseline ID, next contiguous sequence, and previous view hash.
It stages complete enters, absolute replacements, and removals over a clone of
the committed view. Omitted environment, conservation, and actor-private fields
retain their prior values. Actor-private motion replaces the corresponding
motion fields of the retained private player. It then hashes the complete
resulting view and compares the advertised result.

Only one staged state exists per verifier. The client applies the verifier's
typed, sanitized frame to a separate presentation candidate. Only after that
succeeds does it commit the verifier stage and install the presentation
candidate. Commit emits the exact serialized acknowledgement. The client never
constructs a verified acknowledgement from generic parsed numbers or a
server-advertised hash.

Any validation, hash, presentation-staging, timeout, or commit failure produces
no acknowledgement and cannot mutate the last committed view. A recoverable
frontier mismatch requests one bounded fresh baseline. A malformed frame,
digest mismatch, impossible commit, or unavailable verifier closes the state
stream. There is no unverified fallback. Reconnect resets every pending token
and requires a new welcome, registry, session epoch, and baseline.

### Client adapters

The browser uses the verifier through a same-origin Web Worker and WASM adapter
so hashing cannot block rendering. Worker and WASM initialization must succeed
before the state WebSocket opens.

The native client uses the same core through a thin AGPL GDExtension. The
extension accepts `PackedByteArray`, returns sanitized typed JSON for
presentation, and generates acknowledgement JSON. Missing or architecture-
incompatible native verifier libraries are a startup failure.

Both adapters expose the same logical operations:

```text
reset(expected_role)
stage_server_message(raw_utf8) -> stage_token + sanitized_frame
commit(stage_token) -> acknowledgement_json?
discard(stage_token)
```

Stage tokens are bounded, one-use, connection-local values. A verifier permits
at most one pending stage.

### Security and resource bounds

Hash verification is linear in the complete visible view even for a small
delta. Implementations bound raw message bytes, entity count, collection sizes,
string lengths, nesting, and one-frame verification time. They do not retain an
unbounded delta log or more than one pending result. Browser verification runs
off the rendering thread; a verifier crash or watchdog expiry reconnects
without acknowledgement.

The P1.5 default bounds one registry message to 512 bodies and 130,816
pairwise separation comparisons, checked before pair traversal. An unlimited
universe is expanded through a future versioned paging protocol, not by placing
an unbounded registry on a browser worker or Godot's main thread.

## Compatibility boundary

This decision preserves gameplay protocol `16`, projection schema `3`,
interest schema `1`, content schema `11`, and content manifest `p1.5.0`. It
freezes interest-view hash encoding version `1`; an encoding change requires a
new domain separator and coordinated protocol/schema negotiation.

## Migration and rollback

Upgrade drains existing protocol-16 sessions because they have not proved a
verified frontier. Updated clients initialize the verifier before connecting
and begin from a new baseline. The verifier persists no state across process or
page restart.

Rollback drains verified sessions and restores the earlier client build. It
does not reinterpret a verified frontier under another encoding. Production
deployment cannot claim P1.5 client convergence while the unverified build is
active.

## Required evidence

- Frozen positive vectors publish raw frames, canonical material bytes,
  expected BLAKE3 digest, and exact acknowledgement bytes.
- Invalid vectors cover duplicate and unknown keys, type/range faults, every
  frontier mismatch, ordering and identity faults, altered included fields,
  and malformed private motion.
- Values above JavaScript's safe-integer range and signed-zero/fixed-point
  boundaries verify identically in native and browser adapters.
- A rejected frame, failed presentation candidate, discarded token, reset, or
  verifier crash leaves committed state unchanged and emits no acknowledgement.
- Live server baselines and deltas verify through both packaged native clients
  and the browser while the mining, refining, manufacturing, inventory,
  construction, damage, death-drop, oxygen, and respawn scenario remains green.
- Deliberate in-flight payload and hash alteration fails closed.

## Current implementation evidence

The Apache verifier core, browser WASM Worker adapter, and native Godot
GDExtension now share the frozen version-one state machine and portable raw
corpus. Tests exercise exact signed and unsigned integer boundaries, malformed
typed input, registry/manifest/content-root substitution, the 512-body and
130,816-comparison bounds, stage/discard/commit behavior, presentation failure,
and missing-verifier startup failure. The native presentation keeps protocol
`u64` values beyond signed `i64` as canonical decimal text where only identity
and fingerprinting are required, rather than saturating or rounding them.

The live local suite verifies real baselines and deltas through both native
client identities, a direct browser adapter, and the shipped command-center
page in a real headless browser. A transparent test proxy alters one isolated
hash in flight and observes zero applied tampered state and zero tampered
acknowledgements. The complete mining, refining, manufacturing, transfer,
construction, damage, death, oxygen, respawn, two-player, and restart scenario
remains green. The Apple Silicon direct-download package runs the native
verifier and live client from the assembled archive. [Hosted CI run
33128613104](https://github.com/Bittrees-Technology/the-verse/actions/runs/33128613104)
passes those suites, the Linux container probe, and Linux/Apple Silicon
packages for implementation revision `71e955c`.

## Deliberate exclusions

This decision does not make a client authoritative, hide traffic patterns,
verify the global canonical world commitment, replace transport security, or
deliver the final binary codec. A matching client hash proves convergence only
for that session's authorized projected view.
