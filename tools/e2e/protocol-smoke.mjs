// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";

const url = process.argv[2] ?? "ws://127.0.0.1:17777/ws";
const socket = new WebSocket(url);
const buffered = [];
const waiters = [];
let operationSequence = 0;
let authoritativeWorld;

function dispatch(message) {
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

async function intent(type, payload) {
  const operation_id = operationId(type);
  send({ type, operation_id, ...payload });
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
  const eventSequence = result.receipt.event_sequence;
  const acceptsMotionState = type === "set_player_control";
  const minimumStateSequence = Math.max(
    eventSequence,
    authoritativeWorld?.event_sequence ?? 0,
  );
  const state = await waitFor(
    (message) =>
      (message.type === "snapshot" &&
        message.snapshot.event_sequence >= minimumStateSequence) ||
      (acceptsMotionState && message.type === "motion_state" &&
        message.motion.event_sequence >= minimumStateSequence),
    type + " authoritative state",
  );
  authoritativeWorld = state.type === "snapshot"
    ? state.snapshot
    : applyMotion(authoritativeWorld, state.motion);
  assert.equal(authoritativeWorld.conservation.valid, true);
  return authoritativeWorld;
}

function applyMotion(world, motion) {
  assert.ok(world, "a full snapshot precedes motion deltas");
  const movingGrids = new Map(
    motion.grids.map((grid) => [grid.grid_id, grid]),
  );
  const movingPlayers = new Map(
    (motion.players ?? [motion.player]).map((player) => [player.player_id, player]),
  );
  return {
    ...world,
    event_sequence: motion.event_sequence,
    simulation_tick: motion.simulation_tick,
    world_hash: motion.world_hash,
    player: { ...world.player, ...motion.player },
    players: world.players.map((player) => ({
      ...player,
      ...(movingPlayers.get(player.player_id) ?? {}),
    })),
    grids: world.grids.map((grid) => ({
      ...grid,
      ...(movingGrids.get(grid.grid_id) ?? {}),
    })),
  };
}

async function waitForMotionAfter(simulationTick, description) {
  const minimumEventSequence = authoritativeWorld?.event_sequence ?? 0;
  const state = await waitFor(
    (message) =>
      message.type === "motion_state" &&
      message.motion.simulation_tick > simulationTick &&
      message.motion.event_sequence >= minimumEventSequence,
    description,
  );
  authoritativeWorld = applyMotion(authoritativeWorld, state.motion);
  return authoritativeWorld;
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
        (message.type === "snapshot" &&
          message.snapshot.event_sequence > observedEventSequence &&
          (message.snapshot.player.life_state.kind === "incapacitated" ||
            message.snapshot.player.suit_oxygen_milli < observedOxygen)),
      "canonical oxygen depletion progress",
      Math.min(8_000, remaining),
    );
    world = state.snapshot;
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
  throw new Error("timed out while authoritative oxygen depletion was progressing");
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

function rotateVector(quaternion, vector) {
  const tx = 2 * (quaternion.y * vector.z - quaternion.z * vector.y);
  const ty = 2 * (quaternion.z * vector.x - quaternion.x * vector.z);
  const tz = 2 * (quaternion.x * vector.y - quaternion.y * vector.x);
  return {
    x:
      vector.x +
      quaternion.w * tx +
      (quaternion.y * tz - quaternion.z * ty),
    y:
      vector.y +
      quaternion.w * ty +
      (quaternion.z * tx - quaternion.x * tz),
    z:
      vector.z +
      quaternion.w * tz +
      (quaternion.x * ty - quaternion.y * tx),
  };
}

function quaternionAngularDistance(left, right) {
  const dot = Math.abs(
    left.x * right.x +
      left.y * right.y +
      left.z * right.z +
      left.w * right.w,
  );
  return 2 * Math.acos(Math.min(1, Math.max(0, dot)));
}

function coordinateKey(coordinate) {
  return [coordinate.x, coordinate.y, coordinate.z].join(",");
}

function reachableVoxel(world, exclusions = new Set()) {
  return world.voxels
    .filter(
      (voxel) =>
        distanceSquared(voxel.coordinate, world.player.position) <= 8.5 * 8.5 &&
        !exclusions.has(coordinateKey(voxel.coordinate)) &&
        !(
          voxel.coordinate.x >= 7 &&
          Math.abs(voxel.coordinate.y) <= 1 &&
          Math.abs(voxel.coordinate.z) <= 1
        ),
    )
    .sort(
      (left, right) =>
        distanceSquared(left.coordinate, world.player.position) -
        distanceSquared(right.coordinate, world.player.position),
    )[0];
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

async function movePlayerTo(world, target) {
  for (let attempt = 0; attempt < 240; attempt += 1) {
    if (distanceSquared(world.player.position, target) <= 0.25) break;
    const current = world.player.position;
    const delta = {
      x: target.x - current.x,
      y: target.y - current.y,
      z: target.z - current.z,
    };
    const magnitude = Math.sqrt(distanceSquared(delta, { x: 0, y: 0, z: 0 }));
    const worldDirection = {
      x: delta.x / magnitude,
      y: delta.y / magnitude,
      z: delta.z / magnitude,
    };
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
    const localMagnitude = Math.sqrt(
      distanceSquared(rotatedLocalDirection, { x: 0, y: 0, z: 0 }),
    );
    const localScale = Math.max(1, localMagnitude);
    const localDirection = {
      x: rotatedLocalDirection.x / localScale,
      y: rotatedLocalDirection.y / localScale,
      z: rotatedLocalDirection.z / localScale,
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
  assert.ok(
    distanceSquared(world.player.position, target) <= 0.25,
    "authoritative character control reaches the work area",
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

async function completeBlock(world, coordinate, kind) {
  let target = blockAt(world, coordinate, kind);
  assert.ok(target, "construction frame exists");
  while (target.block.health < target.block.max_health) {
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
    protocol_version: 11,
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
  assert.equal(welcome.protocol_version, 11);
  assert.deepEqual(welcome.session_role, {
    kind: "player",
    player_id: "player-local",
  });
  let world = (
    await waitFor(
      (message) => message.type === "snapshot",
      "post-handshake genesis snapshot",
    )
  ).snapshot;
  authoritativeWorld = world;
  assert.equal(world.conservation.valid, true);
  assert.equal(world.content_manifest_version, "p0.10.0");
  assert.equal(world.grids.length, 1);
  assert.deepEqual(
    world.players.map((player) => player.player_id),
    ["player-local", "player-remote"],
  );
  assert.equal(
    new Set(world.players.map((player) => player.inventory_id)).size,
    world.players.length,
    "each authoritative actor owns a distinct carried inventory",
  );
  assert.ok(world.voxels.length > 1_000);
  assert.equal(world.environment.celestial_body_name, "Khepri Prime");
  assert.equal(world.environment.breathable, false);
  assert.ok(world.environment.gravity_m_s2 > 0.3);
  assert.ok(world.environment.gravity_m_s2 < 1.0);
  assert.ok(world.environment.altitude_m > 3_000.0);
  assert.equal(world.environment.atmosphere_density, 0.0);
  assert.equal(world.player.suit_oxygen_milli, 1_000);
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

  const pulseStartOrientation = world.player.orientation;
  const pulseSequence = world.player.last_received_input_sequence + 1;
  const pulseOperation = operationId("character-angular-pulse");
  const releaseOperation = operationId("character-angular-release");
  send({
    type: "set_player_control",
    operation_id: pulseOperation,
    movement_epoch: world.player.movement_epoch,
    input_sequence: pulseSequence,
    linear_input: { x: 0, y: 0, z: 0 },
    angular_input: { x: 0, y: 0, z: 1 },
    boost: false,
    jump: false,
    dampeners: true,
  });
  send({
    type: "set_player_control",
    operation_id: releaseOperation,
    movement_epoch: world.player.movement_epoch,
    input_sequence: pulseSequence + 1,
    linear_input: { x: 0, y: 0, z: 0 },
    angular_input: { x: 0, y: 0, z: 0 },
    boost: false,
    jump: false,
    dampeners: true,
  });
  for (const operationId of [pulseOperation, releaseOperation]) {
    const receipt = await waitFor(
      (message) =>
        message.type === "intent_accepted" &&
        message.receipt.operation_id === operationId,
      operationId + " receipt",
    );
    assert.ok(receipt.receipt.event_sequence > world.event_sequence);
  }
  const pulseState = await waitFor(
    (message) =>
      message.type === "motion_state" &&
      message.motion.player.last_processed_input_sequence >= pulseSequence + 1,
    "successive authoritative pulse consumption",
  );
  world = applyMotion(world, pulseState.motion);
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
    const voxel = reachableVoxel(world, mined);
    assert.ok(voxel, "a reachable unmined voxel is available");
    mined.add(coordinateKey(voxel.coordinate));
    const voxelCount = world.voxels.length;
    const previousHash = world.world_hash;
    world = await intent("mine_voxel", { coordinate: voxel.coordinate });
    assert.equal(world.voxels.length, voxelCount - 1);
    assert.notEqual(world.world_hash, previousHash);
    assert.ok(
      !world.voxels.some(
        (remaining) =>
          coordinateKey(remaining.coordinate) === coordinateKey(voxel.coordinate),
      ),
      "the exact mined coordinate becomes authoritative empty volume",
    );
  }

  world = await intent("refine_ore", {
    inventory_id: "inventory-player-local",
    batches: 1,
  });
  world = await intent("craft_component", {
    inventory_id: "inventory-player-local",
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

  world = await movePlayerTo(world, { x: 10.0, y: 1.0, z: 3.0 });
  world = await intent("build_block", {
    grid_id: "grid-starter",
    coordinate: { x: -2, y: 0, z: 0 },
    kind: "anchor",
    orientation: 3,
  });
  let anchor = blockAt(world, { x: -2, y: 0, z: 0 }, "anchor");
  assert.ok(anchor, "anchor construction frame was placed");
  assert.equal(anchor.block.orientation, 3);
  assert.ok(anchor.block.health < anchor.block.max_health);
  assert.equal(anchor.block.construction_complete, false);
  world = await completeBlock(world, { x: -2, y: 0, z: 0 }, "anchor");
  anchor = blockAt(world, { x: -2, y: 0, z: 0 }, "anchor");
  assert.equal(anchor.block.construction_complete, true);
  world = await intent("toggle_grid_anchor", { grid_id: "grid-starter" });
  assert.equal(world.grids[0].anchored, true);
  world = await intent("toggle_grid_anchor", { grid_id: "grid-starter" });
  assert.equal(world.grids[0].anchored, false);

  const tickBeforeMotion = world.simulation_tick;
  const gridZBeforeMotion = world.grids[0].position.z;
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
  assert.ok(world.grids[0].position.z > gridZBeforeMotion);
  world = await intent("set_grid_control", {
    grid_id: "grid-starter",
    linear_input: { x: 0.0, y: 0.0, z: 0.0 },
    angular_input: { x: 0.0, y: 0.0, z: 0.0 },
    dampeners: true,
  });

  world = await intent("build_block", {
    grid_id: "grid-starter",
    coordinate: { x: 0, y: 1, z: 0 },
    kind: "damage_test",
    orientation: 1,
  });
  world = await completeBlock(world, { x: 0, y: 1, z: 0 }, "damage_test");
  world = await intent("build_block", {
    grid_id: "grid-starter",
    coordinate: { x: 0, y: 2, z: 0 },
    kind: "structural",
    orientation: 2,
  });
  world = await completeBlock(world, { x: 0, y: 2, z: 0 }, "structural");
  const bridge = blockAt(world, { x: 0, y: 1, z: 0 }, "damage_test");
  assert.ok(bridge, "damage bridge was created");
  assert.equal(bridge.block.construction_complete, true);
  world = await intent("damage_block", {
    grid_id: bridge.grid.grid_id,
    block_id: bridge.block.block_id,
  });
  const damagedBridge = blockAt(
    world,
    { x: 0, y: 1, z: 0 },
    "damage_test",
  );
  assert.ok(damagedBridge, "damaged completed armor remains installed");
  assert.ok(damagedBridge.block.health < damagedBridge.block.max_health);
  assert.equal(damagedBridge.block.construction_complete, true);
  world = await intent("damage_block", {
    grid_id: bridge.grid.grid_id,
    block_id: bridge.block.block_id,
  });

  const survivingTop = blockAt(world, { x: 0, y: 2, z: 0 }, "structural");
  assert.ok(survivingTop, "completed armor survives on the detached grid");
  assert.equal(survivingTop.block.construction_complete, true);

  assert.equal(world.grids.length, 2);
  assert.equal(world.conservation.valid, true);
  assert.ok(world.player.level >= 2, "career experience advances clearance level");
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
  assert.equal(
    world.death_drops[0].death_id,
    world.player.life_state.death_id,
  );
  assert.ok(
    world.inventories.some(
      (inventory) =>
        inventory.inventory_id === world.death_drops[0].inventory_id &&
        inventory.used_liters > 0,
    ),
    "the carried inventory is preserved in the canonical death drop",
  );

  const blockedOperation = operationId("dead-mine");
  send({
    type: "mine_voxel",
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
