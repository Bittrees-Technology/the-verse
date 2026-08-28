// SPDX-License-Identifier: AGPL-3.0-or-later

const elements = Object.fromEntries(
  [
    "connection", "universe", "cell", "event-sequence", "world-hash",
    "conservation", "fence", "world-counts", "grid-count", "ore", "refined",
    "components", "selected-grid", "grid-details", "activity", "universe-map",
    "resync", "refine", "craft", "anchor", "stop", "profile-rank",
    "career-progress",
    "active-players", "session-status", "player-roster", "profile-label",
    "map-title", "map-mode-local", "map-mode-universe", "map-zoom-out",
    "map-zoom-in", "map-fit", "map-scale", "layer-players", "layer-grids",
    "layer-voxels",
  ].map((id) => [id, document.getElementById(id)]),
);

const PROTOCOL_VERSION = 17;
const PROJECTION_SCHEMA_VERSION = 3;
const WORLD_SCHEMA_VERSION = 19;
const EVENT_SCHEMA_VERSION = 15;
const CONTENT_SCHEMA_VERSION = 11;
const CONTENT_MANIFEST_VERSION = "p1.5.0";
const EXPECTED_UNIVERSE_ID = "the-verse-local";
const EXPECTED_CELESTIAL_REGISTRY_HASH =
  "4c367bbfa04218ece14104f0a3a7ec2c7e9fefcc37d4cf78a265df2d711a59da";
const EXPECTED_UNIVERSE_MANIFEST_HASH =
  "c9bfd3baa1e64ab7665e60c4f989491e745e9af0d2512989f41625b57b546ace";
const EXPECTED_CONTENT_HASH =
  "fc61c05b335fb951868010ecf2942a92ec4f03d00d0a75d3acba8c6f5162b6bd";
const CELESTIAL_REGISTRY_SCHEMA_VERSION = 1;
const UNIVERSE_MANIFEST_SCHEMA_VERSION = 3;
const LIFECYCLE_CONTROL_SCHEMA_VERSION = 1;
const PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION = 1;
const LIFECYCLE_POLICY_HASH =
  "5bc077cc8a2eb101fcaecdce5513c13aa243e1f68a5af839a602dd689859ff3a";
const INTEREST_SCHEMA_VERSION = 1;
const MAX_RENDER_OFFSET_UM = BigInt(Number.MAX_SAFE_INTEGER);
const VERIFIER_INITIALIZATION_TIMEOUT_MS = 15_000;
const VERIFIER_OPERATION_TIMEOUT_MS = 10_000;
const VERIFIED_PRESENTATION_TIMEOUT_MS = 10_000;
const MAX_AUTOMATIC_RECONNECT_ATTEMPTS = 6;
const RECONNECT_BASE_DELAY_MS = 1_200;
const RECONNECT_MAX_DELAY_MS = 30_000;
const MIN_SAFE_INTEGER_BIGINT = BigInt(Number.MIN_SAFE_INTEGER);
const MAX_SAFE_INTEGER_BIGINT = BigInt(Number.MAX_SAFE_INTEGER);
const MAX_U64_BIGINT = (1n << 64n) - 1n;

let socket;
let verifierWorker;
let pendingPresentation;
let reconnectScheduled = false;
let reconnectAttempt = 0;
const verifiedTransportTimers = new Map();
let world;
let registry;
let universeManifest;
let interestFrontier;
let connectionPhase = "disconnected";
let selectedGridId = "grid-starter";
let selectedMapObject;
let mapMode = "local";
let mapView = { x: 0, z: 0, pixelsPerMeter: 8 };
let mapMarkers = [];
let mapDrag;
let mapViewInitialized = false;
let sessionRole = { kind: "spectator" };

function parseLosslessVerifiedJson(raw) {
  if (typeof raw !== "string") throw new TypeError("verified JSON must be text");
  let encoded = "";
  let inString = false;
  let escaped = false;
  for (let index = 0; index < raw.length;) {
    const character = raw[index];
    if (inString) {
      encoded += character;
      index += 1;
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === '"') {
        inString = false;
      }
      continue;
    }
    if (character === '"') {
      inString = true;
      encoded += character;
      index += 1;
      continue;
    }
    if (character !== "-" && (character < "0" || character > "9")) {
      encoded += character;
      index += 1;
      continue;
    }

    const start = index;
    if (raw[index] === "-") index += 1;
    if (raw[index] === "0") {
      index += 1;
    } else {
      while (raw[index] >= "0" && raw[index] <= "9") index += 1;
    }
    let integer = true;
    if (raw[index] === ".") {
      integer = false;
      index += 1;
      while (raw[index] >= "0" && raw[index] <= "9") index += 1;
    }
    if (raw[index] === "e" || raw[index] === "E") {
      integer = false;
      index += 1;
      if (raw[index] === "+" || raw[index] === "-") index += 1;
      while (raw[index] >= "0" && raw[index] <= "9") index += 1;
    }
    const token = raw.slice(start, index);
    if (integer) {
      const exact = BigInt(token);
      if (exact < MIN_SAFE_INTEGER_BIGINT || exact > MAX_SAFE_INTEGER_BIGINT) {
        encoded += JSON.stringify(token);
        continue;
      }
    }
    encoded += token;
  }
  return JSON.parse(encoded);
}

function exactInteger(value) {
  if (Number.isSafeInteger(value)) return BigInt(value);
  if (typeof value !== "string" || !/^(?:0|-?[1-9][0-9]*)$/.test(value)) {
    return undefined;
  }
  try {
    return BigInt(value);
  } catch {
    return undefined;
  }
}

function isExactUnsigned(value, minimum = 0n) {
  const exact = exactInteger(value);
  return exact !== undefined && exact >= minimum && exact <= MAX_U64_BIGINT;
}

function exactIntegerEquals(left, right) {
  const exactLeft = exactInteger(left);
  const exactRight = exactInteger(right);
  return exactLeft !== undefined && exactRight !== undefined && exactLeft === exactRight;
}

function exactIntegerCompare(left, right) {
  const exactLeft = exactInteger(left);
  const exactRight = exactInteger(right);
  if (exactLeft === undefined || exactRight === undefined) return undefined;
  return exactLeft < exactRight ? -1 : exactLeft > exactRight ? 1 : 0;
}

function exactIntegerIsSuccessor(value, previous) {
  const exactValue = exactInteger(value);
  const exactPrevious = exactInteger(previous);
  return exactValue !== undefined && exactPrevious !== undefined &&
    exactValue === exactPrevious + 1n;
}

function canonicalPlayers(state) {
  const roster = Array.isArray(state?.players) ? state.players : [];
  return [...roster].sort((left, right) =>
    left.player_id.localeCompare(right.player_id),
  );
}

