# Canonical product requirements

**Status:** Accepted baseline with individually marked blockers

Requirement IDs are stable references for issues, pull requests, tests, and releases.

## Platform and access

- **PLAT-001 — Native macOS:** The first playable client shall support Apple Silicon macOS.
- **PLAT-002 — Native Linux:** The project shall support direct-download Linux clients, initially targeting Ubuntu 26.04 LTS-compatible systems.
- **PLAT-003 — Linux servers:** Authoritative services shall run headlessly on Linux.
- **PLAT-004 — Browser control:** A browser application shall support identity, markets, inventory, production, organizations, contracts, maps, and spectating.
- **PLAT-005 — Cloud streaming:** The architecture shall permit optional browser cloud streaming without making it a core simulation dependency.
- **PLAT-006 — Direct download:** macOS and Linux distribution shall not require a third-party storefront.
- **PLAT-007 — Public interfaces:** Versioned APIs and SDKs shall permit approved third-party and Web3 applications.

## Identity and agents

- **ID-001 — Passkeys:** User identity shall use WebAuthn passkeys with email-assisted onboarding and recovery.
- **ID-002 — Hidden wallet complexity:** Routine actions shall not require seed phrases, manual gas selection, or repeated wallet prompts.
- **ID-003 — Smart accounts:** Blockchain-capable profiles shall use scoped smart accounts and revocable session authority.
- **ID-004 — Open viewing:** Public universe and market information shall be viewable without a gameplay account, subject to abuse controls.
- **ID-005 — Agent neutrality:** Human players, bots, NPCs, and AI agents are permitted participants.
- **ID-006 — Equal authority:** Non-human agents shall use the same intent-validation and conservation rules as humans.

## Universe

- **WORLD-001 — Public universe:** Official servers shall expose one shared logical universe and canonical economy.
- **WORLD-002 — Fixed bodies:** Planets, moons, and asteroid fields shall remain at fixed universe coordinates.
- **WORLD-003 — Wide separation:** Generated planets shall satisfy a configurable minimum separation.
- **WORLD-004 — Asteroid groups:** Asteroids may be generated in belts, clusters, fields, or isolated sites.
- **WORLD-005 — Expanding frontier:** New deterministic sectors and resource regions shall be introduced without a practical reachable boundary.
- **WORLD-006 — Long travel:** Interplanetary travel without jump technology shall take real-world days.
- **WORLD-007 — Offline travel:** Autopilot travel shall continue while a user is offline and remain observable through browser interfaces.
- **WORLD-008 — Partitioned execution:** One logical universe shall be simulated by many dynamically managed cells.

## Voxels and grids

- **SIM-001 — Voxel editing:** Mining, impacts, and approved tools shall persistently modify voxel terrain.
- **SIM-002 — Server authority:** The authoritative server shall validate all voxel, physics, inventory, damage, and production changes.
- **SIM-003 — Movable grids:** A constructed grid shall remain movable while it is not anchored to voxel terrain or an explicitly static foundation.
- **SIM-004 — Static transition:** Voxel anchoring shall permit a grid to transition to a static or partitioned structure.
- **SIM-005 — No arbitrary design cap:** Product rules shall not impose a simple maximum structure size.
- **SIM-006 — Finite execution budgets:** Simulation may partition, sleep, or reduce update frequency to operate within finite resources.
- **SIM-007 — Destruction:** Damage shall support block loss, grid separation, cargo release, debris, salvage, and voxel cratering.
- **SIM-008 — Power:** Machines, defenses, respawn systems, control cores, and cleanup exemptions shall depend on authoritative power or contract state.
- **SIM-009 — Character motion authority:** Gameplay clients shall submit bounded character controls only. The authoritative cell shall own character position, orientation, velocity, gravity response, collision, and grounded state.

## Persistence, death, and cleanup

- **LIFE-001 — Offline presence:** Ships and structures remain attackable while owners are offline.
- **LIFE-002 — Powered defense:** Offline turrets and defenses operate only with sufficient power, ammunition, sensors, permissions, and intact control.
- **LIFE-003 — Free respawn:** Character death shall impose no BIT or experience charge.
- **LIFE-004 — Spawn priority:** Respawn shall prefer permitted powered personal, company, or allied facilities.
- **LIFE-005 — Capital fallback:** A player without a valid spawn may respawn at the capital with an empty starter state.
- **LIFE-006 — Inventory drop:** Carried inventory shall drop at the death location.
- **LIFE-007 — Drop expiry:** Dropped carried inventory shall be removed six hours after death if not recovered.
- **LIFE-008 — Derelict deadline:** An eligible object continuously without qualifying power shall be deleted at 36 hours.
- **LIFE-009 — Salvage:** An eligible unpowered object shall become publicly salvageable at 24 hours and remain salvageable until its 36-hour deletion.
- **LIFE-010 — Registered exception:** A claimed station may purchase a BIT-denominated registration or insurance contract that exempts it from ordinary unpowered cleanup while valid.
- **LIFE-011 — No raid immunity:** Registration shall not prevent attack, destruction, capture, or salvage through ordinary gameplay.
- **LIFE-012 — Outage safety:** Verified service outages shall not advance destructive cleanup timers.
- **LIFE-013 — Drop recovery grace:** A death drop shall be accessible only to its owner or team for the first 15 minutes, then publicly salvageable until its six-hour expiry.

## Capital, combat, and administration

