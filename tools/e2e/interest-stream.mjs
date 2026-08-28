// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";

export const COMPATIBILITY = Object.freeze({
  protocol_version: 17,
  projection_schema_version: 3,
  world_schema_version: 19,
  event_schema_version: 15,
  content_schema_version: 11,
  content_manifest_version: "p1.5.0",
  celestial_registry_schema_version: 1,
  universe_manifest_schema_version: 3,
  interest_schema_version: 1,
});

const ENTITY_KIND_ORDER = new Map([
  ["player", 0],
  ["grid", 1],
  ["voxel_chunk", 2],
  ["death_drop", 3],
]);

function assertNonemptyText(value, description) {
  assert.equal(typeof value, "string", `${description} is text`);
  assert.ok(value.length > 0, `${description} is non-empty`);
}

function assertSafeNonnegativeInteger(value, description) {
  assert.ok(
    Number.isSafeInteger(value) && value >= 0,
    `${description} is a safe non-negative integer`,
  );
}

function assertCanonicalSectorComponent(value, description) {
  assert.equal(typeof value, "string", `${description} is JSON text`);
  assert.match(value, /^(?:0|-?[1-9][0-9]*)$/, `${description} is canonical`);
  return BigInt(value);
}

function validateAddress(address, manifest, description) {
  assert.ok(address && typeof address === "object", `${description} exists`);
  assert.equal(address.universe_id, manifest.universe_id);
  const sectors = ["x", "y", "z"].map((axis) =>
    assertCanonicalSectorComponent(
      address.sector?.[axis],
      `${description}.sector.${axis}`,
    ),
  );
  for (const axis of ["x", "y", "z"]) {
    assert.ok(
      Number.isSafeInteger(address.cell?.[axis]) &&
        address.cell[axis] >= 0 &&
        address.cell[axis] < manifest.cells_per_sector_axis,
      `${description}.cell.${axis} is normalized`,
    );
    assert.ok(
      Number.isSafeInteger(address.local_um?.[axis]),
      `${description}.local_um.${axis} is exact`,
    );
    const halfCell = manifest.cell_edge_um / 2;
    assert.ok(
      address.local_um[axis] >= -halfCell && address.local_um[axis] < halfCell,
      `${description}.local_um.${axis} is normalized`,
    );
  }
  return sectors;
}

export function derivePosition(
  address,
  origin,
  manifest,
  description = "address",
) {
  const sectors = validateAddress(address, manifest, description);
  const originSectors = validateAddress(origin, manifest, "renderer origin");
  const sectorEdge = BigInt(manifest.sector_edge_um);
  const cellEdge = BigInt(manifest.cell_edge_um);
  const position = {};
  for (const [index, axis] of ["x", "y", "z"].entries()) {
    const deltaMicrometres =
      (sectors[index] - originSectors[index]) * sectorEdge +
      BigInt(address.cell[axis] - origin.cell[axis]) * cellEdge +
      BigInt(address.local_um[axis] - origin.local_um[axis]);
    const wholeMetres = deltaMicrometres / 1_000_000n;
    assert.ok(
      wholeMetres >= BigInt(Number.MIN_SAFE_INTEGER) &&
        wholeMetres <= BigInt(Number.MAX_SAFE_INTEGER),
      `${description} is within the renderer's safe local range`,
    );
    position[axis] = Number(deltaMicrometres) / 1_000_000;
    assert.ok(
      Number.isFinite(position[axis]),
      `${description}.${axis} is finite`,
    );
  }
  return position;
}

function entityKey(kind, entityId) {
  return `${kind}\0${entityId}`;
}

function payloadIdentity(payload) {
  switch (payload.entity_kind) {
    case "player":
      return payload.value.player_id;
    case "grid":
      return payload.value.grid_id;
    case "voxel_chunk":
      return payload.value.chunk_id;
    case "death_drop":
      return payload.value.drop_id;
    default:
      assert.fail(`unknown interest payload kind ${payload.entity_kind}`);
  }
}

