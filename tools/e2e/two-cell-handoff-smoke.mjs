// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";

import { COMPATIBILITY, Protocol16InterestStream } from "./interest-stream.mjs";

const url = process.argv[2] ?? "ws://127.0.0.1:17778/ws";
const mode = process.argv[3] ?? "--exercise";
const evidencePath = process.argv[4];

if (!["--exercise", "--verify-recovery"].includes(mode) || !evidencePath) {
  throw new Error(
    "usage: two-cell-handoff-smoke.mjs <websocket-url> " +
      "<--exercise|--verify-recovery> <evidence-path>",
  );
}

class InterestClient {
  constructor({
    playerId,
    holdDestinationAck = false,
    recordMessages = false,
  }) {
    this.playerId = playerId;
    this.name = playerId ?? "spectator";
    this.holdDestinationAck = holdDestinationAck;
    this.socket = new WebSocket(url);
    this.buffered = [];
    this.waiters = [];
    this.failure = undefined;
    this.heldDestinationAck = undefined;
    this.recordMessages = recordMessages;
    this.received = [];
    this.messageInvariant = undefined;
    this.interestStream = new Protocol16InterestStream({
      expectedPlayerId: playerId,
      send: (message) => {
        if (
          this.holdDestinationAck &&
          message.type === "acknowledge_interest" &&
          message.interest_epoch > 1
        ) {
          assert.equal(
            this.heldDestinationAck,
            undefined,
            "only one destination frame can await acknowledgement",
          );
          this.heldDestinationAck = structuredClone(message);
          return;
        }
        this.send(message);
      },
    });
    this.socket.addEventListener("message", (event) => {
      try {
        this.dispatch(JSON.parse(event.data));
      } catch (error) {
        this.fail(error);
      }
    });
    this.socket.addEventListener("close", () => {
      if (!this.failure && this.waiters.length > 0) {
        this.fail(new Error(`${this.name} socket closed unexpectedly`));
      }
    });
    this.socket.addEventListener("error", () => {
      this.fail(new Error(`${this.name} socket failed`));
    });
  }

  dispatch(rawMessage) {
    const message = this.interestStream.receive(rawMessage);
    if (message.type === "fatal") {
      throw new Error(
        `${this.name} received fatal ${message.code}: ${message.message}`,
      );
    }
    if (this.recordMessages) this.received.push(structuredClone(message));
    if (this.messageInvariant) this.messageInvariant(message);
    const index = this.waiters.findIndex((waiter) => waiter.predicate(message));
    if (index >= 0) {
      const [waiter] = this.waiters.splice(index, 1);
      clearTimeout(waiter.timeout);
      waiter.resolve(message);
    } else {
      this.buffered.push(message);
    }
  }

  fail(error) {
    if (this.failure) return;
    this.failure = error instanceof Error ? error : new Error(String(error));
    for (const waiter of this.waiters.splice(0)) {
      clearTimeout(waiter.timeout);
      waiter.reject(this.failure);
    }
  }

  waitFor(predicate, description, timeoutMillis = 20_000) {
    if (this.failure) return Promise.reject(this.failure);
    const index = this.buffered.findIndex(predicate);
    if (index >= 0) {
      return Promise.resolve(this.buffered.splice(index, 1)[0]);
    }
    return new Promise((resolve, reject) => {
      const waiter = { predicate, resolve, reject, timeout: undefined };
      waiter.timeout = setTimeout(() => {
        const position = this.waiters.indexOf(waiter);
        if (position >= 0) this.waiters.splice(position, 1);
        reject(new Error(`${this.name} timed out waiting for ${description}`));
      }, timeoutMillis);
      this.waiters.push(waiter);
    });
  }

  send(message) {
    assert.equal(
      this.socket.readyState,
      WebSocket.OPEN,
      `${this.name} socket is open`,
    );
    this.socket.send(JSON.stringify(message));
  }

