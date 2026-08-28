// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";
import { performance } from "node:perf_hooks";

import { COMPATIBILITY, Protocol16InterestStream } from "./interest-stream.mjs";

const url = process.argv[2] ?? "ws://127.0.0.1:17779/ws";
const DISTRIBUTIONS = Object.freeze([2, 8, 16, 32, 64]);
const SESSION_TIMEOUT_MS = 45_000;
const HARNESS_TIMEOUT_MS = 120_000;
const clients = [];

function elapsedMillis(startedAt) {
  return Number((performance.now() - startedAt).toFixed(3));
}

function sum(values) {
  return values.reduce((total, value) => total + value, 0);
}

function summarize(values) {
  assert.ok(values.length > 0, "a metric summary has samples");
  const sorted = [...values].sort((left, right) => left - right);
  const percentile = (ratio) =>
    sorted[Math.max(0, Math.ceil(sorted.length * ratio) - 1)];
  return {
    samples: sorted.length,
    minimum: sorted[0],
    median: percentile(0.5),
    p95: percentile(0.95),
    maximum: sorted.at(-1),
    mean: Number((sum(sorted) / sorted.length).toFixed(3)),
  };
}

function wireByteSummary(activeClients) {
  const field = (name) =>
    sum(activeClients.map((client) => client.wireBytes[name]));
  return {
    welcome: field("welcome"),
    registry: field("registry"),
    initial_baseline: field("initialBaseline"),
    resync_baseline: field("resyncBaseline"),
    other_server_messages: field("other"),
    client_messages: field("client"),
  };
}

class SpectatorSession {
  constructor(index) {
    this.index = index;
    this.socket = new WebSocket(url);
    this.buffered = [];
    this.waiters = [];
    this.failure = undefined;
    this.acknowledgements = 0;
    this.initialBaselineReceived = false;
    this.wireBytes = {
      welcome: 0,
      registry: 0,
      initialBaseline: 0,
      resyncBaseline: 0,
      other: 0,
      client: 0,
    };
    this.stream = new Protocol16InterestStream({
      send: (message) => {
        if (message.type === "acknowledge_interest") {
          this.acknowledgements += 1;
        }
        this.send(message);
      },
    });
    this.socket.addEventListener("message", (event) => {
      try {
        assert.equal(
          typeof event.data,
          "string",
          "the server uses JSON text frames",
        );
        const message = JSON.parse(event.data);
        const bytes = Buffer.byteLength(event.data);
        if (message.type === "welcome") this.wireBytes.welcome += bytes;
        else if (message.type === "registry") this.wireBytes.registry += bytes;
        else if (message.type === "interest_baseline") {
          if (this.initialBaselineReceived)
            this.wireBytes.resyncBaseline += bytes;
          else this.wireBytes.initialBaseline += bytes;
        } else this.wireBytes.other += bytes;
        const applied = this.stream.receive(message);
        if (
          applied.type === "interest_state" &&
          applied.frame_kind === "baseline"
        ) {
          this.initialBaselineReceived = true;
        }
        this.dispatch(applied);
      } catch (error) {
        this.fail(error);
      }
    });
    this.socket.addEventListener("error", () => {
      this.fail(new Error(`spectator ${this.index} socket failed`));
    });
    this.socket.addEventListener("close", () => {
      if (!this.closing && !this.failure) {
        this.fail(new Error(`spectator ${this.index} socket closed early`));
      }
    });
  }

  dispatch(message) {
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
    this.failure = error instanceof Error ? error : new Error(String(error));
    for (const waiter of this.waiters.splice(0)) {
      clearTimeout(waiter.timeout);
      waiter.reject(this.failure);
    }
  }

