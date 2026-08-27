// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";

const url = process.argv[2] ?? "ws://127.0.0.1:17777/ws";
const recoveryMode = process.argv[3] === "--verify-recovery";
const expectedRecoveryHash = process.argv[4];
const expectedRecoverySequence = Number.parseInt(process.argv[5] ?? "", 10);
const expectedRoster = ["player-local", "player-remote"];
const sharedControlOperation = "two-player-e2e-shared-control-operation";
const sharedReleaseOperation = "two-player-e2e-shared-release-operation";
const remoteMiningOperation = "two-player-e2e-remote-mining-operation";
const remoteRefiningOperation = "two-player-e2e-remote-refining-operation";
const remoteCraftingOperation = "two-player-e2e-remote-crafting-operation";
const remoteDamageOperation = "two-player-e2e-non-owner-damage-operation";
const PROTOCOL_VERSION = 14;
const CHARACTER_EYE_OFFSET = 1.62 - 1.8 / 2;
const CHARACTER_MAXIMUM_ANGULAR_SPEED = 2.5;
const TOOL_RANGE = 9.0;
const TARGET_ALIGNMENT_RADIANS = 0.006;
let targetingOperationSequence = 0;

class ProtocolClient {
  constructor(playerId) {
    this.playerId = playerId;
    this.socket = new WebSocket(url);
    this.buffered = [];
    this.waiters = [];
    this.lastReceiptEventSequence = 0;
    this.committedOperationSequence = 0;
    this.lastProjectedOperationSequence = 0;
    this.operationSequenceById = new Map();
    this.socket.addEventListener("message", (event) => {
      this.dispatch(JSON.parse(event.data));
    });
    this.socket.addEventListener("close", () => {
      this.rejectWaiters(new Error(`${this.playerId} socket closed`));
    });
    this.socket.addEventListener("error", () => {
      this.rejectWaiters(new Error(`${this.playerId} socket failed`));
    });
  }

  dispatch(message) {
    if (message.type === "snapshot" && message.snapshot.actor_private) {
      const frontier = message.snapshot.actor_private.committed_operation_sequence;
      assert.ok(
        Number.isSafeInteger(frontier) && frontier >= 0,
        `${this.playerId} receives a valid private operation frontier`,
      );
      assert.ok(
        frontier >= this.lastProjectedOperationSequence &&
          frontier >= this.committedOperationSequence,
        `${this.playerId} private operation frontier never regresses`,
      );
      this.lastProjectedOperationSequence = frontier;
      this.committedOperationSequence = frontier;
    }
    const index = this.waiters.findIndex((waiter) => waiter.predicate(message));
    if (index >= 0) {
      const [waiter] = this.waiters.splice(index, 1);
      clearTimeout(waiter.timeout);
      waiter.resolve(message);
    } else {
      this.buffered.push(message);
    }
  }

  rejectWaiters(error) {
    for (const waiter of this.waiters.splice(0)) {
      clearTimeout(waiter.timeout);
      waiter.reject(error);
    }
  }

  waitFor(predicate, description, timeoutMillis = 8_000) {
    const index = this.buffered.findIndex(predicate);
    if (index >= 0) {
      return Promise.resolve(this.buffered.splice(index, 1)[0]);
    }
    return new Promise((resolve, reject) => {
      const waiter = { predicate, resolve, reject, timeout: undefined };
      waiter.timeout = setTimeout(() => {
        const position = this.waiters.indexOf(waiter);
        if (position >= 0) this.waiters.splice(position, 1);
        reject(
          new Error(`${this.playerId} timed out waiting for ${description}`),
        );
      }, timeoutMillis);
      this.waiters.push(waiter);
    });
  }

  send(message) {
    let outbound = message;
    if (typeof message.operation_id === "string") {
      let operationSequence = message.operation_sequence;
      if (operationSequence === undefined) {
        operationSequence = this.operationSequenceById.get(message.operation_id);
      }
      if (operationSequence === undefined) {
        operationSequence = this.committedOperationSequence + 1;
      }
      assert.ok(
        Number.isSafeInteger(operationSequence) && operationSequence > 0,
        `${this.playerId} sends a positive operation sequence`,
      );
      this.operationSequenceById.set(message.operation_id, operationSequence);
      outbound = { ...message, operation_sequence: operationSequence };
    }
    this.socket.send(JSON.stringify(outbound));
    return outbound;
  }

  operationSequenceFor(operationId) {
    return this.operationSequenceById.get(operationId);
  }

  async connect() {
    await new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener(
        "error",
        () => reject(new Error(`failed to connect ${this.playerId} to ${url}`)),
        { once: true },
      );
    });
    this.send({
      type: "hello",
      protocol_version: PROTOCOL_VERSION,
      client_name: `node-two-player-e2e-${this.playerId}`,
      authentication: {
        kind: "local_development",
        player_id: this.playerId,
      },
    });
    const welcome = await this.waitFor(
      (message) => message.type === "welcome",
      "authenticated welcome",
    );
    assert.equal(welcome.protocol_version, PROTOCOL_VERSION);
    assert.deepEqual(welcome.session_role, {
      kind: "player",
      player_id: this.playerId,
    });
    return (
      await this.waitFor(
        (message) => message.type === "snapshot",
        "initial authoritative snapshot",
      )
    ).snapshot;
  }

  async close() {
    if (this.socket.readyState === WebSocket.CLOSED) return;
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(
        () => reject(new Error(`${this.playerId} did not close cleanly`)),
        2_000,
      );
      this.socket.addEventListener(
        "close",
        () => {
          clearTimeout(timeout);
          resolve();
        },
        { once: true },
      );
      this.socket.close(1000, "two-player smoke complete");
    });
  }
}

function rosterIds(state) {
  return state.players.map((player) => player.player_id);
}

function assertCanonicalRoster(state, description) {
  const roster = rosterIds(state);
  assert.deepEqual(
    roster,
    expectedRoster,
    `${description} has the expected roster`,
  );
  assert.deepEqual(
    roster,
    [...roster].sort(),
    `${description} roster is canonically sorted`,
  );
}

function publicProjection(state) {
  const { actor_private: _privateState, ...shared } = state;
  return shared;
}

function assertActorPrivate(state, expectedPlayerId, description) {
  assert.ok(state.actor_private, `${description} has an actor-private overlay`);
  assert.equal(
    state.actor_private.player?.player_id ?? state.actor_private.player_id,
    expectedPlayerId,
    `${description} is bound to the authenticated actor`,
  );
  if (state.actor_private.player) {
    assert.ok(
      Number.isSafeInteger(state.actor_private.committed_operation_sequence) &&
        state.actor_private.committed_operation_sequence >= 0,
      `${description} includes only its actor's valid operation frontier`,
    );
  } else {
    assert.equal(
      state.actor_private.committed_operation_sequence,
      undefined,
      `${description} motion does not disclose durable operation history`,
    );
  }
}

