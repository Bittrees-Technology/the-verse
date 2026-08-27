// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";
import { COMPATIBILITY, Protocol16InterestStream } from "./interest-stream.mjs";

const url = process.argv[2] ?? "ws://127.0.0.1:17777/ws";
const socket = new WebSocket(url);
const buffered = [];
const waiters = [];
let operationSequence = 0;
let committedOperationSequence = 0;
let authoritativeWorld;
const PROTOCOL_VERSION = COMPATIBILITY.protocol_version;
const interestStream = new Protocol16InterestStream({
  expectedPlayerId: "player-local",
  send: (message) => socket.send(JSON.stringify(message)),
});
const CHARACTER_EYE_OFFSET = 1.62 - 1.8 / 2;
const CHARACTER_MAXIMUM_SPEED = 12.0;
const CHARACTER_TEST_APPROACH_SPEED = 4.0;
const CHARACTER_MAXIMUM_ANGULAR_SPEED = 2.5;
const TOOL_RANGE = 9.0;
const TARGET_ALIGNMENT_RADIANS = 0.006;

function dispatch(rawMessage) {
  const message = interestStream.receive(rawMessage);
  const index = waiters.findIndex((waiter) => waiter.predicate(message));
  if (index >= 0) {
    const [waiter] = waiters.splice(index, 1);
    clearTimeout(waiter.timeout);
    waiter.resolve(message);
  } else {
    buffered.push(message);
  }
}

function waitFor(predicate, description, timeoutMillis = 8_000) {
  const index = buffered.findIndex(predicate);
  if (index >= 0) {
    return Promise.resolve(buffered.splice(index, 1)[0]);
  }
  return new Promise((resolve, reject) => {
    const waiter = { predicate, resolve, timeout: undefined };
    waiter.timeout = setTimeout(() => {
      const position = waiters.indexOf(waiter);
      if (position >= 0) waiters.splice(position, 1);
      reject(new Error("timed out waiting for " + description));
    }, timeoutMillis);
    waiters.push(waiter);
  });
}

function operationId(prefix) {
  operationSequence += 1;
  return ["e2e", prefix, Date.now(), operationSequence].join("-");
}

function send(message) {
  socket.send(JSON.stringify(message));
}

function hydrateActorSnapshot(snapshot) {
  const privateState = snapshot.actor_private;
  assert.ok(
    privateState,
    "an authenticated pilot receives an actor-private overlay",
  );
  assert.equal(privateState.player.player_id, "player-local");
  assert.ok(
    Number.isSafeInteger(privateState.committed_operation_sequence) &&
      privateState.committed_operation_sequence >= 0,
    "the private overlay carries a valid actor-local operation frontier",
  );
  assert.ok(
    privateState.committed_operation_sequence >= committedOperationSequence,
    "a full snapshot cannot regress the actor-local operation frontier",
  );
  committedOperationSequence = privateState.committed_operation_sequence;
  return snapshot;
}

async function waitForWorld(predicate, description, timeoutMillis = 12_000) {
  const deadline = Date.now() + timeoutMillis;
  while (Date.now() < deadline) {
    const state = await waitFor(
      (message) =>
        message.type === "interest_state" &&
        message.state.event_sequence >=
          (authoritativeWorld?.event_sequence ?? 0) &&
        message.state.actor_private?.committed_operation_sequence >=
          committedOperationSequence,
      description,
      Math.max(1, deadline - Date.now()),
    );
    authoritativeWorld = hydrateActorSnapshot(state.state);
    if (predicate(authoritativeWorld)) return authoritativeWorld;
  }
  throw new Error("timed out waiting for " + description);
}

async function intent(type, payload) {
  const operation_id = operationId(type);
  const operation_sequence = committedOperationSequence + 1;
  send({ type, operation_sequence, operation_id, ...payload });
  const result = await waitFor(
    (message) =>
      (message.type === "intent_accepted" &&
        message.receipt.operation_id === operation_id) ||
      (message.type === "intent_rejected" &&
        message.operation_id === operation_id),
    type + " receipt",
  );
  if (result.type === "intent_rejected") {
    throw new Error(
      [type, " rejected: ", result.code, ": ", result.message].join(""),
    );
  }
  assert.equal(result.receipt.operation_sequence, operation_sequence);
  committedOperationSequence = operation_sequence;
  const eventSequence = result.receipt.event_sequence;
  const minimumStateSequence = Math.max(
    eventSequence,
    authoritativeWorld?.event_sequence ?? 0,
  );
  const state = await waitFor(
    (message) =>
      message.type === "interest_state" &&
      message.state.event_sequence >= minimumStateSequence,
    type + " authoritative state",
  );
  authoritativeWorld = hydrateActorSnapshot(state.state);
  assert.equal(authoritativeWorld.conservation.valid, true);
  return authoritativeWorld;
}

async function waitForMotionAfter(simulationTick, description) {
  const minimumEventSequence = authoritativeWorld?.event_sequence ?? 0;
  const state = await waitFor(
    (message) =>
      message.type === "interest_state" &&
      message.state.simulation_tick > simulationTick &&
      message.state.event_sequence >= minimumEventSequence,
    description,
  );
  authoritativeWorld = hydrateActorSnapshot(state.state);
  return authoritativeWorld;
}

async function expectRejectedIntent(type, payload, expectedCode) {
  const operation_id = operationId(`rejected-${type}`);
  const operation_sequence = committedOperationSequence + 1;
  const sequence = authoritativeWorld.event_sequence;
  const hash = authoritativeWorld.world_hash;
  send({ type, operation_sequence, operation_id, ...payload });
  const rejection = await waitFor(
    (message) =>
      message.type === "intent_rejected" &&
      message.operation_id === operation_id,
    `${type} rejection`,
  );
  assert.equal(rejection.code, expectedCode);
  assert.equal(rejection.operation_sequence, operation_sequence);
  assert.equal(authoritativeWorld.event_sequence, sequence);
  assert.equal(authoritativeWorld.world_hash, hash);
  return rejection;
}

