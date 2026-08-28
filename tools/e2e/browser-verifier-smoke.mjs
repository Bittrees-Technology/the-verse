// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import initializeWasm, {
  BrowserInterestVerifier,
} from "../../apps/web-command-center/generated/verse_interest_verifier.js";

const url = process.argv[2] ?? "ws://127.0.0.1:17777/ws";
const FRAME_TIMEOUT_MILLIS = 8_000;
const DELTA_GRACE_MILLIS = 1_500;
const COMPATIBILITY = Object.freeze({
  protocol_version: 17,
  world_schema_version: "19",
  event_schema_version: "15",
  content_schema_version: "11",
  content_manifest_version: "p1.5.0",
  expected_universe_id: "the-verse-local",
  expected_celestial_registry_hash:
    "4c367bbfa04218ece14104f0a3a7ec2c7e9fefcc37d4cf78a265df2d711a59da",
  expected_universe_manifest_hash:
    "c9bfd3baa1e64ab7665e60c4f989491e745e9af0d2512989f41625b57b546ace",
  expected_content_hash:
    "fc61c05b335fb951868010ecf2942a92ec4f03d00d0a75d3acba8c6f5162b6bd",
});

class FrameTimeout extends Error {}

class RawFrameQueue {
  constructor(socket, label) {
    this.socket = socket;
    this.label = label;
    this.frames = [];
    this.waiters = [];
    this.closed = false;
    this.failure = undefined;
    socket.addEventListener("message", (event) => {
      if (typeof event.data !== "string") {
        this.rejectAll(new Error(`${label} received a non-text server frame`));
        return;
      }
      const waiter = this.waiters.shift();
      if (waiter) {
        clearTimeout(waiter.timeout);
        waiter.resolve(event.data);
      } else {
        this.frames.push(event.data);
      }
    });
    socket.addEventListener("error", () => {
      this.rejectAll(new Error(`${label} WebSocket failed`));
    });
    socket.addEventListener("close", (event) => {
      this.closed = true;
      if (event.code !== 1000) {
        this.rejectAll(
          new Error(
            `${label} WebSocket closed with code ${event.code}: ${event.reason}`,
          ),
        );
      }
    });
  }

  rejectAll(error) {
    this.failure ??= error;
    for (const waiter of this.waiters.splice(0)) {
      clearTimeout(waiter.timeout);
      waiter.reject(error);
    }
  }

  next(description, timeoutMillis = FRAME_TIMEOUT_MILLIS) {
    if (this.frames.length > 0) return Promise.resolve(this.frames.shift());
    if (this.failure) return Promise.reject(this.failure);
    if (this.closed) {
      return Promise.reject(
        new Error(`${this.label} closed while waiting for ${description}`),
      );
    }
    return new Promise((resolve, reject) => {
      const waiter = { resolve, reject, timeout: undefined };
      waiter.timeout = setTimeout(() => {
        const index = this.waiters.indexOf(waiter);
        if (index >= 0) this.waiters.splice(index, 1);
        reject(
          new FrameTimeout(
            `${this.label} timed out waiting for ${description} after ` +
              `${timeoutMillis}ms`,
          ),
        );
      }, timeoutMillis);
      this.waiters.push(waiter);
    });
  }
}

function verifierConfig() {
  return JSON.stringify({
    expected_role: "spectator",
    world_schema_version: COMPATIBILITY.world_schema_version,
    event_schema_version: COMPATIBILITY.event_schema_version,
    content_schema_version: COMPATIBILITY.content_schema_version,
    content_manifest_version: COMPATIBILITY.content_manifest_version,
    expected_universe_id: COMPATIBILITY.expected_universe_id,
    expected_celestial_registry_hash:
      COMPATIBILITY.expected_celestial_registry_hash,
    expected_universe_manifest_hash:
      COMPATIBILITY.expected_universe_manifest_hash,
    expected_content_hash: COMPATIBILITY.expected_content_hash,
  });
}

function response(raw, operation) {
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    throw new Error(`${operation} returned invalid adapter JSON: ${error.message}`);
  }
  assert.equal(typeof parsed?.ok, "boolean", `${operation} reports ok`);
  return parsed;
}

