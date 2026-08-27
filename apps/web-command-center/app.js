// SPDX-License-Identifier: AGPL-3.0-or-later

const elements = Object.fromEntries(
  [
    "connection", "universe", "cell", "event-sequence", "world-hash",
    "conservation", "fence", "world-counts", "grid-count", "ore", "refined",
    "components", "selected-grid", "grid-details", "activity", "universe-map",
    "resync", "refine", "craft", "anchor", "stop", "profile-rank",
    "career-progress",
    "active-players", "session-status", "player-roster", "profile-label",
  ].map((id) => [id, document.getElementById(id)]),
);

let socket;
let world;
let actorPrivate;
let operationSequence = 0;
let selectedGridId = "grid-starter";
let sessionRole = { kind: "spectator" };

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

function connect() {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  socket = new WebSocket(protocol + "//" + location.host + "/ws");
  socket.addEventListener("open", () => {
    elements.connection.textContent = "● CONNECTED";
    elements.connection.className = "connection online";
    socket.send(JSON.stringify({
      type: "hello",
      protocol_version: 13,
      client_name: "browser-command-center-p1.2",
      authentication: { kind: "spectator" },
    }));
  });
  socket.addEventListener("close", () => {
    world = undefined;
    actorPrivate = undefined;
    sessionRole = { kind: "spectator" };
    elements.connection.textContent = "○ RECONNECTING";
    elements.connection.className = "connection offline";
    setTimeout(connect, 1200);
  });
  socket.addEventListener("message", ({ data }) => {
    const message = JSON.parse(data);
    if (message.type === "welcome") {
      sessionRole = message.session_role ?? { kind: "spectator" };
      elements["session-status"].textContent = sessionDescription();
      elements.connection.textContent = sessionRole.kind === "player"
        ? "● PILOT LINK"
        : "● SPECTATING";
      activity(
        sessionRole.kind === "spectator"
          ? "Public spectator session — gameplay controls are read-only"
          : "Gameplay session bound to " + sessionRole.player_id,
        false,
      );
    } else if (message.type === "snapshot") {
      actorPrivate = undefined;
      world = publicProjection(message.snapshot);
      render();
    } else if (message.type === "motion_state" && world) {
      const motion = message.motion;
      if (motion.event_sequence <= world.event_sequence) return;
      world = mergeMotionState(world, motion);
      render();
    } else if (message.type === "intent_accepted") {
      activity(message.receipt.message, false);
    } else if (message.type === "intent_rejected") {
      activity(message.code + ": " + message.message, true);
    } else if (message.type === "fatal") {
      activity("FATAL " + message.code + ": " + message.message, true);
    }
  });
}

function publicProjection(projected) {
  if (!projected || typeof projected !== "object") return undefined;
  const { actor_private: _private, ...publicState } = projected;
  return publicState;
}

function intent(type, payload = {}) {
  if (!socket || socket.readyState !== WebSocket.OPEN) {
    activity("No authoritative connection", true);
    return;
  }
  if (sessionRole.kind !== "player") {
    activity("This public spectator session is read-only", true);
    return;
  }
  operationSequence += 1;
  socket.send(JSON.stringify({
    type,
    operation_id: [
      "browser", type, Date.now(), operationSequence,
    ].join("-"),
    ...payload,
  }));
}

function selectedGrid() {
  const preferred = world?.grids.find((grid) => grid.grid_id === selectedGridId);
  return preferred ?? world?.grids[0];
}

function environmentForPlayer(state, player = selectedPlayer(state)) {
  return player?.environment ?? state?.environment;
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

  const grid = selectedGrid();
  if (grid) {
    selectedGridId = grid.grid_id;
    const frames = grid.blocks.filter(
      (block) => !block.construction_complete,
    ).length;
    const damaged = grid.blocks.filter(
      (block) => block.construction_complete && block.health < block.max_health,
    ).length;
    elements["selected-grid"].textContent = grid.grid_id.toUpperCase();
    elements["grid-details"].textContent =
      grid.blocks.length + " blocks • " +
      frames + " frames • " + damaged + " damaged • " +
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
      grid.linear_velocity.x === 0 &&
      grid.linear_velocity.y === 0 &&
      grid.linear_velocity.z === 0 &&
      grid.angular_velocity.x === 0 &&
      grid.angular_velocity.y === 0 &&
      grid.angular_velocity.z === 0
    );
  } else {
    elements.anchor.disabled = true;
    elements.stop.disabled = true;
  }
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