async function waitForCanonicalIncapacitation(world) {
  const startingOxygen = world.player.suit_oxygen_milli;
  const wallDeadline = Date.now() + 90_000;
  let observedOxygen = startingOxygen;
  let observedEventSequence = world.event_sequence;
  while (Date.now() < wallDeadline) {
    const remaining = wallDeadline - Date.now();
    const state = await waitFor(
      (message) =>
        message.type === "interest_state" &&
        message.state.event_sequence > observedEventSequence &&
        (message.state.actor_private?.player.life_state.kind ===
          "incapacitated" ||
          message.state.actor_private?.player.suit_oxygen_milli <
            observedOxygen),
      "canonical oxygen depletion progress",
      Math.min(8_000, remaining),
    );
    world = hydrateActorSnapshot(state.state);
    authoritativeWorld = world;
    const depleted = observedOxygen - world.player.suit_oxygen_milli;
    assert.ok(depleted > 0, "each observed oxygen snapshot makes progress");
    if (world.player.life_state.kind === "incapacitated") {
      assert.equal(world.player.suit_oxygen_milli, 0);
      assert.equal(depleted, observedOxygen);
    } else {
      assert.equal(
        depleted % 40,
        0,
        "open-vacuum oxygen depletion remains an exact multiple of 40 milli",
      );
    }
    observedOxygen = world.player.suit_oxygen_milli;
    observedEventSequence = world.event_sequence;
    if (world.player.life_state.kind === "incapacitated") return world;
  }
  throw new Error(
    "timed out while authoritative oxygen depletion was progressing",
  );
}

function playerInventory(world) {
  return world.inventories.find(
    (inventory) => inventory.inventory_id === "inventory-player-local",
  );
}

