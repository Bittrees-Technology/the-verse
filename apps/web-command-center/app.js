// SPDX-License-Identifier: AGPL-3.0-or-later

const elements = Object.fromEntries(
  [
    "connection", "universe", "cell", "event-sequence", "world-hash",
    "conservation", "fence", "world-counts", "grid-count", "ore", "refined",
    "components", "selected-grid", "grid-details", "activity", "universe-map",
    "resync", "refine", "craft", "anchor", "stop", "profile-rank",
    "career-progress",
  ].map((id) => [id, document.getElementById(id)]),
);

let socket;
let world;
let operationSequence = 0;
let selectedGridId = "grid-starter";

function connect() {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  socket = new WebSocket(protocol + "//" + location.host + "/ws");
  socket.addEventListener("open", () => {
    elements.connection.textContent = "● CONNECTED";
    elements.connection.className = "connection online";
    socket.send(JSON.stringify({
      type: "hello",
      protocol_version: 8,
      client_name: "browser-command-center-p0.9",
    }));
  });
  socket.addEventListener("close", () => {
    elements.connection.textContent = "○ RECONNECTING";
    elements.connection.className = "connection offline";
    setTimeout(connect, 1200);
  });
  socket.addEventListener("message", ({ data }) => {
    const message = JSON.parse(data);
    if (message.type === "snapshot") {
      world = message.snapshot;
      render();
    } else if (message.type === "motion_state" && world) {
      const motion = message.motion;
      if (motion.event_sequence <= world.event_sequence) return;
      const gridMotion = new Map(
        motion.grids.map((grid) => [grid.grid_id, grid]),
      );
      world = {
        ...world,
        event_sequence: motion.event_sequence,
        simulation_tick: motion.simulation_tick,
        world_hash: motion.world_hash,
        player: { ...world.player, ...motion.player },
        grids: world.grids.map((grid) => ({
          ...grid,
          ...(gridMotion.get(grid.grid_id) ?? {}),
        })),
      };
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

function intent(type, payload = {}) {
  if (!socket || socket.readyState !== WebSocket.OPEN) {
    activity("No authoritative connection", true);
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

function playerInventory() {
  return world?.inventories.find(
    (inventory) => inventory.inventory_id === "inventory-player-local",
  );
}

function selectedGrid() {
  const preferred = world?.grids.find((grid) => grid.grid_id === selectedGridId);
  return preferred ?? world?.grids[0];
}

function render() {
  elements.universe.textContent = world.universe_id;
  elements.cell.textContent = world.cell_id;
  elements["event-sequence"].textContent =
    world.event_sequence.toLocaleString() + " / " +
    world.simulation_tick.toLocaleString();
  elements["world-hash"].textContent = "hash " + world.world_hash.slice(0, 16);
  elements.conservation.textContent = world.conservation.valid ? "VALID" : "INVALID";
  elements.conservation.style.color = world.conservation.valid
    ? "var(--green)"
    : "var(--red)";
  elements.fence.textContent = "fence " + world.fencing_token;
  elements["world-counts"].textContent =
    world.voxels.length.toLocaleString() + " voxels";
  elements["grid-count"].textContent =
    world.grids.length.toLocaleString() + " grids • " +
    world.environment.celestial_body_name + " • " +
    (world.environment.gravity_m_s2 / 9.80665).toFixed(2) + " g • " +
    Math.round(world.environment.atmosphere_density * 100) + "% atmosphere";

  const inventory = playerInventory()?.contents ?? {};
  const ore = inventory.ore ?? 0;
  const refined = inventory.refined_material ?? 0;
  elements.ore.textContent = (inventory.ore ?? 0).toLocaleString();
  elements.refined.textContent =
    (inventory.refined_material ?? 0).toLocaleString();
  elements.components.textContent =
    (inventory.components ?? 0).toLocaleString();
  const career = world.player.career ?? {};
  elements["profile-rank"].textContent =
    "SALVAGER // LEVEL " + (world.player.level ?? 1);
  elements["career-progress"].textContent =
    (world.player.experience ?? 0).toLocaleString() + " / " +
    (world.player.next_level_experience ?? 100).toLocaleString() + " XP • " +
    (career.voxels_mined ?? 0).toLocaleString() + " VOXELS • " +
    (career.blocks_built ?? 0).toLocaleString() + " BLOCKS";
  elements.refine.disabled = ore < 2;
  elements.craft.disabled = refined < 1;

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
    elements.anchor.disabled = !grid.anchored && (!hasAnchor || !grid.power.online);
    elements.stop.disabled = grid.anchored || (
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
  drawMap();
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
  const player = project(world.player.position);
  context.fillStyle = "#49e29a";
  context.beginPath();
  context.moveTo(player.x, player.y - 8);
  context.lineTo(player.x + 7, player.y + 6);
  context.lineTo(player.x - 7, player.y + 6);
  context.closePath();
  context.fill();
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

document.getElementById("resync").addEventListener("click", () => {
  socket?.send(JSON.stringify({ type: "request_snapshot" }));
});
document.getElementById("refine").addEventListener("click", () => {
  intent("refine_ore", {
    inventory_id: "inventory-player-local",
    batches: 1,
  });
});
document.getElementById("craft").addEventListener("click", () => {
  intent("craft_component", {
    inventory_id: "inventory-player-local",
    quantity: 1,
  });
});
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
  const current = world.grids.findIndex((grid) => grid.grid_id === selectedGridId);
  selectedGridId = world.grids[(current + 1) % world.grids.length].grid_id;
  render();
});

connect();
