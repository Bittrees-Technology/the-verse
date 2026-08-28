// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createServer, request as httpRequest } from "node:http";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const upstreamOrigin = new URL(
  process.argv[2] ?? "http://127.0.0.1:17777/",
);
const TIMEOUT_MILLIS = 15_000;
const WEBSOCKET_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const REQUIRED_SHIPPED_ASSETS = [
  "/",
  "/app.js",
  "/styles.css",
  "/verifier-worker.js",
  "/verifier-worker-core.js",
  "/generated/verse_interest_verifier.js",
  "/generated/verse_interest_verifier_bg.wasm",
];

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function eventually(operation, description, timeoutMillis = TIMEOUT_MILLIS) {
  const deadline = Date.now() + timeoutMillis;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await operation();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await delay(50);
  }
  throw new Error(
    `timed out waiting for ${description}` +
      (lastError ? `: ${lastError.message}` : ""),
  );
}

function findBrowserBinary() {
  const configured = process.env.VERSE_BROWSER_BIN;
  if (configured) return configured;
  const fixedCandidates = process.platform === "darwin"
    ? [
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ]
    : [];
  for (const candidate of fixedCandidates) {
    const result = spawnSync(candidate, ["--version"], { encoding: "utf8" });
    if (result.status === 0) return candidate;
  }
  for (const candidate of [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
  ]) {
    const result = spawnSync(candidate, ["--version"], { encoding: "utf8" });
    if (result.status === 0) return candidate;
  }
  throw new Error(
    "a Chrome/Chromium browser is required for the shipped-page E2E; " +
      "install it or set VERSE_BROWSER_BIN to its executable",
  );
}

class DevToolsClient {
  constructor(websocketUrl) {
    this.socket = new WebSocket(websocketUrl);
    this.sequence = 0;
    this.pending = new Map();
    this.events = [];
    this.opened = new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", () => {
        reject(new Error("Chrome DevTools connection failed"));
      }, { once: true });
    });
    this.socket.addEventListener("message", ({ data }) => {
      const message = JSON.parse(data);
      if (message.id !== undefined) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        if (message.error) {
          pending.reject(new Error(message.error.message));
        } else {
          pending.resolve(message.result);
        }
      } else {
        this.events.push(message);
      }
    });
  }

  async send(method, params = {}) {
    await this.opened;
    const id = ++this.sequence;
    const response = new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
    this.socket.send(JSON.stringify({ id, method, params }));
    return response;
  }

  async evaluate(expression) {
    const result = await this.send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.text ?? "page evaluation failed");
    }
    return result.result.value;
  }

  close() {
    this.socket.close();
  }
}

async function launchBrowser() {
  const browserBinary = findBrowserBinary();
  const versionResult = spawnSync(browserBinary, ["--version"], { encoding: "utf8" });
  assert.equal(versionResult.status, 0, "the selected browser reports its version");
  const browserVersion = (versionResult.stdout || versionResult.stderr).trim();
  const profileDirectory = await mkdtemp(join(tmpdir(), "verse-browser-e2e."));
  const arguments_ = [
    "--headless=new",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-default-apps",
    "--disable-extensions",
    "--disable-features=Translate,OptimizationHints",
    "--disable-sync",
    "--metrics-recording-only",
    "--no-first-run",
    "--no-default-browser-check",
    "--remote-allow-origins=*",
    "--remote-debugging-port=0",
    `--user-data-dir=${profileDirectory}`,
    "about:blank",
  ];
  if (typeof process.getuid === "function" && process.getuid() === 0) {
    arguments_.push("--no-sandbox");
  }
  const child = spawn(browserBinary, arguments_, {
    stdio: ["ignore", "ignore", "pipe"],
  });
  const exited = new Promise((resolve) => child.once("exit", resolve));
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  const debuggerUrl = await eventually(() => {
    const match = stderr.match(/DevTools listening on (ws:\/\/[^\s]+)/);
    if (child.exitCode !== null) {
      throw new Error(`Chrome exited with status ${child.exitCode}: ${stderr}`);
    }
    return match?.[1];
  }, "Chrome DevTools endpoint");
  const endpoint = new URL(debuggerUrl);

  return {
    async newPage() {
      const descriptorUrl =
        `http://${endpoint.host}/json/new?${encodeURIComponent("about:blank")}`;
      const response = await fetch(descriptorUrl, { method: "PUT" });
      assert.equal(response.ok, true, "Chrome creates a new isolated page");
      const descriptor = await response.json();
      const client = new DevToolsClient(descriptor.webSocketDebuggerUrl);
      await Promise.all([
        client.send("Page.enable"),
        client.send("Runtime.enable"),
        client.send("Log.enable"),
        client.send("Network.enable"),
      ]);
      return client;
    },
    binary: browserBinary,
    version: browserVersion,
    async close() {
      child.kill("SIGTERM");
      await Promise.race([
        exited,
        delay(2_000),
      ]);
      if (child.exitCode === null) child.kill("SIGKILL");
      await rm(profileDirectory, { recursive: true, force: true });
    },
  };
}

