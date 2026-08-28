THE VERSE — DEVELOPMENT BUILD

This archive contains the native game client and the local authoritative server.
It is an unsigned development build for testing, not a production release and
not connected to a real-value economy.

macOS: double-click "Launch The Verse.command". Gatekeeper may require you to
approve the unsigned development build in System Settings.

Linux: run "./the-verse" from a terminal. The initial target is x86_64 Ubuntu.

The launcher starts a server bound only to 127.0.0.1:7777, creates a fresh
Earthlike surface playtest in its own platform user-data directory, then
connects the native client. Existing orbital saves are not changed. The local
browser command center is available at http://127.0.0.1:7777 while playing.
Both clients independently verify each authorized interest view before applying
it or acknowledging it; verifier failure closes the stream without a fallback.

Corresponding source for this development build and its MPL-covered godot-rust
dependency is identified by VERSION.txt and Cargo.lock and is available from
https://github.com/Bittrees-Technology/the-verse.

No wallet, token, marketplace, or blockchain transaction is active in this
development package. See VERSION.txt for the exact source and protocol versions.
