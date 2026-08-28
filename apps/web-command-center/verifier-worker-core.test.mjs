// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";
import test from "node:test";

import { VerifiedFrameBridge } from "./verifier-worker-core.js";

function harness(stageResponses, commitResponse = { ok: true, stage_id: "s1" }) {
  const sent = [];
  const closed = [];
  const posted = [];
  const calls = [];
  const verifier = {
    stage(raw) {
      calls.push(["stage", raw]);
      return JSON.stringify(stageResponses.shift());
    },
    commit(id) {
      calls.push(["commit", id]);
      return JSON.stringify(commitResponse);
    },
    discard(id) {
      calls.push(["discard", id]);
      return JSON.stringify({ ok: true, stage_id: id });
    },
  };
  const socket = {
    send(raw) { sent.push(raw); },
    close(code, reason) { closed.push({ code, reason }); },
  };
  return {
    bridge: new VerifiedFrameBridge(verifier, socket, (message) => posted.push(message)),
    sent,
    closed,
    posted,
    calls,
  };
}

test("raw frames are serialized through prepare, commit, install, and exact ACK", () => {
  const first = {
    ok: true,
    stage_id: "s1",
    kind: "baseline",
    message_json: "{\"type\":\"interest_baseline\"}",
  };
  const second = {
    ok: true,
    stage_id: "s2",
    kind: "delta",
    message_json: "{\"type\":\"interest_delta\"}",
  };
  const exactAck = "{\"type\":\"acknowledge_interest\",\"delta_sequence\":0}";
  const state = harness([first, second], {
    ok: true,
    stage_id: "s1",
    acknowledgement_json: exactAck,
  });

  state.bridge.receive("raw-baseline");
  state.bridge.receive("raw-delta");
  assert.deepEqual(state.calls, [["stage", "raw-baseline"]]);
  assert.equal(state.posted[0].type, "prepare_verified_frame");
  assert.deepEqual(state.sent, []);

  state.bridge.prepared("1");
  assert.deepEqual(state.calls[1], ["commit", "s1"]);
  assert.equal(state.posted[1].type, "install_verified_frame");
  assert.deepEqual(state.sent, [], "ACK stays worker-owned until presentation install");

  state.bridge.installed("1");
  assert.deepEqual(state.sent, [exactAck]);
  assert.deepEqual(state.calls[2], ["stage", "raw-delta"]);
});

test("verification failures close without acknowledgement", () => {
  const state = harness([{ ok: false, code: "hash_mismatch", detail: "bad hash" }]);
  state.bridge.receive("tampered");
  assert.deepEqual(state.sent, []);
  assert.equal(state.closed.length, 1);
  assert.equal(state.posted[0].type, "verification_failed");
  assert.equal(state.posted[0].code, "hash_mismatch");
});

test("only one frontier mismatch requests one bounded recovery baseline", () => {
  const state = harness([
    { ok: false, code: "frontier_mismatch", detail: "gap" },
    { ok: false, code: "frontier_mismatch", detail: "another gap" },
  ]);
  state.bridge.receive("first-gap");
  assert.deepEqual(state.sent, ["{\"type\":\"request_snapshot\"}"]);
  assert.deepEqual(state.closed, []);
  state.bridge.receive("second-gap");
  assert.equal(state.sent.length, 1);
  assert.equal(state.closed.length, 1);
});

test("presentation rejection discards an uncommitted stage and closes", () => {
  const state = harness([{
    ok: true,
    stage_id: "s1",
    kind: "welcome",
    message_json: "{\"type\":\"welcome\"}",
  }]);
  state.bridge.receive("raw-welcome");
  state.bridge.rejected("1", "UI tuple mismatch");
  assert.deepEqual(state.calls[1], ["discard", "s1"]);
  assert.equal(state.closed.length, 1);
  assert.deepEqual(state.sent, []);
});