function compareEntityOperation(left, right) {
  return (
    left.entity_id.localeCompare(right.entity_id) ||
    ENTITY_KIND_ORDER.get(left.kind) - ENTITY_KIND_ORDER.get(right.kind)
  );
}

function validateCanonicalOperations(operations, description) {
  assert.deepEqual(
    operations,
    [...operations].sort(compareEntityOperation),
    `${description} is in canonical identity/kind order`,
  );
}

function hydratePublicValue(kind, value, origin, manifest) {
  const hydrated = structuredClone(value);
  if (kind !== "voxel_chunk") {
    hydrated.position = derivePosition(
      hydrated.address,
      origin,
      manifest,
      `${kind} ${payloadIdentity({ entity_kind: kind, value })}`,
    );
  }
  return hydrated;
}

function hydrateActorPrivate(value, origin, manifest, expectedPlayerId) {
  if (value === undefined || value === null) return undefined;
  const hydrated = structuredClone(value);
  assert.equal(
    hydrated.player.player_id,
    expectedPlayerId,
    "the actor-private replacement is bound to the authenticated player",
  );
  hydrated.player.position = derivePosition(
    hydrated.player.address,
    origin,
    manifest,
    `private player ${expectedPlayerId}`,
  );
  hydrated.death_drops = hydrated.death_drops.map((drop) => ({
    ...drop,
    position: derivePosition(
      drop.address,
      origin,
      manifest,
      `private death drop ${drop.drop_id}`,
    ),
  }));
  return hydrated;
}

function hydrateActorMotion(value, origin, manifest, expectedPlayerId) {
  if (value === undefined || value === null) return undefined;
  assert.equal(value.player_id, expectedPlayerId);
  return {
    ...structuredClone(value),
    position: derivePosition(
      value.address,
      origin,
      manifest,
      `private player motion ${expectedPlayerId}`,
    ),
  };
}

function projectionCollections(entities, origin, manifest) {
  const values = {
    players: [],
    grids: [],
    voxel_chunks: [],
    death_drops: [],
  };
  for (const operation of [...entities.values()].sort(compareEntityOperation)) {
    const value = hydratePublicValue(
      operation.kind,
      operation.payload.value,
      origin,
      manifest,
    );
    switch (operation.kind) {
      case "player":
        values.players.push(value);
        break;
      case "grid":
        values.grids.push(value);
        break;
      case "voxel_chunk":
        values.voxel_chunks.push(value);
        break;
      case "death_drop":
        values.death_drops.push(value);
        break;
      default:
        assert.fail(`unsupported interest kind ${operation.kind}`);
    }
  }
  for (const [collection, id] of [
    [values.players, "player_id"],
    [values.grids, "grid_id"],
    [values.voxel_chunks, "chunk_id"],
    [values.death_drops, "drop_id"],
  ]) {
    collection.sort((left, right) => left[id].localeCompare(right[id]));
  }
  return values;
}

function compatibilityWorld(projection) {
  const privateState = projection.actor_private;
  if (!privateState) {
    return {
      ...projection,
      voxels: projection.voxel_chunks.flatMap((chunk) => chunk.voxels),
      inventories: [],
      conservation: { valid: projection.conservation_valid },
    };
  }
  const privateMasses = new Map(
    privateState.owned_grid_masses.map((entry) => [
      entry.grid_id,
      entry.mass_kg,
    ]),
  );
  return {
    ...projection,
    player: privateState.player,
    players: projection.players.map((player) =>
      player.player_id === privateState.player.player_id
        ? privateState.player
        : player,
    ),
    grids: projection.grids.map((grid) => ({
      ...grid,
      ...(privateMasses.has(grid.grid_id)
        ? { mass_kg: privateMasses.get(grid.grid_id) }
        : {}),
    })),
    voxels: projection.voxel_chunks.flatMap((chunk) => chunk.voxels),
    inventories: privateState.inventories,
    death_drops: privateState.death_drops,
    production_queues: privateState.production_queues ?? [],
    conservation: { valid: projection.conservation_valid },
  };
}

