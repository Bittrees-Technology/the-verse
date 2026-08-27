# P1.5 third-party dependency record

**Updated:** 2026-08-27

This record covers the direct dependencies and build tools used through P1.5. Exact Rust package versions and transitive packages are locked in `Cargo.lock`; each package's registry metadata remains the canonical license declaration.

## Runtime and build tools

| Component | Version | License | Source |
| --- | --- | --- | --- |
| Godot Engine | 4.7.2 | MIT | Official Godot release archive; bundled notice in `Godot-MIT.txt` |
| Rust toolchain | 1.96.0 | Apache-2.0 and MIT components | Rust project distribution |
| Node.js | 24.8.0 in CI; 22 or newer locally | MIT and bundled third-party notices | Node.js distribution |
| CMake | 3.16 or newer | BSD-3-Clause | CMake project distribution |
| Clang and libclang | 17 or compatible | Apache-2.0 WITH LLVM-exception | LLVM project distribution |
| GitHub checkout action | v7.0.1 | MIT | `actions/checkout` |
| GitHub cache action | v6.1.0 | MIT | `actions/cache` |
| GitHub artifact action | v7.0.1 | MIT | `actions/upload-artifact` |
| GitHub setup-node action | v6.0.0 | MIT | `actions/setup-node` |
| markdownlint-cli2 | 0.23.2 | MIT | Locked build-time documentation linter; not shipped |
| wasm-bindgen CLI | 0.2.127 | MIT OR Apache-2.0 | Browser-verifier binding generator; build-time only |

The macOS and Linux CI scripts pin and verify Godot release archive checksums. The repository contains the required Godot license notice but no copied Godot source or third-party art assets.

## Direct Rust dependencies

| Crate | License declared by package |
| --- | --- |
| anyhow | MIT OR Apache-2.0 |
| axum | MIT |
| blake3 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| clap | MIT OR Apache-2.0 |
| futures-util | MIT OR Apache-2.0 |
| fs2 | MIT OR Apache-2.0 |
| godot and godot-rust support crates 0.5.4 | MPL-2.0 |
| http | MIT OR Apache-2.0 |
| joltc-sys 0.3.1+Jolt-5.0.0 package metadata, git revision below | MIT OR Apache-2.0 |
| parking_lot | MIT OR Apache-2.0 |
| proptest | MIT OR Apache-2.0 |
| rolt 0.3.1+Jolt-5.0.0 package metadata, git revision below | MIT OR Apache-2.0 |
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
| wasm-bindgen 0.2.127 | MIT OR Apache-2.0 |

## Native physics sources

| Component | Version | License | Source |
| --- | --- | --- | --- |
| `rolt` | git `72ac0cb1acc2037c72dc29865da6f52a5483dadc` | MIT OR Apache-2.0 | [SecondHalfGames/jolt-rust](https://github.com/SecondHalfGames/jolt-rust) |
| `joltc-sys` | git `72ac0cb1acc2037c72dc29865da6f52a5483dadc` | MIT OR Apache-2.0 | [SecondHalfGames/jolt-rust](https://github.com/SecondHalfGames/jolt-rust) |
| JoltC | `2982004387a9e36ca89525a87d983709d3666da7` | MIT OR Apache-2.0 | [SecondHalfGames/JoltC](https://github.com/SecondHalfGames/JoltC) |
| Jolt Physics | 5.3 source at `0373ec0dd762e4bc2f6acdb08371ee84fa23c6db` | MIT | [jrouwe/JoltPhysics](https://github.com/jrouwe/JoltPhysics) |

The Rust packages retain upstream's older `0.3.1+Jolt-5.0.0` metadata, but the
repository pins the immutable git revision shown above. The embedded submodule
commits, rather than that package label, identify the native source actually
compiled by this checkpoint.

No JavaScript package is shipped in the browser command center. The end-to-end
runner uses the WebSocket implementation bundled with Node.js. Documentation
lint dependencies are build-time-only and locked with exact registry integrity
metadata in `tools/markdownlint/package-lock.json`.

The committed browser verifier loader and WebAssembly module are generated
from the Apache-2.0 verifier plus wasm-bindgen 0.2.127. The native verifier
links godot-rust 0.5.4; its MPL-2.0 license text is distributed as
[`MPL-2.0.txt`](MPL-2.0.txt), and the corresponding source is identified by
`Cargo.lock` and this repository's native-adapter manifest.