function selectedPlayer(state = world) {
  const players = canonicalPlayers(state);
  if (sessionRole.kind === "player") {
    const boundPlayer = players.find(
      (player) => player.player_id === sessionRole.player_id,
    );
    if (boundPlayer) return boundPlayer;
  }
  return players[0];
}

function protocolTupleMatches(message) {
  return message?.protocol_version === PROTOCOL_VERSION &&
    message?.projection_schema_version === PROJECTION_SCHEMA_VERSION &&
    message?.world_schema_version === WORLD_SCHEMA_VERSION &&
    message?.event_schema_version === EVENT_SCHEMA_VERSION &&
    message?.content_schema_version === CONTENT_SCHEMA_VERSION &&
    message?.content_manifest_version === CONTENT_MANIFEST_VERSION &&
    message?.celestial_registry_schema_version === CELESTIAL_REGISTRY_SCHEMA_VERSION &&
    message?.universe_manifest_schema_version === UNIVERSE_MANIFEST_SCHEMA_VERSION &&
    message?.interest_schema_version === INTEREST_SCHEMA_VERSION;
}

function registryBindingIsValid(registryValue, manifestValue) {
  return registryValue?.schema_version === CELESTIAL_REGISTRY_SCHEMA_VERSION &&
    manifestValue?.schema_version === UNIVERSE_MANIFEST_SCHEMA_VERSION &&
    typeof registryValue.registry_hash === "string" && registryValue.registry_hash.length > 0 &&
    typeof manifestValue.manifest_hash === "string" && manifestValue.manifest_hash.length > 0 &&
    registryValue.universe_id === manifestValue.universe_id &&
    manifestValue.celestial_registry_schema_version === registryValue.schema_version &&
    manifestValue.celestial_registry_hash === registryValue.registry_hash &&
    manifestValue.content_schema_version === CONTENT_SCHEMA_VERSION &&
    manifestValue.content_manifest_version === CONTENT_MANIFEST_VERSION &&
    manifestValue.world_schema_version === WORLD_SCHEMA_VERSION &&
    manifestValue.event_schema_version === EVENT_SCHEMA_VERSION &&
    manifestValue.lifecycle_control_schema_version === LIFECYCLE_CONTROL_SCHEMA_VERSION &&
    manifestValue.production_schedule_occurrence_schema_version ===
      PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION &&
    manifestValue.lifecycle_policy_hash === LIFECYCLE_POLICY_HASH &&
    isExactUnsigned(manifestValue.cell_edge_um, 1n) &&
    isExactUnsigned(manifestValue.cells_per_sector_axis, 1n) &&
    Array.isArray(registryValue.bodies) && registryValue.bodies.every((body) =>
      body.center?.universe_id === registryValue.universe_id &&
      body.content_manifest_version === manifestValue.content_manifest_version &&
      body.content_hash === manifestValue.content_hash
    );
}

function exactAddressOffsetMeters(address, origin, manifest = universeManifest) {
  if (!address || !origin || !manifest ||
      address.universe_id !== origin.universe_id ||
      address.universe_id !== manifest.universe_id) return undefined;
  try {
    const cellsPerSector = BigInt(manifest.cells_per_sector_axis);
    const cellEdgeUm = BigInt(manifest.cell_edge_um);
    const axis = (name) => {
      const sector = BigInt(address.sector[name]) - BigInt(origin.sector[name]);
      const cell = BigInt(address.cell[name]) - BigInt(origin.cell[name]);
      const local = BigInt(address.local_um[name]) - BigInt(origin.local_um[name]);
      const offset = (sector * cellsPerSector + cell) * cellEdgeUm + local;
      if (offset < -MAX_RENDER_OFFSET_UM || offset > MAX_RENDER_OFFSET_UM) {
        throw new RangeError("address is outside the bounded renderer frame");
      }
      return Number(offset) / 1_000_000;
    };
    return { x: axis("x"), y: axis("y"), z: axis("z") };
  } catch {
    return undefined;
  }
}

function withRendererPosition(value, origin) {
  if (!value || typeof value !== "object") return value;
  const position = exactAddressOffsetMeters(value.address, origin);
  return position ? { ...value, position } : undefined;
}

function entityKey(kind, id) {
  return `${kind}\u0000${id}`;
}

function entityIdentity(projection) {
  const kind = projection?.kind;
  const payload = projection?.payload;
  if (!payload || payload.entity_kind !== kind || typeof payload.value !== "object") {
    return undefined;
  }
  const idField = {
    player: "player_id",
    grid: "grid_id",
    voxel_chunk: "chunk_id",
    death_drop: "drop_id",
  }[kind];
  const id = idField ? payload.value[idField] : undefined;
  if (typeof id !== "string" || id.length === 0 || id !== projection.entity_id) {
    return undefined;
  }
  return { kind, id, key: entityKey(kind, id), value: payload.value };
}

function interestFrontierFrom(value) {
  return {
    session_epoch: value.session_epoch,
    interest_epoch: value.interest_epoch,
    baseline_id: value.baseline_id,
    delta_sequence: value.delta_sequence,
    view_hash: value.view_hash,
  };
}

function mergeMotionState(state, motion) {
  const motionRoster = Array.isArray(motion.players) && motion.players.length > 0
    ? motion.players
    : motion.player
      ? [motion.player]
      : [];
  const motionById = new Map(
    motionRoster.map((player) => [player.player_id, player]),
  );
  const players = canonicalPlayers(state).map((player) => ({
    ...player,
    ...(motionById.get(player.player_id) ?? {}),
  }));
  const gridMotion = new Map(
    motion.grids.map((grid) => [grid.grid_id, grid]),
  );
  return {
    ...state,
    event_sequence: motion.event_sequence,
    simulation_tick: motion.simulation_tick,
    world_hash: motion.world_hash,
    players,
    grids: state.grids.map((grid) => ({
      ...grid,
      ...(gridMotion.get(grid.grid_id) ?? {}),
    })),
  };
}

function validInterestBinding(frame, expectedKind) {
  return frame?.schema_version === INTEREST_SCHEMA_VERSION &&
    frame?.frame_kind === expectedKind &&
    frame?.registry_hash === registry?.registry_hash &&
    frame?.universe_manifest_hash === universeManifest?.manifest_hash &&
    frame?.cell_address?.universe_id === universeManifest?.universe_id &&
    frame?.local_origin_address?.universe_id === universeManifest?.universe_id &&
    typeof frame?.session_epoch === "string" && frame.session_epoch.length > 0 &&
    isExactUnsigned(frame?.interest_epoch, 1n) &&
    typeof frame?.baseline_id === "string" && frame.baseline_id.length > 0 &&
    isExactUnsigned(frame?.delta_sequence) &&
    typeof frame?.view_hash === "string" && frame.view_hash.length > 0;
}