async function connect(label) {
  const socket = new WebSocket(url);
  const queue = new RawFrameQueue(socket, label);
  const outboundFrames = [];
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error(`${label} timed out connecting to ${url}`));
    }, FRAME_TIMEOUT_MILLIS);
    socket.addEventListener(
      "open",
      () => {
        clearTimeout(timeout);
        resolve();
      },
      { once: true },
    );
    socket.addEventListener(
      "error",
      () => {
        clearTimeout(timeout);
        reject(new Error(`${label} failed to connect to ${url}`));
      },
      { once: true },
    );
  });
  const send = (raw, source) => {
    assert.equal(typeof raw, "string", `${label} sends text frames only`);
    outboundFrames.push({ raw, source });
    socket.send(raw);
  };
  send(JSON.stringify({
    type: "hello",
    protocol_version: COMPATIBILITY.protocol_version,
    client_name: label,
    authentication: { kind: "spectator" },
  }), "handshake");
  return { socket, queue, send, outboundFrames };
}

function newVerifier(label) {
  const verifier = new BrowserInterestVerifier(verifierConfig());
  const readiness = response(verifier.readiness(), `${label} readiness`);
  assert.equal(
    readiness.ok,
    true,
    `${label} verifier initializes: ${readiness.code ?? "unknown"} ` +
      `${readiness.detail ?? ""}`,
  );
  return verifier;
}

function wireType(message) {
  if (message.type === "interest_baseline") return "baseline";
  if (message.type === "interest_delta") return "delta";
  return message.type;
}

function preparePresentation(staged, expectedKind, label) {
  assert.equal(staged.kind, expectedKind, `${label} stages ${expectedKind}`);
  assert.equal(typeof staged.stage_id, "string", `${label} has a stage id`);
  assert.equal(
    typeof staged.message_json,
    "string",
    `${label} has verifier-sanitized presentation JSON`,
  );
  const candidate = JSON.parse(staged.message_json);
  assert.equal(
    wireType(candidate),
    expectedKind,
    `${label} presentation consumes only the verifier-sanitized frame`,
  );
  return { candidate, expectedKind, prepared: true, installed: false };
}

function stageInstallCommit(verifier, rawFrame, expectedKind, label) {
  const staged = response(verifier.stage(rawFrame), `${label} stage`);
  assert.equal(
    staged.ok,
    true,
    `${label} stage succeeds: ${staged.code ?? "unknown"} ` +
      `${staged.detail ?? ""}`,
  );
  const presentation = preparePresentation(staged, expectedKind, label);
  const committed = response(
    verifier.commit(staged.stage_id),
    `${label} commit`,
  );
  assert.equal(
    committed.ok,
    true,
    `${label} commit succeeds: ${committed.code ?? "unknown"} ` +
      `${committed.detail ?? ""}`,
  );
  assert.equal(committed.kind, expectedKind, `${label} commits ${expectedKind}`);
  // This is the smoke's bounded stand-in for the main thread atomically
  // installing the already-sanitized candidate and reporting success to the
  // worker. No acknowledgement can be sent before this marker is set.
  presentation.installed = true;
  return { committed, presentation };
}

function sendVerifierAcknowledgement(connection, committed, presentation, label) {
  assert.equal(
    presentation.installed,
    true,
    `${label} is acknowledged only after successful presentation`,
  );
  const exact = committed.acknowledgement_json;
  assert.equal(
    typeof exact,
    "string",
    `${label} acknowledgement is owned and serialized by the verifier`,
  );
  const decoded = JSON.parse(exact);
  assert.equal(decoded.type, "acknowledge_interest");
  assert.deepEqual(
    Object.keys(decoded),
    [
      "type",
      "session_epoch",
      "interest_epoch",
      "baseline_id",
      "delta_sequence",
      "view_hash",
    ],
    `${label} uses the verifier's exact protocol acknowledgement shape`,
  );
  connection.send(exact, "verifier_ack");
  const sent = connection.outboundFrames.at(-1);
  assert.equal(
    sent.raw,
    committed.acknowledgement_json,
    `${label} sends the exact verifier-returned bytes without reconstruction`,
  );
  assert.equal(sent.source, "verifier_ack");
}

async function close(socket) {
  if (socket.readyState === WebSocket.CLOSED) return;
  const closed = new Promise((resolve) => {
    socket.addEventListener("close", resolve, { once: true });
  });
  socket.close(1000, "browser verifier smoke complete");
  await Promise.race([
    closed,
    new Promise((resolve) => setTimeout(resolve, 1_000)),
  ]);
}