export class Protocol16InterestStream {
  constructor({ expectedPlayerId, send }) {
    this.expectedPlayerId = expectedPlayerId;
    this.expectedSessionRole = expectedPlayerId
      ? { kind: "player", player_id: expectedPlayerId }
      : { kind: "spectator" };
    this.expectedObserverClass = expectedPlayerId
      ? "bound_player"
      : "public_origin_spectator";
    this.send = send;
    this.phase = "welcome";
    this.welcome = undefined;
    this.registry = undefined;
    this.manifest = undefined;
    this.entities = new Map();
    this.revisionFrontier = new Map();
    this.frontier = undefined;
    this.projection = undefined;
  }

  receive(message) {
    assert.ok(
      message && typeof message === "object",
      "server message is an object",
    );
    if (message.type === "snapshot" || message.type === "motion_state") {
      assert.fail(
        `protocol 17 mixed legacy ${message.type} into the interest stream`,
      );
    }
    if (message.type === "welcome") return this.receiveWelcome(message);
    if (message.type === "registry") return this.receiveRegistry(message);
    if (message.type === "interest_baseline") {
      return this.receiveBaseline(message.baseline);
    }
    if (message.type === "interest_delta")
      return this.receiveDelta(message.delta);
    assert.equal(
      this.phase,
      "stream",
      `${message.type} cannot precede Welcome -> Registry -> InterestBaseline`,
    );
    return message;
  }

  receiveWelcome(message) {
    assert.equal(this.phase, "welcome", "welcome is the first server message");
    for (const [field, expected] of Object.entries(COMPATIBILITY)) {
      assert.equal(message[field], expected, `welcome.${field} is compatible`);
    }
    assert.deepEqual(message.session_role, this.expectedSessionRole);
    assertNonemptyText(message.server_name, "welcome.server_name");
    this.welcome = structuredClone(message);
    this.phase = "registry";
    return message;
  }

  receiveRegistry(message) {
    assert.equal(
      this.phase,
      "registry",
      "registry follows welcome exactly once",
    );
    const { registry, universe_manifest: manifest } = message;
    assert.equal(
      registry.schema_version,
      COMPATIBILITY.celestial_registry_schema_version,
    );
    assert.equal(
      manifest.schema_version,
      COMPATIBILITY.universe_manifest_schema_version,
    );
    assert.equal(manifest.address_schema_version, 1);
    assert.equal(
      manifest.content_schema_version,
      this.welcome.content_schema_version,
    );
    assert.equal(
      manifest.content_manifest_version,
      this.welcome.content_manifest_version,
    );
    assert.equal(
      manifest.world_schema_version,
      this.welcome.world_schema_version,
    );
    assert.equal(
      manifest.event_schema_version,
      this.welcome.event_schema_version,
    );
    assert.equal(manifest.lifecycle_control_schema_version, 1);
    assert.equal(manifest.production_schedule_occurrence_schema_version, 1);
    assert.equal(
      manifest.lifecycle_policy_hash,
      "5bc077cc8a2eb101fcaecdce5513c13aa243e1f68a5af839a602dd689859ff3a",
    );
    assert.equal(
      manifest.celestial_registry_schema_version,
      registry.schema_version,
    );
    assert.equal(manifest.celestial_registry_hash, registry.registry_hash);
    assert.equal(manifest.universe_id, registry.universe_id);
    assertNonemptyText(registry.registry_hash, "registry hash");
    assertNonemptyText(manifest.manifest_hash, "universe manifest hash");
    assertNonemptyText(manifest.content_hash, "content hash");
    assert.ok(
      Number.isSafeInteger(manifest.sector_edge_um) &&
        manifest.sector_edge_um > 0,
    );
    assert.ok(
      Number.isSafeInteger(manifest.cell_edge_um) && manifest.cell_edge_um > 0,
    );
    assert.ok(
      Number.isSafeInteger(manifest.cells_per_sector_axis) &&
        manifest.cells_per_sector_axis > 0,
    );
    assert.equal(
      manifest.sector_edge_um,
      manifest.cell_edge_um * manifest.cells_per_sector_axis,
    );
    assert.ok(
      registry.bodies.length >= 2,
      "the public universe has celestial bodies",
    );
    assert.deepEqual(
      registry.bodies.map((body) => body.body_id),
      registry.bodies.map((body) => body.body_id).sort(),
      "celestial bodies are canonically ordered",
    );
    for (const body of registry.bodies) {
      validateAddress(body.center, manifest, `celestial body ${body.body_id}`);
      assert.equal(
        body.content_manifest_version,
        manifest.content_manifest_version,
      );
      assert.equal(body.content_hash, manifest.content_hash);
    }
    this.registry = structuredClone(registry);
    this.manifest = structuredClone(manifest);
    this.phase = "baseline";
    return message;
  }