function addProjectedEntity(entities, projection, origin, requireAbsent) {
  const identity = entityIdentity(projection);
  if (!identity || projection.component_schema_version !== PROJECTION_SCHEMA_VERSION ||
      !isExactUnsigned(projection.projected_revision, 1n) ||
      (requireAbsent && entities.has(identity.key))) return false;
  let value = identity.value;
  if (["player", "grid", "death_drop"].includes(identity.kind)) {
    value = withRendererPosition(value, origin);
    if (!value) return false;
  }
  entities.set(identity.key, {
    kind: identity.kind,
    id: identity.id,
    projected_revision: projection.projected_revision,
    value,
  });
  return true;
}

function entitiesFromBaseline(frame) {
  if (!Array.isArray(frame.entered) || frame.replaced?.length || frame.removed?.length) {
    return undefined;
  }
  const entities = new Map();
  for (const projection of frame.entered) {
    if (!addProjectedEntity(entities, projection, frame.local_origin_address, true)) {
      return undefined;
    }
  }
  return entities;
}

function materializeEntityArrays(entities) {
  const values = [...entities.values()].sort((left, right) =>
    left.kind.localeCompare(right.kind) || left.id.localeCompare(right.id),
  );
  const byKind = (kind) => values
    .filter((entry) => entry.kind === kind)
    .map((entry) => entry.value);
  const voxelChunks = byKind("voxel_chunk");
  return {
    players: byKind("player"),
    grids: byKind("grid"),
    voxel_chunks: voxelChunks,
    voxels: voxelChunks.flatMap((chunk) => chunk.voxels ?? []),
    death_drops: byKind("death_drop"),
    _entity_revisions: Object.fromEntries(
      values.map((entry) => [entityKey(entry.kind, entry.id), entry.projected_revision]),
    ),
  };
}

function entitiesFromWorld(state) {
  const entities = new Map();
  for (const [kind, idField, values] of [
    ["player", "player_id", state.players],
    ["grid", "grid_id", state.grids],
    ["voxel_chunk", "chunk_id", state.voxel_chunks],
    ["death_drop", "drop_id", state.death_drops],
  ]) {
    for (const value of values ?? []) {
      const id = value?.[idField];
      if (typeof id !== "string" || id.length === 0) return undefined;
      const key = entityKey(kind, id);
      const projectedRevision = state._entity_revisions?.[key];
      if (!isExactUnsigned(projectedRevision, 1n)) {
        return undefined;
      }
      entities.set(key, { kind, id, value, projected_revision: projectedRevision });
    }
  }
  return entities;
}

function worldFromInterestBaseline(projected) {
  const frame = projected?.interest;
  if (!registry || !universeManifest ||
      !["registry", "stale", "live"].includes(connectionPhase) ||
      projected?.projection_schema_version !== PROJECTION_SCHEMA_VERSION ||
      projected?.schema_version !== WORLD_SCHEMA_VERSION ||
      projected?.content_manifest_version !== CONTENT_MANIFEST_VERSION ||
      projected?.universe_id !== universeManifest.universe_id ||
      projected?.universe_manifest_hash !== universeManifest.manifest_hash ||
      projected?.celestial_registry_hash !== registry.registry_hash ||
      !validInterestBinding(frame, "baseline") ||
      !exactIntegerEquals(frame.delta_sequence, 0) ||
      frame.previous_view_hash != null) return undefined;
  const entities = entitiesFromBaseline(frame);
  if (!entities) return undefined;
  const { actor_private: _private, players: _players, grids: _grids,
    voxel_chunks: _chunks, death_drops: _drops, ...headers } = projected;
  return {
    ...headers,
    ...materializeEntityArrays(entities),
    celestial_bodies: registry.bodies.map((body) => ({
      ...body,
      position: exactAddressOffsetMeters(body.center, frame.local_origin_address),
    })),
  };
}

function applyInterestDelta(state, delta) {
  const frame = delta?.interest;
  if (!state || !interestFrontier ||
      delta?.projection_schema_version !== PROJECTION_SCHEMA_VERSION ||
      delta?.schema_version !== WORLD_SCHEMA_VERSION ||
      delta?.content_manifest_version !== CONTENT_MANIFEST_VERSION ||
      delta?.universe_id !== universeManifest?.universe_id ||
      delta?.universe_manifest_hash !== universeManifest?.manifest_hash ||
      delta?.celestial_registry_hash !== registry?.registry_hash ||
      !validInterestBinding(frame, "delta") ||
      frame.session_epoch !== interestFrontier.session_epoch ||
      !exactIntegerEquals(frame.interest_epoch, interestFrontier.interest_epoch) ||
      frame.baseline_id !== interestFrontier.baseline_id ||
      !exactIntegerIsSuccessor(frame.delta_sequence, interestFrontier.delta_sequence) ||
      frame.previous_view_hash !== interestFrontier.view_hash) return undefined;
  const entities = entitiesFromWorld(state);
  if (!entities) return undefined;
  for (const projection of frame.entered ?? []) {
    if (!addProjectedEntity(entities, projection, frame.local_origin_address, true)) {
      return undefined;
    }
  }
  for (const projection of frame.replaced ?? []) {
    const identity = entityIdentity(projection);
    const prior = identity ? entities.get(identity.key) : undefined;
    if (!prior || exactIntegerCompare(
      projection.projected_revision,
      prior.projected_revision,
    ) !== 1 ||
        !addProjectedEntity(entities, projection, frame.local_origin_address, false)) {
      return undefined;
    }
  }
  const removedKeys = new Set();
  for (const removal of frame.removed ?? []) {
    const key = entityKey(removal?.kind, removal?.entity_id);
    if (!["out_of_interest", "destroyed", "transferred"].includes(removal?.reason) ||
        removedKeys.has(key) || !entities.delete(key)) return undefined;
    removedKeys.add(key);
  }
  const next = {
    ...state,
    ...materializeEntityArrays(entities),
    event_sequence: delta.event_sequence,
    simulation_tick: delta.simulation_tick,
    world_hash: delta.world_hash,
    cell_address: delta.cell_address,
    gravity_body_id: delta.gravity_body_id,
    voxel_body_id: delta.voxel_body_id,
    interest: frame,
  };
  if (delta.environment != null) next.environment = delta.environment;
  if (delta.conservation_valid != null) {
    next.conservation_valid = delta.conservation_valid;
  }
  next.celestial_bodies = registry.bodies.map((body) => ({
    ...body,
    position: exactAddressOffsetMeters(body.center, frame.local_origin_address),
  }));
  return next;
}