async function verifyLiveStream() {
  const verifier = newVerifier("live");
  let connection;
  try {
    connection = await connect("node-browser-verifier-live-e2e");
    const { queue } = connection;

    for (const expectedKind of ["welcome", "registry", "baseline"]) {
      const rawFrame = await queue.next(`original raw ${expectedKind}`);
      const { committed, presentation } = stageInstallCommit(
        verifier,
        rawFrame,
        expectedKind,
        `live ${expectedKind}`,
      );
      if (expectedKind === "baseline") {
        sendVerifierAcknowledgement(
          connection,
          committed,
          presentation,
          "live baseline",
        );
      } else {
        assert.equal(
          committed.acknowledgement_json,
          undefined,
          `${expectedKind} does not produce an interest acknowledgement`,
        );
      }
    }

    let rawSubsequent;
    try {
      rawSubsequent = await queue.next(
        "an advancing verified interest delta",
        DELTA_GRACE_MILLIS,
      );
    } catch (error) {
      if (!(error instanceof FrameTimeout)) throw error;
      connection.send(
        JSON.stringify({ type: "request_snapshot" }),
        "recovery_request",
      );
      rawSubsequent = await queue.next(
        "a bounded verified recovery baseline",
        FRAME_TIMEOUT_MILLIS,
      );
    }
    const parsedSubsequent = JSON.parse(rawSubsequent);
    const subsequentKind = wireType(parsedSubsequent);
    assert.ok(
      subsequentKind === "delta" || subsequentKind === "baseline",
      `expected a subsequent delta or recovery baseline, got ${subsequentKind}`,
    );
    const { committed, presentation } = stageInstallCommit(
      verifier,
      rawSubsequent,
      subsequentKind,
      `live subsequent ${subsequentKind}`,
    );
    sendVerifierAcknowledgement(
      connection,
      committed,
      presentation,
      `live subsequent ${subsequentKind}`,
    );
    const sentAcknowledgements = connection.outboundFrames.filter(
      ({ source }) => source === "verifier_ack",
    );
    assert.equal(
      sentAcknowledgements.length,
      2,
      "only verifier-produced baseline/delta acknowledgements were sent",
    );
    return subsequentKind;
  } finally {
    verifier.free();
    if (connection) await close(connection.socket);
  }
}

async function verifyTamperFailsClosed() {
  const verifier = newVerifier("tamper");
  let connection;
  try {
    connection = await connect("node-browser-verifier-tamper-e2e");
    const { queue } = connection;

    for (const expectedKind of ["welcome", "registry"]) {
      const rawFrame = await queue.next(`tamper path raw ${expectedKind}`);
      const { committed } = stageInstallCommit(
        verifier,
        rawFrame,
        expectedKind,
        `tamper ${expectedKind}`,
      );
      assert.equal(committed.acknowledgement_json, undefined);
    }

    const originalBaseline = await queue.next("tamper path raw baseline");
    const tampered = JSON.parse(originalBaseline);
    assert.equal(tampered.type, "interest_baseline");
    assert.equal(typeof tampered.baseline.conservation_valid, "boolean");
    tampered.baseline.conservation_valid =
      !tampered.baseline.conservation_valid;
    const rejected = response(
      verifier.stage(JSON.stringify(tampered)),
      "tampered baseline stage",
    );
    assert.equal(rejected.ok, false, "tampered baseline is rejected");
    assert.equal(
      rejected.code,
      "hash_mismatch",
      `tampered included view material fails its wire hash: ${rejected.detail}`,
    );
    const impossibleCommit = response(
      verifier.commit("3"),
      "tampered baseline impossible commit",
    );
    assert.equal(
      impossibleCommit.code,
      "invalid_stage_token",
      "hash mismatch creates no committable stage",
    );
    assert.deepEqual(
      connection.outboundFrames.filter(({ raw }) => {
        try {
          return JSON.parse(raw).type === "acknowledge_interest";
        } catch {
          return false;
        }
      }),
      [],
      "tampered path sends no interest acknowledgement",
    );
  } finally {
    verifier.free();
    if (connection) await close(connection.socket);
  }
}

const wasmBytes = await readFile(
  new URL(
    "../../apps/web-command-center/generated/verse_interest_verifier_bg.wasm",
    import.meta.url,
  ),
);
await initializeWasm({ module_or_path: wasmBytes });

const subsequentKind = await verifyLiveStream();
await verifyTamperFailsClosed();
console.log(
  `VERSE_BROWSER_VERIFIER_LIVE_OK subsequent=${subsequentKind} ` +
    "tamper=hash_mismatch ack_source=wasm",
);