function distanceSquared(left, right) {
  const x = left.x - right.x;
  const y = left.y - right.y;
  const z = left.z - right.z;
  return x * x + y * y + z * z;
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

function quaternionAngularDistance(left, right) {
  const dot = Math.abs(
    left.x * right.x + left.y * right.y + left.z * right.z + left.w * right.w,
  );
  return 2 * Math.acos(Math.min(1, Math.max(0, dot)));
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

function rayUnitCube(origin, direction, center) {
  let entry = -Infinity;
  let exit = Infinity;
  let entryNormal;
  for (const axis of ["x", "y", "z"]) {
    const minimum = center[axis] - 0.5;
    const maximum = center[axis] + 0.5;
    if (Math.abs(direction[axis]) <= 1e-12) {
      if (origin[axis] < minimum || origin[axis] > maximum) return undefined;
      continue;
    }
    let near = (minimum - origin[axis]) / direction[axis];
    let far = (maximum - origin[axis]) / direction[axis];
    let normalSign = -1;
    if (near > far) {
      [near, far] = [far, near];
      normalSign = 1;
    }
    if (near > entry) {
      entry = near;
      entryNormal = { x: 0, y: 0, z: 0 };
      entryNormal[axis] = normalSign;
    }
    exit = Math.min(exit, far);
    if (entry > exit) return undefined;
  }
  if (exit < 0 || entry > TOOL_RANGE) return undefined;
  return {
    distance: Math.max(0, entry),
    normal: entryNormal ?? { x: 0, y: 0, z: 0 },
  };
}

function gridBlockWorldPosition(grid, block) {
  return addVector(
    grid.position,
    rotateVector(grid.orientation, block.coordinate),
  );
}

function canonicalRayHits(world, player, direction = playerForward(player)) {
  const eye = playerEye(player);
  const hits = [];
  for (const grid of world.grids) {
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
      if (hit) {
        hits.push({
          type: "block",
          ...hit,
          grid,
          block,
          worldPosition: gridBlockWorldPosition(grid, block),
        });
      }
    }
  }
  for (const voxel of world.voxels) {
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

function visibleVoxel(world, exclusions = new Set()) {
  const eye = playerEye(world.player);
  const candidates = world.voxels
    .filter((voxel) => !exclusions.has(coordinateKey(voxel.coordinate)))
    .sort(
      (left, right) =>
        distanceSquared(left.coordinate, eye) -
        distanceSquared(right.coordinate, eye),
    );
  for (const voxel of candidates) {
    const direction = normalizeVector(subtractVector(voxel.coordinate, eye));
    const hit = canonicalRayHits(world, world.player, direction)[0];
    if (
      hit?.type === "voxel" &&
      coordinateKey(hit.voxel.coordinate) === coordinateKey(voxel.coordinate)
    ) {
      return voxel;
    }
  }
  return undefined;
}

function blockAt(world, coordinate, kind) {
  for (const grid of world.grids) {
    const block = grid.blocks.find(
      (candidate) =>
        candidate.coordinate.x === coordinate.x &&
        candidate.coordinate.y === coordinate.y &&
        candidate.coordinate.z === coordinate.z &&
        (!kind || candidate.kind === kind),
    );
    if (block) return { grid, block };
  }
  return undefined;
}

async function movePlayerTo(
  world,
  targetOrProvider,
  description = "work area",
  targetDistance = 0.5,
) {
  const targetForWorld =
    typeof targetOrProvider === "function"
      ? targetOrProvider
      : () => targetOrProvider;
  const targetDistanceSquared = targetDistance * targetDistance;
  for (let attempt = 0; attempt < 480; attempt += 1) {
    const target = targetForWorld(world);
    const targetDeltaSquared = distanceSquared(world.player.position, target);
    if (
      targetDeltaSquared <= targetDistanceSquared &&
      vectorMagnitude(world.player.linear_velocity) <= 0.05
    ) {
      break;
    }
    const current = world.player.position;
    const delta = {
      x: target.x - current.x,
      y: target.y - current.y,
      z: target.z - current.z,
    };
    const magnitude = Math.sqrt(targetDeltaSquared);
    const desiredSpeed =
      magnitude <= targetDistance
        ? 0
        : Math.min(
            CHARACTER_TEST_APPROACH_SPEED,
            Math.max(0.15, (magnitude - targetDistance) * 1.5),
          );
    const worldDirection =
      magnitude > 1e-9
        ? {
            x: (delta.x / magnitude) * desiredSpeed,
            y: (delta.y / magnitude) * desiredSpeed,
            z: (delta.z / magnitude) * desiredSpeed,
          }
        : { x: 0, y: 0, z: 0 };
    const orientation = world.player.orientation;
    const rotatedLocalDirection = rotateVector(
      {
        x: -orientation.x,
        y: -orientation.y,
        z: -orientation.z,
        w: orientation.w,
      },
      worldDirection,
    );
    const localDirection = {
      x: rotatedLocalDirection.x / CHARACTER_MAXIMUM_SPEED,
      y: rotatedLocalDirection.y / CHARACTER_MAXIMUM_SPEED,
      z: rotatedLocalDirection.z / CHARACTER_MAXIMUM_SPEED,
    };
    const inputSequence = world.player.last_received_input_sequence + 1;
    const tick = world.simulation_tick;
    world = await intent("set_player_control", {
      movement_epoch: world.player.movement_epoch,
      input_sequence: inputSequence,
      linear_input: localDirection,
      angular_input: { x: 0, y: 0, z: 0 },
      boost: false,
      jump: false,
      dampeners: true,
    });
    world = await waitForMotionAfter(tick, "integrated character motion");
  }
  const finalTarget = targetForWorld(world);
  const finalDistance = Math.sqrt(
    distanceSquared(world.player.position, finalTarget),
  );
  assert.ok(
    finalDistance <= targetDistance,
    `authoritative character control reaches the ${description}; ` +
      `remaining distance ${finalDistance.toFixed(6)}m exceeds ${targetDistance.toFixed(6)}m; ` +
      `player=${JSON.stringify(world.player.position)} ` +
      `target=${JSON.stringify(finalTarget)} ` +
      `velocity=${JSON.stringify(world.player.linear_velocity)}`,
  );
  assert.ok(
    vectorMagnitude(world.player.linear_velocity) <= 0.05,
    `authoritative character control settles at the ${description}; ` +
      `remaining speed ${vectorMagnitude(world.player.linear_velocity).toFixed(6)}m/s exceeds 0.050000m/s`,
  );
  const tick = world.simulation_tick;
  world = await intent("set_player_control", {
    movement_epoch: world.player.movement_epoch,
    input_sequence: world.player.last_received_input_sequence + 1,
    linear_input: { x: 0, y: 0, z: 0 },
    angular_input: { x: 0, y: 0, z: 0 },
    boost: false,
    jump: false,
    dampeners: true,
  });
  world = await waitForMotionAfter(tick, "neutral character control settles");
  return world;
}

async function aimPlayerAt(world, targetForWorld, description) {
  let alignedSamples = 0;
  for (let attempt = 0; attempt < 420; attempt += 1) {
    const player = world.player;
    const target = targetForWorld(world);
    const forward = playerForward(player);
    const desired = normalizeVector(subtractVector(target, playerEye(player)));
    const cosine = Math.min(1, Math.max(-1, dotVector(forward, desired)));
    const angle = Math.acos(cosine);
    const angularSpeed = vectorMagnitude(player.angular_velocity);
    if (angle <= TARGET_ALIGNMENT_RADIANS && angularSpeed <= 0.025) {
      alignedSamples += 1;
      if (alignedSamples >= 2) return world;
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
    const tick = world.simulation_tick;
    world = await intent("set_player_control", {
      movement_epoch: player.movement_epoch,
      input_sequence: player.last_received_input_sequence + 1,
      linear_input: { x: 0, y: 0, z: 0 },
      angular_input: angularInput,
      boost: false,
      jump: false,
      dampeners: true,
    });
    world = await waitForMotionAfter(
      tick,
      `${description} orientation integration`,
    );
  }
  const target = targetForWorld(world);
  const remaining = Math.acos(
    Math.min(
      1,
      Math.max(
        -1,
        dotVector(
          playerForward(world.player),
          normalizeVector(subtractVector(target, playerEye(world.player))),
        ),
      ),
    ),
  );
  assert.fail(`${description} did not align; remaining angle ${remaining}`);
}

function assertVoxelIsCanonicalHit(world, coordinate, description) {
  const hit = canonicalRayHits(world, world.player)[0];
  assert.equal(hit?.type, "voxel", `${description} resolves to a voxel`);
  assert.equal(
    coordinateKey(hit.voxel.coordinate),
    coordinateKey(coordinate),
    `${description} resolves to the intended closest voxel`,
  );
}

function assertBlockIsCanonicalHit(world, gridId, blockId, description) {
  const hit = canonicalRayHits(world, world.player)[0];
  const expectedGrid = world.grids.find((grid) => grid.grid_id === gridId);
  const expectedBlock = expectedGrid?.blocks.find(
    (block) => block.block_id === blockId,
  );
  assert.equal(
    hit?.type,
    "block",
    `${description} resolves to a block; ${JSON.stringify({
      eye: playerEye(world.player),
      forward: playerForward(world.player),
      position: world.player.position,
      orientation: world.player.orientation,
      locomotion: world.player.locomotion,
      linear_velocity: world.player.linear_velocity,
      expected_target_position:
        expectedGrid && expectedBlock
          ? gridBlockWorldPosition(expectedGrid, expectedBlock)
          : undefined,
      first_hit: hit,
    })}`,
  );
  assert.equal(
    hit.grid.grid_id,
    gridId,
    `${description} resolves to the intended grid`,
  );
  assert.equal(
    hit.block.block_id,
    blockId,
    `${description} resolves to the intended closest block`,
  );
  return hit;
}

function visibleBlockViewPosition(world, target, description) {
  const targetPosition = gridBlockWorldPosition(target.grid, target.block);
  const upOffset = scaleVector(playerUp(world.player), CHARACTER_EYE_OFFSET);
  for (const localOffset of [
    { x: -6, y: 0, z: 0 },
    { x: 6, y: 0, z: 0 },
    { x: 0, y: 0, z: -6 },
    { x: 0, y: 0, z: 6 },
    { x: -4, y: 0, z: -4 },
    { x: -4, y: 0, z: 4 },
    { x: 4, y: 0, z: -4 },
    { x: 4, y: 0, z: 4 },
  ]) {
    const desiredEye = addVector(
      targetPosition,
      rotateVector(target.grid.orientation, localOffset),
    );
    const candidatePlayer = {
      ...world.player,
      position: subtractVector(desiredEye, upOffset),
    };
    const direction = normalizeVector(
      subtractVector(targetPosition, playerEye(candidatePlayer)),
    );
    const hit = canonicalRayHits(world, candidatePlayer, direction)[0];
    if (
      hit?.type === "block" &&
      hit.grid.grid_id === target.grid.grid_id &&
      hit.block.block_id === target.block.block_id
    ) {
      return candidatePlayer.position;
    }
  }
  assert.fail(`${description} has no deterministic clear hand-tool approach`);
}

async function completeBlock(world, coordinate, kind) {
  let target = blockAt(world, coordinate, kind);
  assert.ok(target, "construction frame exists");
  while (target.block.health < target.block.max_health) {
    world = await aimPlayerAt(
      world,
      (state) => {
        const current = blockAt(state, coordinate, kind);
        assert.ok(current, "weld target remains present while aiming");
        return gridBlockWorldPosition(current.grid, current.block);
      },
      `weld ${target.block.block_id}`,
    );
    target = blockAt(world, coordinate, kind);
    assertBlockIsCanonicalHit(
      world,
      target.grid.grid_id,
      target.block.block_id,
      "weld eye ray",
    );
    world = await intent("weld_block", {
      grid_id: target.grid.grid_id,
      block_id: target.block.block_id,
    });
    target = blockAt(world, coordinate, kind);
    assert.ok(target, "welded block remains on its grid");
  }
  return world;
}

async function run() {
  socket.addEventListener("message", (event) => {
    dispatch(JSON.parse(event.data));
  });
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener(
      "error",
      () => reject(new Error("failed to connect to " + url)),
      { once: true },
    );
  });

  send({
    type: "hello",
    protocol_version: PROTOCOL_VERSION,
    client_name: "node-authoritative-e2e",
    authentication: {
      kind: "local_development",
      player_id: "player-local",
    },
  });
  const welcome = await waitFor(
    (message) => message.type === "welcome",
    "authenticated welcome",
  );
  assert.equal(welcome.protocol_version, PROTOCOL_VERSION);
  assert.deepEqual(welcome.session_role, {
    kind: "player",
    player_id: "player-local",
  });
  const registryMessage = await waitFor(
    (message) => message.type === "registry",
    "immutable celestial registry",
  );
  assert.equal(
    registryMessage.universe_manifest.celestial_registry_hash,
    registryMessage.registry.registry_hash,
  );
  let initialState = await waitFor(
    (message) =>
      message.type === "interest_state" && message.frame_kind === "baseline",
    "post-handshake interest baseline",
  );
  const firstFrontier = structuredClone(initialState.interest);
  let world = hydrateActorSnapshot(initialState.state);
  authoritativeWorld = world;
  assert.equal(world.conservation.valid, true);
  assert.equal(world.content_manifest_version, "p1.5.0");
  assert.equal(world.grids.length, 2);
  assert.deepEqual(
    world.players.map((player) => player.player_id),
    ["player-local", "player-remote"],
  );
  assert.equal(world.player.inventory_id, "inventory-player-local");
  assert.ok(
    world.players
      .filter((player) => player.player_id !== "player-local")
      .every((player) => player.inventory_id === undefined),
    "other players' carried inventory identities stay private",
  );
  assert.ok(world.voxels.length > 1_000);
  assert.equal(world.environment.celestial_body_name, "Khepri Prime");
  assert.equal(world.environment.breathable, false);
  assert.ok(world.environment.gravity_m_s2 > 0.3);
  assert.ok(world.environment.gravity_m_s2 < 1.0);
  assert.ok(world.environment.altitude_m > 3_000.0);
  assert.equal(world.environment.atmosphere_density, 0.0);
  for (const player of world.players) {
    if (player.player_id === "player-local") continue;
    assert.equal(player.environment, undefined);
    assert.equal(player.experience, undefined);
    assert.equal(player.suit_oxygen_milli, undefined);
  }
  assert.deepEqual(
    world.player.environment,
    world.environment,
    "the compatibility environment remains the primary pilot environment",
  );
  assert.ok(
    world.player.suit_oxygen_milli > 0 &&
      world.player.suit_oxygen_milli <= 1_000,
    "the live initial baseline contains a valid non-empty suit oxygen reserve",
  );
  assert.equal(world.player.critical_oxygen_milli, 200);
  assert.deepEqual(world.player.life_state, { kind: "alive" });
  assert.equal(world.player.helmet_closed, true);
  assert.equal(world.player.jetpack_enabled, true);
  assert.equal(world.player.locomotion.kind, "eva");
  assert.equal(world.player.locomotion.magnetic_boots_enabled, false);
  assert.deepEqual(world.death_drops, []);
  assert.ok(playerInventory(world).capacity_liters > 0);
  assert.ok(playerInventory(world).used_liters > 0);
  assert.ok(playerInventory(world).mass_grams > 0);

  // A stale/wrong acknowledgement must not advance a session cursor. The
  // server fails closed by issuing a new complete baseline, which this client
  // validates and acknowledges before accepting any more deltas.
  send({
    type: "acknowledge_interest",
    session_epoch: firstFrontier.session_epoch,
    interest_epoch: firstFrontier.interest_epoch,
    baseline_id: firstFrontier.baseline_id,
    delta_sequence: firstFrontier.delta_sequence,
    view_hash: `${firstFrontier.view_hash}-wrong`,
  });
  initialState = await waitFor(
    (message) =>
      message.type === "interest_state" && message.frame_kind === "baseline",
    "wrong-frontier baseline recovery",
  );
  assert.equal(
    initialState.interest.session_epoch,
    firstFrontier.session_epoch,
  );
  assert.equal(
    initialState.interest.interest_epoch,
    firstFrontier.interest_epoch + 1,
  );
  assert.notEqual(initialState.interest.baseline_id, firstFrontier.baseline_id);
  world = hydrateActorSnapshot(initialState.state);
  authoritativeWorld = world;

  for (const [operation_sequence, expectedCode] of [
    [0, "operation_sequence_invalid"],
    [committedOperationSequence + 2, "operation_sequence_gap"],
  ]) {
    const operation_id = operationId(`sequence-${expectedCode}`);
    send({
      type: "set_suit_mode",
      operation_sequence,
      operation_id,
      helmet_closed: world.player.helmet_closed,
      jetpack_enabled: world.player.jetpack_enabled,
      magnetic_boots_enabled: world.player.locomotion.magnetic_boots_enabled,
    });
    const rejected = await waitFor(
      (message) =>
        message.type === "intent_rejected" &&
        message.operation_id === operation_id,
      `${expectedCode} rejection`,
    );
    assert.equal(rejected.operation_sequence, operation_sequence);
    assert.equal(rejected.code, expectedCode);
  }

  const pulseStartOrientation = world.player.orientation;
  const pulseSequence = world.player.last_received_input_sequence + 1;
  const pulseOperation = operationId("character-angular-pulse");
  const releaseOperation = operationId("character-angular-release");
  const pulseOperationSequence = committedOperationSequence + 1;
  const releaseOperationSequence = pulseOperationSequence + 1;
  const pulseMessage = {
    type: "set_player_control",
    operation_sequence: pulseOperationSequence,
    operation_id: pulseOperation,
    movement_epoch: world.player.movement_epoch,
    input_sequence: pulseSequence,
    linear_input: { x: 0, y: 0, z: 0 },
    angular_input: { x: 0, y: 0, z: 1 },
    boost: false,
    jump: false,
    dampeners: true,
  };
  const releaseMessage = {
    type: "set_player_control",
    operation_sequence: releaseOperationSequence,
    operation_id: releaseOperation,
    movement_epoch: world.player.movement_epoch,
    input_sequence: pulseSequence + 1,
    linear_input: { x: 0, y: 0, z: 0 },
    angular_input: { x: 0, y: 0, z: 0 },
    boost: false,
    jump: false,
    dampeners: true,
  };
  send(pulseMessage);
  send(releaseMessage);
  const pulseReceipts = new Map();
  for (const [operationId, operation_sequence] of [
    [pulseOperation, pulseOperationSequence],
    [releaseOperation, releaseOperationSequence],
  ]) {
    const receipt = await waitFor(
      (message) =>
        message.type === "intent_accepted" &&
        message.receipt.operation_id === operationId,
      operationId + " receipt",
    );
    assert.equal(receipt.receipt.operation_sequence, operation_sequence);
    committedOperationSequence = operation_sequence;
    assert.ok(receipt.receipt.event_sequence > world.event_sequence);
    pulseReceipts.set(operationId, receipt.receipt);
  }
  send(pulseMessage);
  assert.deepEqual(
    (
      await waitFor(
        (message) =>
          message.type === "intent_accepted" &&
          message.receipt.operation_id === pulseOperation,
        "exact control retry receipt",
      )
    ).receipt,
    pulseReceipts.get(pulseOperation),
    "an exact retained retry returns its original receipt",
  );
  send({
    ...pulseMessage,
    angular_input: { x: 0, y: 1, z: 0 },
  });
  const conflict = await waitFor(
    (message) =>
      message.type === "intent_rejected" &&
      message.operation_id === pulseOperation,
    "changed-payload operation conflict",
  );
  assert.equal(conflict.operation_sequence, pulseOperationSequence);
  assert.equal(conflict.code, "operation_conflict");
  const pulseState = await waitFor(
    (message) =>
      message.type === "interest_state" &&
      message.state.actor_private?.player.last_processed_input_sequence >=
        pulseSequence + 1,
    "successive authoritative pulse consumption",
  );
  world = hydrateActorSnapshot(pulseState.state);
  authoritativeWorld = world;
  assert.equal(world.player.last_received_input_sequence, pulseSequence + 1);
  assert.equal(world.player.last_processed_input_sequence, pulseSequence + 1);
  assert.ok(
    quaternionAngularDistance(pulseStartOrientation, world.player.orientation) >
      0.000_001,
    "a back-to-back angular press and release rotates the canonical player",
  );

  const mined = new Set();
  while (playerInventory(world).contents.ore < 4 || mined.size < 3) {
    const voxel = visibleVoxel(world, mined);
    assert.ok(voxel, "a visible unmined voxel is available");
    mined.add(coordinateKey(voxel.coordinate));
    world = await aimPlayerAt(
      world,
      () => voxel.coordinate,
      `mine voxel ${coordinateKey(voxel.coordinate)}`,
    );
    assertVoxelIsCanonicalHit(world, voxel.coordinate, "mining eye ray");
    const voxelCount = world.voxels.length;
    const previousHash = world.world_hash;
    world = await intent("mine_voxel", { coordinate: voxel.coordinate });
    world = await waitForWorld(
      (state) =>
        !state.voxels.some(
          (remaining) =>
            coordinateKey(remaining.coordinate) ===
            coordinateKey(voxel.coordinate),
        ),
      "voxel-chunk replacement after mining",
    );
    assert.equal(world.voxels.length, voxelCount - 1);
    assert.notEqual(world.world_hash, previousHash);
    assert.ok(
      !world.voxels.some(
        (remaining) =>
          coordinateKey(remaining.coordinate) ===
          coordinateKey(voxel.coordinate),
      ),
      "the exact mined coordinate becomes authoritative empty volume",
    );
  }

  const industryCargoId = "inventory-cargo-industry-starter";
  world = await intent("transfer_inventory", {
    source_inventory_id: "inventory-player-local",
    destination_inventory_id: industryCargoId,
    resource: "ore",
    quantity: 2,
  });
  world = await intent("queue_production", {
    machine_block_id: "block-refinery",
    recipe: "refining",
    batches: 1,
    source_inventory_id: industryCargoId,
    destination_inventory_id: industryCargoId,
  });
  assert.ok(
    world.production_queues.some(
      (queue) => queue.machine_block_id === "block-refinery",
    ),
    "refinery work exists only in the authoritative private queue",
  );
  world = await waitForWorld(
    (state) =>
      !state.production_queues.some(
        (queue) => queue.machine_block_id === "block-refinery",
      ) &&
      state.inventories.find(
        (inventory) => inventory.inventory_id === industryCargoId,
      )?.contents.refined_material >= 1,
    "physical refinery completion",
  );
  world = await intent("queue_production", {
    machine_block_id: "block-assembler",
    recipe: "component",
    batches: 1,
    source_inventory_id: industryCargoId,
    destination_inventory_id: industryCargoId,
  });
  world = await waitForWorld(
    (state) =>
      !state.production_queues.some(
        (queue) => queue.machine_block_id === "block-assembler",
      ) &&
      state.inventories.find(
        (inventory) => inventory.inventory_id === industryCargoId,
      )?.contents.components >= 1,
    "physical assembler completion",
  );
  world = await intent("transfer_inventory", {
    source_inventory_id: industryCargoId,
    destination_inventory_id: "inventory-player-local",
    resource: "component",
    quantity: 1,
  });
  const cargo = world.inventories.find(
    (inventory) => inventory.domain.kind === "cargo",
  );
  assert.ok(cargo, "starter cargo inventory exists");
  assert.ok(cargo.capacity_liters > playerInventory(world).capacity_liters);
  world = await intent("transfer_inventory", {
    source_inventory_id: "inventory-player-local",
    destination_inventory_id: cargo.inventory_id,
    resource: "ore",
    quantity: 1,
  });

  const cargoBeforeAnchorView = blockAt(world, { x: -1, y: 0, z: 0 }, "cargo");
  assert.ok(
    cargoBeforeAnchorView,
    "starter cargo remains available for anchor work",
  );
  const anchorViewEye = addVector(
    gridBlockWorldPosition(
      cargoBeforeAnchorView.grid,
      cargoBeforeAnchorView.block,
    ),
    rotateVector(cargoBeforeAnchorView.grid.orientation, {
      x: -0.6,
      y: 7.0,
      z: 0.0,
    }),
  );
  world = await movePlayerTo(
    world,
    subtractVector(
      anchorViewEye,
      scaleVector(playerUp(world.player), CHARACTER_EYE_OFFSET),
    ),
    "unobstructed asteroid-side anchor work position",
    0.5,
  );
  const cargoBlock = blockAt(world, { x: -1, y: 0, z: 0 }, "cargo");
  assert.ok(
    cargoBlock,
    "starter cargo provides the asteroid-facing anchor mount",
  );
  world = await aimPlayerAt(
    world,
    (state) => {
      const current = blockAt(state, { x: -1, y: 0, z: 0 }, "cargo");
      assert.ok(current, "anchor mount remains present while aiming");
      return addVector(
        gridBlockWorldPosition(current.grid, current.block),
        rotateVector(current.grid.orientation, { x: -0.5, y: 0, z: 0 }),
      );
    },
    "anchor mount face",
  );
  const anchorMountHit = assertBlockIsCanonicalHit(
    world,
    cargoBlock.grid.grid_id,
    cargoBlock.block.block_id,
    "anchor build eye ray",
  );
  assert.deepEqual(
    anchorMountHit.normal,
    { x: -1, y: 0, z: 0 },
    "the canonical build face points toward the asteroid",
  );
  const anchorCoordinate = addVector(
    cargoBlock.block.coordinate,
    anchorMountHit.normal,
  );
  assert.deepEqual(anchorCoordinate, { x: -2, y: 0, z: 0 });
  world = await intent("build_block", {
    grid_id: "grid-starter",
    coordinate: anchorCoordinate,
    kind: "anchor",
    orientation: 3,
  });
  let anchor = blockAt(world, anchorCoordinate, "anchor");
  assert.ok(anchor, "anchor construction frame was placed");
  assert.equal(anchor.block.orientation, 3);
  assert.ok(anchor.block.health < anchor.block.max_health);
  assert.equal(anchor.block.construction_complete, false);
  world = await completeBlock(world, anchorCoordinate, "anchor");
  anchor = blockAt(world, anchorCoordinate, "anchor");
  assert.equal(anchor.block.construction_complete, true);
  world = await intent("toggle_grid_anchor", { grid_id: "grid-starter" });
  assert.equal(
    world.grids.find((grid) => grid.grid_id === "grid-starter").anchored,
    true,
  );
  world = await intent("toggle_grid_anchor", { grid_id: "grid-starter" });
  assert.equal(
    world.grids.find((grid) => grid.grid_id === "grid-starter").anchored,
    false,
  );

  const tickBeforeMotion = world.simulation_tick;
  const gridZBeforeMotion = world.grids.find(
    (grid) => grid.grid_id === "grid-starter",
  ).position.z;
  world = await intent("set_grid_control", {
    grid_id: "grid-starter",
    linear_input: { x: 0.0, y: 0.0, z: 0.5 },
    angular_input: { x: 0.0, y: 0.1, z: 0.0 },
    dampeners: true,
  });
  while (world.simulation_tick < tickBeforeMotion + 6) {
    world = await waitForMotionAfter(
      world.simulation_tick,
      "integrated grid motion",
    );
  }
  assert.ok(
    world.grids.find((grid) => grid.grid_id === "grid-starter").position.z >
      gridZBeforeMotion,
  );
  world = await intent("set_grid_control", {
    grid_id: "grid-starter",
    linear_input: { x: 0.0, y: 0.0, z: 0.0 },
    angular_input: { x: 0.0, y: 0.0, z: 0.0 },
    dampeners: true,
  });

  const starterAfterMotion = world.grids.find(
    (grid) => grid.grid_id === "grid-starter",
  );
  const core = blockAt(world, { x: 0, y: 0, z: 0 }, "control_core");
  assert.ok(core, "starter control core remains available after grid motion");
  world = await movePlayerTo(
    world,
    addVector(starterAfterMotion.position, { x: 0, y: 6, z: 0 }),
  );
  world = await aimPlayerAt(
    world,
    (state) => {
      const current = blockAt(state, { x: 0, y: 0, z: 0 }, "control_core");
      assert.ok(current, "control core remains present while aiming");
      return addVector(
        gridBlockWorldPosition(current.grid, current.block),
        rotateVector(current.grid.orientation, { x: 0, y: 0.5, z: 0 }),
      );
    },
    "damage bridge mount face",
  );
  const bridgeMountHit = assertBlockIsCanonicalHit(
    world,
    core.grid.grid_id,
    core.block.block_id,
    "damage bridge build eye ray",
  );
  assert.deepEqual(bridgeMountHit.normal, { x: 0, y: 1, z: 0 });
  const bridgeCoordinate = addVector(
    core.block.coordinate,
    bridgeMountHit.normal,
  );
  world = await intent("build_block", {
    grid_id: "grid-starter",
    coordinate: bridgeCoordinate,
    kind: "damage_test",
    orientation: 1,
  });
  world = await completeBlock(world, bridgeCoordinate, "damage_test");
  const completedBridge = blockAt(world, bridgeCoordinate, "damage_test");
  world = await aimPlayerAt(
    world,
    (state) => {
      const current = blockAt(state, bridgeCoordinate, "damage_test");
      assert.ok(current, "damage bridge remains present while aiming");
      return addVector(
        gridBlockWorldPosition(current.grid, current.block),
        rotateVector(current.grid.orientation, { x: 0, y: 0.5, z: 0 }),
      );
    },
    "detached armor mount face",
  );
  const topMountHit = assertBlockIsCanonicalHit(
    world,
    completedBridge.grid.grid_id,
    completedBridge.block.block_id,
    "detached armor build eye ray",
  );
  assert.deepEqual(topMountHit.normal, { x: 0, y: 1, z: 0 });
  const topCoordinate = addVector(
    completedBridge.block.coordinate,
    topMountHit.normal,
  );
  world = await intent("build_block", {
    grid_id: "grid-starter",
    coordinate: topCoordinate,
    kind: "structural",
    orientation: 2,
  });
  world = await completeBlock(world, topCoordinate, "structural");
  const bridge = blockAt(world, bridgeCoordinate, "damage_test");
  assert.ok(bridge, "damage bridge was created");
  assert.equal(bridge.block.construction_complete, true);

  const top = blockAt(world, topCoordinate, "structural");
  assert.ok(top, "detached armor is complete before occlusion validation");
  assertBlockIsCanonicalHit(
    world,
    top.grid.grid_id,
    top.block.block_id,
    "occluding armor eye ray",
  );
  const bridgeHealthBeforeOcclusion = bridge.block.health;
  await expectRejectedIntent(
    "damage_block",
    { grid_id: bridge.grid.grid_id, block_id: bridge.block.block_id },
    "block_not_targeted",
  );
  assert.equal(
    blockAt(world, bridgeCoordinate, "damage_test").block.health,
    bridgeHealthBeforeOcclusion,
    "an occluded damage request cannot mutate bridge integrity",
  );

  for (let approach = 0; approach < 4; approach += 1) {
    world = await movePlayerTo(
      world,
      (state) => {
        const current = blockAt(state, bridgeCoordinate, "damage_test");
        assert.ok(current, "damage bridge remains present while approaching");
        return visibleBlockViewPosition(state, current, "damage bridge");
      },
      "clear damage bridge approach",
      0.35,
    );
    world = await aimPlayerAt(
      world,
      (state) => {
        const current = blockAt(state, bridgeCoordinate, "damage_test");
        assert.ok(current, "damage bridge remains present while aiming");
        return gridBlockWorldPosition(current.grid, current.block);
      },
      "damage bridge",
    );
    const hit = canonicalRayHits(world, world.player)[0];
    if (
      hit?.type === "block" &&
      hit.grid.grid_id === bridge.grid.grid_id &&
      hit.block.block_id === bridge.block.block_id
    ) {
      break;
    }
  }
  assertBlockIsCanonicalHit(
    world,
    bridge.grid.grid_id,
    bridge.block.block_id,
    "damage eye ray",
  );
  world = await intent("damage_block", {
    grid_id: bridge.grid.grid_id,
    block_id: bridge.block.block_id,
  });
  const damagedBridge = blockAt(world, bridgeCoordinate, "damage_test");
  assert.ok(damagedBridge, "damaged completed armor remains installed");
  assert.ok(damagedBridge.block.health < damagedBridge.block.max_health);
  assert.equal(damagedBridge.block.construction_complete, true);
  world = await aimPlayerAt(
    world,
    (state) => {
      const current = blockAt(state, bridgeCoordinate, "damage_test");
      assert.ok(current, "damaged bridge remains present while aiming");
      return gridBlockWorldPosition(current.grid, current.block);
    },
    "final damage bridge",
  );
  assertBlockIsCanonicalHit(
    world,
    damagedBridge.grid.grid_id,
    damagedBridge.block.block_id,
    "final damage eye ray",
  );
  world = await intent("damage_block", {
    grid_id: bridge.grid.grid_id,
    block_id: bridge.block.block_id,
  });

  const survivingTop = blockAt(world, topCoordinate, "structural");
  assert.ok(survivingTop, "completed armor survives on the detached grid");
  assert.equal(survivingTop.block.construction_complete, true);

  assert.equal(world.grids.length, 3);
  assert.equal(world.conservation.valid, true);
  assert.ok(
    world.player.level >= 2,
    "career experience advances clearance level",
  );
  assert.ok(world.player.career.voxels_mined >= 3);
  assert.equal(world.player.career.refining_batches, 1);
  assert.equal(world.player.career.components_crafted, 1);
  assert.equal(world.player.career.blocks_built, 3);
  assert.equal(world.player.career.anchors_engaged, 1);
  assert.ok(
    world.inventories.some(
      (inventory) =>
        inventory.inventory_id === cargo.inventory_id &&
        inventory.contents.ore === 1,
    ),
  );

  world = await intent("set_suit_mode", {
    helmet_closed: false,
    jetpack_enabled: true,
    magnetic_boots_enabled: false,
  });
  world = await waitForCanonicalIncapacitation(world);
  assert.equal(world.player.suit_oxygen_milli, 0);
  assert.equal(world.player.jetpack_enabled, false);
  assert.equal(playerInventory(world).used_liters, 0);
  assert.equal(world.death_drops.length, 1);
  assert.equal(world.death_drops[0].death_id, world.player.life_state.death_id);
  assert.ok(
    world.inventories.some(
      (inventory) =>
        inventory.inventory_id === world.death_drops[0].inventory_id &&
        inventory.used_liters > 0,
    ),
    "the carried inventory is preserved in the canonical death drop",
  );

  const blockedOperation = operationId("dead-mine");
  const blockedOperationSequence = committedOperationSequence + 1;
  send({
    type: "mine_voxel",
    operation_sequence: blockedOperationSequence,
    operation_id: blockedOperation,
    coordinate: { x: 0, y: 0, z: 0 },
  });
  const blocked = await waitFor(
    (message) =>
      message.type === "intent_rejected" &&
      message.operation_id === blockedOperation,
    "incapacitated mutation rejection",
  );
  assert.equal(blocked.code, "player_incapacitated");
  assert.equal(blocked.operation_sequence, blockedOperationSequence);

  const progressionAtDeath = {
    experience: world.player.experience,
    career: world.player.career,
  };
  world = await intent("respawn_player", {});
  assert.deepEqual(world.player.life_state, { kind: "alive" });
  assert.ok(
    world.player.suit_oxygen_milli > 0 &&
      world.player.suit_oxygen_milli <= 1_000,
    "respawn restores oxygen before canonical life support resumes",
  );
  assert.equal(world.player.helmet_closed, true);
  assert.equal(world.player.jetpack_enabled, true);
  assert.equal(playerInventory(world).used_liters, 0);
  assert.equal(world.death_drops.length, 1);
  assert.equal(world.player.experience, progressionAtDeath.experience);
  assert.deepEqual(world.player.career, progressionAtDeath.career);
  assert.equal(world.conservation.valid, true);

  console.log(
    JSON.stringify({
      result: "VERSE_E2E_OK",
      event_sequence: world.event_sequence,
      simulation_tick: world.simulation_tick,
      grids: world.grids.length,
      player_level: world.player.level,
      experience: world.player.experience,
      voxels_remaining: world.voxels.length,
      world_hash: world.world_hash,
      conservation: world.conservation.valid,
    }),
  );
  socket.close();
}

run().catch((error) => {
  console.error(error);
  socket.close();
  process.exitCode = 1;
});