function playerColor(playerId, rosterIndex = 0) {
  let hash = 2166136261;
  for (const character of playerId) {
    hash ^= character.codePointAt(0);
    hash = Math.imul(hash, 16777619);
  }
  const identityJitter = (Math.abs(hash) % 31) - 15;
  const hue = (145 + identityJitter + rosterIndex * 137.508) % 360;
  return `hsl(${hue.toFixed(3)} 78% 64%)`;
}

function playerPresentations(state, role = sessionRole) {
  return canonicalPlayers(state).map((player, rosterIndex) => {
    const isBound = role.kind === "player" &&
      player.player_id === role.player_id;
    const tags = [];
    if (isBound) tags.push("BOUND");
    tags.push(String(player.life_state ?? "unknown").toUpperCase());
    return {
      player,
      color: playerColor(player.player_id, rosterIndex),
      isBound,
      isPrimary: false,
      label: player.player_id + (isBound ? " [YOU]" : ""),
      status: tags.join(" // "),
    };
  });
}

function sessionDescription() {
  return sessionRole.kind === "player"
    ? "PLAYER // " + sessionRole.player_id
    : "PUBLIC SPECTATOR // READ-ONLY";
}

function scheduleReconnect(schedule, reconnect) {
  if (reconnectScheduled || reconnectAttempt >= MAX_AUTOMATIC_RECONNECT_ATTEMPTS) {
    return false;
  }
  const delay = Math.min(
    RECONNECT_MAX_DELAY_MS,
    RECONNECT_BASE_DELAY_MS * (2 ** reconnectAttempt),
  );
  reconnectAttempt += 1;
  reconnectScheduled = true;
  schedule(() => {
    reconnectScheduled = false;
    reconnect();
  }, delay);
  return true;
}

function endVerifiedTransport(worker, reconnect = true) {
  if (worker !== verifierWorker) return false;
  const timers = verifiedTransportTimers.get(worker);
  if (timers) {
    clearTimeout(timers.initialization);
    clearTimeout(timers.presentation);
    for (const timer of timers.operations.values()) clearTimeout(timer);
    verifiedTransportTimers.delete(worker);
  }
  verifierWorker = undefined;
  socket = undefined;
  pendingPresentation = undefined;
  interestFrontier = undefined;
  connectionPhase = reconnect ? "disconnected" : "fatal";
  sessionRole = { kind: "spectator" };
  worker.terminate();
  const reconnectQueued = reconnect && scheduleReconnect(
    (callback, delay) => setTimeout(callback, delay),
    connect,
  );
  try {
    elements.connection.textContent = reconnectQueued
      ? `○ RECONNECTING ${reconnectAttempt}/${MAX_AUTOMATIC_RECONNECT_ATTEMPTS}`
      : "× STREAM HALTED // RELOAD TO RETRY";
    elements.connection.className = "connection offline";
    if (world) drawMap();
  } catch {
    // Transport recovery cannot depend on presentation health.
  }
  return true;
}

function connect() {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const worker = new Worker("/verifier-worker.js", { type: "module" });
  const timers = {
    initialization: undefined,
    presentation: undefined,
    operations: new Map(),
  };
  timers.initialization = setTimeout(() => {
    try {
      activity("FATAL browser verifier initialization timed out", true);
    } catch {
      // Recovery below is independent of presentation health.
    }
    endVerifiedTransport(worker);
  }, VERIFIER_INITIALIZATION_TIMEOUT_MS);
  verifiedTransportTimers.set(worker, timers);
  verifierWorker = worker;
  socket = {
    send(messageJson) {
      worker.postMessage({ type: "send", messageJson });
    },
    close(code, reason) {
      worker.postMessage({ type: "close", code, reason });
    },
  };
  worker.addEventListener("message", ({ data }) => {
    if (worker !== verifierWorker) return;
    if (data?.type === "transport_open") {
      clearTimeout(timers.initialization);
      timers.initialization = undefined;
      connectionPhase = "hello";
      elements.connection.textContent = "● CONNECTED";
      elements.connection.className = "connection online";
    } else if (data?.type === "verifier_operation_started") {
      if (timers.operations.has(data.operationId)) {
        endVerifiedTransport(worker);
        return;
      }
      const timer = setTimeout(() => {
        try {
          activity(`FATAL browser verifier ${data.operation} timed out`, true);
        } catch {
          // Recovery below is independent of presentation health.
        }
        endVerifiedTransport(worker);
      }, VERIFIER_OPERATION_TIMEOUT_MS);
      timers.operations.set(data.operationId, timer);
    } else if (data?.type === "verifier_operation_completed") {
      const timer = timers.operations.get(data.operationId);
      if (timer !== undefined) clearTimeout(timer);
      timers.operations.delete(data.operationId);
    } else if (data?.type === "prepare_verified_frame") {
      if (pendingPresentation || timers.presentation !== undefined) {
        endVerifiedTransport(worker);
        return;
      }
      let message;
      try {
        message = parseLosslessVerifiedJson(data.messageJson);
        pendingPresentation = prepareVerifiedPresentation(message, data.frameId);
      } catch {
        pendingPresentation = undefined;
      }
      if (!pendingPresentation) {
        worker.postMessage({
          type: "presentation_rejected",
          frameId: data.frameId,
          reason: "verified frame could not be presented safely",
        });
        endVerifiedTransport(worker);
      } else {
        timers.presentation = setTimeout(() => {
          try {
            activity("FATAL verified presentation transition timed out", true);
          } catch {
            // Recovery below is independent of presentation health.
          }
          endVerifiedTransport(worker);
        }, VERIFIED_PRESENTATION_TIMEOUT_MS);
        worker.postMessage({
          type: "presentation_prepared",
          frameId: data.frameId,
        });
      }
    } else if (data?.type === "install_verified_frame") {
      clearTimeout(timers.presentation);
      timers.presentation = undefined;
      if (!pendingPresentation || pendingPresentation.frameId !== data.frameId) {
        worker.postMessage({
          type: "presentation_rejected",
          frameId: data.frameId,
          reason: "presentation candidate was not staged",
        });
        endVerifiedTransport(worker);
        return;
      }
      const installed = pendingPresentation;
      commitVerifiedPresentation(installed);
      pendingPresentation = undefined;
      worker.postMessage({
        type: "presentation_installed",
        frameId: data.frameId,
      });
      presentCommittedPresentation(installed);
      if (installed.kind === "fatal") endVerifiedTransport(worker, false);
    } else if (data?.type === "verification_failed") {
      try {
        activity(`FATAL verified transport ${data.code}: ${data.detail}`, true);
      } finally {
        endVerifiedTransport(worker);
      }
    } else if (data?.type === "transport_error") {
      activity(data.detail, true);
    } else if (data?.type === "transport_closed") {
      endVerifiedTransport(worker);
    }
  });
  worker.addEventListener("error", (event) => {
    event.preventDefault();
    try {
      activity("FATAL browser verifier worker runtime failed", true);
    } catch {
      // Recovery below is independent of presentation health.
    }
    endVerifiedTransport(worker);
  });
  worker.postMessage({
    type: "start",
    websocketUrl: protocol + "//" + location.host + "/ws",
    helloJson: JSON.stringify({
      type: "hello",
      protocol_version: PROTOCOL_VERSION,
      client_name: "browser-command-center-p1.5-verified",
      authentication: { kind: "spectator" },
    }),
    verifierConfigJson: JSON.stringify({
      expected_role: "spectator",
      expected_universe_id: EXPECTED_UNIVERSE_ID,
      expected_celestial_registry_hash: EXPECTED_CELESTIAL_REGISTRY_HASH,
      expected_universe_manifest_hash: EXPECTED_UNIVERSE_MANIFEST_HASH,
      expected_content_hash: EXPECTED_CONTENT_HASH,
      world_schema_version: String(WORLD_SCHEMA_VERSION),
      event_schema_version: String(EVENT_SCHEMA_VERSION),
      content_schema_version: String(CONTENT_SCHEMA_VERSION),
      content_manifest_version: CONTENT_MANIFEST_VERSION,
    }),
  });
}