function encodeServerFrame(opcode, payload = Buffer.alloc(0)) {
  const body = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  let header;
  if (body.length < 126) {
    header = Buffer.from([0x80 | opcode, body.length]);
  } else if (body.length <= 0xffff) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(body.length, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(body.length), 2);
  }
  return Buffer.concat([header, body]);
}

class BrowserFrameReader {
  constructor(onText, onClose, onPong) {
    this.buffer = Buffer.alloc(0);
    this.fragments = [];
    this.fragmentOpcode = undefined;
    this.onText = onText;
    this.onClose = onClose;
    this.onPong = onPong;
  }

  receive(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (this.buffer.length >= 2) {
      const first = this.buffer[0];
      const second = this.buffer[1];
      const final = (first & 0x80) !== 0;
      const opcode = first & 0x0f;
      const masked = (second & 0x80) !== 0;
      let length = second & 0x7f;
      let offset = 2;
      if (length === 126) {
        if (this.buffer.length < 4) return;
        length = this.buffer.readUInt16BE(2);
        offset = 4;
      } else if (length === 127) {
        if (this.buffer.length < 10) return;
        const wideLength = this.buffer.readBigUInt64BE(2);
        assert.ok(
          wideLength <= BigInt(Number.MAX_SAFE_INTEGER),
          "browser WebSocket frame length is representable",
        );
        length = Number(wideLength);
        offset = 10;
      }
      const maskLength = masked ? 4 : 0;
      if (this.buffer.length < offset + maskLength + length) return;
      const mask = masked ? this.buffer.subarray(offset, offset + 4) : undefined;
      offset += maskLength;
      const payload = Buffer.from(this.buffer.subarray(offset, offset + length));
      this.buffer = this.buffer.subarray(offset + length);
      if (mask) {
        for (let index = 0; index < payload.length; index += 1) {
          payload[index] ^= mask[index % 4];
        }
      }
      this.frame(opcode, final, payload);
    }
  }

  frame(opcode, final, payload) {
    if (opcode === 0x8) {
      this.onClose(payload);
      return;
    }
    if (opcode === 0x9) {
      this.onPong(payload);
      return;
    }
    if (opcode === 0x1 || opcode === 0x2) {
      this.fragmentOpcode = opcode;
      this.fragments = [payload];
    } else if (opcode === 0x0 && this.fragmentOpcode !== undefined) {
      this.fragments.push(payload);
    } else {
      return;
    }
    if (!final) return;
    const complete = Buffer.concat(this.fragments);
    const completeOpcode = this.fragmentOpcode;
    this.fragments = [];
    this.fragmentOpcode = undefined;
    if (completeOpcode === 0x1) this.onText(complete.toString("utf8"));
  }
}

function canonicalAcknowledgement(rawInterestFrame) {
  const message = JSON.parse(rawInterestFrame);
  const interest = message.baseline?.interest ?? message.delta?.interest;
  assert.ok(interest, "interest frame carries acknowledgement frontier");
  return JSON.stringify({
    type: "acknowledge_interest",
    session_epoch: interest.session_epoch,
    interest_epoch: interest.interest_epoch,
    baseline_id: interest.baseline_id,
    delta_sequence: interest.delta_sequence,
    view_hash: interest.view_hash,
  });
}