  async connect() {
    await new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener(
        "error",
        () => reject(new Error(`failed to connect ${this.name} to ${url}`)),
        { once: true },
      );
    });
    this.send({
      type: "hello",
      protocol_version: COMPATIBILITY.protocol_version,
      client_name: `node-two-cell-${this.name}`,
      authentication: this.playerId
        ? { kind: "local_development", player_id: this.playerId }
        : { kind: "spectator" },
    });
    await this.waitFor(
      (message) => message.type === "welcome",
      "protocol welcome",
    );
    await this.waitFor(
      (message) => message.type === "registry",
      "universe registry",
    );
    return this.waitFor(
      (message) =>
        message.type === "interest_state" &&
        message.frame_kind === "baseline",
      "initial interest baseline",
    );
  }

  releaseDestinationAck() {
    assert.ok(
      this.heldDestinationAck,
      "the destination baseline acknowledgement is held",
    );
    const acknowledgement = this.heldDestinationAck;
    this.heldDestinationAck = undefined;
    this.send(acknowledgement);
  }

  installMessageInvariant(invariant) {
    assert.equal(
      this.recordMessages,
      true,
      `${this.name} records messages before installing a stream invariant`,
    );
    assert.equal(
      this.messageInvariant,
      undefined,
      `${this.name} installs its stream invariant once`,
    );
    for (const message of this.received) invariant(message);
    this.messageInvariant = invariant;
  }

  assertHealthy() {
    if (this.failure) throw this.failure;
  }

  async close() {
    if (this.socket.readyState === WebSocket.CLOSED) return;
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(
        () => reject(new Error(`${this.name} did not close cleanly`)),
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
      this.socket.close(1000, "two-cell handoff smoke complete");
    });
  }
}

function actorPrivate(message, description) {
  const state = message.projection.actor_private;
  assert.ok(state, `${description} has an actor-private projection`);
  assert.equal(state.player.player_id, "player-local");
  return state;
}

function cellIdentity(address) {
  return {
    universe_id: address.universe_id,
    sector: address.sector,
    cell: address.cell,
  };
}

function playerInventory(privateState) {
  const inventory = privateState.inventories.find(
    (candidate) => candidate.inventory_id === "inventory-player-local",
  );
  assert.ok(inventory, "the carried player inventory is projected privately");
  return inventory;
}