function prepareVerifiedPresentation(message, frameId) {
  if (message.type === "welcome") {
    if (connectionPhase !== "hello" || !protocolTupleMatches(message)) {
      return undefined;
    }
    const nextRole = message.session_role ?? { kind: "spectator" };
    if (nextRole.kind !== "spectator") return undefined;
    return { frameId, kind: "welcome", sessionRole: nextRole };
  } else if (message.type === "registry") {
    if (connectionPhase !== "welcome" ||
        !registryBindingIsValid(message.registry, message.universe_manifest)) {
      return undefined;
    }
    return {
      frameId,
      kind: "registry",
      registry: JSON.parse(JSON.stringify(message.registry)),
      universeManifest: JSON.parse(JSON.stringify(message.universe_manifest)),
    };
  } else if (message.type === "interest_baseline") {
    const baseline = worldFromInterestBaseline(message.baseline);
    return baseline ? { frameId, kind: "baseline", world: baseline } : undefined;
  } else if (message.type === "interest_delta") {
    if (connectionPhase !== "live") return undefined;
    const next = applyInterestDelta(world, message.delta);
    return next ? { frameId, kind: "delta", world: next } : undefined;
  } else if (message.type === "intent_accepted") {
    return { frameId, kind: "activity", message: message.receipt.message, error: false };
  } else if (message.type === "intent_rejected") {
    return {
      frameId,
      kind: "activity",
      message: message.code + ": " + message.message,
      error: true,
    };
  } else if (message.type === "fatal") {
    return {
      frameId,
      kind: "fatal",
      message: "FATAL " + message.code + ": " + message.message,
      error: true,
    };
  }
  return undefined;
}

function commitVerifiedPresentation(candidate) {
  if (candidate.kind === "welcome") {
    sessionRole = candidate.sessionRole;
    connectionPhase = "welcome";
  } else if (candidate.kind === "registry") {
    registry = candidate.registry;
    universeManifest = candidate.universeManifest;
    connectionPhase = "registry";
  } else if (candidate.kind === "baseline" || candidate.kind === "delta") {
    world = candidate.world;
    interestFrontier = interestFrontierFrom(world.interest);
    connectionPhase = "live";
    reconnectAttempt = 0;
  } else if (candidate.kind === "fatal") {
    connectionPhase = "fatal";
  }
}

function presentCommittedPresentation(candidate, effects = {}) {
  const showActivity = effects.activity ?? activity;
  const fitMap = effects.fitCurrentMap ?? fitCurrentMap;
  const showWorld = effects.render ?? render;
  try {
    if (candidate.kind === "welcome") {
      elements["session-status"].textContent = sessionDescription();
      elements.connection.textContent = sessionRole.kind === "player"
        ? "● PILOT LINK"
        : "● SPECTATING";
      showActivity(
        sessionRole.kind === "spectator"
          ? "Public spectator session — gameplay controls are read-only"
          : "Gameplay session bound to " + sessionRole.player_id,
        false,
      );
    } else if (candidate.kind === "registry") {
      showActivity(`Registry verified // ${registry.bodies.length} fixed bodies`, false);
    } else if (candidate.kind === "baseline" || candidate.kind === "delta") {
      if (!mapViewInitialized) fitMap();
      showWorld();
    } else if (candidate.kind === "activity" || candidate.kind === "fatal") {
      showActivity(candidate.message, candidate.error);
    }
    return true;
  } catch {
    return false;
  }
}

function publicProjection(projected) {
  if (!projected || typeof projected !== "object") return undefined;
  const { actor_private: _private, ...publicState } = projected;
  return publicState;
}

function selectedGrid() {
  const preferred = world?.grids.find((grid) => grid.grid_id === selectedGridId);
  return preferred ?? world?.grids[0];
}

function environmentForPlayer(state, player = selectedPlayer(state)) {
  return player?.environment ?? state?.environment;
}

function selectedObjectValue() {
  if (!world) return undefined;
  const selection = selectedMapObject;
  if (selection?.kind === "celestial") {
    return world.celestial_bodies?.find((body) => body.body_id === selection.id);
  }
  const collection = {
    player: world.players,
    grid: world.grids,
    voxel: world.voxels,
    death_drop: world.death_drops,
  }[selection?.kind];
  const idField = {
    player: "player_id",
    grid: "grid_id",
    voxel: "map_id",
    death_drop: "drop_id",
  }[selection?.kind];
  return collection?.find((value, index) =>
    selection.kind === "voxel"
      ? `voxel-${index}` === selection.id
      : value[idField] === selection.id,
  );
}

