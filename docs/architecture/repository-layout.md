# Repository layout

**Status:** Accepted; populated incrementally by roadmap phase

The project is a monorepo so schemas, tests, documentation, and compatible releases change atomically. Directories without an implementation are created only when their roadmap phase begins.

```text
the-verse/
├── apps/
│   ├── native-client/          # Godot project
│   └── web-command-center/     # Browser application
├── services/
│   ├── gateway/
│   ├── identity/
│   ├── universe-directory/
│   ├── simulation-worker/
│   ├── route-service/
│   ├── asset-registry/
│   ├── market-reconciler/
│   ├── contract-service/
│   ├── settlement-batcher/
│   └── chain-indexer/
├── crates/
│   ├── verse-protocol/        # Apache-licensed wire types
│   └── verse-simulation/      # P0 authoritative kernel
├── contracts/
│   ├── src/
│   ├── test/
│   ├── script/
│   └── deployments/
├── sdk/
│   ├── typescript/
│   └── rust/
├── schemas/
│   ├── events/
│   ├── api/
│   ├── content/
│   └── settlement/
├── content/
│   ├── definitions/
│   └── manifests/
├── assets/
│   ├── source/
│   └── metadata/
├── tools/
│   ├── benchmarks/
│   ├── validators/
│   └── release/
├── infra/
│   ├── local/
│   ├── containers/
│   └── deployment/
├── docs/
└── LICENSES/
```

## Licensing boundaries

- `apps/`, `services/`, `crates/`, and `contracts/`: AGPL-3.0-or-later unless noted.
- `sdk/` and public `schemas/`: Apache-2.0.
- `assets/`: per-file metadata, reusable assets defaulting to CC BY-SA 4.0.
- Generated files must identify their source and license.

## Dependency direction

- Schemas and pure domain crates do not depend on services.
- Services may depend on schemas and domain crates.
- The client uses public schemas and presentation-specific adapters.
- Contracts do not import server implementation.
- Chain adapters implement domain interfaces rather than leaking provider APIs through the codebase.
- Content definitions are data, not hard-coded into the simulation kernel.

## Implemented P0.1 paths

- `apps/native-client`: Godot native flight and construction client.
- `apps/web-command-center`: zero-build browser management and spectating client.
- `content/definitions`: versioned authoritative gameplay rules.
- `crates/verse-protocol`: shared protocol and snapshot types.
- `crates/verse-simulation`: deterministic world, rules, ledger, events, and persistence.
- `services/simulation-worker`: headless HTTP and WebSocket host.
- `tools/e2e`, `tools/benchmarks`, and `tools/ci`: verification and baseline tooling.
- `infra/local` and `infra/containers`: local Linux-compatible server packaging.

Contracts and chain services remain intentionally absent until the gameplay economy has been validated under the roadmap gates.