async function startShippedPageProxy({ tamperBaseline }) {
  const servedPaths = new Set();
  const originalInterestFrames = [];
  const browserFrames = [];
  const forwardedFrames = [];
  const sockets = new Set();
  const upstreamSockets = new Set();
  let tamperedFrames = 0;

  const server = createServer((request, response) => {
    const target = new URL(request.url, upstreamOrigin);
    servedPaths.add(target.pathname);
    const proxyRequest = httpRequest(target, {
      method: request.method,
      headers: { ...request.headers, host: upstreamOrigin.host },
    }, (proxyResponse) => {
      response.writeHead(proxyResponse.statusCode ?? 502, proxyResponse.headers);
      proxyResponse.pipe(response);
    });
    proxyRequest.on("error", (error) => {
      response.writeHead(502, { "content-type": "text/plain; charset=utf-8" });
      response.end(`command-center asset proxy failed: ${error.message}`);
    });
    request.pipe(proxyRequest);
  });

  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
  });
  server.on("upgrade", (request, browserSocket) => {
    const path = new URL(request.url, upstreamOrigin).pathname;
    assert.equal(path, "/ws", "command center connects only to the shipped WS route");
    const key = request.headers["sec-websocket-key"];
    assert.equal(typeof key, "string", "browser supplies a WebSocket key");
    const accept = createHash("sha1")
      .update(key + WEBSOCKET_GUID)
      .digest("base64");
    browserSocket.write(
      "HTTP/1.1 101 Switching Protocols\r\n" +
        "Upgrade: websocket\r\n" +
        "Connection: Upgrade\r\n" +
        `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
    );
    browserSocket.setNoDelay(true);

    const upstreamWebSocketUrl = new URL("/ws", upstreamOrigin);
    upstreamWebSocketUrl.protocol = upstreamOrigin.protocol === "https:"
      ? "wss:"
      : "ws:";
    const upstream = new WebSocket(upstreamWebSocketUrl);
    upstreamSockets.add(upstream);
    const pending = [];
    upstream.addEventListener("open", () => {
      for (const raw of pending.splice(0)) {
        upstream.send(raw);
        forwardedFrames.push(raw);
      }
    });
    upstream.addEventListener("message", ({ data }) => {
      assert.equal(typeof data, "string", "authoritative server sends text frames");
      let delivered = data;
      const message = JSON.parse(data);
      if (message.type === "interest_baseline" || message.type === "interest_delta") {
        originalInterestFrames.push(data);
      }
      if (tamperBaseline && message.type === "interest_baseline") {
        assert.equal(typeof message.baseline.conservation_valid, "boolean");
        message.baseline.conservation_valid = !message.baseline.conservation_valid;
        delivered = JSON.stringify(message);
        tamperedFrames += 1;
      }
      if (!browserSocket.destroyed) {
        browserSocket.write(encodeServerFrame(0x1, delivered));
      }
    });
    upstream.addEventListener("close", ({ code, reason }) => {
      upstreamSockets.delete(upstream);
      if (!browserSocket.destroyed) {
        const reasonBytes = Buffer.from(reason ?? "", "utf8").subarray(0, 123);
        const closePayload = Buffer.alloc(2 + reasonBytes.length);
        closePayload.writeUInt16BE(code || 1000, 0);
        reasonBytes.copy(closePayload, 2);
        browserSocket.end(encodeServerFrame(0x8, closePayload));
      }
    });
    upstream.addEventListener("error", () => browserSocket.destroy());

    const reader = new BrowserFrameReader(
      (raw) => {
        browserFrames.push(raw);
        if (upstream.readyState === WebSocket.OPEN) {
          upstream.send(raw);
          forwardedFrames.push(raw);
        } else {
          pending.push(raw);
        }
      },
      (payload) => {
        if (upstream.readyState < WebSocket.CLOSING) upstream.close();
        if (!browserSocket.destroyed) browserSocket.end(encodeServerFrame(0x8, payload));
      },
      (payload) => {
        if (!browserSocket.destroyed) browserSocket.write(encodeServerFrame(0xA, payload));
      },
    );
    browserSocket.on("data", (chunk) => reader.receive(chunk));
    browserSocket.once("close", () => {
      if (upstream.readyState < WebSocket.CLOSING) upstream.close();
    });
    browserSocket.on("error", () => upstream.close());
  });

  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  assert.ok(address && typeof address === "object");
  return {
    origin: `http://127.0.0.1:${address.port}/`,
    servedPaths,
    originalInterestFrames,
    browserFrames,
    forwardedFrames,
    get tamperedFrames() {
      return tamperedFrames;
    },
    async close() {
      for (const upstream of upstreamSockets) upstream.close();
      for (const socket of sockets) socket.destroy();
      await new Promise((resolve) => server.close(resolve));
    },
  };
}

function browserAcknowledgements(proxy) {
  return proxy.browserFrames.filter((raw) => {
    try {
      return JSON.parse(raw).type === "acknowledge_interest";
    } catch {
      return false;
    }
  });
}

function assertShippedAssets(proxy, label) {
  for (const path of REQUIRED_SHIPPED_ASSETS) {
    assert.ok(proxy.servedPaths.has(path), `${label} loaded shipped asset ${path}`);
  }
}

async function navigate(client, url) {
  await client.send("Page.navigate", { url });
  await eventually(
    () => client.evaluate("document.readyState === 'complete'"),
    `${url} document load`,
  );
}

async function scenarioDiagnostics(page, proxy) {
  let visibleState;
  try {
    visibleState = await page.evaluate(`(() => ({
      connection: document.querySelector("#connection")?.textContent,
      counts: document.querySelector("#world-counts")?.textContent,
      activity: [...document.querySelectorAll("#activity li")].map((item) => item.textContent),
    }))()`);
  } catch (error) {
    visibleState = { evaluation_error: error.message };
  }
  return JSON.stringify({
    visibleState,
    servedPaths: [...proxy.servedPaths].sort(),
    serverFrameTypes: proxy.originalInterestFrames.map((raw) => JSON.parse(raw).type),
    browserFrameTypes: proxy.browserFrames.map((raw) => {
      try {
        return JSON.parse(raw).type;
      } catch {
        return "invalid_json";
      }
    }),
    devtools: page.events
      .filter(({ method }) => method === "Runtime.exceptionThrown" ||
        method === "Log.entryAdded")
      .map(({ method, params }) => ({ method, params })),
  });
}

async function verifyInstalledStream(browser) {
  const proxy = await startShippedPageProxy({ tamperBaseline: false });
  const page = await browser.newPage();
  try {
    await navigate(page, proxy.origin);
    const acknowledgement = await eventually(
      () => browserAcknowledgements(proxy)[0],
      "verifier-owned baseline acknowledgement",
    );
    const originalBaseline = proxy.originalInterestFrames.find(
      (raw) => JSON.parse(raw).type === "interest_baseline",
    );
    assert.equal(typeof originalBaseline, "string");
    assert.equal(
      acknowledgement,
      canonicalAcknowledgement(originalBaseline),
      "the exact canonical ACK bytes are forwarded to the authoritative server",
    );
    assert.ok(
      proxy.forwardedFrames.includes(acknowledgement),
      "the proxy observed the same ACK bytes on the authoritative-server leg",
    );
    await eventually(async () => {
      const state = await page.evaluate(`(() => ({
        connection: document.querySelector("#connection")?.textContent,
        session: document.querySelector("#session-status")?.textContent,
        counts: document.querySelector("#world-counts")?.textContent,
        activity: [...document.querySelectorAll("#activity li")].map((item) => item.textContent),
      }))()`);
      return state.connection === "● SPECTATING" &&
        state.session === "PUBLIC SPECTATOR // READ-ONLY" &&
        state.counts !== "0 voxels" &&
        state.activity.some((line) => line.startsWith("Registry verified //"));
    }, "installed verified command-center presentation");
    assertShippedAssets(proxy, "live command center");
    return acknowledgement;
  } catch (error) {
    throw new Error(`${error.message}; diagnostics=${await scenarioDiagnostics(page, proxy)}`);
  } finally {
    page.close();
    await proxy.close();
  }
}

async function verifyTamperFailsClosed(browser) {
  const proxy = await startShippedPageProxy({ tamperBaseline: true });
  const page = await browser.newPage();
  try {
    await navigate(page, proxy.origin);
    const fatal = await eventually(async () => {
      const state = await page.evaluate(`(() => ({
        connection: document.querySelector("#connection")?.textContent,
        counts: document.querySelector("#world-counts")?.textContent,
        activity: [...document.querySelectorAll("#activity li")].map((item) => item.textContent),
      }))()`);
      return state.activity.some((line) => line.includes("hash_mismatch"))
        ? state
        : undefined;
    }, "hash-mismatch fatal presentation");
    assert.ok(proxy.tamperedFrames >= 1, "the E2E altered a live baseline frame");
    assert.equal(fatal.counts, "0 voxels", "tampered state never reaches presentation");
    await delay(400);
    assert.deepEqual(
      browserAcknowledgements(proxy),
      [],
      "tampered live frames produce zero interest acknowledgements",
    );
    assertShippedAssets(proxy, "tampered command center");
  } catch (error) {
    throw new Error(`${error.message}; diagnostics=${await scenarioDiagnostics(page, proxy)}`);
  } finally {
    page.close();
    await proxy.close();
  }
}

const browser = await launchBrowser();
try {
  const acknowledgement = await verifyInstalledStream(browser);
  await verifyTamperFailsClosed(browser);
  console.log(
    `VERSE_BROWSER_COMMAND_CENTER_OK browser=${JSON.stringify(browser.version)} ` +
      `ack=${acknowledgement} tamper=hash_mismatch tampered_ack_count=0`,
  );
} finally {
  await browser.close();
}
