# P0 third-party dependency record

**Updated:** 2026-08-26

This record covers the direct dependencies and build tools used by P0. Exact Rust package versions and transitive packages are locked in `Cargo.lock`; each package's registry metadata remains the canonical license declaration.

## Runtime and build tools

| Component | Version | License | Source |
| --- | --- | --- | --- |
| Godot Engine | 4.7.2 | MIT | Official Godot release archive |
| Rust toolchain | 1.96.0 | Apache-2.0 and MIT components | Rust project distribution |
| Node.js | 25 or compatible | MIT and bundled third-party notices | Node.js distribution |
| GitHub checkout action | v4 | MIT | `actions/checkout` |

The macOS and Linux CI scripts pin and verify Godot release archive checksums. The repository contains no copied Godot source or third-party art assets.

## Direct Rust dependencies

| Crate | License declared by package |
| --- | --- |
| anyhow | MIT OR Apache-2.0 |
| axum | MIT |
| blake3 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| clap | MIT OR Apache-2.0 |
| futures-util | MIT OR Apache-2.0 |
| fs2 | MIT OR Apache-2.0 |
| http | MIT OR Apache-2.0 |
| parking_lot | MIT OR Apache-2.0 |
| proptest | MIT OR Apache-2.0 |
| serde | MIT OR Apache-2.0 |
| serde_json | MIT OR Apache-2.0 |
| tempfile | MIT OR Apache-2.0 |
| thiserror | MIT OR Apache-2.0 |
| tokio | MIT |
| tower | MIT |
| tower-http | MIT |
| tracing | MIT |
| tracing-subscriber | MIT |
| uuid | Apache-2.0 OR MIT |

No JavaScript package is shipped in the browser command center. The end-to-end runner uses the WebSocket implementation bundled with Node.js.