  validateCommonFrame(frame, expectedKind) {
    assert.equal(
      frame.projection_schema_version,
      COMPATIBILITY.projection_schema_version,
    );
    assert.equal(frame.schema_version, COMPATIBILITY.world_schema_version);
    assert.equal(
      frame.content_manifest_version,
      COMPATIBILITY.content_manifest_version,
    );
    assert.equal(frame.universe_id, this.manifest.universe_id);
    assert.equal(frame.universe_manifest_hash, this.manifest.manifest_hash);
    assert.equal(frame.celestial_registry_hash, this.registry.registry_hash);
    validateAddress(frame.cell_address, this.manifest, "frame.cell_address");
    const interest = frame.interest;
    assert.equal(
      interest.schema_version,
      COMPATIBILITY.interest_schema_version,
    );
    assert.equal(interest.frame_kind, expectedKind);
    assert.equal(interest.registry_hash, this.registry.registry_hash);
    assert.equal(interest.universe_manifest_hash, this.manifest.manifest_hash);
    assert.deepEqual(interest.cell_address, frame.cell_address);
    validateAddress(
      interest.local_origin_address,
      this.manifest,
      "interest local origin",
    );
    assert.equal(interest.canonical_event_sequence, frame.event_sequence);
    assert.equal(interest.canonical_tick, frame.simulation_tick);
    assert.equal(interest.canonical_world_hash, frame.world_hash);
    assertSafeNonnegativeInteger(
      interest.canonical_event_sequence,
      "event sequence",
    );
    assertSafeNonnegativeInteger(interest.canonical_tick, "simulation tick");
    assertNonemptyText(interest.session_epoch, "session epoch");
    assertNonemptyText(interest.baseline_id, "baseline id");
    assertNonemptyText(interest.view_hash, "view hash");
    assert.equal(interest.observer_class, this.expectedObserverClass);
    validateCanonicalOperations(
      interest.entered,
      "interest entered operations",
    );
    validateCanonicalOperations(
      interest.replaced,
      "interest replacement operations",
    );
    validateCanonicalOperations(
      interest.removed,
      "interest removal operations",
    );
    return interest;
  }

  validateEntityOperation(operation, description) {
    assert.ok(
      ENTITY_KIND_ORDER.has(operation.kind),
      `${description} has a known kind`,
    );
    assert.equal(operation.payload.entity_kind, operation.kind);
    assert.equal(payloadIdentity(operation.payload), operation.entity_id);
    assert.equal(
      operation.component_schema_version,
      COMPATIBILITY.projection_schema_version,
    );
    assert.ok(
      Number.isSafeInteger(operation.projected_revision) &&
        operation.projected_revision > 0,
      `${description} has a positive projected revision`,
    );
    if (operation.kind !== "voxel_chunk") {
      validateAddress(
        operation.payload.value.address,
        this.manifest,
        `${description} address`,
      );
    }
  }

