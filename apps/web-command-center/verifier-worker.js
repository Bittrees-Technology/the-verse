// SPDX-License-Identifier: AGPL-3.0-or-later

import { VerifiedFrameBridge } from "/verifier-worker-core.js";

let started = false;
let bridge;
let rawSocket;
let verifierOperationSequence = 0;

function decode(raw) {
  try {
    return JSON.parse(raw);
  } catch {
    return undefined;
  }
}

function reportVerifierOperation(operation, callback) {
  const operationId = String(++verifierOperationSequence);
  postMessage({ type: "verifier_operation_started", operationId, operation });
  try {
    return callback();
  } finally {
    postMessage({ type: "verifier_operation_completed", operationId, operation });
  }
}

function guardedVerifier(verifier) {
  return {
    stage(raw) {
      return reportVerifierOperation("stage", () => verifier.stage(raw));
    },
    commit(stageId) {
      return reportVerifierOperation("commit", () => verifier.commit(stageId));
    },
    discard(stageId) {
      return reportVerifierOperation("discard", () => verifier.discard(stageId));
    },
  };
}

async function start(config) {
  if (started) return;
  started = true;
  try {
    const wasmModule = await import("/generated/verse_interest_verifier.js");
    await wasmModule.default({
      module_or_path: "/generated/verse_interest_verifier_bg.wasm",
    });
    const verifier = new wasmModule.BrowserInterestVerifier(
      config.verifierConfigJson,
    );
    const readiness = decode(verifier.readiness());
    if (!readiness?.ok) {
      postMessage({
        type: "verification_failed",
        code: readiness?.code ?? "initialization",
        detail: readiness?.detail ?? "browser verifier initialization failed",
      });
      return;
    }
    const socket = new WebSocket(config.websocketUrl);
    rawSocket = socket;
    bridge = new VerifiedFrameBridge(
      guardedVerifier(verifier),
      socket,
      (message) => self.postMessage(message),
    );
    socket.addEventListener("open", () => {
      socket.send(config.helloJson);
      postMessage({ type: "transport_open" });
    });
    socket.addEventListener("message", ({ data }) => bridge.receive(data));
    socket.addEventListener("error", () => {
      postMessage({
        type: "transport_error",
        detail: "verified WebSocket transport failed",
      });
    });
    socket.addEventListener("close", ({ code, reason }) => {
      postMessage({ type: "transport_closed", code, reason });
    });
  } catch (error) {
    postMessage({
      type: "verification_failed",
      code: "initialization",
      detail: String(error),
    });
  }
}

self.addEventListener("message", ({ data }) => {
  if (data?.type === "start") {
    void start(data);
  } else if (data?.type === "presentation_prepared") {
    bridge?.prepared(data.frameId);
  } else if (data?.type === "presentation_installed") {
    bridge?.installed(data.frameId);
  } else if (data?.type === "presentation_rejected") {
    bridge?.rejected(data.frameId, data.reason);
  } else if (data?.type === "send") {
    bridge?.send(data.messageJson);
  } else if (data?.type === "close") {
    rawSocket?.close(data.code, data.reason);
  }
});
