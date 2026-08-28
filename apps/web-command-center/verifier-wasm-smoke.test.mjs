// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";

const generatedDirectory = process.env.VERSE_BROWSER_VERIFIER_GENERATED_DIR;
const loaderUrl = generatedDirectory
  ? pathToFileURL(resolve(generatedDirectory, "verse_interest_verifier.js"))
  : new URL("./generated/verse_interest_verifier.js", import.meta.url);
const wasmUrl = generatedDirectory
  ? pathToFileURL(resolve(generatedDirectory, "verse_interest_verifier_bg.wasm"))
  : new URL("./generated/verse_interest_verifier_bg.wasm", import.meta.url);
const generatedModule = await import(loaderUrl.href);
const {
  default: initializeWasm,
  BrowserInterestVerifier,
} = generatedModule;

const wasmBytes = await readFile(wasmUrl);
await initializeWasm({ module_or_path: wasmBytes });

const proofCommitments = {
  expected_content_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  expected_universe_id: "universe-vector",
  expected_celestial_registry_hash: "f00517b0fbef09d7924fde2cb11f2c74066627992ab900a6a9e0bd3ac3dc7311",
  expected_universe_manifest_hash: "a3c80a0f93da41f3409918795a5a11a5369fe4046e48993c887d3cd60ade7975",
};

async function readVector(name) {
  const raw = await readFile(
    new URL(`../../crates/verse-interest-verifier/test-vectors/v1/${name}`, import.meta.url),
    "utf8",
  );
  assert.equal(raw.endsWith("\n"), true, `${name} must end in one LF`);
  assert.equal(raw.endsWith("\n\n"), false, `${name} must end in exactly one LF`);
  return raw.slice(0, -1);
}

function stageAndCommit(verifier, raw) {
  const staged = JSON.parse(verifier.stage(raw));
  assert.equal(staged.ok, true, staged.detail);
  const committed = JSON.parse(verifier.commit(staged.stage_id));
  assert.equal(committed.ok, true, committed.detail);
  return { staged, committed };
}

function portableVerifier() {
  return new BrowserInterestVerifier(JSON.stringify({
    expected_role: "player",
    expected_player_id: "player-vector",
    world_schema_version: "18",
    event_schema_version: "14",
    content_schema_version: "11",
    content_manifest_version: "p1.5.0",
    ...proofCommitments,
  }));
}

test("generated string-only WASM verifier initializes and rejects malformed frames", () => {
  const verifier = new BrowserInterestVerifier(JSON.stringify({
    expected_role: "spectator",
    world_schema_version: "18",
    event_schema_version: "14",
    content_schema_version: "11",
    content_manifest_version: "p1.5.0",
    ...proofCommitments,
  }));
  assert.equal(JSON.parse(verifier.readiness()).ok, true);
  const rejected = JSON.parse(verifier.stage("{not-json"));
  assert.equal(rejected.ok, false);
  assert.equal(rejected.code, "invalid_json");
  verifier.free();
});

test("generated WASM verifies the exact bound-player portable stream without losing u64 precision", async () => {
  const [welcome, registry, baseline, delta, baselineAck, deltaAck] = await Promise.all([
    readVector("welcome.json"),
    readVector("registry.json"),
    readVector("baseline.json"),
    readVector("delta.json"),
    readVector("baseline.ack.json"),
    readVector("delta.ack.json"),
  ]);
  const verifier = new BrowserInterestVerifier(JSON.stringify({
    expected_role: "player",
    expected_player_id: "player-vector",
    world_schema_version: "18",
    event_schema_version: "14",
    content_schema_version: "11",
    content_manifest_version: "p1.5.0",
    ...proofCommitments,
  }));

  assert.equal(JSON.parse(verifier.readiness()).ok, true);
  stageAndCommit(verifier, welcome);
  stageAndCommit(verifier, registry);

  const installedBaseline = stageAndCommit(verifier, baseline);
  assert.equal(installedBaseline.staged.kind, "baseline");
  assert.equal(
    installedBaseline.staged.message_json.includes('"interest_epoch":9007199254740993'),
    true,
    "sanitized message must preserve the exact integer token",
  );
  assert.equal(installedBaseline.committed.acknowledgement_json, baselineAck);

  const installedDelta = stageAndCommit(verifier, delta);
  assert.equal(installedDelta.staged.kind, "delta");
  assert.equal(installedDelta.committed.acknowledgement_json, deltaAck);
  verifier.free();
});

test("generated WASM rejects a tampered portable baseline without advancing state", async () => {
  const [welcome, registry, baseline, baselineAck] = await Promise.all([
    readVector("welcome.json"),
    readVector("registry.json"),
    readVector("baseline.json"),
    readVector("baseline.ack.json"),
  ]);
  const verifier = new BrowserInterestVerifier(JSON.stringify({
    expected_role: "player",
    expected_player_id: "player-vector",
    world_schema_version: "18",
    event_schema_version: "14",
    content_schema_version: "11",
    content_manifest_version: "p1.5.0",
    ...proofCommitments,
  }));

  stageAndCommit(verifier, welcome);
  stageAndCommit(verifier, registry);
  const tampered = baseline.replace('"ore":7', '"ore":8');
  assert.notEqual(tampered, baseline, "tamper fixture must modify the baseline");
  const rejected = JSON.parse(verifier.stage(tampered));
  assert.equal(rejected.ok, false);
  assert.equal(rejected.code, "hash_mismatch");

  const installed = stageAndCommit(verifier, baseline);
  assert.equal(installed.committed.acknowledgement_json, baselineAck);
  verifier.free();
});

test("generated WASM rejects every frozen raw invalid frame and recovers exactly", async () => {
  const corpus = JSON.parse(await readVector("invalid-corpus.json"));
  assert.equal(corpus.schema_version, 1);
  assert.equal(corpus.cases.length, 16);

  for (const fixture of corpus.cases) {
    const verifier = portableVerifier();
    for (const prerequisite of fixture.prerequisites) {
      stageAndCommit(verifier, await readVector(prerequisite));
    }
    const rejected = JSON.parse(verifier.stage(await readVector(fixture.frame)));
    assert.equal(rejected.ok, false, `${fixture.name} must fail`);
    assert.equal(rejected.code, fixture.expected_code, fixture.name);
    assert.equal(rejected.stage_id, undefined, `${fixture.name} must not stage`);
    assert.equal(
      rejected.acknowledgement_json,
      undefined,
      `${fixture.name} must not acknowledge`,
    );

    const recovered = stageAndCommit(
      verifier,
      await readVector(fixture.recovery_frame),
    );
    assert.equal(
      recovered.committed.acknowledgement_json !== undefined,
      fixture.target === "baseline" || fixture.target === "delta",
      `${fixture.name} recovery ACK shape`,
    );
    verifier.free();
  }
});