function renderSelectedObject(canMutate) {
  const selected = selectedObjectValue();
  let grid = selectedMapObject?.kind === "grid" ? selected : selectedGrid();
  if (grid && (!selectedMapObject || (selectedMapObject.kind === "grid" && !selected))) {
    selectedMapObject = { kind: "grid", id: grid.grid_id };
  }
  elements.anchor.disabled = true;
  elements.stop.disabled = true;

  if (selectedMapObject?.kind === "celestial" && selected) {
    const rangeKm = Math.hypot(
      selected.position.x, selected.position.y, selected.position.z,
    ) / 1000;
    elements["selected-grid"].textContent = selected.display_name.toUpperCase();
    elements["grid-details"].textContent =
      `${String(selected.kind).toUpperCase()} • ${selected.scale_class}\n` +
      `Surface radius ${(Number(selected.surface_radius_um) / 1e6).toFixed(1)} m\n` +
      `Range ${rangeKm.toFixed(2)} km • fixed registry coordinate`;
    return;
  }
  if (selectedMapObject?.kind === "player" && selected) {
    elements["selected-grid"].textContent = selected.player_id.toUpperCase();
    elements["grid-details"].textContent =
      `PILOT • ${String(selected.life_state).toUpperCase()}\n` +
      `Position ${selected.position.x.toFixed(1)}, ` +
      `${selected.position.y.toFixed(1)}, ${selected.position.z.toFixed(1)}\n` +
      "Public authoritative state";
    return;
  }
  if (selectedMapObject?.kind === "voxel" && selected) {
    elements["selected-grid"].textContent = String(selected.material).toUpperCase();
    elements["grid-details"].textContent =
      `VOXEL • ${selectedMapObject.id}\n` +
      `Coordinate ${selected.coordinate.x}, ${selected.coordinate.y}, ` +
      `${selected.coordinate.z}\nMineable local terrain sample`;
    return;
  }
  if (selectedMapObject?.kind === "death_drop" && selected) {
    elements["selected-grid"].textContent = selected.drop_id.toUpperCase();
    elements["grid-details"].textContent =
      `SALVAGE DROP\nPosition ${selected.position.x.toFixed(1)}, ` +
      `${selected.position.y.toFixed(1)}, ${selected.position.z.toFixed(1)}`;
    return;
  }
  if (!grid) {
    elements["selected-grid"].textContent = "NO SELECTION";
    elements["grid-details"].textContent = "No public object is in this interest view.";
    return;
  }

  selectedGridId = grid.grid_id;
  const frames = grid.blocks.filter((block) => !block.construction_complete).length;
  const damaged = grid.blocks.filter(
    (block) => block.construction_complete && block.health < block.max_health,
  ).length;
  elements["selected-grid"].textContent = grid.grid_id.toUpperCase();
  elements["grid-details"].textContent =
    grid.blocks.length + " blocks • " + frames + " frames • " + damaged + " damaged • " +
    (grid.anchored ? "ANCHORED" : "DYNAMIC") + "\n" +
    "Power " + grid.power.produced.toFixed(1) + " / " +
    grid.power.required.toFixed(1) + " • " +
    (grid.power.online ? "ONLINE" : "OFFLINE") + "\n" +
    "Position " + grid.position.x.toFixed(1) + ", " +
    grid.position.y.toFixed(1) + ", " + grid.position.z.toFixed(1);
  const hasAnchor = grid.blocks.some(
    (block) => block.kind === "anchor" && block.construction_complete,
  );
  elements.anchor.disabled = !canMutate || (
    !grid.anchored && (!hasAnchor || !grid.power.online)
  );
  elements.stop.disabled = !canMutate || grid.anchored || (
    grid.linear_velocity.x === 0 && grid.linear_velocity.y === 0 &&
    grid.linear_velocity.z === 0 && grid.angular_velocity.x === 0 &&
    grid.angular_velocity.y === 0 && grid.angular_velocity.z === 0
  );
}

function render() {
  const canMutate = false;
  const players = canonicalPlayers(world);
  const profile = selectedPlayer();
  const environment = environmentForPlayer(world, profile);
  elements.universe.textContent = world.universe_id;
  elements.cell.textContent = world.cell_id;
  elements["event-sequence"].textContent =
    world.event_sequence.toLocaleString() + " / " +
    world.simulation_tick.toLocaleString();
  elements["world-hash"].textContent = "hash " + world.world_hash.slice(0, 16);
  elements.conservation.textContent = world.conservation_valid ? "VALID" : "INVALID";
  elements.conservation.style.color = world.conservation_valid
    ? "var(--green)"
    : "var(--red)";
  elements.fence.textContent = "fence " + world.fencing_token;
  elements["world-counts"].textContent =
    world.voxels.length.toLocaleString() + " voxels";
  elements["grid-count"].textContent =
    world.grids.length.toLocaleString() + " grids • " +
    environment.celestial_body_name + " • " +
    (environment.gravity_m_s2 / 9.80665).toFixed(2) + " g • " +
    Math.round(environment.atmosphere_density * 100) + "% atmosphere";
  elements["active-players"].textContent =
    players.length.toLocaleString() +
    (players.length === 1 ? " AUTHORITATIVE PLAYER" : " AUTHORITATIVE PLAYERS");
  elements["session-status"].textContent = sessionDescription();

  elements.ore.textContent = "PRIVATE";
  elements.refined.textContent = "PRIVATE";
  elements.components.textContent = "PRIVATE";
  elements["profile-label"].textContent = "PUBLIC PILOT PROFILE";
  elements["profile-rank"].textContent =
    (profile?.player_id ?? "NO ACTIVE PILOT").toUpperCase();
  elements["career-progress"].textContent = "PRIVATE TO PILOT";
  elements.refine.disabled = true;
  elements.craft.disabled = true;

  renderSelectedObject(canMutate);
  renderPlayerRoster();
  drawMap();
}

function renderPlayerRoster() {
  elements["player-roster"].replaceChildren();
  for (const presentation of playerPresentations(world)) {
    const player = presentation.player;
    const item = document.createElement("li");
    const marker = document.createElement("i");
    const label = document.createElement("span");
    const state = document.createElement("small");
    marker.style.background = presentation.color;
    label.textContent = player.player_id;
    state.textContent = presentation.status;
    item.append(marker, label, state);
    elements["player-roster"].append(item);
  }
}

function mapObjectsForState(state, mode = mapMode, layers = {}) {
  if (!state) return [];
  const show = {
    players: layers.players ?? true,
    grids: layers.grids ?? true,
    voxels: layers.voxels ?? true,
  };
  const objects = [];
  if (mode === "universe") {
    for (const body of state.celestial_bodies ?? []) {
      if (!body.position) continue;
      objects.push({
        kind: "celestial", id: body.body_id, label: body.display_name,
        position: body.position,
        radiusM: Number(body.surface_radius_um ?? 0) / 1_000_000,
        color: { planet: "#4f8fc4", moon: "#9c8d82", asteroid: "#bd7954",
          asteroid_field: "#96735d" }[body.kind] ?? "#8d9aa3",
      });
    }
  }
  if (show.grids) {
    for (const grid of state.grids ?? []) {
      objects.push({ kind: "grid", id: grid.grid_id, label: grid.grid_id,
        position: grid.position, radiusM: 1, color: "#73d7ff" });
    }
    for (const drop of state.death_drops ?? []) {
      objects.push({ kind: "death_drop", id: drop.drop_id, label: drop.drop_id,
        position: drop.position, radiusM: 0.5, color: "#d975c5" });
    }
  }
  if (show.players) {
    for (const presentation of playerPresentations(state)) {
      objects.push({ kind: "player", id: presentation.player.player_id,
        label: presentation.label, position: presentation.player.position,
        radiusM: 0.5, color: presentation.color });
    }
  }
  if (mode === "local" && show.voxels) {
    for (const [index, voxel] of (state.voxels ?? []).entries()) {
      objects.push({ kind: "voxel", id: `voxel-${index}`, label: voxel.material,
        position: voxel.coordinate, radiusM: 0.25,
        color: voxel.material === "ferrite_ore" ? "#e97c2c" : "#667684" });
    }
  }
  return objects;
}