  waitFor(predicate, description, timeoutMillis = SESSION_TIMEOUT_MS) {
    if (this.failure) return Promise.reject(this.failure);
    const index = this.buffered.findIndex(predicate);
    if (index >= 0) return Promise.resolve(this.buffered.splice(index, 1)[0]);
    return new Promise((resolve, reject) => {
      const waiter = { predicate, resolve, reject, timeout: undefined };
      waiter.timeout = setTimeout(() => {
        const index = this.waiters.indexOf(waiter);
        if (index >= 0) this.waiters.splice(index, 1);
        reject(
          new Error(
            `spectator ${this.index} timed out waiting for ${description}`,
          ),
        );
      }, timeoutMillis);
      this.waiters.push(waiter);
    });
  }

  send(message) {
    assert.equal(this.socket.readyState, WebSocket.OPEN);
    const text = JSON.stringify(message);
    this.wireBytes.client += Buffer.byteLength(text);
    this.socket.send(text);
  }

  async connect() {
    const startedAt = performance.now();
    await new Promise((resolve, reject) => {
      if (this.socket.readyState === WebSocket.OPEN) {
        resolve();
        return;
      }
      const timeout = setTimeout(
        () => reject(new Error(`spectator ${this.index} open timed out`)),
        SESSION_TIMEOUT_MS,
      );
      this.socket.addEventListener(
        "open",
        () => {
          clearTimeout(timeout);
          resolve();
        },
        { once: true },
      );
      this.socket.addEventListener(
        "error",
        () => {
          clearTimeout(timeout);
          reject(new Error(`spectator ${this.index} could not connect`));
        },
        { once: true },
      );
    });
    this.send({
      type: "hello",
      protocol_version: COMPATIBILITY.protocol_version,
      client_name: `node-p15-scale-spectator-${this.index}`,
      authentication: { kind: "spectator" },
    });
    await this.waitFor(
      (message) => message.type === "welcome",
      "protocol welcome",
    );
    await this.waitFor(
      (message) => message.type === "registry",
      "celestial registry",
    );
    const baseline = await this.waitFor(
      (message) =>
        message.type === "interest_state" && message.frame_kind === "baseline",
      "initial interest baseline",
    );
    assert.equal(this.acknowledgements, 1);
    return {
      elapsed_ms: elapsedMillis(startedAt),
      canonical_event_sequence: baseline.projection.event_sequence,
      canonical_tick: baseline.projection.simulation_tick,
      visible_entities:
        baseline.interest.entered.length - baseline.interest.removed.length,
      registry_hash: this.stream.registry.registry_hash,
      universe_manifest_hash: this.stream.manifest.manifest_hash,
    };
  }

  async requestFreshBaseline() {
    const prior = structuredClone(this.stream.frontier);
    const startedAt = performance.now();
    this.send({ type: "request_snapshot" });
    const baseline = await this.waitFor(
      (message) =>
        message.type === "interest_state" &&
        message.frame_kind === "baseline" &&
        message.interest.interest_epoch === prior.interest_epoch + 1,
      "fresh interest baseline",
    );
    assert.equal(baseline.interest.session_epoch, prior.session_epoch);
    assert.notEqual(baseline.interest.baseline_id, prior.baseline_id);
    return elapsedMillis(startedAt);
  }

  async close() {
    this.closing = true;
    if (
      this.socket.readyState === WebSocket.CLOSED ||
      this.socket.readyState === WebSocket.CLOSING
    ) {
      return;
    }
    await new Promise((resolve) => {
      const timeout = setTimeout(resolve, 2_000);
      this.socket.addEventListener(
        "close",
        () => {
          clearTimeout(timeout);
          resolve();
        },
        { once: true },
      );
      this.socket.close(1000, "P1.5 scale evidence complete");
    });
  }
}