async function exerciseHandoff() {
  const player = new InterestClient({
    playerId: "player-local",
    holdDestinationAck: true,
  });
  let spectator;
  try {
    const source = await player.connect();
    console.log("VERSE_TWO_CELL_STEP player_source_connected");
    spectator = new InterestClient({
      playerId: undefined,
      recordMessages: true,
    });
    const publicSource = await spectator.connect();
    console.log("VERSE_TWO_CELL_STEP spectator_origin_connected");
    assert.equal(source.projection.interest.interest_epoch, 1);
    assert.equal(source.projection.interest.transfer_link, undefined);
    assert.equal(publicSource.projection.cell_id, source.projection.cell_id);
    assert.equal(publicSource.projection.actor_private, undefined);
    spectator.installMessageInvariant((message) => {
      assert.notEqual(
        message.type,
        "handoff",
        "the origin spectator never receives private handoff phases",
      );
      if (message.type === "interest_state") {
        assert.equal(
          message.projection.cell_id,
          source.projection.cell_id,
          "every spectator frame remains pinned to the origin cell",
        );
        assert.equal(
          message.projection.actor_private,
          undefined,
          "every spectator frame omits the actor-private overlay",
        );
        assert.equal(
          message.projection.interest.transfer_link,
          undefined,
          "the spectator never receives a private transfer link",
        );
      }
      assert.ok(
        !JSON.stringify(message).includes("inventory-player-local"),
        "no spectator message discloses the carried inventory identity",
      );
    });
    const publicInitiallySawPlayer = publicSource.projection.players.some(
      (candidate) => candidate.player_id === "player-local",
    );
    const sourceCellId = source.projection.cell_id;
    const sourcePrivate = actorPrivate(source, "source baseline");

    const barrierId = "two-cell-source-ack-barrier";
    player.send({
      type: "set_suit_mode",
      operation_sequence: 1,
      operation_id: barrierId,
      helmet_closed: !sourcePrivate.player.helmet_closed,
      jetpack_enabled: true,
      magnetic_boots_enabled: false,
    });
    const barrier = await player.waitFor(
      (message) =>
        message.type === "intent_accepted" &&
        message.receipt.operation_id === barrierId,
      "source acknowledgement barrier receipt",
    );
    console.log("VERSE_TWO_CELL_STEP source_ack_barrier_committed");
    await player.waitFor(
      (message) =>
        message.type === "interest_state" &&
        message.frame_kind === "delta" &&
        message.projection.cell_id === sourceCellId &&
        message.projection.event_sequence >= barrier.receipt.event_sequence,
      "acknowledged source mutation",
    );

    const thrustId = "two-cell-eastbound-thrust";
    player.send({
      type: "set_player_control",
      operation_sequence: 2,
      operation_id: thrustId,
      movement_epoch: sourcePrivate.player.movement_epoch,
      input_sequence: sourcePrivate.player.last_received_input_sequence + 1,
      linear_input: { x: 1, y: 0, z: 0 },
      angular_input: { x: 0, y: 0, z: 0 },
      boost: true,
      dampeners: true,
      jump: false,
    });
    const thrustReceipt = await player.waitFor(
      (message) =>
        message.type === "intent_accepted" &&
        message.receipt.operation_id === thrustId,
      "eastbound control receipt",
    );
    console.log("VERSE_TWO_CELL_STEP eastbound_thrust_committed");
    const publicControl = await spectator.waitFor(
      (message) =>
        message.type === "interest_state" &&
        message.frame_kind === "delta" &&
        message.projection.event_sequence >=
          thrustReceipt.receipt.event_sequence,
      "origin-cell control frontier",
    );

    let transfer;
    for (const phase of ["preparing", "importing", "verifying_destination"]) {
      const message = await player.waitFor(
        (candidate) => candidate.type === "handoff",
        `${phase} handoff phase`,
      );
      assert.equal(message.handoff.phase, phase);
      if (transfer) {
        assert.equal(message.handoff.transfer_id, transfer.transfer_id);
        assert.deepEqual(
          message.handoff.destination_cell_key,
          transfer.destination_cell_key,
        );
        assert.equal(
          message.handoff.placement_generation,
          transfer.placement_generation,
        );
      } else {
        transfer = structuredClone(message.handoff);
      }
    }

    const destination = await player.waitFor(
      (message) =>
        message.type === "interest_state" &&
        message.frame_kind === "baseline" &&
        message.projection.cell_id !== sourceCellId,
      "transfer-linked destination baseline",
    );
    console.log("VERSE_TWO_CELL_STEP destination_baseline_staged");
    assert.equal(
      destination.projection.interest.session_epoch,
      source.projection.interest.session_epoch,
    );
    assert.equal(destination.projection.interest.interest_epoch, 2);
    assert.deepEqual(
      destination.projection.interest.transfer_link,
      {
        transfer_id: transfer.transfer_id,
        destination_cell_key: transfer.destination_cell_key,
        placement_generation: transfer.placement_generation,
      },
    );
    assert.deepEqual(
      cellIdentity(destination.projection.cell_address),
      cellIdentity(transfer.destination_cell_key),
    );
    const destinationPrivate = actorPrivate(
      destination,
      "destination baseline",
    );

    const gatedId = "two-cell-destination-ack-gate";
    const gatedControl = {
      type: "set_player_control",
      operation_sequence: 3,
      operation_id: gatedId,
      movement_epoch: destinationPrivate.player.movement_epoch,
      input_sequence: destinationPrivate.player.last_received_input_sequence + 1,
      linear_input: { x: 0, y: 0, z: 0 },
      angular_input: { x: 0, y: 0, z: 0 },
      boost: false,
      dampeners: true,
      jump: false,
    };
    player.send(gatedControl);
    const rejection = await player.waitFor(
      (message) =>
        message.type === "intent_rejected" &&
        message.operation_id === gatedId,
      "pre-acknowledgement control rejection",
    );
    assert.equal(rejection.code, "player_route_stale");
    assert.equal(rejection.operation_sequence, 3);
    console.log("VERSE_TWO_CELL_STEP destination_pre_ack_control_rejected");

    player.releaseDestinationAck();
    player.send(gatedControl);
    const accepted = await player.waitFor(
      (message) =>
        message.type === "intent_accepted" &&
        message.receipt.operation_id === gatedId,
      "post-acknowledgement control acceptance",
    );
    assert.equal(accepted.receipt.operation_sequence, 3);
    console.log("VERSE_TWO_CELL_STEP destination_post_ack_control_committed");
    const committedDestination = await player.waitFor(
      (message) =>
        message.type === "interest_state" &&
        message.frame_kind === "delta" &&
        message.projection.cell_id === destination.projection.cell_id &&
        message.projection.event_sequence >= accepted.receipt.event_sequence &&
        message.projection.actor_private?.committed_operation_sequence >= 3,
      "destination control projection",
    );
    assert.equal(
      committedDestination.projection.interest.transfer_link,
      undefined,
    );

    const publicHandoff = await spectator.waitFor(
      (message) =>
        message.type === "interest_state" &&
        message.frame_kind === "delta" &&
        message.projection.event_sequence >
          publicControl.projection.event_sequence,
      "origin-cell handoff frontier",
    );
    assert.equal(publicHandoff.projection.cell_id, sourceCellId);
    assert.equal(publicHandoff.projection.actor_private, undefined);
    assert.equal(publicHandoff.projection.interest.transfer_link, undefined);
    const transferredRemoval = publicHandoff.projection.interest.removed.find(
      (removed) =>
        removed.entity_id === "player-local" && removed.kind === "player",
    );
    if (publicInitiallySawPlayer) {
      assert.equal(
        transferredRemoval?.reason,
        "transferred",
        "a prior-visible origin observer receives exact transfer evidence",
      );
    } else {
      assert.equal(
        transferredRemoval,
        undefined,
        "an observer cannot learn the identity of an unseen transfer",
      );
    }
    assert.ok(
      !publicHandoff.projection.players.some(
        (candidate) => candidate.player_id === "player-local",
      ),
      "the transferred pilot is absent from the origin-cell view",
    );
    assert.ok(
      !JSON.stringify(publicHandoff).includes("inventory-player-local"),
      "the public origin feed discloses no private inventory",
    );
    await new Promise((resolve) => setTimeout(resolve, 100));
    spectator.assertHealthy();

    const committedPrivate = actorPrivate(
      committedDestination,
      "committed destination delta",
    );
    const evidence = {
      schema_version: 1,
      universe_id: committedDestination.projection.universe_id,
      source_cell_id: sourceCellId,
      destination_cell_id: committedDestination.projection.cell_id,
      destination_cell_key: transfer.destination_cell_key,
      transfer_id: transfer.transfer_id,
      placement_generation: transfer.placement_generation,
      movement_epoch: committedPrivate.player.movement_epoch,
      committed_operation_sequence:
        committedPrivate.committed_operation_sequence,
      player_inventory: playerInventory(committedPrivate),
      event_sequence: committedDestination.projection.event_sequence,
      world_hash: committedDestination.projection.world_hash,
    };
    await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, {
      flag: "wx",
    });
    console.log(
      `VERSE_TWO_CELL_HANDOFF_OK transfer=${evidence.transfer_id} ` +
        `source=${evidence.source_cell_id} destination=${evidence.destination_cell_id} ` +
        `generation=${evidence.placement_generation}`,
    );
  } finally {
    await Promise.allSettled([player.close(), spectator?.close()]);
  }
}