function fitMapView(objects, width, height, padding = 64) {
  if (!objects.length) return { x: 0, z: 0, pixelsPerMeter: 8 };
  let minX = Infinity;
  let maxX = -Infinity;
  let minZ = Infinity;
  let maxZ = -Infinity;
  for (const object of objects) {
    const radius = Math.max(0, object.radiusM ?? 0);
    minX = Math.min(minX, object.position.x - radius);
    maxX = Math.max(maxX, object.position.x + radius);
    minZ = Math.min(minZ, object.position.z - radius);
    maxZ = Math.max(maxZ, object.position.z + radius);
  }
  const spanX = Math.max(10, maxX - minX);
  const spanZ = Math.max(10, maxZ - minZ);
  const pixelsPerMeter = Math.max(0.000_02, Math.min(
    100,
    Math.max(1, width - padding * 2) / spanX,
    Math.max(1, height - padding * 2) / spanZ,
  ));
  return { x: (minX + maxX) / 2, z: (minZ + maxZ) / 2, pixelsPerMeter };
}

function projectMapPoint(position, view, width, height) {
  return {
    x: width / 2 + (position.x - view.x) * view.pixelsPerMeter,
    y: height / 2 + (position.z - view.z) * view.pixelsPerMeter,
  };
}

function nearestMapMarker(markers, x, y, maxDistance = 20) {
  let nearest;
  let nearestDistance = Infinity;
  for (const marker of markers) {
    const distance = Math.hypot(marker.x - x, marker.y - y);
    const threshold = Math.max(maxDistance, marker.hitRadius ?? 0);
    if (distance <= threshold && distance < nearestDistance) {
      nearest = marker;
      nearestDistance = distance;
    }
  }
  return nearest;
}

function visibleMapObjects() {
  return mapObjectsForState(world, mapMode, {
    players: elements["layer-players"].checked,
    grids: elements["layer-grids"].checked,
    voxels: elements["layer-voxels"].checked,
  });
}

function fitCurrentMap() {
  const canvas = elements["universe-map"];
  mapView = fitMapView(visibleMapObjects(), canvas.width, canvas.height);
  mapViewInitialized = true;
}

function niceGridStep(pixelsPerMeter) {
  const raw = 90 / pixelsPerMeter;
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  const normalized = raw / magnitude;
  const multiple = normalized <= 1 ? 1 : normalized <= 2 ? 2 : normalized <= 5 ? 5 : 10;
  return multiple * magnitude;
}

function drawMap() {
  const canvas = elements["universe-map"];
  const context = canvas.getContext("2d");
  const width = canvas.width;
  const height = canvas.height;
  canvas.classList.toggle("stale", connectionPhase !== "live");
  context.clearRect(0, 0, width, height);
  context.fillStyle = "#03070c";
  context.fillRect(0, 0, width, height);
  mapMarkers = [];
  elements["map-title"].textContent = mapMode === "universe"
    ? "UNIVERSE REGISTRY // XZ PLANE"
    : "LOCAL INTEREST VIEW // XZ PLANE";
  if (!world) {
    context.fillStyle = "#7590a1";
    context.font = "600 13px system-ui";
    context.fillText("AWAITING VERIFIED INTEREST BASELINE", 24, 36);
    elements["map-scale"].textContent = "—";
    return;
  }

  const stepM = niceGridStep(mapView.pixelsPerMeter);
  const origin = projectMapPoint({ x: 0, z: 0 }, mapView, width, height);
  context.strokeStyle = "rgba(69, 122, 155, 0.15)";
  context.fillStyle = "rgba(117, 144, 161, 0.7)";
  context.font = "10px system-ui";
  context.lineWidth = 1;
  const stepPx = stepM * mapView.pixelsPerMeter;
  const startX = ((origin.x % stepPx) + stepPx) % stepPx;
  const startY = ((origin.y % stepPx) + stepPx) % stepPx;
  for (let x = startX; x < width; x += stepPx) {
    context.beginPath(); context.moveTo(x, 0); context.lineTo(x, height); context.stroke();
  }
  for (let y = startY; y < height; y += stepPx) {
    context.beginPath(); context.moveTo(0, y); context.lineTo(width, y); context.stroke();
  }
  elements["map-scale"].textContent = stepM >= 1000
    ? `GRID ${(stepM / 1000).toLocaleString()} km`
    : `GRID ${stepM.toLocaleString()} m`;

  for (const object of visibleMapObjects()) {
    const point = projectMapPoint(object.position, mapView, width, height);
    if (point.x < -50 || point.y < -50 || point.x > width + 50 || point.y > height + 50) {
      continue;
    }
    const physicalRadius = (object.radiusM ?? 0) * mapView.pixelsPerMeter;
    const radius = object.kind === "celestial"
      ? Math.max(4, Math.min(90, physicalRadius))
      : object.kind === "voxel" ? Math.max(2, Math.min(4, physicalRadius)) : 7;
    context.fillStyle = object.color;
    context.strokeStyle = "rgba(3, 7, 12, 0.9)";
    context.lineWidth = 2;
    if (object.kind === "player") {
      context.save();
      context.translate(point.x, point.y);
      context.beginPath(); context.moveTo(0, -9); context.lineTo(8, 7);
      context.lineTo(-8, 7); context.closePath(); context.fill(); context.stroke();
      context.restore();
    } else if (object.kind === "grid") {
      context.fillRect(point.x - radius, point.y - radius, radius * 2, radius * 2);
    } else if (object.kind === "voxel") {
      context.fillRect(point.x - radius, point.y - radius, radius * 2, radius * 2);
    } else {
      context.beginPath(); context.arc(point.x, point.y, radius, 0, Math.PI * 2);
      context.fill(); context.stroke();
    }
    const isSelected = selectedMapObject?.kind === object.kind &&
      selectedMapObject?.id === object.id;
    if (isSelected) {
      context.strokeStyle = "#ffffff";
      context.lineWidth = 2;
      context.strokeRect(point.x - radius - 5, point.y - radius - 5,
        radius * 2 + 10, radius * 2 + 10);
    }
    if (object.kind !== "voxel") {
      context.fillStyle = "#d8e8f1";
      context.font = "600 11px system-ui";
      context.fillText(object.label, point.x + radius + 6, point.y + 4);
    }
    mapMarkers.push({ ...point, hitRadius: radius + 6, object });
  }

  if (connectionPhase !== "live") {
    context.fillStyle = "rgba(117, 31, 38, 0.88)";
    context.fillRect(0, 0, width, 30);
    context.fillStyle = "#ffd3d6";
    context.font = "700 11px system-ui";
    context.fillText("STALE VIEW // RECONNECTING TO AUTHORITATIVE STREAM", 12, 19);
  }
}

