THE VERSE — DEVELOPMENT BUILD

This archive contains the native game client and the local authoritative server.
It is an unsigned development build for testing, not a production release and
not connected to a real-value economy.

macOS: double-click "Launch The Verse.command". Gatekeeper may require you to
approve the unsigned development build in System Settings.

Linux: run "./the-verse" from a terminal. The initial target is x86_64 Ubuntu.

The launcher starts a server bound only to 127.0.0.1:7777, stores the universe
in the platform user-data directory, then connects the native client. The local
browser command center is available at http://127.0.0.1:7777 while playing.

No wallet, token, marketplace, or blockchain transaction is active in this
development package. See VERSION.txt for the exact source and protocol versions.