  receiveBaseline(baseline) {
    assert.ok(
      this.phase === "baseline" || this.phase === "stream",
      "baseline follows registry or an explicit resynchronization",
    );
    const interest = this.validateCommonFrame(baseline, "baseline");
    assert.equal(interest.delta_sequence, 0);
    assert.equal(interest.previous_view_hash, undefined);
    assert.deepEqual(interest.replaced, []);
    assert.deepEqual(interest.removed, []);
    if (this.frontier) {
      assert.equal(interest.session_epoch, this.frontier.session_epoch);
      assert.equal(interest.interest_epoch, this.frontier.interest_epoch + 1);
      assert.notEqual(interest.baseline_id, this.frontier.baseline_id);
    } else {
      assert.equal(interest.interest_epoch, 1);
    }
    const entities = new Map();
    for (const operation of interest.entered) {
      this.validateEntityOperation(
        operation,
        `baseline entity ${operation.entity_id}`,
      );
      const key = entityKey(operation.kind, operation.entity_id);
      assert.ok(
        !entities.has(key),
        `baseline entity ${operation.entity_id} is unique`,
      );
      entities.set(key, structuredClone(operation));
    }
    this.entities = entities;
    this.revisionFrontier = new Map(
      [...entities].map(([key, operation]) => [
        key,
        operation.projected_revision,
      ]),
    );
    const rawCollections = projectionCollections(
      entities,
      baseline.cell_address,
      this.manifest,
    );
    for (const name of ["players", "grids", "voxel_chunks", "death_drops"]) {
      const expectedRaw = rawCollections[name].map(
        ({ position: _position, ...value }) => value,
      );
      assert.deepEqual(
        baseline[name],
        expectedRaw,
        `baseline ${name} matches entered payloads`,
      );
    }
    const actorPrivate = hydrateActorPrivate(
      baseline.actor_private,
      baseline.cell_address,
      this.manifest,
      this.expectedPlayerId,
    );
    if (this.expectedPlayerId === undefined) {
      assert.equal(
        actorPrivate,
        undefined,
        "a public spectator baseline has no actor-private overlay",
      );
    }
    this.projection = {
      ...structuredClone(baseline),
      ...rawCollections,
      actor_private: actorPrivate,
    };
    this.installFrontier(interest);
    this.phase = "stream";
    this.acknowledge(interest);
    return this.appliedMessage("baseline", interest);
  }

