// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";

const url = process.argv[2] ?? "ws://127.0.0.1:17777/ws";
const expectedRoster = ["player-local", "player-remote"];
const sharedControlOperation = "two-player-e2e-shared-control-operation";
const sharedReleaseOperation = "two-player-e2e-shared-release-operation";
const remoteMiningOperation = "two-player-e2e-remote-mining-operation";

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
        reject(new Error(`${this.playerId} timed out waiting for ${description}`));
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
  assert.deepEqual(roster, expectedRoster, `${description} has the expected roster`);
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
    (left.x - right.x) ** 2 +
    (left.y - right.y) ** 2 +
    (left.z - right.z) ** 2
  );
}

function playerInventory(state, player) {
  return state.inventories.find(
    (inventory) => inventory.inventory_id === player.inventory_id,
  );
}

function reachableVoxel(state, player) {
  return state.voxels
    .filter(
      (voxel) =>
        distanceSquared(voxel.coordinate, player.position) <= 8.5 ** 2 &&
        !(
          voxel.coordinate.x >= 7 &&
          Math.abs(voxel.coordinate.y) <= 1 &&
          Math.abs(voxel.coordinate.z) <= 1
        ),
    )
    .sort(
      (left, right) =>
        distanceSquared(left.coordinate, player.position) -
        distanceSquared(right.coordinate, player.position),
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
    localMessage.snapshot.event_sequence !== remoteMessage.snapshot.event_sequence
  ) {
    if (
      localMessage.snapshot.event_sequence < remoteMessage.snapshot.event_sequence
    ) {
      const minimum = remoteMessage.snapshot.event_sequence;
      localMessage = await local.waitFor(
        (message) => predicate(message) && message.snapshot.event_sequence >= minimum,
        `${description} convergence on local`,
        timeoutMillis,
      );
    } else {
      const minimum = localMessage.snapshot.event_sequence;
      remoteMessage = await remote.waitFor(
        (message) => predicate(message) && message.snapshot.event_sequence >= minimum,
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

function controlFor(player, operationId, inputSequence, linearInput) {
  return {
    type: "set_player_control",
    operation_id: operationId,
    movement_epoch: player.movement_epoch,
    input_sequence: inputSequence,
    linear_input: linearInput,
    angular_input: { x: 0, y: 0, z: 0 },
    boost: false,
    jump: false,
    dampeners: true,
  };
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
    const target = reachableVoxel(localSnapshot, initialRemote);
    assert.ok(target, "the remote actor has a reachable unanchored voxel");
    const primaryInventoryBefore = structuredClone(
      playerInventory(localSnapshot, initialPrimary).contents,
    );
    const remoteInventoryBefore = structuredClone(
      playerInventory(localSnapshot, initialRemote).contents,
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