function combineActorSnapshots(local, remote, description) {
  assert.deepEqual(
    publicProjection(local),
    publicProjection(remote),
    `${description} exposes identical public state`,
  );
  assertActorPrivate(local, "player-local", `${description} local projection`);
  assertActorPrivate(remote, "player-remote", `${description} remote projection`);

  const localInventoryIds = new Set(
    local.actor_private.inventories.map((inventory) => inventory.inventory_id),
  );
  const remoteInventoryIds = new Set(
    remote.actor_private.inventories.map((inventory) => inventory.inventory_id),
  );
  assert.ok(
    [...localInventoryIds].every((inventoryId) => !remoteInventoryIds.has(inventoryId)),
    `${description} keeps actor inventory overlays disjoint`,
  );

  const privatePlayers = new Map([
    [local.actor_private.player.player_id, local.actor_private.player],
    [remote.actor_private.player.player_id, remote.actor_private.player],
  ]);
  const privateMasses = new Map(
    [
      ...local.actor_private.owned_grid_masses,
      ...remote.actor_private.owned_grid_masses,
    ].map((entry) => [entry.grid_id, entry.mass_kg]),
  );
  const shared = publicProjection(local);
  return {
    ...shared,
    player: local.actor_private.player,
    players: shared.players.map(
      (player) => privatePlayers.get(player.player_id) ?? player,
    ),
    grids: shared.grids.map((grid) => ({
      ...grid,
      ...(privateMasses.has(grid.grid_id)
        ? { mass_kg: privateMasses.get(grid.grid_id) }
        : {}),
    })),
    inventories: [
      ...local.actor_private.inventories,
      ...remote.actor_private.inventories,
    ].sort((left, right) => left.inventory_id.localeCompare(right.inventory_id)),
    death_drops: [
      ...local.actor_private.death_drops,
      ...remote.actor_private.death_drops,
    ].sort((left, right) => left.death_id.localeCompare(right.death_id)),
    conservation: { valid: shared.conservation_valid },
  };
}

function combineActorMotion(local, remote, description) {
  assert.deepEqual(
    publicProjection(local),
    publicProjection(remote),
    `${description} exposes identical public motion`,
  );
  assertActorPrivate(local, "player-local", `${description} local motion`);
  assertActorPrivate(remote, "player-remote", `${description} remote motion`);
  const privatePlayers = new Map([
    [local.actor_private.player_id, local.actor_private],
    [remote.actor_private.player_id, remote.actor_private],
  ]);
  const shared = publicProjection(local);
  return {
    ...shared,
    player: local.actor_private,
    players: shared.players.map(
      (player) => privatePlayers.get(player.player_id) ?? player,
    ),
  };
}

function playerById(motion, playerId) {
  return motion.players.find((player) => player.player_id === playerId);
}

function distanceSquared(left, right) {
  return (
    (left.x - right.x) ** 2 + (left.y - right.y) ** 2 + (left.z - right.z) ** 2
  );
}

function addVector(left, right) {
  return {
    x: left.x + right.x,
    y: left.y + right.y,
    z: left.z + right.z,
  };
}

function subtractVector(left, right) {
  return {
    x: left.x - right.x,
    y: left.y - right.y,
    z: left.z - right.z,
  };
}

function scaleVector(vector, scale) {
  return { x: vector.x * scale, y: vector.y * scale, z: vector.z * scale };
}

function vectorMagnitude(vector) {
  return Math.sqrt(vector.x ** 2 + vector.y ** 2 + vector.z ** 2);
}

function normalizeVector(vector) {
  const magnitude = vectorMagnitude(vector);
  assert.ok(magnitude > 1e-12, "a targeting direction has non-zero length");
  return scaleVector(vector, 1 / magnitude);
}

function dotVector(left, right) {
  return left.x * right.x + left.y * right.y + left.z * right.z;
}

function crossVector(left, right) {
  return {
    x: left.y * right.z - left.z * right.y,
    y: left.z * right.x - left.x * right.z,
    z: left.x * right.y - left.y * right.x,
  };
}

function rotateVector(quaternion, vector) {
  const tx = 2 * (quaternion.y * vector.z - quaternion.z * vector.y);
  const ty = 2 * (quaternion.z * vector.x - quaternion.x * vector.z);
  const tz = 2 * (quaternion.x * vector.y - quaternion.y * vector.x);
  return {
    x: vector.x + quaternion.w * tx + (quaternion.y * tz - quaternion.z * ty),
    y: vector.y + quaternion.w * ty + (quaternion.z * tx - quaternion.x * tz),
    z: vector.z + quaternion.w * tz + (quaternion.x * ty - quaternion.y * tx),
  };
}

function playerInventory(state, player) {
  return state.inventories.find(
    (inventory) => inventory.inventory_id === player.inventory_id,
  );
}

function playerUp(player) {
  if (player.locomotion.kind !== "eva") {
    return normalizeVector(player.locomotion.up);
  }
  return normalizeVector(
    rotateVector(player.orientation, { x: 0, y: 1, z: 0 }),
  );
}

function playerEye(player) {
  return addVector(
    player.position,
    scaleVector(playerUp(player), CHARACTER_EYE_OFFSET),
  );
}

function playerForward(player) {
  const pitch =
    player.locomotion.kind === "eva" ? 0 : player.locomotion.view_pitch_radians;
  return normalizeVector(
    rotateVector(player.orientation, {
      x: 0,
      y: Math.sin(pitch),
      z: -Math.cos(pitch),
    }),
  );
}

function coordinateKey(coordinate) {
  return [coordinate.x, coordinate.y, coordinate.z].join(",");
}