- **CAP-001 — Absolute safe zone:** The capital protected zone shall prohibit all damage, weapon discharge, destructive collision effects, and theft.
- **CAP-002 — Restricted construction:** Ordinary construction in the capital shall require explicit permission.
- **CAP-003 — Creative administration:** Authorized administrators may build in creative mode.
- **CAP-004 — Non-economic admin assets:** Creative assets shall never enter canonical markets, inventories, salvage, or production.
- **PVP-001 — PvP:** PvP is a core official-universe activity outside protected zones.
- **PVE-001 — PvE:** NPC factions, hazards, missions, derelicts, and hostile encounters are core activities.

## Industry and contracts

- **IND-001 — Transformation chain:** The canonical loop shall include extraction, hauling, refining, manufacturing, assembly, construction, use, repair, recycling, and destruction.
- **IND-002 — Conservation:** Every canonical item transformation shall conserve registered inputs and outputs within defined loss rules.
- **IND-003 — Formal work:** Players and agents may enter employment, delivery, mining, manufacturing, construction, defense, and service contracts.
- **IND-004 — Verifiable completion:** Contract completion shall use authoritative events where technically possible.
- **IND-005 — Company ownership:** Companies may own treasuries, inventories, grids, facilities, blueprints, and market positions.

## Markets and tokens

- **MKT-001 — BIT base pair:** BIT shall be the default base pair in official markets.
- **MKT-002 — Capital market:** The capital shall host the primary protected market and simplest selling experience.
- **MKT-003 — Multiple markets:** Users, companies, stations, and approved applications may operate additional markets.
- **MKT-004 — Location-specific custody:** Market goods retain a physical custody location and do not teleport between venues.
- **MKT-005 — AMM commodities:** Standardized fungible commodity markets shall use algorithmic automated-market-maker pools.
- **MKT-006 — Unique goods:** Unique ships, blueprints, skins, and non-fungible goods shall use listings, auctions, or specialized pools rather than forced fungibility.
- **MKT-007 — Funded liquidity:** Market purchases shall be limited by deposited liquidity; no market promises unlimited fixed-price BIT redemption.
- **MKT-008 — Ordinary volatility:** Governance shall not pause or alter markets merely because of legitimate price movements, deposits, or sales.
- **MKT-009 — Permissionless participation:** Any eligible profile may access markets, subject to the Open Metaverse governing framework, published protocol rules, and applicable nonwaivable law.
- **MKT-010 — Canonical AMM authority:** The Verse DAO shall deploy official AMMs and provide the capital market's initial BIT and commodity liquidity under publicly recorded parameters.

## Blockchain

- **CHAIN-001 — Deposited representation:** Only assets intentionally deposited into a market, escrow, or export process require direct tokenized representation.
- **CHAIN-002 — Internal inventory:** Ordinary personal, company, container, and world inventory remains authoritative off-chain.
- **CHAIN-003 — Lifecycle proofs:** Creation, transformation, consumption, and destruction events shall be included in publicly verifiable batched commitments.
- **CHAIN-004 — Gasless routine UX:** Routine gameplay and market actions shall hide transaction construction and gas handling.
- **CHAIN-005 — Explicit custody:** High-value withdrawal, recovery, and authority changes shall require explicit confirmation.
- **CHAIN-006 — Multi-chain registry:** Every supported token, Safe, bridge, and market contract shall be registered by chain ID.
- **CHAIN-007 — Upgrade monitoring:** Upgradeable dependencies shall be monitored and automatically quarantined after unexpected implementation changes.
- **CHAIN-008 — BIT sourcing:** BIT acquisition may integrate Bittrees bNOTE and approved WBTC/BTREE routes after ABI, audit, and slippage behavior are specified. **Deferred:** this interface is not required for gameplay, voxel mining, or the internal economic proof, but is required before P3 testnet integration.

## Governance and modding

- **GOV-001 — Initial executor:** The Ethereum Verse DAO Safe shall use a two-of-three threshold.
- **GOV-002 — Security delegation:** The Safe may delegate narrowly scoped pause and malicious-mod removal powers to a Security Council.
- **GOV-003 — Persistent pause:** A pause remains active until the Verse DAO or authorized Security Council unpauses it.
- **GOV-004 — Narrow reasons:** Emergency intervention is limited to safety failures such as duplication, unlimited resource creation, insolvency, bridge compromise, or malicious code.
- **GOV-005 — Public record:** Emergency actions shall include public scope, reason, signer, affected systems, and remediation state.
- **GOV-006 — Open Metaverse framework:** The Verse shall adopt the Open Metaverse contractual governing framework defined by Section 13 of the Bittrees Bounties Terms of Use effective 2026-08-12.
- **GOV-007 — Mandatory-rights boundary:** The Verse shall not represent that framework as a nation, sovereign, court, territorial jurisdiction, or immunity from law; applicable nonwaivable rights and laws prevail to the extent of conflict.
- **MOD-001 — Official approval:** Official servers shall load only Verse DAO-approved signed mod manifests.
- **MOD-002 — Private freedom:** Private servers may load unrestricted mods.
- **MOD-003 — Economic isolation:** Private-server items and resources shall never enter the canonical universe or its markets.
- **MOD-004 — UGC sales:** Approved blueprints, skins, avatars, clothing, and other content may be sold under declared licenses.

## Open source and provenance

- **OSS-001 — Game license:** Game and authoritative server code shall use AGPL-3.0-or-later.
- **OSS-002 — SDK license:** SDKs and public schemas shall use Apache-2.0.
- **OSS-003 — Reusable asset license:** Designated reusable assets shall use CC BY-SA 4.0.
- **OSS-004 — Clean-room implementation:** No Space Engineers or franchise source code or extracted assets may enter the project.
- **OSS-005 — Original identity:** The Verse shall use original names, visual designs, interface, lore, audio, and content.
