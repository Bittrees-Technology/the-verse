// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";

const url = process.argv[2] ?? "ws://127.0.0.1:17777/ws";
const expectedRoster = ["player-local", "player-remote"];
const sharedControlOperation = "two-player-e2e-shared-control-operation";
const sharedReleaseOperation = "two-player-e2e-shared-release-operation";
const remoteMiningOperation = "two-player-e2e-remote-mining-operation";
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
    this.socket.send(JSON.stringify(message));
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
      protocol_version: 11,
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
    assert.equal(welcome.protocol_version, 11);
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

function visibleVoxel(state, player) {
  const eye = playerEye(player);
  const candidates = [...state.voxels].sort(
    (left, right) =>
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
  assert.deepEqual(
    localMessage.snapshot,
    remoteMessage.snapshot,
    `${description} exposes identical shared state`,
  );
  return localMessage.snapshot;
}

async function waitForCommonMotion(
  local,
  remote,
  predicate,
  description,
  timeoutMillis = 12_000,
) {
  const motionPredicate = (message) =>
    message.type === "motion_state" &&
    message.motion.players !== undefined &&
    predicate(message.motion);
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

  assert.equal(
    localMessage.motion.world_hash,
    remoteMessage.motion.world_hash,
    `${description} converges on one authoritative hash`,
  );
  assert.deepEqual(
    localMessage.motion.players,
    remoteMessage.motion.players,
    `${description} exposes the same player poses and frontiers`,
  );
  assertCanonicalRoster(localMessage.motion, `${description} local motion`);
  assertCanonicalRoster(remoteMessage.motion, `${description} remote motion`);
  return localMessage.motion;
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
  return message.receipt;
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

async function run() {
  const local = new ProtocolClient("player-local");
  const remote = new ProtocolClient("player-remote");
  try {
    const [localSnapshot, remoteSnapshot] = await Promise.all([
      local.connect(),
      remote.connect(),
    ]);
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
    const target = visibleVoxel(localSnapshot, initialRemote);
    assert.ok(target, "the remote actor has a visible unanchored voxel");
    const targetedSnapshot = await aimActorAt(
      local,
      remote,
      localSnapshot,
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

    const initialPlayers = new Map(
      minedSnapshot.players.map((player) => [player.player_id, player]),
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