function compareText(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function compareCoordinate(left, right) {
  return left.x - right.x || left.y - right.y || left.z - right.z;
}

function rayUnitCube(origin, direction, center) {
  let entry = -Infinity;
  let exit = Infinity;
  for (const axis of ["x", "y", "z"]) {
    const minimum = center[axis] - 0.5;
    const maximum = center[axis] + 0.5;
    if (Math.abs(direction[axis]) <= 1e-12) {
      if (origin[axis] < minimum || origin[axis] > maximum) return undefined;
      continue;
    }
    let near = (minimum - origin[axis]) / direction[axis];
    let far = (maximum - origin[axis]) / direction[axis];
    if (near > far) [near, far] = [far, near];
    entry = Math.max(entry, near);
    exit = Math.min(exit, far);
    if (entry > exit) return undefined;
  }
  if (exit < 0 || entry > TOOL_RANGE) return undefined;
  return { distance: Math.max(0, entry) };
}

function canonicalRayHits(state, player, direction) {
  const eye = playerEye(player);
  const hits = [];
  for (const grid of state.grids) {
    const inverse = {
      x: -grid.orientation.x,
      y: -grid.orientation.y,
      z: -grid.orientation.z,
      w: grid.orientation.w,
    };
    const localOrigin = rotateVector(
      inverse,
      subtractVector(eye, grid.position),
    );
    const localDirection = rotateVector(inverse, direction);
    for (const block of grid.blocks) {
      const hit = rayUnitCube(localOrigin, localDirection, block.coordinate);
      if (hit) hits.push({ type: "block", ...hit, grid, block });
    }
  }
  for (const voxel of state.voxels) {
    const hit = rayUnitCube(eye, direction, voxel.coordinate);
    if (hit) hits.push({ type: "voxel", ...hit, voxel });
  }
  return hits.sort((left, right) => {
    const distance = left.distance - right.distance;
    if (Math.abs(distance) > 1e-9) return distance;
    if (left.type !== right.type) return left.type === "block" ? -1 : 1;
    if (left.type === "block") {
      return (
        compareText(left.grid.grid_id, right.grid.grid_id) ||
        compareText(left.block.block_id, right.block.block_id)
      );
    }
    return compareCoordinate(left.voxel.coordinate, right.voxel.coordinate);
  });
}

function visibleVoxel(state, player, preferredMaterial = undefined) {
  const eye = playerEye(player);
  const candidates = [...state.voxels].sort(
    (left, right) =>
      Number(right.material === preferredMaterial) -
        Number(left.material === preferredMaterial) ||
      distanceSquared(left.coordinate, eye) -
      distanceSquared(right.coordinate, eye),
  );
  for (const voxel of candidates) {
    const direction = normalizeVector(subtractVector(voxel.coordinate, eye));
    const hit = canonicalRayHits(state, player, direction)[0];
    if (
      hit?.type === "voxel" &&
      coordinateKey(hit.voxel.coordinate) === coordinateKey(voxel.coordinate)
    ) {
      return voxel;
    }
  }
  return undefined;
}

function gridBlockWorldPosition(grid, block) {
  return addVector(grid.position, rotateVector(grid.orientation, block.coordinate));
}

function visibleBlock(state, player) {
  const eye = playerEye(player);
  const candidates = state.grids
    .flatMap((grid) => grid.blocks.map((block) => ({ grid, block })))
    .sort(
      (left, right) =>
        distanceSquared(gridBlockWorldPosition(left.grid, left.block), eye) -
        distanceSquared(gridBlockWorldPosition(right.grid, right.block), eye),
    );
  for (const candidate of candidates) {
    const target = gridBlockWorldPosition(candidate.grid, candidate.block);
    const direction = normalizeVector(subtractVector(target, eye));
    const hit = canonicalRayHits(state, player, direction)[0];
    if (
      hit?.type === "block" &&
      hit.grid.grid_id === candidate.grid.grid_id &&
      hit.block.block_id === candidate.block.block_id
    ) {
      return { ...candidate, target };
    }
  }
  return undefined;
}

function nearestBlock(state, player) {
  const eye = playerEye(player);
  return state.grids
    .flatMap((grid) => grid.blocks.map((block) => ({ grid, block })))
    .sort(
      (left, right) =>
        distanceSquared(gridBlockWorldPosition(left.grid, left.block), eye) -
        distanceSquared(gridBlockWorldPosition(right.grid, right.block), eye),
    )[0];
}

async function waitForCommonSnapshot(
  local,
  remote,
  minimumEventSequence,
  description,
  timeoutMillis = 12_000,
) {
  const predicate = (message) =>
    message.type === "snapshot" &&
    message.snapshot.event_sequence >= minimumEventSequence;
  let [localMessage, remoteMessage] = await Promise.all([
    local.waitFor(predicate, `${description} on local`, timeoutMillis),
    remote.waitFor(predicate, `${description} on remote`, timeoutMillis),
  ]);
  while (
    localMessage.snapshot.event_sequence !==
    remoteMessage.snapshot.event_sequence
  ) {
    if (
      localMessage.snapshot.event_sequence <
      remoteMessage.snapshot.event_sequence
    ) {
      const minimum = remoteMessage.snapshot.event_sequence;
      localMessage = await local.waitFor(
        (message) =>
          predicate(message) && message.snapshot.event_sequence >= minimum,
        `${description} convergence on local`,
        timeoutMillis,
      );
    } else {
      const minimum = localMessage.snapshot.event_sequence;
      remoteMessage = await remote.waitFor(
        (message) =>
          predicate(message) && message.snapshot.event_sequence >= minimum,
        `${description} convergence on remote`,
        timeoutMillis,
      );
    }
  }
  assert.equal(
    localMessage.snapshot.world_hash,
    remoteMessage.snapshot.world_hash,
    `${description} converges on one structural hash`,
  );
  return combineActorSnapshots(
    localMessage.snapshot,
    remoteMessage.snapshot,
    description,
  );
}

async function waitForCommonMotion(
  local,
  remote,
  predicate,
  description,
  timeoutMillis = 12_000,
) {
  const minimumReceiptSequence = Math.max(
    local.lastReceiptEventSequence,
    remote.lastReceiptEventSequence,
  );
  const motionPredicate = (message) =>
    message.type === "motion_state" &&
    message.motion.players !== undefined &&
    message.motion.event_sequence >= minimumReceiptSequence;
  let [localMessage, remoteMessage] = await Promise.all([
    local.waitFor(motionPredicate, `${description} on local`, timeoutMillis),
    remote.waitFor(motionPredicate, `${description} on remote`, timeoutMillis),
  ]);

  while (
    localMessage.motion.event_sequence !== remoteMessage.motion.event_sequence
  ) {
    if (
      localMessage.motion.event_sequence < remoteMessage.motion.event_sequence
    ) {
      const minimum = remoteMessage.motion.event_sequence;
      localMessage = await local.waitFor(
        (message) =>
          motionPredicate(message) && message.motion.event_sequence >= minimum,
        `${description} convergence on local`,
        timeoutMillis,
      );
    } else {
      const minimum = localMessage.motion.event_sequence;
      remoteMessage = await remote.waitFor(
        (message) =>
          motionPredicate(message) && message.motion.event_sequence >= minimum,
        `${description} convergence on remote`,
        timeoutMillis,
      );
    }
  }

  for (;;) {
    assert.equal(
      localMessage.motion.world_hash,
      remoteMessage.motion.world_hash,
      `${description} converges on one authoritative hash`,
    );
    const combined = combineActorMotion(
      localMessage.motion,
      remoteMessage.motion,
      description,
    );
    assertCanonicalRoster(combined, `${description} combined motion`);
    if (predicate(combined)) return combined;

    const currentSequence = combined.event_sequence;
    [localMessage, remoteMessage] = await Promise.all([
      local.waitFor(
        (message) =>
          motionPredicate(message) &&
          message.motion.event_sequence > currentSequence,
        `${description} progress on local`,
        timeoutMillis,
      ),
      remote.waitFor(
        (message) =>
          motionPredicate(message) &&
          message.motion.event_sequence > currentSequence,
        `${description} progress on remote`,
        timeoutMillis,
      ),
    ]);
    while (
      localMessage.motion.event_sequence !== remoteMessage.motion.event_sequence
    ) {
      if (
        localMessage.motion.event_sequence < remoteMessage.motion.event_sequence
      ) {
        const minimum = remoteMessage.motion.event_sequence;
        localMessage = await local.waitFor(
          (message) =>
            motionPredicate(message) && message.motion.event_sequence >= minimum,
          `${description} progress convergence on local`,
          timeoutMillis,
        );
      } else {
        const minimum = localMessage.motion.event_sequence;
        remoteMessage = await remote.waitFor(
          (message) =>
            motionPredicate(message) && message.motion.event_sequence >= minimum,
          `${description} progress convergence on remote`,
          timeoutMillis,
        );
      }
    }
  }
}

function applyMotionToSnapshot(state, motion) {
  const movingPlayers = new Map(
    motion.players.map((player) => [player.player_id, player]),
  );
  const movingGrids = new Map(motion.grids.map((grid) => [grid.grid_id, grid]));
  return {
    ...state,
    event_sequence: motion.event_sequence,
    simulation_tick: motion.simulation_tick,
    world_hash: motion.world_hash,
    player: { ...state.player, ...motion.player },
    players: state.players.map((player) => ({
      ...player,
      ...(movingPlayers.get(player.player_id) ?? {}),
    })),
    grids: state.grids.map((grid) => ({
      ...grid,
      ...(movingGrids.get(grid.grid_id) ?? {}),
    })),
  };
}

async function waitForReceipt(client, operationId, description) {
  const message = await client.waitFor(
    (candidate) =>
      (candidate.type === "intent_accepted" &&
        candidate.receipt.operation_id === operationId) ||
      (candidate.type === "intent_rejected" &&
        candidate.operation_id === operationId),
    description,
  );
  if (message.type === "intent_rejected") {
    throw new Error(
      `${client.playerId} operation rejected: ${message.code}: ${message.message}`,
    );
  }
  assert.equal(
    message.receipt.operation_sequence,
    client.operationSequenceFor(operationId),
    `${description} echoes the actor-local operation sequence`,
  );
  client.committedOperationSequence = Math.max(
    client.committedOperationSequence,
    message.receipt.operation_sequence,
  );
  client.lastReceiptEventSequence = Math.max(
    client.lastReceiptEventSequence,
    message.receipt.event_sequence,
  );
  return message.receipt;
}

async function expectRejection(
  client,
  intent,
  expectedCode,
  description,
) {
  const outbound = client.send(intent);
  const message = await client.waitFor(
    (candidate) =>
      candidate.type === "intent_rejected" &&
      candidate.operation_id === intent.operation_id,
    description,
  );
  assert.equal(message.code, expectedCode, `${description} fails closed`);
  assert.equal(
    message.operation_sequence,
    outbound.operation_sequence,
    `${description} echoes the rejected operation sequence only to its requester`,
  );
  return message;
}

async function requestCommonSnapshot(local, remote, minimumSequence, description) {
  local.send({ type: "request_snapshot" });
  remote.send({ type: "request_snapshot" });
  return waitForCommonSnapshot(
    local,
    remote,
    minimumSequence,
    description,
  );
}

function controlFor(
  player,
  operationId,
  inputSequence,
  linearInput,
  angularInput = { x: 0, y: 0, z: 0 },
) {
  return {
    type: "set_player_control",
    operation_id: operationId,
    movement_epoch: player.movement_epoch,
    input_sequence: inputSequence,
    linear_input: linearInput,
    angular_input: angularInput,
    boost: false,
    jump: false,
    dampeners: true,
  };
}

async function aimActorAt(local, remote, state, playerId, target, description) {
  const client = playerId === local.playerId ? local : remote;
  let alignedSamples = 0;
  for (let attempt = 0; attempt < 420; attempt += 1) {
    const player = state.players.find(
      (candidate) => candidate.player_id === playerId,
    );
    assert.ok(player, `${description} actor remains in the canonical roster`);
    const forward = playerForward(player);
    const desired = normalizeVector(subtractVector(target, playerEye(player)));
    const cosine = Math.min(1, Math.max(-1, dotVector(forward, desired)));
    const angle = Math.acos(cosine);
    const angularSpeed = vectorMagnitude(player.angular_velocity);
    if (angle <= TARGET_ALIGNMENT_RADIANS && angularSpeed <= 0.025) {
      alignedSamples += 1;
      if (alignedSamples >= 2) return state;
    } else {
      alignedSamples = 0;
    }

    let axis = crossVector(forward, desired);
    if (vectorMagnitude(axis) <= 1e-9) {
      axis = playerUp(player);
    } else {
      axis = normalizeVector(axis);
    }
    const desiredAngularSpeed =
      angle <= TARGET_ALIGNMENT_RADIANS
        ? 0
        : Math.min(1.2, Math.max(0.04, angle * 4));
    const desiredWorldAngularVelocity = scaleVector(axis, desiredAngularSpeed);
    const localDesiredAngularVelocity = rotateVector(
      {
        x: -player.orientation.x,
        y: -player.orientation.y,
        z: -player.orientation.z,
        w: player.orientation.w,
      },
      desiredWorldAngularVelocity,
    );
    const angularInput = scaleVector(
      localDesiredAngularVelocity,
      1 / CHARACTER_MAXIMUM_ANGULAR_SPEED,
    );
    targetingOperationSequence += 1;
    const operationId = `two-player-target-${playerId}-${targetingOperationSequence}`;
    const inputSequence = player.last_received_input_sequence + 1;
    client.send(
      controlFor(
        player,
        operationId,
        inputSequence,
        { x: 0, y: 0, z: 0 },
        angularInput,
      ),
    );
    await waitForReceipt(client, operationId, `${description} control receipt`);
    const motion = await waitForCommonMotion(
      local,
      remote,
      (candidate) =>
        playerById(candidate, playerId)?.last_processed_input_sequence >=
        inputSequence,
      `${description} orientation integration`,
    );
    state = applyMotionToSnapshot(state, motion);
  }
  assert.fail(
    `${description} did not align within the deterministic control budget`,
  );
}

async function approachVisibleBlock(
  local,
  remote,
  state,
  playerId,
  description,
) {
  const client = playerId === local.playerId ? local : remote;
  for (let attempt = 0; attempt < 360; attempt += 1) {
    const player = state.players.find(
      (candidate) => candidate.player_id === playerId,
    );
    assert.ok(player, `${description} actor remains in the canonical roster`);
    const visible = visibleBlock(state, player);
    const speed = vectorMagnitude(player.linear_velocity);
    if (visible && speed <= 0.2) return state;

    let linearInput = { x: 0, y: 0, z: 0 };
    if (!visible) {
      const nearest = nearestBlock(state, player);
      assert.ok(nearest, `${description} has a grid block to approach`);
      const worldDirection = normalizeVector(
        subtractVector(
          gridBlockWorldPosition(nearest.grid, nearest.block),
          player.position,
        ),
      );
      linearInput = scaleVector(
        rotateVector(
          {
            x: -player.orientation.x,
            y: -player.orientation.y,
            z: -player.orientation.z,
            w: player.orientation.w,
          },
          worldDirection,
        ),
        0.45,
      );
    }

    targetingOperationSequence += 1;
    const operationId = `two-player-approach-${playerId}-${targetingOperationSequence}`;
    const inputSequence = player.last_received_input_sequence + 1;
    client.send(
      controlFor(
        player,
        operationId,
        inputSequence,
        linearInput,
        { x: 0, y: 0, z: 0 },
      ),
    );
    await waitForReceipt(client, operationId, `${description} control receipt`);
    const motion = await waitForCommonMotion(
      local,
      remote,
      (candidate) =>
        playerById(candidate, playerId)?.last_processed_input_sequence >=
        inputSequence,
      `${description} movement integration`,
    );
    state = applyMotionToSnapshot(state, motion);
  }
  assert.fail(`${description} did not reach a visible block and settle`);
}

async function run() {
  const local = new ProtocolClient("player-local");
  const remote = new ProtocolClient("player-remote");
  try {
    const [localProjection, remoteProjection] = await Promise.all([
      local.connect(),
      remote.connect(),
    ]);
    const localSnapshot = combineActorSnapshots(
      localProjection,
      remoteProjection,
      "initial snapshot",
    );
    const remoteSnapshot = localSnapshot;
    assertCanonicalRoster(localSnapshot, "local initial snapshot");
    assertCanonicalRoster(remoteSnapshot, "remote initial snapshot");
    assert.deepEqual(
      rosterIds(localSnapshot),
      rosterIds(remoteSnapshot),
      "both initial snapshots expose the same sorted roster",
    );
    assert.equal(
      localSnapshot.event_sequence,
      remoteSnapshot.event_sequence,
      "idle initial snapshots share one canonical event",
    );
    assert.equal(
      localSnapshot.world_hash,
      remoteSnapshot.world_hash,
      "initial snapshots have one authoritative hash",
    );

    const initialPrimary = localSnapshot.players.find(
      (player) => player.player_id === "player-local",
    );
    const initialRemote = localSnapshot.players.find(
      (player) => player.player_id === "player-remote",
    );
    const starterGrid = localSnapshot.grids.find(
      (grid) => grid.owner_player_id === "player-local",
    );
    assert.ok(starterGrid, "the starter grid exposes its canonical local owner");
    assert.ok(initialPrimary, "the local player is in the canonical roster");
    assert.ok(initialRemote, "the remote player is in the canonical roster");

    if (recoveryMode) {
      assert.ok(expectedRecoveryHash, "recovery verification receives a hash");
      assert.ok(
        Number.isSafeInteger(expectedRecoverySequence),
        "recovery verification receives an event sequence",
      );
      assert.equal(localSnapshot.world_hash, expectedRecoveryHash);
      assert.equal(localSnapshot.event_sequence, expectedRecoverySequence);
      console.log(
        JSON.stringify({
          result: "VERSE_TWO_PLAYER_RECOVERY_OK",
          players: rosterIds(localSnapshot),
          grid_owner: starterGrid.owner_player_id,
          world_hash: localSnapshot.world_hash,
          event_sequence: localSnapshot.event_sequence,
        }),
      );
      return;
    }

    const primaryInventory = playerInventory(localSnapshot, initialPrimary);
    const remoteInventory = playerInventory(localSnapshot, initialRemote);
    const firstStarterBlock = starterGrid.blocks[0];
    assert.ok(primaryInventory, "the local carried inventory is present");
    assert.ok(remoteInventory, "the remote carried inventory is present");
    assert.ok(firstStarterBlock, "the starter grid has a denial target");
    await expectRejection(
      remote,
      {
        type: "set_suit_mode",
        operation_sequence: remote.committedOperationSequence + 2,
        operation_id: "two-player-remote-operation-gap",
        helmet_closed: initialRemote.helmet_closed,
        jetpack_enabled: initialRemote.jetpack_enabled,
        magnetic_boots_enabled:
          initialRemote.locomotion.magnetic_boots_enabled,
      },
      "operation_sequence_gap",
      "remote actor operation gap",
    );
    assert.equal(
      local.committedOperationSequence,
      localProjection.actor_private.committed_operation_sequence,
      "a remote rejected sequence cannot advance the local private frontier",
    );
    assert.equal(
      remote.committedOperationSequence,
      remoteProjection.actor_private.committed_operation_sequence,
      "a rejected gap cannot advance the remote private frontier",
    );
    const deniedIntents = [
      {
        intent: {
          type: "refine_ore",
          operation_id: "two-player-deny-refine-primary",
          inventory_id: primaryInventory.inventory_id,
          batches: 1,
        },
        code: "inventory_access_denied",
      },
      {
        intent: {
          type: "craft_component",
          operation_id: "two-player-deny-craft-primary",
          inventory_id: primaryInventory.inventory_id,
          quantity: 1,
        },
        code: "inventory_access_denied",
      },
      {
        intent: {
          type: "transfer_inventory",
          operation_id: "two-player-deny-withdraw-primary",
          source_inventory_id: primaryInventory.inventory_id,
          destination_inventory_id: remoteInventory.inventory_id,
          resource: "component",
          quantity: 1,
        },
        code: "inventory_access_denied",
      },
      {
        intent: {
          type: "transfer_inventory",
          operation_id: "two-player-deny-deposit-primary",
          source_inventory_id: remoteInventory.inventory_id,
          destination_inventory_id: primaryInventory.inventory_id,
          resource: "ore",
          quantity: 1,
        },
        code: "inventory_access_denied",
      },
      {
        intent: {
          type: "build_block",
          operation_id: "two-player-deny-build-primary-grid",
          grid_id: starterGrid.grid_id,
          coordinate: firstStarterBlock.coordinate,
          kind: "structural",
          orientation: 0,
        },
        code: "grid_access_denied",
      },
      {
        intent: {
          type: "weld_block",
          operation_id: "two-player-deny-weld-primary-grid",
          grid_id: starterGrid.grid_id,
          block_id: firstStarterBlock.block_id,
        },
        code: "grid_access_denied",
      },
      {
        intent: {
          type: "set_grid_control",
          operation_id: "two-player-deny-control-primary-grid",
          grid_id: starterGrid.grid_id,
          linear_input: { x: 0, y: 0, z: 0 },
          angular_input: { x: 0, y: 0, z: 0 },
          dampeners: true,
        },
        code: "grid_access_denied",
      },
      {
        intent: {
          type: "toggle_grid_anchor",
          operation_id: "two-player-deny-anchor-primary-grid",
          grid_id: starterGrid.grid_id,
        },
        code: "grid_access_denied",
      },
    ];
    for (const { intent, code } of deniedIntents) {
      await expectRejection(
        remote,
        intent,
        code,
        `${intent.type} against another player's authority`,
      );
    }
    const denialSnapshot = await requestCommonSnapshot(
      local,
      remote,
      localSnapshot.event_sequence,
      "cross-player denial convergence",
    );
    assert.deepEqual(
      playerInventory(denialSnapshot, initialPrimary).contents,
      primaryInventory.contents,
      "denied operations do not spend the primary inventory",
    );
    assert.deepEqual(
      playerInventory(denialSnapshot, initialRemote).contents,
      remoteInventory.contents,
      "denied operations do not alter the remote inventory",
    );
    assert.deepEqual(
      denialSnapshot.grids.find((grid) => grid.grid_id === starterGrid.grid_id)
        .blocks,
      starterGrid.blocks,
      "denied operations do not change primary grid topology or integrity",
    );

    const localFrontierBeforeIsolation = local.committedOperationSequence;
    const remoteFrontierBeforeLocalIsolation = remote.committedOperationSequence;
    const localIsolationOperation = "two-player-local-frontier-isolation";
    local.send({
      type: "set_suit_mode",
      operation_id: localIsolationOperation,
      helmet_closed: initialPrimary.helmet_closed,
      jetpack_enabled: initialPrimary.jetpack_enabled,
      magnetic_boots_enabled:
        !initialPrimary.locomotion.magnetic_boots_enabled,
    });
    const localIsolationReceipt = await waitForReceipt(
      local,
      localIsolationOperation,
      "local-only operation frontier receipt",
    );
    const localIsolationSnapshot = await requestCommonSnapshot(
      local,
      remote,
      localIsolationReceipt.event_sequence,
      "local-only operation frontier publication",
    );
    assert.equal(
      local.lastProjectedOperationSequence,
      localFrontierBeforeIsolation + 1,
      "the local private projection advances for its accepted operation",
    );
    assert.equal(
      local.lastProjectedOperationSequence,
      localIsolationReceipt.operation_sequence,
    );
    assert.equal(
      remote.lastProjectedOperationSequence,
      remoteFrontierBeforeLocalIsolation,
      "the local accepted operation cannot advance the remote private frontier",
    );
    const localIsolationRestoreOperation =
      "two-player-local-frontier-isolation-restore";
    local.send({
      type: "set_suit_mode",
      operation_id: localIsolationRestoreOperation,
      helmet_closed: initialPrimary.helmet_closed,
      jetpack_enabled: initialPrimary.jetpack_enabled,
      magnetic_boots_enabled:
        initialPrimary.locomotion.magnetic_boots_enabled,
    });
    const localIsolationRestoreReceipt = await waitForReceipt(
      local,
      localIsolationRestoreOperation,
      "local-only operation frontier restore receipt",
    );
    const postDenialSnapshot = await requestCommonSnapshot(
      local,
      remote,
      localIsolationRestoreReceipt.event_sequence,
      "local-only operation frontier restore publication",
    );
    assert.equal(
      local.lastProjectedOperationSequence,
      localIsolationReceipt.operation_sequence + 1,
    );
    assert.equal(
      remote.lastProjectedOperationSequence,
      remoteFrontierBeforeLocalIsolation,
      "the local restore operation also remains actor-private",
    );
    assert.notEqual(
      localIsolationSnapshot.world_hash,
      postDenialSnapshot.world_hash,
      "the restore is a distinct canonical operation",
    );

    const damageStagingSnapshot = await approachVisibleBlock(
      local,
      remote,
      postDenialSnapshot,
      "player-remote",
      "non-owner damage approach",
    );
    const denialRemote = damageStagingSnapshot.players.find(
      (player) => player.player_id === "player-remote",
    );
    const damageTarget = visibleBlock(damageStagingSnapshot, denialRemote);
    assert.ok(
      damageTarget,
      "the remote actor has a closest-visible non-owned block",
    );
    const damageTargetedSnapshot = await aimActorAt(
      local,
      remote,
      damageStagingSnapshot,
      "player-remote",
      damageTarget.target,
      "non-owner damage target",
    );
    const damageActor = damageTargetedSnapshot.players.find(
      (player) => player.player_id === "player-remote",
    );
    const canonicalDamageHit = canonicalRayHits(
      damageTargetedSnapshot,
      damageActor,
      playerForward(damageActor),
    )[0];
    assert.equal(canonicalDamageHit?.type, "block");
    const damagedGridId = canonicalDamageHit.grid.grid_id;
    const damagedBlockId = canonicalDamageHit.block.block_id;
    const damageExperienceBefore = damageActor.experience;
    const primaryExperienceBeforeDamage = damageTargetedSnapshot.players.find(
      (player) => player.player_id === "player-local",
    ).experience;
    remote.send({
      type: "damage_block",
      operation_id: remoteDamageOperation,
      grid_id: damagedGridId,
      block_id: damagedBlockId,
    });
    const damageReceipt = await waitForReceipt(
      remote,
      remoteDamageOperation,
      "non-owner PvP damage receipt",
    );
    const damageSnapshot = await waitForCommonSnapshot(
      local,
      remote,
      damageReceipt.event_sequence,
      "non-owner PvP damage publication",
    );
    const damagedGrid = damageSnapshot.grids.find(
      (grid) =>
        grid.grid_id === damagedGridId ||
        grid.blocks.some((block) => block.block_id === damagedBlockId),
    );
    const damagedBlock = damagedGrid?.blocks.find(
      (block) => block.block_id === damagedBlockId,
    );
    assert.ok(
      damagedBlock === undefined ||
        damagedBlock.health < canonicalDamageHit.block.health,
      "non-owner closest-hit damage changes only the targeted integrity or removes the block",
    );
    assert.ok(
      damageSnapshot.grids.every(
        (grid) => grid.owner_player_id === "player-local",
      ),
      "PvP damage and any resulting split do not transfer ownership",
    );
    assert.equal(
      damageSnapshot.players.find(
        (player) => player.player_id === "player-remote",
      ).experience,
      damageExperienceBefore,
      "non-owner PvP damage awards no attacker experience",
    );
    assert.equal(
      damageSnapshot.players.find(
        (player) => player.player_id === "player-local",
      ).experience,
      primaryExperienceBeforeDamage,
      "non-owner PvP damage awards no owner experience",
    );

    const remoteAfterDamage = damageSnapshot.players.find(
      (player) => player.player_id === "player-remote",
    );
    const target =
      visibleVoxel(damageSnapshot, remoteAfterDamage, "ferrite_ore") ??
      visibleVoxel(damageSnapshot, remoteAfterDamage);
    assert.ok(target, "the remote actor has a visible unanchored voxel");
    const targetedSnapshot = await aimActorAt(
      local,
      remote,
      damageSnapshot,
      "player-remote",
      target.coordinate,
      "remote mining target",
    );
    const targetedRemote = targetedSnapshot.players.find(
      (player) => player.player_id === "player-remote",
    );
    const canonicalMiningHit = canonicalRayHits(
      targetedSnapshot,
      targetedRemote,
      playerForward(targetedRemote),
    )[0];
    assert.equal(canonicalMiningHit?.type, "voxel");
    assert.equal(
      coordinateKey(canonicalMiningHit.voxel.coordinate),
      coordinateKey(target.coordinate),
      "the remote pilot's canonical eye ray resolves to the requested voxel",
    );
    const primaryInventoryBefore = structuredClone(
      playerInventory(targetedSnapshot, initialPrimary).contents,
    );
    const remoteInventoryBefore = structuredClone(
      playerInventory(targetedSnapshot, initialRemote).contents,
    );
    const localFrontierBeforeRemoteMining = local.committedOperationSequence;
    const remoteFrontierBeforeMining = remote.committedOperationSequence;
    remote.send({
      type: "mine_voxel",
      operation_id: remoteMiningOperation,
      coordinate: target.coordinate,
    });
    const miningReceipt = await waitForReceipt(
      remote,
      remoteMiningOperation,
      "remote mining receipt",
    );
    const minedSnapshot = await waitForCommonSnapshot(
      local,
      remote,
      miningReceipt.event_sequence,
      "remote mining publication",
    );
    assert.equal(
      remote.lastProjectedOperationSequence,
      remoteFrontierBeforeMining + 1,
      "the remote private projection advances for remote mining",
    );
    assert.equal(
      remote.lastProjectedOperationSequence,
      miningReceipt.operation_sequence,
    );
    assert.equal(
      local.lastProjectedOperationSequence,
      localFrontierBeforeRemoteMining,
      "remote mining cannot advance the local private operation frontier",
    );
    const minedPrimary = minedSnapshot.players.find(
      (player) => player.player_id === "player-local",
    );
    const minedRemote = minedSnapshot.players.find(
      (player) => player.player_id === "player-remote",
    );
    assert.ok(
      !minedSnapshot.voxels.some(
        (voxel) =>
          voxel.coordinate.x === target.coordinate.x &&
          voxel.coordinate.y === target.coordinate.y &&
          voxel.coordinate.z === target.coordinate.z,
      ),
      "the shared voxel is removed exactly once",
    );
    assert.deepEqual(
      playerInventory(minedSnapshot, minedPrimary).contents,
      primaryInventoryBefore,
      "remote mining does not credit the primary inventory",
    );
    assert.ok(
      playerInventory(minedSnapshot, minedRemote).contents.ore >
        remoteInventoryBefore.ore,
      "remote mining credits the remote carried inventory",
    );
    assert.equal(
      minedPrimary.experience,
      initialPrimary.experience,
      "remote mining does not credit primary experience",
    );
    assert.ok(
      minedRemote.experience > initialRemote.experience &&
        minedRemote.career.voxels_mined ===
          initialRemote.career.voxels_mined + 1,
      "remote mining credits only the remote career",
    );
    remote.send({
      type: "mine_voxel",
      operation_id: remoteMiningOperation,
      coordinate: target.coordinate,
    });
    assert.deepEqual(
      await waitForReceipt(
        remote,
        remoteMiningOperation,
        "idempotent remote mining retry receipt",
      ),
      miningReceipt,
      "remote mining retry returns the original actor-scoped receipt",
    );
    await expectRejection(
      remote,
      {
        type: "mine_voxel",
        operation_id: remoteMiningOperation,
        coordinate: {
          x: target.coordinate.x + 1,
          y: target.coordinate.y,
          z: target.coordinate.z,
        },
      },
      "operation_conflict",
      "changed-payload remote mining retry",
    );

    const reusedOperationSequence = remote.committedOperationSequence + 1;
    const localFrontierBeforeIdReuse = local.committedOperationSequence;
    remote.send({
      type: "set_suit_mode",
      operation_sequence: reusedOperationSequence,
      operation_id: remoteMiningOperation,
      helmet_closed: minedRemote.helmet_closed,
      jetpack_enabled: minedRemote.jetpack_enabled,
      magnetic_boots_enabled: !minedRemote.locomotion.magnetic_boots_enabled,
    });
    const reusedIdReceipt = await waitForReceipt(
      remote,
      remoteMiningOperation,
      "diagnostic operation ID reuse at the next sequence",
    );
    assert.equal(reusedIdReceipt.operation_sequence, reusedOperationSequence);
    assert.notDeepEqual(
      reusedIdReceipt,
      miningReceipt,
      "the same diagnostic ID at a new sequence is a distinct operation",
    );
    const reusedIdSnapshot = await waitForCommonSnapshot(
      local,
      remote,
      reusedIdReceipt.event_sequence,
      "same-ID next-sequence publication",
    );
    assert.equal(
      remote.lastProjectedOperationSequence,
      reusedOperationSequence,
    );
    assert.equal(
      local.lastProjectedOperationSequence,
      localFrontierBeforeIdReuse,
      "same-ID reuse remains isolated to the committing actor",
    );
    const reuseRestoreOperation = "two-player-e2e-id-reuse-restore";
    remote.send({
      type: "set_suit_mode",
      operation_id: reuseRestoreOperation,
      helmet_closed: minedRemote.helmet_closed,
      jetpack_enabled: minedRemote.jetpack_enabled,
      magnetic_boots_enabled: minedRemote.locomotion.magnetic_boots_enabled,
    });
    const reuseRestoreReceipt = await waitForReceipt(
      remote,
      reuseRestoreOperation,
      "same-ID probe suit restore receipt",
    );
    let industrySnapshot = await waitForCommonSnapshot(
      local,
      remote,
      reuseRestoreReceipt.event_sequence,
      "same-ID probe suit restore publication",
    );
    assert.equal(
      remote.lastProjectedOperationSequence,
      reusedOperationSequence + 1,
    );
    assert.equal(
      local.lastProjectedOperationSequence,
      localFrontierBeforeIdReuse,
      "the restore after same-ID reuse stays isolated to the remote actor",
    );
    assert.notEqual(
      reusedIdSnapshot.world_hash,
      industrySnapshot.world_hash,
      "the remote suit restore is a distinct canonical operation",
    );
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const industryRemote = industrySnapshot.players.find(
        (player) => player.player_id === "player-remote",
      );
      if (playerInventory(industrySnapshot, industryRemote).contents.ore >= 2) {
        break;
      }
      const additionalTarget = visibleVoxel(industrySnapshot, industryRemote);
      assert.ok(
        additionalTarget,
        "the remote actor can expose enough ore for its own proof production",
      );
      industrySnapshot = await aimActorAt(
        local,
        remote,
        industrySnapshot,
        "player-remote",
        additionalTarget.coordinate,
        `remote additional mining target ${attempt + 1}`,
      );
      const operationId = `two-player-e2e-remote-mining-extra-${attempt + 1}`;
      remote.send({
        type: "mine_voxel",
        operation_id: operationId,
        coordinate: additionalTarget.coordinate,
      });
      const receipt = await waitForReceipt(
        remote,
        operationId,
        `remote additional mining receipt ${attempt + 1}`,
      );
      industrySnapshot = await waitForCommonSnapshot(
        local,
        remote,
        receipt.event_sequence,
        `remote additional mining publication ${attempt + 1}`,
      );
    }

    const productionPrimaryBefore = industrySnapshot.players.find(
      (player) => player.player_id === "player-local",
    );
    const productionRemoteBefore = industrySnapshot.players.find(
      (player) => player.player_id === "player-remote",
    );
    const productionPrimaryInventoryBefore = structuredClone(
      playerInventory(industrySnapshot, productionPrimaryBefore).contents,
    );
    const productionRemoteInventoryBefore = structuredClone(
      playerInventory(industrySnapshot, productionRemoteBefore).contents,
    );
    assert.ok(
      productionRemoteInventoryBefore.ore >= 2,
      "the remote actor owns enough mined ore for one proof refining batch",
    );
    remote.send({
      type: "refine_ore",
      operation_id: remoteRefiningOperation,
      inventory_id: productionRemoteBefore.inventory_id,
      batches: 1,
    });
    const refiningReceipt = await waitForReceipt(
      remote,
      remoteRefiningOperation,
      "remote actor-owned refining receipt",
    );
    const refinedSnapshot = await waitForCommonSnapshot(
      local,
      remote,
      refiningReceipt.event_sequence,
      "remote actor-owned refining publication",
    );
    const refinedRemote = refinedSnapshot.players.find(
      (player) => player.player_id === "player-remote",
    );
    const refinedInventory = playerInventory(refinedSnapshot, refinedRemote);
    assert.equal(
      refinedInventory.contents.ore,
      productionRemoteInventoryBefore.ore - 2,
    );
    assert.equal(
      refinedInventory.contents.refined_material,
      productionRemoteInventoryBefore.refined_material + 1,
    );
    assert.equal(
      refinedRemote.experience,
      productionRemoteBefore.experience + 12,
      "proof refining credits only its authenticated actor",
    );
    assert.equal(
      refinedRemote.career.refining_batches,
      productionRemoteBefore.career.refining_batches + 1,
    );
    assert.deepEqual(
      playerInventory(
        refinedSnapshot,
        refinedSnapshot.players.find(
          (player) => player.player_id === "player-local",
        ),
      ).contents,
      productionPrimaryInventoryBefore,
      "remote refining cannot spend or credit the primary inventory",
    );

    remote.send({
      type: "craft_component",
      operation_id: remoteCraftingOperation,
      inventory_id: refinedRemote.inventory_id,
      quantity: 1,
    });
    const craftingReceipt = await waitForReceipt(
      remote,
      remoteCraftingOperation,
      "remote actor-owned crafting receipt",
    );
    const craftedSnapshot = await waitForCommonSnapshot(
      local,
      remote,
      craftingReceipt.event_sequence,
      "remote actor-owned crafting publication",
    );
    const craftedRemote = craftedSnapshot.players.find(
      (player) => player.player_id === "player-remote",
    );
    const craftedInventory = playerInventory(craftedSnapshot, craftedRemote);
    assert.equal(
      craftedInventory.contents.refined_material,
      refinedInventory.contents.refined_material - 1,
    );
    assert.equal(
      craftedInventory.contents.components,
      refinedInventory.contents.components + 1,
    );
    assert.equal(
      craftedRemote.experience,
      refinedRemote.experience + 18,
      "proof crafting credits only its authenticated actor",
    );
    assert.equal(
      craftedRemote.career.components_crafted,
      refinedRemote.career.components_crafted + 1,
    );
    assert.equal(
      craftedSnapshot.players.find(
        (player) => player.player_id === "player-local",
      ).experience,
      productionPrimaryBefore.experience,
      "remote production never credits the primary career",
    );

    const initialPlayers = new Map(
      craftedSnapshot.players.map((player) => [player.player_id, player]),
    );
    const localPlayer = initialPlayers.get("player-local");
    const remotePlayer = initialPlayers.get("player-remote");
    assert.ok(localPlayer, "local actor exists in the initial motion roster");
    assert.ok(remotePlayer, "remote actor exists in the initial motion roster");

    const localSequence = localPlayer.last_received_input_sequence + 1;
    const remoteSequence = remotePlayer.last_received_input_sequence + 1;
    const localControl = controlFor(
      localPlayer,
      sharedControlOperation,
      localSequence,
      { x: 1, y: 0, z: 0 },
    );
    const remoteControl = controlFor(
      remotePlayer,
      sharedControlOperation,
      remoteSequence,
      { x: -1, y: 0, z: 0 },
    );
    local.send(localControl);
    remote.send(remoteControl);
    const [localReceipt, remoteReceipt] = await Promise.all([
      waitForReceipt(local, sharedControlOperation, "local control receipt"),
      waitForReceipt(remote, sharedControlOperation, "remote control receipt"),
    ]);
    assert.equal(localReceipt.operation_id, remoteReceipt.operation_id);

    const moved = await waitForCommonMotion(
      local,
      remote,
      (motion) => {
        const localMotion = playerById(motion, "player-local");
        const remoteMotion = playerById(motion, "player-remote");
        return (
          localMotion?.last_received_input_sequence >= localSequence &&
          localMotion?.last_processed_input_sequence >= localSequence &&
          remoteMotion?.last_received_input_sequence >= remoteSequence &&
          remoteMotion?.last_processed_input_sequence >= remoteSequence
        );
      },
      "independent control processing",
    );
    const movedLocal = playerById(moved, "player-local");
    const movedRemote = playerById(moved, "player-remote");
    assert.ok(
      distanceSquared(movedLocal.position, localPlayer.position) > 1e-12,
      "local pose advances through motion.players",
    );
    assert.ok(
      distanceSquared(movedRemote.position, remotePlayer.position) > 1e-12,
      "remote pose advances through motion.players",
    );

    local.send(localControl);
    const retryReceipt = await waitForReceipt(
      local,
      sharedControlOperation,
      "idempotent local control retry receipt",
    );
    assert.deepEqual(
      retryReceipt,
      localReceipt,
      "an actor-scoped retry returns its original receipt",
    );

    const releaseLocal = controlFor(
      movedLocal,
      sharedReleaseOperation,
      localSequence + 1,
      { x: 0, y: 0, z: 0 },
    );
    const releaseRemote = controlFor(
      movedRemote,
      sharedReleaseOperation,
      remoteSequence + 1,
      { x: 0, y: 0, z: 0 },
    );
    local.send(releaseLocal);
    remote.send(releaseRemote);
    await Promise.all([
      waitForReceipt(local, sharedReleaseOperation, "local release receipt"),
      waitForReceipt(remote, sharedReleaseOperation, "remote release receipt"),
    ]);
    const settled = await waitForCommonMotion(
      local,
      remote,
      (motion) => {
        const localMotion = playerById(motion, "player-local");
        const remoteMotion = playerById(motion, "player-remote");
        return (
          localMotion?.last_processed_input_sequence >= localSequence + 1 &&
          remoteMotion?.last_processed_input_sequence >= remoteSequence + 1
        );
      },
      "neutral control convergence",
    );

    console.log(
      JSON.stringify({
        result: "VERSE_TWO_PLAYER_E2E_OK",
        players: rosterIds(settled),
        initial_world_hash: localSnapshot.world_hash,
        mined_world_hash: minedSnapshot.world_hash,
        industry_world_hash: craftedSnapshot.world_hash,
        final_world_hash: settled.world_hash,
        event_sequence: settled.event_sequence,
        local_input_frontier: localSequence + 1,
        remote_input_frontier: remoteSequence + 1,
      }),
    );
  } finally {
    await Promise.allSettled([local.close(), remote.close()]);
  }
}

run().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