  receiveDelta(delta) {
    assert.equal(this.phase, "stream", "a delta requires an applied baseline");
    const interest = this.validateCommonFrame(delta, "delta");
    assert.equal(interest.session_epoch, this.frontier.session_epoch);
    assert.equal(interest.interest_epoch, this.frontier.interest_epoch);
    assert.equal(interest.baseline_id, this.frontier.baseline_id);
    assert.equal(interest.delta_sequence, this.frontier.delta_sequence + 1);
    assert.equal(interest.previous_view_hash, this.frontier.view_hash);

    const touched = new Set();
    for (const operation of interest.entered) {
      this.validateEntityOperation(
        operation,
        `entered entity ${operation.entity_id}`,
      );
      const key = entityKey(operation.kind, operation.entity_id);
      assert.ok(
        !touched.has(key),
        `${operation.entity_id} has one delta operation`,
      );
      assert.ok(
        !this.entities.has(key),
        `${operation.entity_id} enters from outside the view`,
      );
      assert.ok(
        operation.projected_revision > (this.revisionFrontier.get(key) ?? 0),
        `${operation.entity_id} re-entry advances its revision`,
      );
      touched.add(key);
      this.entities.set(key, structuredClone(operation));
      this.revisionFrontier.set(key, operation.projected_revision);
    }
    for (const operation of interest.replaced) {
      this.validateEntityOperation(
        operation,
        `replaced entity ${operation.entity_id}`,
      );
      const key = entityKey(operation.kind, operation.entity_id);
      assert.ok(
        !touched.has(key),
        `${operation.entity_id} has one delta operation`,
      );
      assert.ok(
        this.entities.has(key),
        `${operation.entity_id} is visible before replacement`,
      );
      assert.ok(
        operation.projected_revision >
          this.entities.get(key).projected_revision,
        `${operation.entity_id} replacement advances its revision`,
      );
      touched.add(key);
      this.entities.set(key, structuredClone(operation));
      this.revisionFrontier.set(key, operation.projected_revision);
    }
    for (const removal of interest.removed) {
      const key = entityKey(removal.kind, removal.entity_id);
      assert.ok(
        !touched.has(key),
        `${removal.entity_id} has one delta operation`,
      );
      assert.ok(
        this.entities.has(key),
        `${removal.entity_id} is visible before removal`,
      );
      assert.ok(
        ["out_of_interest", "destroyed", "transferred"].includes(
          removal.reason,
        ),
        `${removal.entity_id} has a supported removal reason`,
      );
      touched.add(key);
      this.entities.delete(key);
    }

    const collections = projectionCollections(
      this.entities,
      delta.cell_address,
      this.manifest,
    );
    let actorPrivate = delta.actor_private
      ? hydrateActorPrivate(
          delta.actor_private,
          delta.cell_address,
          this.manifest,
          this.expectedPlayerId,
        )
      : this.projection.actor_private;
    if (this.expectedPlayerId === undefined) {
      assert.equal(
        actorPrivate,
        undefined,
        "a public spectator delta has no actor-private overlay",
      );
    }
    const actorMotion = hydrateActorMotion(
      delta.actor_private_motion,
      delta.cell_address,
      this.manifest,
      this.expectedPlayerId,
    );
    if (actorMotion) {
      actorPrivate = {
        ...actorPrivate,
        player: { ...actorPrivate.player, ...actorMotion },
      };
    }
    this.projection = {
      ...this.projection,
      projection_schema_version: delta.projection_schema_version,
      schema_version: delta.schema_version,
      content_manifest_version: delta.content_manifest_version,
      universe_id: delta.universe_id,
      cell_id: delta.cell_id,
      universe_manifest_hash: delta.universe_manifest_hash,
      celestial_registry_hash: delta.celestial_registry_hash,
      cell_address: structuredClone(delta.cell_address),
      gravity_body_id: delta.gravity_body_id,
      voxel_body_id: delta.voxel_body_id,
      event_sequence: delta.event_sequence,
      simulation_tick: delta.simulation_tick,
      world_hash: delta.world_hash,
      environment: delta.environment ?? this.projection.environment,
      conservation_valid:
        delta.conservation_valid ?? this.projection.conservation_valid,
      interest: structuredClone(interest),
      actor_private: actorPrivate,
      ...collections,
    };
    this.installFrontier(interest);
    this.acknowledge(interest);
    return this.appliedMessage("delta", interest);
  }

  installFrontier(interest) {
    this.frontier = {
      session_epoch: interest.session_epoch,
      interest_epoch: interest.interest_epoch,
      baseline_id: interest.baseline_id,
      delta_sequence: interest.delta_sequence,
      view_hash: interest.view_hash,
    };
  }

  acknowledge(interest) {
    this.send({
      type: "acknowledge_interest",
      session_epoch: interest.session_epoch,
      interest_epoch: interest.interest_epoch,
      baseline_id: interest.baseline_id,
      delta_sequence: interest.delta_sequence,
      view_hash: interest.view_hash,
    });
  }

  appliedMessage(frameKind, interest) {
    const projection = structuredClone(this.projection);
    return {
      type: "interest_state",
      frame_kind: frameKind,
      interest: structuredClone(interest),
      projection,
      state: compatibilityWorld(projection),
    };
  }
}