function activity(text, error) {
  const item = document.createElement("li");
  item.textContent = text;
  if (error) item.style.color = "var(--red)";
  elements.activity.prepend(item);
  while (elements.activity.children.length > 10) {
    elements.activity.lastElementChild.remove();
  }
}

function start() {
  document.getElementById("resync").addEventListener("click", () => {
    socket?.send(JSON.stringify({ type: "request_snapshot" }));
  });
  document.getElementById("refine").disabled = true;
  document.getElementById("craft").disabled = true;
  document.getElementById("anchor").disabled = true;
  document.getElementById("stop").disabled = true;
  for (const mode of ["local", "universe"]) {
    elements[`map-mode-${mode}`].addEventListener("click", () => {
      mapMode = mode;
      elements["map-mode-local"].setAttribute("aria-pressed", String(mode === "local"));
      elements["map-mode-universe"].setAttribute("aria-pressed", String(mode === "universe"));
      fitCurrentMap();
      drawMap();
    });
  }
  elements["map-fit"].addEventListener("click", () => {
    fitCurrentMap();
    drawMap();
  });
  const zoom = (factor) => {
    mapView.pixelsPerMeter = Math.max(
      0.000_02,
      Math.min(100, mapView.pixelsPerMeter * factor),
    );
    mapViewInitialized = true;
    drawMap();
  };
  elements["map-zoom-in"].addEventListener("click", () => zoom(1.5));
  elements["map-zoom-out"].addEventListener("click", () => zoom(1 / 1.5));
  for (const layer of ["players", "grids", "voxels"]) {
    elements[`layer-${layer}`].addEventListener("change", drawMap);
  }
  const canvas = elements["universe-map"];
  const eventPoint = (event) => {
    const bounds = canvas.getBoundingClientRect();
    return {
      x: (event.clientX - bounds.left) * canvas.width / bounds.width,
      y: (event.clientY - bounds.top) * canvas.height / bounds.height,
    };
  };
  canvas.addEventListener("pointerdown", (event) => {
    const point = eventPoint(event);
    mapDrag = { pointerId: event.pointerId, start: point, last: point, moved: false };
    canvas.setPointerCapture(event.pointerId);
  });
  canvas.addEventListener("pointermove", (event) => {
    if (!mapDrag || mapDrag.pointerId !== event.pointerId) return;
    const point = eventPoint(event);
    const dx = point.x - mapDrag.last.x;
    const dy = point.y - mapDrag.last.y;
    if (Math.hypot(point.x - mapDrag.start.x, point.y - mapDrag.start.y) > 4) {
      mapDrag.moved = true;
    }
    if (mapDrag.moved) {
      mapView.x -= dx / mapView.pixelsPerMeter;
      mapView.z -= dy / mapView.pixelsPerMeter;
      mapViewInitialized = true;
      drawMap();
    }
    mapDrag.last = point;
  });
  const finishPointer = (event) => {
    if (!mapDrag || mapDrag.pointerId !== event.pointerId) return;
    const point = eventPoint(event);
    if (!mapDrag.moved) {
      const marker = nearestMapMarker(mapMarkers, point.x, point.y);
      if (marker) {
        selectedMapObject = {
          kind: marker.object.kind,
          id: marker.object.id,
        };
        if (marker.object.kind === "grid") selectedGridId = marker.object.id;
        render();
      }
    }
    mapDrag = undefined;
    canvas.releasePointerCapture(event.pointerId);
  };
  canvas.addEventListener("pointerup", finishPointer);
  canvas.addEventListener("pointercancel", finishPointer);
  canvas.addEventListener("wheel", (event) => {
    event.preventDefault();
    const point = eventPoint(event);
    const priorScale = mapView.pixelsPerMeter;
    const worldPoint = {
      x: mapView.x + (point.x - canvas.width / 2) / priorScale,
      z: mapView.z + (point.y - canvas.height / 2) / priorScale,
    };
    const nextScale = Math.max(0.000_02, Math.min(
      100,
      priorScale * (event.deltaY < 0 ? 1.25 : 0.8),
    ));
    mapView = {
      x: worldPoint.x - (point.x - canvas.width / 2) / nextScale,
      z: worldPoint.z - (point.y - canvas.height / 2) / nextScale,
      pixelsPerMeter: nextScale,
    };
    mapViewInitialized = true;
    drawMap();
  }, { passive: false });
  canvas.addEventListener("keydown", (event) => {
    if (event.key === "+" || event.key === "=") zoom(1.5);
    if (event.key === "-") zoom(1 / 1.5);
  });
  connect();
}

if (globalThis.__VERSE_BROWSER_TEST__) {
  globalThis.__VERSE_BROWSER_TEST_API__ = {
    canonicalPlayers,
    mergeMotionState,
    publicProjection,
    playerColor,
    playerPresentations,
    environmentForPlayer,
    protocolTupleMatches,
    registryBindingIsValid,
    exactAddressOffsetMeters,
    parseLosslessVerifiedJson,
    exactIntegerEquals,
    exactIntegerCompare,
    exactIntegerIsSuccessor,
    worldFromInterestBaseline,
    applyInterestDelta,
    mapObjectsForState,
    fitMapView,
    projectMapPoint,
    nearestMapMarker,
    commitVerifiedPresentation,
    presentCommittedPresentation,
    scheduleReconnectForTest: scheduleReconnect,
    resetReconnectForTest() {
      reconnectScheduled = false;
      reconnectAttempt = 0;
    },
    verifiedPresentationStateForTest() {
      return { connectionPhase, world, interestFrontier };
    },
    setRegistryForTest(registryValue, manifestValue, phase = "registry") {
      registry = JSON.parse(JSON.stringify(registryValue));
      universeManifest = JSON.parse(JSON.stringify(manifestValue));
      connectionPhase = phase;
    },
    setFrontierForTest(value) {
      interestFrontier = interestFrontierFrom(value);
    },
  };
} else {
  start();
}