async function verifyRecovery() {
  const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
  assert.equal(evidence.schema_version, 1);
  const player = new InterestClient({ playerId: "player-local" });
  try {
    const recovered = await player.connect();
    assert.equal(recovered.projection.universe_id, evidence.universe_id);
    assert.equal(recovered.projection.cell_id, evidence.destination_cell_id);
    assert.notEqual(recovered.projection.cell_id, evidence.source_cell_id);
    assert.equal(recovered.projection.interest.interest_epoch, 1);
    assert.equal(recovered.projection.interest.transfer_link, undefined);
    assert.deepEqual(
      cellIdentity(recovered.projection.cell_address),
      cellIdentity(evidence.destination_cell_key),
    );
    const privateState = actorPrivate(recovered, "recovered destination");
    assert.equal(privateState.player.movement_epoch, evidence.movement_epoch);
    assert.equal(
      privateState.committed_operation_sequence,
      evidence.committed_operation_sequence,
    );
    assert.deepEqual(
      playerInventory(privateState),
      evidence.player_inventory,
      "carried inventory survives the worker restart exactly",
    );
    assert.ok(
      recovered.projection.event_sequence >= evidence.event_sequence,
      "the recovered destination never regresses its durable event frontier",
    );
    console.log(
      `VERSE_TWO_CELL_RECOVERY_OK transfer=${evidence.transfer_id} ` +
        `destination=${evidence.destination_cell_id} ` +
        `operation_sequence=${evidence.committed_operation_sequence}`,
    );
  } finally {
    await player.close();
  }
}

if (mode === "--exercise") {
  await exerciseHandoff();
} else {
  await verifyRecovery();
}
