<!-- SPDX-License-Identifier: Apache-2.0 -->

# Generated browser verifier

The JavaScript loader and WebAssembly module in this directory are the pinned,
platform-independent browser distribution of the Apache-2.0
`verse-interest-verifier` crate. They are generated with Rust `1.96.0` and
wasm-bindgen `0.2.127` through `tools/ci/build-browser-verifier.sh`. The
checked-in WebAssembly bytes are produced on `x86_64-unknown-linux-gnu`.
Rust/LLVM may assign a different function order while producing an equivalent
module on another host or runner. The build remaps every checkout root to
`/the-verse` and rejects ambient compiler flags so the runner's absolute
workspace path cannot change code-generation identity.

They are committed so a source checkout and packaged authoritative server can
serve the fail-closed browser command center without installing a Rust-to-WASM
toolchain at runtime. Continuous integration regenerates both files, rejects
exact drift in the platform-neutral JavaScript glue, and independently runs
the full portable-vector, invalid-frame, 64-bit-value, and tamper-recovery
suite against both the committed WebAssembly module and the freshly rebuilt
module. CI does not equate binary identity with semantic identity across
different Rust/LLVM runner environments.
