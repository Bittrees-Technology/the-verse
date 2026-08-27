<!-- SPDX-License-Identifier: Apache-2.0 -->

# Generated browser verifier

The JavaScript loader and WebAssembly module in this directory are the pinned,
platform-independent browser distribution of the Apache-2.0
`verse-interest-verifier` crate. They are generated with Rust `1.96.0` and
wasm-bindgen `0.2.127` through `tools/ci/build-browser-verifier.sh`. The
canonical checked-in WebAssembly bytes are produced on
`x86_64-unknown-linux-gnu`; Rust/LLVM may assign a different function order
when producing an equivalent module on another host architecture.

They are committed so a source checkout and packaged authoritative server can
serve the fail-closed browser command center without installing a Rust-to-WASM
toolchain at runtime. Continuous integration regenerates both files on the
canonical host and rejects byte drift. Other hosts still rebuild the module,
compare the platform-neutral JavaScript glue, and run the committed module's
functional and tamper tests, but do not claim cross-host byte identity.
