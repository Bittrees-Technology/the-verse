// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";

const url = process.argv[2] ?? "ws://127.0.0.1:17777/ws";
const socket = new WebSocket(url);
const buffered = [];
const waiters = [];
let operationSequence = 0;

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
  const state = await waitFor(
    (message) =>
      message.type === "snapshot" &&
      message.snapshot.event_sequence >= eventSequence,
    type + " authoritative snapshot",
  );
  assert.equal(state.snapshot.conservation.valid, true);
  return state.snapshot;
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

  await waitFor((message) => message.type === "welcome", "welcome");
  let world = (
    await waitFor((message) => message.type === "snapshot", "genesis snapshot")
  ).snapshot;
  send({
    type: "hello",
    protocol_version: 1,
    client_name: "node-authoritative-e2e",
  });
  assert.equal(world.conservation.valid, true);
  assert.equal(world.content_manifest_version, "p0.1.0");
  assert.equal(world.grids.length, 1);
  assert.ok(world.voxels.length > 1_000);

  const mined = new Set();
  while (playerInventory(world).contents.ore < 4) {
    const voxel = reachableVoxel(world, mined);
    assert.ok(voxel, "a reachable unmined voxel is available");
    mined.add(coordinateKey(voxel.coordinate));
    world = await intent("mine_voxel", { coordinate: voxel.coordinate });
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
  world = await intent("transfer_inventory", {
    source_inventory_id: "inventory-player-local",
    destination_inventory_id: cargo.inventory_id,
    resource: "ore",
    quantity: 1,
  });

  world = await intent("build_block", {
    grid_id: "grid-starter",
    coordinate: { x: -2, y: 0, z: 0 },
    kind: "anchor",
  });
  world = await intent("toggle_grid_anchor", { grid_id: "grid-starter" });
  assert.equal(world.grids[0].anchored, true);
  world = await intent("toggle_grid_anchor", { grid_id: "grid-starter" });
  assert.equal(world.grids[0].anchored, false);

  const tickBeforeMotion = world.simulation_tick;
  world = await intent("set_grid_motion", {
    grid_id: "grid-starter",
    linear_velocity: { x: 0.0, y: 0.0, z: 0.5 },
    angular_velocity: 0.1,
  });
  world = (
    await waitFor(
      (message) =>
        message.type === "snapshot" &&
        message.snapshot.simulation_tick > tickBeforeMotion,
      "integrated grid motion",
    )
  ).snapshot;
  assert.ok(world.grids[0].position.z > 0.0);
  world = await intent("set_grid_motion", {
    grid_id: "grid-starter",
    linear_velocity: { x: 0.0, y: 0.0, z: 0.0 },
    angular_velocity: 0.0,
  });

  world = await intent("build_block", {
    grid_id: "grid-starter",
    coordinate: { x: 0, y: 1, z: 0 },
    kind: "damage_test",
  });
  world = await intent("build_block", {
    grid_id: "grid-starter",
    coordinate: { x: 0, y: 2, z: 0 },
    kind: "structural",
  });
  const bridge = blockAt(world, { x: 0, y: 1, z: 0 }, "damage_test");
  assert.ok(bridge, "damage bridge was created");
  world = await intent("damage_block", {
    grid_id: bridge.grid.grid_id,
    block_id: bridge.block.block_id,
  });
  world = await intent("damage_block", {
    grid_id: bridge.grid.grid_id,
    block_id: bridge.block.block_id,
  });

  assert.equal(world.grids.length, 2);
  assert.equal(world.conservation.valid, true);
  assert.ok(
    world.inventories.some(
      (inventory) =>
        inventory.inventory_id === cargo.inventory_id &&
        inventory.contents.ore === 1,
    ),
  );
  console.log(
    JSON.stringify({
      result: "VERSE_E2E_OK",
      event_sequence: world.event_sequence,
      simulation_tick: world.simulation_tick,
      grids: world.grids.length,
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
