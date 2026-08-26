# Proposed repository layout

**Status:** Proposed

The project should begin as a monorepo so schemas, tests, documentation, and compatible releases change atomically.

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
│   ├── coordinates/
│   ├── events/
│   ├── inventory/
│   ├── voxel/
│   ├── grid/
│   ├── physics/
│   ├── power/
│   └── production/
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

## Initial creation order

1. `schemas/`
2. Coordinate, event, and inventory crates.
3. Benchmark tooling.
4. Minimal simulation worker.
5. Native client.
6. Local infrastructure.
7. Browser application.
8. Contracts and chain services after economic specifications are accepted.