async function collectEvidence() {
  const measurements = [];
  const allConnectionSamples = [];
  const registryHashes = new Set();
  const manifestHashes = new Set();
  for (const target of DISTRIBUTIONS) {
    const cohort = [];
    for (let index = clients.length; index < target; index += 1) {
      const client = new SpectatorSession(index + 1);
      clients.push(client);
      cohort.push(client);
    }
    const cohortResults = await Promise.all(
      cohort.map((client) => client.connect()),
    );
    allConnectionSamples.push(
      ...cohortResults.map((result) => result.elapsed_ms),
    );
    for (const result of cohortResults) {
      registryHashes.add(result.registry_hash);
      manifestHashes.add(result.universe_manifest_hash);
    }
    assert.equal(clients.length, target);
    assert.ok(
      clients.every((client) => client.socket.readyState === WebSocket.OPEN),
      `${target} spectator sessions remain concurrently open`,
    );

    // Probe the first and last members of the newly admitted cohort. This
    // distributes recovery work and stays below the server's intentional
    // per-session recovery-rate limit.
    const firstCohortIndex = target - cohort.length;
    const probeIndexes =
      firstCohortIndex === target - 1
        ? [firstCohortIndex]
        : [firstCohortIndex, target - 1];
    const resyncSamples = await Promise.all(
      probeIndexes.map((index) => clients[index].requestFreshBaseline()),
    );
    const activeResults = clients.map((client) => ({
      event_sequence: client.stream.projection.event_sequence,
      simulation_tick: client.stream.projection.simulation_tick,
      visible_entities: client.stream.entities.size,
    }));
    measurements.push({
      observer_sessions: target,
      newly_connected_sessions: cohort.length,
      successful_sessions: target,
      failed_sessions: 0,
      cohort_handshake_to_applied_baseline_ms: summarize(
        cohortResults.map((result) => result.elapsed_ms),
      ),
      cumulative_handshake_to_applied_baseline_ms:
        summarize(allConnectionSamples),
      resync_probe_sessions: probeIndexes.map((index) => index + 1),
      resync_to_applied_baseline_ms: summarize(resyncSamples),
      acknowledgements_sent: sum(
        clients.map((client) => client.acknowledgements),
      ),
      visible_entities_per_session: summarize(
        activeResults.map((result) => result.visible_entities),
      ),
      canonical_event_sequence: summarize(
        activeResults.map((result) => result.event_sequence),
      ),
      canonical_tick: summarize(
        activeResults.map((result) => result.simulation_tick),
      ),
      wire_bytes_cumulative: wireByteSummary(clients),
    });
  }
  assert.equal(
    registryHashes.size,
    1,
    "all observers bind one celestial registry",
  );
  assert.equal(
    manifestHashes.size,
    1,
    "all observers bind one universe manifest",
  );
  return {
    evidence_schema_version: 1,
    scenario_id: "p1.5-local-public-origin-observer-distribution-v1",
    production_readiness_claim: false,
    protocol_compatibility: COMPATIBILITY,
    distribution_targets: DISTRIBUTIONS,
    maximum_concurrent_sessions: DISTRIBUTIONS.at(-1),
    runtime_bound_ms: HARNESS_TIMEOUT_MS,
    registry_hash: [...registryHashes][0],
    universe_manifest_hash: [...manifestHashes][0],
    measurements,
    limitations: [
      "single local simulation-worker process",
      "paused deterministic proof universe",
      "public-origin spectator observers only",
      "JSON WebSocket protocol-17 proof transport",
      "no WAN latency, packet loss, multi-region, or failover",
      "not evidence of thousand-player or production capacity",
    ],
  };
}

async function main() {
  let timeout;
  try {
    const evidence = await Promise.race([
      collectEvidence(),
      new Promise((_, reject) => {
        timeout = setTimeout(
          () =>
            reject(new Error(`scale harness exceeded ${HARNESS_TIMEOUT_MS}ms`)),
          HARNESS_TIMEOUT_MS,
        );
      }),
    ]);
    console.log(JSON.stringify(evidence));
  } finally {
    clearTimeout(timeout);
    await Promise.allSettled(clients.map((client) => client.close()));
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
