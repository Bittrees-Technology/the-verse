THE VERSE — DEVELOPMENT BUILD

This archive contains the native game client and the local authoritative server.
It is an unsigned development build for testing, not a production release and
not connected to a real-value economy.

macOS: open "The Verse.app", then click "Enter the Verse" when ready.
The app starts its bundled local server and uses a separate Capital Playtest save.
Close the app to stop its server. Esc returns to the entry menu.

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

ENGINEERING WORKSHOP

macOS: double-click "Launch Engineering Workshop.command".
Linux: run "./the-verse-engineering".

This starts beside the asteroid, salvage skiff, and powered industrial platform
in a separate persistent save. Close other Verse sessions before launching.

1 = mining drill; 2 = grinder; 3 = welder; 4 = short-range pulse tool.
Hold primary for work tools; click primary to fire a pulse at a block within 9 m.
I opens Inventory, Tools, and Production. Tools are permanent suit equipment,
not tradable cargo. B opens construction; 1-8 then choose block kinds.
B or right-click exits construction. Release primary after switching tools.

Mine ore. Transfer at least two ore from suit to industrial cargo in Inventory.
Queue refining in Production, then a component batch after the alloy appears.
Transfer the component from cargo back into your suit. Close inventory, press B,
select 1, aim at an owned block face, and hold primary to place and weld a frame.
The in-game work guide tracks authoritative completion through these steps.

The orbital start is vacuum: keep the helmet sealed. Survival and recovery are
still active; this is not an unlimited-oxygen sandbox. Pulse shots affect blocks,
not players. Ammunition, tradable tools, and long-range weapons are later work.