function drawMap() {
  const canvas = elements["universe-map"];
  const context = canvas.getContext("2d");
  const width = canvas.width;
  const height = canvas.height;
  context.clearRect(0, 0, width, height);
  context.fillStyle = "#03070c";
  context.fillRect(0, 0, width, height);

  context.strokeStyle = "rgba(69, 122, 155, 0.15)";
  context.lineWidth = 1;
  for (let x = 0; x < width; x += 45) {
    context.beginPath();
    context.moveTo(x, 0);
    context.lineTo(x, height);
    context.stroke();
  }
  for (let y = 0; y < height; y += 45) {
    context.beginPath();
    context.moveTo(0, y);
    context.lineTo(width, y);
    context.stroke();
  }

  const scale = 21;
  const project = ({ x, z }) => ({
    x: width / 2 + x * scale,
    y: height / 2 + z * scale,
  });
  for (const voxel of world.voxels) {
    const point = project(voxel.coordinate);
    context.fillStyle = voxel.material === "ferrite_ore"
      ? "rgba(233, 124, 44, 0.58)"
      : "rgba(102, 118, 132, 0.24)";
    context.fillRect(point.x - 2, point.y - 2, 4, 4);
  }
  for (const grid of world.grids) {
    const point = project(grid.position);
    context.fillStyle = "#73d7ff";
    context.beginPath();
    context.arc(point.x, point.y, 7, 0, Math.PI * 2);
    context.fill();
    context.fillStyle = "#a9c6d7";
    context.font = "11px system-ui";
    context.fillText(grid.grid_id, point.x + 11, point.y + 4);
  }
  for (const [index, presentation] of playerPresentations(world).entries()) {
    const pilot = presentation.player;
    const point = project(pilot.position);
    context.save();
    context.translate(point.x, point.y);
    context.rotate((index % 2 === 0 ? 1 : -1) * Math.PI / 4);
    context.fillStyle = presentation.color;
    context.strokeStyle = presentation.isBound
      ? "#ffffff"
      : "rgba(3, 7, 12, 0.9)";
    context.lineWidth = presentation.isBound ? 3 : 2;
    context.beginPath();
    context.moveTo(0, -9);
    context.lineTo(8, 7);
    context.lineTo(-8, 7);
    context.closePath();
    context.fill();
    context.stroke();
    if (presentation.isPrimary) {
      context.strokeStyle = "rgba(255, 255, 255, 0.7)";
      context.lineWidth = 1;
      context.strokeRect(-11, -11, 22, 22);
    }
    context.restore();
    context.fillStyle = presentation.color;
    context.font = "600 11px system-ui";
    context.fillText(
      presentation.label,
      point.x + 13,
      point.y - 7,
    );
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
  document.getElementById("anchor").addEventListener("click", () => {
    const grid = selectedGrid();
    if (grid) intent("toggle_grid_anchor", { grid_id: grid.grid_id });
  });
  document.getElementById("stop").addEventListener("click", () => {
    const grid = selectedGrid();
    if (grid) {
      intent("set_grid_control", {
        grid_id: grid.grid_id,
        linear_input: { x: 0, y: 0, z: 0 },
        angular_input: { x: 0, y: 0, z: 0 },
        dampeners: true,
      });
    }
  });
  elements["universe-map"].addEventListener("click", () => {
    if (!world?.grids.length) return;
    const current = world.grids.findIndex(
      (grid) => grid.grid_id === selectedGridId,
    );
    selectedGridId = world.grids[(current + 1) % world.grids.length].grid_id;
    render();
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
  };
} else {
  start();
}
