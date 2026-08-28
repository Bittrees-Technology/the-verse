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
const verifierWorkerSource = await readFile(
  new URL("./verifier-worker.js", import.meta.url),
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

function address(xUm = 0, cellX = 500, sectorX = "0") {
  return {
    universe_id: "the-verse-local",
    sector: { x: sectorX, y: "0", z: "0" },
    cell: { x: cellX, y: 500, z: 500 },
    local_um: { x: xUm, y: 0, z: 0 },
  };
}

const manifest = {
  schema_version: 2,
  manifest_hash: "manifest-hash",
  universe_id: "the-verse-local",
  address_schema_version: 1,
  sector_edge_um: 20_000_000_000_000,
  cell_edge_um: 20_000_000_000,
  cells_per_sector_axis: 1000,
  celestial_registry_schema_version: 1,
  celestial_registry_hash: "registry-hash",
  content_schema_version: 11,
  content_manifest_version: "p1.5.0",
  content_hash: "content-hash",
  world_schema_version: 18,
  event_schema_version: 14,
};

const celestialRegistry = {
  schema_version: 1,
  registry_hash: "registry-hash",
  universe_id: "the-verse-local",
  bodies: [{
    body_id: "khepri-prime",
    display_name: "Khepri Prime",
    kind: "planet",
    center: address(900_000_000),
    surface_radius_um: 1_200_000_000,
    content_manifest_version: "p1.5.0",
    content_hash: "content-hash",
    scale_class: "proof",
  }],
};

function entity(kind, id, value, revision = 1) {
  return {
    entity_id: id,
    kind,
    projected_revision: revision,
    component_schema_version: 3,
    payload: { entity_kind: kind, value },
  };
}

function interest(frameKind, sequence, viewHash, operations = {}) {
  return {
    schema_version: 1,
    frame_kind: frameKind,
    session_epoch: "session-a",
    interest_epoch: 1,
    baseline_id: "baseline-a",
    delta_sequence: sequence,
    observer_class: "public_origin_spectator",
    cell_address: address(),
    local_origin_address: address(),
    registry_hash: "registry-hash",
    universe_manifest_hash: "manifest-hash",
    canonical_event_sequence: sequence + 10,
    canonical_tick: sequence + 20,
    canonical_world_hash: `world-${sequence}`,
    previous_view_hash: sequence === 0 ? undefined : `view-${sequence - 1}`,
    view_hash: viewHash,
    entered: operations.entered ?? [],
    replaced: operations.replaced ?? [],
    removed: operations.removed ?? [],
  };
}

function baselineFixture() {
  const frame = interest("baseline", 0, "view-0", {
    entered: [
      entity("player", "pilot-a", {
        player_id: "pilot-a",
        address: address(1_000_000),
        orientation: { x: 0, y: 0, z: 0, w: 1 },
        linear_velocity: { x: 0, y: 0, z: 0 },
        angular_velocity: { x: 0, y: 0, z: 0 },
        life_state: "alive",
      }),
      entity("voxel_chunk", "origin:chunk:0:0:0", {
        chunk_id: "origin:chunk:0:0:0",
        body_id: "origin-asteroid",
        revision: 1,
        voxels: [{ coordinate: { x: 0, y: 0, z: 0 }, material: "stone" }],
      }),
    ],
  });
  return {
    projection_schema_version: 3,
    schema_version: 18,
    content_manifest_version: "p1.5.0",
    universe_id: "the-verse-local",
    cell_id: "cell-origin",
    universe_manifest_hash: "manifest-hash",
    celestial_registry_hash: "registry-hash",
    cell_address: address(),
    gravity_body_id: "khepri-prime",
    voxel_body_id: "origin-asteroid",
    event_sequence: 10,
    simulation_tick: 20,
    fencing_token: 1,
    world_hash: "world-0",
    players: [],
    environment: {
      celestial_body_name: "Khepri Prime",
      gravity_m_s2: 0,
      atmosphere_density: 0,
    },
    voxel_chunks: [],
    grids: [],
    death_drops: [],
    conservation_valid: true,
    interest: frame,
    actor_private: { inventories: [{ contents: { ore: 99 } }] },
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

test("protocol 16 tuple and registry binding fail closed on any incompatible field", () => {
  const welcome = {
    protocol_version: 16,
    projection_schema_version: 3,
    world_schema_version: 18,
    event_schema_version: 14,
    content_schema_version: 11,
    content_manifest_version: "p1.5.0",
    celestial_registry_schema_version: 1,
    universe_manifest_schema_version: 2,
    interest_schema_version: 1,
  };
  assert.equal(api.protocolTupleMatches(welcome), true);
  assert.equal(api.protocolTupleMatches({ ...welcome, event_schema_version: 13 }), false);
  assert.equal(api.registryBindingIsValid(celestialRegistry, manifest), true);
  assert.equal(api.registryBindingIsValid(celestialRegistry, {
    ...manifest,
    celestial_registry_hash: "substituted",
  }), false);
});

test("exact address projection crosses a sector boundary without JavaScript precision loss", () => {
  const origin = address(0, 999, "0");
  const adjacent = address(1_000_000, 0, "1");
  assert.deepEqual(
    plain(api.exactAddressOffsetMeters(adjacent, origin, manifest)),
    { x: 20_001, y: 0, z: 0 },
  );
  const tooFar = address(0, 0, "999999999999999999999999999999999999");
  assert.equal(api.exactAddressOffsetMeters(tooFar, origin, manifest), undefined);
});

test("verified presentation JSON preserves unsafe protocol integers exactly", () => {
  const raw = "{\"safe\":9007199254740991,\"unsafe\":9007199254740992," +
    "\"negative\":-9007199254740992,\"fraction\":1.25," +
    "\"escaped\":\"integer 9007199254740993 and \\\"quote\\\"\"}";
  const parsed = plain(api.parseLosslessVerifiedJson(raw));
  assert.equal(parsed.safe, 9_007_199_254_740_991);
  assert.equal(parsed.unsafe, "9007199254740992");
  assert.equal(parsed.negative, "-9007199254740992");
  assert.equal(parsed.fraction, 1.25);
  assert.equal(parsed.escaped, 'integer 9007199254740993 and "quote"');
  assert.equal(api.exactIntegerEquals(parsed.unsafe, 9_007_199_254_740_992), false);
  assert.equal(
    api.exactIntegerIsSuccessor("9007199254740992", 9_007_199_254_740_991),
    true,
  );
  assert.equal(
    api.exactIntegerCompare("18446744073709551615", "18446744073709551614"),
    1,
  );
});

test("interest baseline materializes only complete operations and strips actor-private data", () => {
  api.setRegistryForTest(celestialRegistry, manifest);
  const state = plain(api.worldFromInterestBaseline(baselineFixture()));
  assert.equal(state.players.length, 1);
  assert.equal(state.players[0].player_id, "pilot-a");
  assert.equal(state.players[0].position.x, 1);
  assert.equal(state.voxel_chunks.length, 1);
  assert.equal(state.voxels.length, 1);
  assert.equal(state.celestial_bodies[0].position.x, 900);
  assert.equal("actor_private" in state, false);
  assert.equal(JSON.stringify(state).includes("\"ore\":99"), false);

  const malformed = baselineFixture();
  malformed.interest.entered.push(malformed.interest.entered[0]);
  assert.equal(api.worldFromInterestBaseline(malformed), undefined);
});

test("contiguous interest deltas replace, enter, and remove atomically", () => {
  api.setRegistryForTest(celestialRegistry, manifest);
  const baseline = api.worldFromInterestBaseline(baselineFixture());
  api.setFrontierForTest(baseline.interest);
  const deltaFrame = interest("delta", 1, "view-1", {
    entered: [entity("grid", "grid-a", {
      grid_id: "grid-a",
      owner_player_id: "pilot-a",
      address: address(3_000_000),
      orientation: { x: 0, y: 0, z: 0, w: 1 },
      linear_velocity: { x: 0, y: 0, z: 0 },
      angular_velocity: { x: 0, y: 0, z: 0 },
      anchored: false,
      power: { produced: 1, required: 0, stored: 0, online: true },
      blocks: [],
    })],
    replaced: [entity("player", "pilot-a", {
      player_id: "pilot-a",
      address: address(2_000_000),
      orientation: { x: 0, y: 0, z: 0, w: 1 },
      linear_velocity: { x: 1, y: 0, z: 0 },
      angular_velocity: { x: 0, y: 0, z: 0 },
      life_state: "alive",
    }, 2)],
    removed: [{
      entity_id: "origin:chunk:0:0:0",
      kind: "voxel_chunk",
      reason: "out_of_interest",
    }],
  });
  const next = plain(api.applyInterestDelta(baseline, {
    projection_schema_version: 3,
    schema_version: 18,
    content_manifest_version: "p1.5.0",
    universe_id: "the-verse-local",
    universe_manifest_hash: "manifest-hash",
    celestial_registry_hash: "registry-hash",
    cell_address: address(),
    gravity_body_id: "khepri-prime",
    voxel_body_id: "origin-asteroid",
    event_sequence: 11,
    simulation_tick: 21,
    world_hash: "world-1",
    interest: deltaFrame,
  }));
  assert.equal(next.players[0].position.x, 2);
  assert.equal(next.grids[0].position.x, 3);
  assert.equal(next.voxel_chunks.length, 0);
  assert.equal(next.voxels.length, 0);

  api.setFrontierForTest(baseline.interest);
  deltaFrame.previous_view_hash = "wrong";
  assert.equal(api.applyInterestDelta(baseline, {
    ...next,
    interest: deltaFrame,
  }), undefined);
});

test("local and universe maps expose distinct bounded layers", () => {
  const state = api.worldFromInterestBaseline(baselineFixture());
  const local = plain(api.mapObjectsForState(state, "local"));
  const universe = plain(api.mapObjectsForState(state, "universe", {
    players: false,
    grids: false,
    voxels: false,
  }));
  assert.deepEqual(local.map((object) => object.kind).sort(), ["player", "voxel"]);
  assert.deepEqual(universe.map((object) => object.kind), ["celestial"]);
  assert.equal(universe[0].radiusM, 1_200);
});

test("map fit, projection, and nearest-marker selection are deterministic", () => {
  const objects = [
    { kind: "grid", id: "left", position: { x: -10, z: 0 }, radiusM: 1 },
    { kind: "grid", id: "right", position: { x: 10, z: 0 }, radiusM: 1 },
  ];
  const view = plain(api.fitMapView(objects, 900, 560));
  assert.equal(view.x, 0);
  assert.equal(view.z, 0);
  assert.ok(view.pixelsPerMeter > 1);
  const point = plain(api.projectMapPoint(objects[1].position, view, 900, 560));
  const markers = [
    { x: 20, y: 20, hitRadius: 5, object: objects[0] },
    { x: point.x, y: point.y, hitRadius: 7, object: objects[1] },
  ];
  assert.equal(api.nearestMapMarker(markers, point.x + 2, point.y).object.id, "right");
  assert.equal(api.nearestMapMarker(markers, 899, 559), undefined);
});

test("raw spectator UI marks economics private and ships no inventory reader", () => {
  assert.match(indexSource, /PRIVATE TO PILOT/);
  assert.match(indexSource, /id="refine" disabled/);
  assert.match(indexSource, /id="craft" disabled/);
  assert.equal(source.includes(".inventories"), false);
  assert.equal(source.includes("inventory_id"), false);
  assert.match(source, /const PROTOCOL_VERSION = 16/);
  assert.match(source, /interest_baseline/);
  assert.match(source, /new Worker\("\/verifier-worker\.js", \{ type: "module" \}\)/);
  assert.match(source, /prepare_verified_frame/);
  assert.equal(source.includes("new WebSocket"), false);
  assert.equal(source.includes("sendInterestAcknowledgement"), false);
  assert.match(indexSource, /Interactive public universe map/);
  assert.equal(source.includes("operation_id"), false);
  assert.equal(source.includes("operation_sequence"), false);
  assert.equal(source.includes("function intent"), false);
});

test("browser verifier is pinned to the proof universe commitments", () => {
  assert.match(source, /expected_universe_id: EXPECTED_UNIVERSE_ID/);
  assert.match(source, /expected_celestial_registry_hash: EXPECTED_CELESTIAL_REGISTRY_HASH/);
  assert.match(source, /expected_universe_manifest_hash: EXPECTED_UNIVERSE_MANIFEST_HASH/);
  assert.match(source, /expected_content_hash: EXPECTED_CONTENT_HASH/);
  assert.match(source, /4c367bbfa04218ece14104f0a3a7ec2c7e9fefcc37d4cf78a265df2d711a59da/);
  assert.match(source, /08f96738abee769d2f9998a9666970ef6cd8474f3270977aec1a50672aad814e/);
  assert.match(source, /fc61c05b335fb951868010ecf2942a92ec4f03d00d0a75d3acba8c6f5162b6bd/);
});

test("reconnect scheduling coalesces worker error and close recovery", () => {
  api.resetReconnectForTest();
  const scheduled = [];
  let reconnects = 0;
  const schedule = (callback, delay) => scheduled.push({ callback, delay });
  const reconnect = () => { reconnects += 1; };

  assert.equal(api.scheduleReconnectForTest(schedule, reconnect), true);
  assert.equal(api.scheduleReconnectForTest(schedule, reconnect), false);
  assert.equal(scheduled.length, 1);
  assert.equal(scheduled[0].delay, 1200);
  scheduled[0].callback();
  assert.equal(reconnects, 1);

  assert.equal(api.scheduleReconnectForTest(schedule, reconnect), true);
  assert.equal(scheduled.length, 2);
  assert.equal(scheduled[1].delay, 2400);
  scheduled[1].callback();

  for (const expectedDelay of [4800, 9600, 19200, 30000]) {
    assert.equal(api.scheduleReconnectForTest(schedule, reconnect), true);
    assert.equal(scheduled.at(-1).delay, expectedDelay);
    scheduled.at(-1).callback();
  }
  assert.equal(reconnects, 6);
  assert.equal(api.scheduleReconnectForTest(schedule, reconnect), false);
});

test("verified state commits before best-effort rendering and survives render failure", () => {
  const installedWorld = {
    marker: "verified-world",
    interest: {
      session_epoch: "session-render",
      interest_epoch: 7,
      baseline_id: "baseline-render",
      delta_sequence: 9,
      view_hash: "verified-hash",
    },
  };
  const candidate = { frameId: "render-1", kind: "delta", world: installedWorld };
  api.commitVerifiedPresentation(candidate);
  const beforeRender = plain(api.verifiedPresentationStateForTest());
  assert.equal(beforeRender.connectionPhase, "live");
  assert.equal(beforeRender.world.marker, "verified-world");
  assert.equal(beforeRender.interestFrontier.view_hash, "verified-hash");

  const presented = api.presentCommittedPresentation(candidate, {
    fitCurrentMap() {},
    render() { throw new Error("synthetic canvas failure"); },
    activity() {},
  });
  assert.equal(presented, false);
  assert.deepEqual(plain(api.verifiedPresentationStateForTest()), beforeRender);

  const commitAt = source.indexOf("commitVerifiedPresentation(installed)");
  const confirmAt = source.indexOf('type: "presentation_installed"', commitAt);
  const renderAt = source.indexOf("presentCommittedPresentation(installed)", confirmAt);
  assert.ok(commitAt >= 0 && confirmAt > commitAt && renderAt > confirmAt);
});

test("verified fatal state is terminal and cannot schedule a normal reconnect", () => {
  const candidate = {
    frameId: "fatal-1",
    kind: "fatal",
    message: "FATAL halted: authoritative stream stopped",
    error: true,
  };
  api.commitVerifiedPresentation(candidate);
  assert.equal(api.verifiedPresentationStateForTest().connectionPhase, "fatal");
  assert.match(source, /installed\.kind === "fatal"\) endVerifiedTransport\(worker, false\)/);
  assert.match(source, /connectionPhase = reconnect \? "disconnected" : "fatal"/);
});

test("worker initialization, verification, and presentation have hard deadlines", () => {
  assert.match(source, /VERIFIER_INITIALIZATION_TIMEOUT_MS = 15_000/);
  assert.match(source, /VERIFIER_OPERATION_TIMEOUT_MS = 10_000/);
  assert.match(source, /VERIFIED_PRESENTATION_TIMEOUT_MS = 10_000/);
  assert.match(source, /verified presentation transition timed out/);
  assert.match(source, /verifier_operation_started/);
  assert.match(source, /verifier_operation_completed/);
  assert.match(verifierWorkerSource, /reportVerifierOperation\("stage"/);
  assert.match(verifierWorkerSource, /reportVerifierOperation\("commit"/);
  assert.match(verifierWorkerSource, /reportVerifierOperation\("discard"/);
});
