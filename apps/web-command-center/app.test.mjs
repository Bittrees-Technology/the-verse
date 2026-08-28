// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = await readFile(
  new URL("./app.js", import.meta.url),
  "utf8",
);
const indexSource = await readFile(
  new URL("./index.html", import.meta.url),
  "utf8",
);
const elements = new Map();
const context = {
  __VERSE_BROWSER_TEST__: true,
  document: {
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, { id });
      return elements.get(id);
    },
  },
};
vm.createContext(context);
vm.runInContext(source, context, { filename: "app.js" });
const api = context.__VERSE_BROWSER_TEST_API__;

function player(playerId, x, lifeState = "alive") {
  return {
    player_id: playerId,
    position: { x, y: 2, z: 3 },
    orientation: { x: 0, y: 0, z: 0, w: 1 },
    life_state: lifeState,
  };
}

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

test("canonicalPlayers orders the roster by authoritative player_id", () => {
  const state = {
    player: player("player-local", 1),
    players: [player("player-zulu", 3), player("player-local", 1)],
  };

  assert.deepEqual(
    plain(api.canonicalPlayers(state).map((candidate) => candidate.player_id)),
    ["player-local", "player-zulu"],
  );
  assert.deepEqual(plain(api.canonicalPlayers({})), []);
});

test("mergeMotionState merges every public player and preserves structural state", () => {
  const local = player("player-local", 1);
  const remote = player("player-remote", 4);
  const state = {
    event_sequence: 8,
    simulation_tick: 20,
    world_hash: "before",
    player: local,
    players: [remote, local],
    grids: [
      { grid_id: "grid-b", position: { x: 2, y: 0, z: 0 }, blocks: [1] },
      { grid_id: "grid-a", position: { x: 1, y: 0, z: 0 }, blocks: [2] },
    ],
  };
  const motion = {
    event_sequence: 9,
    simulation_tick: 21,
    world_hash: "after",
    player: { player_id: "player-local", position: { x: 10, y: 2, z: 3 } },
    players: [
      { player_id: "player-remote", position: { x: 40, y: 2, z: 3 } },
      { player_id: "player-local", position: { x: 10, y: 2, z: 3 } },
    ],
    grids: [
      { grid_id: "grid-a", position: { x: 11, y: 0, z: 0 } },
    ],
  };

  const merged = plain(api.mergeMotionState(state, motion));
  assert.equal(merged.event_sequence, 9);
  assert.equal(merged.simulation_tick, 21);
  assert.equal(merged.world_hash, "after");
  assert.deepEqual(
    merged.players.map((candidate) => candidate.player_id),
    ["player-local", "player-remote"],
  );
  assert.equal(merged.players[0].position.x, 10);
  assert.equal(merged.players[1].position.x, 40);
  assert.equal("inventory_id" in merged.players[1], false);
  assert.equal("career" in merged.players[1], false);
  assert.equal(merged.grids[0].position.x, 2);
  assert.equal(merged.grids[1].position.x, 11);
  assert.deepEqual(merged.grids[1].blocks, [2]);
  assert.equal(state.players[0].position.x, 4, "the prior snapshot stays immutable");
});

test("motion without a public roster cannot synthesize private player state", () => {
  const local = player("player-local", 1);
  const remote = player("player-remote", 4);
  const merged = plain(api.mergeMotionState(
    { player: local, players: [remote, local], grids: [] },
    {
      event_sequence: 2,
      simulation_tick: 3,
      world_hash: "legacy",
      actor_private: {
        player_id: "player-local",
        position: { x: 8, y: 2, z: 3 },
        last_processed_input_sequence: 99,
      },
      players: [],
      grids: [],
    },
  ));

  assert.equal(merged.players[0].position.x, 1);
  assert.equal(merged.players[1].position.x, 4);
  assert.equal("actor_private" in merged, false);
});

test("player presentations distinguish primary and session-bound pilots", () => {
  const state = {
    player: player("player-local", 1),
    players: [
      player("player-remote", 4, "incapacitated"),
      player("player-local", 1),
    ],
  };
  const spectator = plain(api.playerPresentations(state, { kind: "spectator" }));
  const bound = plain(api.playerPresentations(state, {
    kind: "player",
    player_id: "player-remote",
  }));

  assert.deepEqual(
    spectator.map((presentation) => presentation.player.player_id),
    ["player-local", "player-remote"],
  );
  assert.equal(spectator[0].status, "ALIVE");
  assert.equal(spectator[1].status, "INCAPACITATED");
  assert.equal(bound[0].isPrimary, false);
  assert.equal(bound[0].isBound, false);
  assert.equal(bound[1].isBound, true);
  assert.equal(bound[1].label, "player-remote [YOU]");
  assert.equal(bound[1].status, "BOUND // INCAPACITATED");
  assert.equal(bound[1].color, api.playerColor("player-remote", 1));
  assert.notEqual(bound[0].color, bound[1].color);
});

test("environment presentation follows the selected pilot with legacy fallback", () => {
  const local = player("player-local", 1);
  const remote = player("player-remote", 4);
  local.environment = { celestial_body_name: "Khepri Prime", gravity_m_s2: 9.8 };
  remote.environment = { celestial_body_name: "Deep Space", gravity_m_s2: 0 };
  const state = {
    player: local,
    players: [remote, local],
    environment: { celestial_body_name: "Legacy Primary", gravity_m_s2: 9.8 },
  };

  assert.equal(api.environmentForPlayer(state, remote).celestial_body_name, "Deep Space");
  delete remote.environment;
  assert.equal(
    api.environmentForPlayer(state, remote).celestial_body_name,
    "Legacy Primary",
  );
});

test("publicProjection strips any private overlay before browser state storage", () => {
  const projected = {
    projection_schema_version: 1,
    event_sequence: 12,
    players: [player("player-local", 1)],
    actor_private: {
      player: { player_id: "player-local", inventory_id: "secret-suit" },
      inventories: [{ inventory_id: "secret-suit", contents: { ore: 7 } }],
      death_drops: [{ drop_id: "secret-drop" }],
    },
  };

  const publicState = plain(api.publicProjection(projected));
  assert.equal("actor_private" in publicState, false);
  assert.equal(JSON.stringify(publicState).includes("secret"), false);
  assert.equal("actor_private" in projected, true, "the received object stays immutable");
});

test("raw spectator UI marks economics private and ships no inventory reader", () => {
  assert.match(indexSource, /PRIVATE TO PILOT/);
  assert.match(indexSource, /id="refine" disabled/);
  assert.match(indexSource, /id="craft" disabled/);
  assert.equal(source.includes(".inventories"), false);
  assert.equal(source.includes("inventory_id"), false);
  assert.match(source, /protocol_version: 14/);
  assert.equal(source.includes("operation_id"), false);
  assert.equal(source.includes("operation_sequence"), false);
  assert.equal(source.includes("function intent"), false);
});
