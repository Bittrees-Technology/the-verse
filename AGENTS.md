# Repository instructions

These instructions apply to human and AI contributors.

## Specification-first rule

- Treat files under `docs/` as the current design baseline.
- Do not implement a subsystem while its requirements are marked unresolved.
- Record durable architecture changes as ADRs under `docs/decisions/`.
- Update requirement IDs and the feature catalog when scope changes.
- Never copy Space Engineers or other third-party source code, assets, UI, names, sounds, or protected visual designs.

## Safety and economy

- The authoritative server owns gameplay state.
- Clients and third-party applications submit intents, never direct state mutations.
- Preserve asset conservation and idempotency in every inventory or market change.
- Creative/admin assets must remain non-economic.
- Private-server assets must never enter the canonical universe.
- Never commit credentials, wallet keys, API keys, private RPC URLs, or signing material.
- Never perform a mainnet transaction as part of a test.

## Licensing

- Client/server code: AGPL-3.0-or-later.
- SDKs and schemas: Apache-2.0.
- Reusable assets: CC BY-SA 4.0 unless explicitly stated.
- Every new dependency or asset must record its license and source.

## Change quality

- Add tests with implementation.
- Keep network and persistence behavior deterministic where specified.
- Treat public APIs and event schemas as versioned interfaces.
- Document migrations and rollback behavior.
- Prefer small, reviewable changes linked to requirement IDs.
