// SPDX-License-Identifier: AGPL-3.0-or-later

const MAX_QUEUED_FRAMES = 16;
const MAX_QUEUED_BYTES = 16 * 1024 * 1024;

function decodeVerifierResponse(raw) {
  try {
    const response = JSON.parse(raw);
    if (!response || typeof response.ok !== "boolean") return undefined;
    return response;
  } catch {
    return undefined;
  }
}

/**
 * Serializes raw WebSocket frames through one staged verifier transition.
 * The main thread receives only verifier-reserialized JSON and never controls
 * the exact acknowledgement bytes.
 */
export class VerifiedFrameBridge {
  constructor(verifier, socket, postToMain) {
    this.verifier = verifier;
    this.socket = socket;
    this.postToMain = postToMain;
    this.queue = [];
    this.queuedBytes = 0;
    this.pending = undefined;
    this.nextFrameId = 1;
    this.recoveryRequested = false;
    this.closed = false;
  }

  receive(rawFrame) {
    if (this.closed) return;
    if (typeof rawFrame !== "string") {
      this.fail("binary_frame", "binary server frames are forbidden");
      return;
    }
    const bytes = new TextEncoder().encode(rawFrame).byteLength;
    if (this.queue.length >= MAX_QUEUED_FRAMES ||
        this.queuedBytes + bytes > MAX_QUEUED_BYTES) {
      this.fail("frame_queue_limit", "verified presentation queue limit exceeded");
      return;
    }
    this.queue.push({ rawFrame, bytes });
    this.queuedBytes += bytes;
    this.drain();
  }

  prepared(frameId) {
    if (!this.matches(frameId, "staged")) return this.failProtocol();
    const committed = decodeVerifierResponse(
      this.verifier.commit(this.pending.stageId),
    );
    if (!committed?.ok || committed.stage_id !== this.pending.stageId) {
      this.fail(
        committed?.code ?? "commit_failure",
        committed?.detail ?? "verifier commit failed",
      );
      return;
    }
    this.pending.phase = "committed";
    this.pending.acknowledgementJson = committed.acknowledgement_json;
    this.postToMain({ type: "install_verified_frame", frameId });
  }

  installed(frameId) {
    if (!this.matches(frameId, "committed")) return this.failProtocol();
    const acknowledgement = this.pending.acknowledgementJson;
    if (acknowledgement !== undefined) this.socket.send(acknowledgement);
    this.pending = undefined;
    this.drain();
  }

  rejected(frameId, reason = "presentation rejected verified frame") {
    if (!this.pending || this.pending.frameId !== frameId) {
      return this.failProtocol();
    }
    if (this.pending.phase === "staged") {
      const discarded = decodeVerifierResponse(
        this.verifier.discard(this.pending.stageId),
      );
      if (!discarded?.ok) {
        return this.fail(
          discarded?.code ?? "discard_failure",
          discarded?.detail ?? "verifier discard failed",
        );
      }
    }
    this.fail("presentation_rejected", reason);
  }

  send(rawClientMessage) {
    if (this.closed || typeof rawClientMessage !== "string") return;
    this.socket.send(rawClientMessage);
  }

  drain() {
    if (this.closed || this.pending || this.queue.length === 0) return;
    const queued = this.queue.shift();
    this.queuedBytes -= queued.bytes;
    const staged = decodeVerifierResponse(this.verifier.stage(queued.rawFrame));
    if (!staged?.ok) {
      if (staged?.code === "frontier_mismatch" && !this.recoveryRequested) {
        this.recoveryRequested = true;
        this.socket.send(JSON.stringify({ type: "request_snapshot" }));
        this.drain();
        return;
      }
      this.fail(
        staged?.code ?? "verifier_response_invalid",
        staged?.detail ?? "verifier returned an invalid response",
      );
      return;
    }
    if (typeof staged.stage_id !== "string" ||
        typeof staged.message_json !== "string" ||
        typeof staged.kind !== "string") {
      this.fail("verifier_response_invalid", "staged response is incomplete");
      return;
    }
    const frameId = String(this.nextFrameId++);
    this.pending = {
      frameId,
      stageId: staged.stage_id,
      phase: "staged",
      acknowledgementJson: undefined,
    };
    this.postToMain({
      type: "prepare_verified_frame",
      frameId,
      kind: staged.kind,
      messageJson: staged.message_json,
    });
  }

  matches(frameId, phase) {
    return this.pending?.frameId === frameId && this.pending.phase === phase;
  }

  failProtocol() {
    this.fail("presentation_sequence", "presentation confirmation is out of sequence");
  }

  fail(code, detail) {
    if (this.closed) return;
    this.closed = true;
    this.postToMain({ type: "verification_failed", code, detail });
    this.socket.close(1002, String(code).slice(0, 123));
  }
}

export const __VERSE_VERIFIER_WORKER_TEST_API__ = {
  decodeVerifierResponse,
  MAX_QUEUED_FRAMES,
  MAX_QUEUED_BYTES,
};
