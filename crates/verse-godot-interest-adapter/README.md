# Godot interest verifier adapter

This AGPL GDExtension is the native-client boundary for the Apache-licensed
`verse-interest-verifier` core. It accepts original WebSocket
`PackedByteArray` frames before Godot JSON parsing, exposes only the core's
sanitized frame, and returns acknowledgements produced by a committed core
stage.

Before the sanitized frame crosses Godot's JSON parser, integer tokens outside
JavaScript's exact range are encoded as reserved lossless decimal strings.
The native client converts them to exact signed 64-bit values before model
staging. Values outside that native arithmetic range are discarded before the
verifier stage can commit or acknowledge; reserved-prefix protocol strings are
escaped and restored without reinterpretation.

The crate denies unsafe code. Godot's extension-registration ABI requires the
single `unsafe impl ExtensionLibrary` at the end of `src/lib.rs`; that item has
a narrow lint allowance and contains no executable unsafe block.

Generated native libraries are copied to `apps/native-client/bin/` by
`tools/ci/build-native-verifier.sh` and are not source-controlled.
