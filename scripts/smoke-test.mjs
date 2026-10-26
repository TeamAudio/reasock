#!/usr/bin/env node
// Checks the JSON-RPC protocol against a running REAPER with ReaSock loaded.
// Registers and runs a throwaway action; does not touch the project.
//
//   node scripts/smoke-test.mjs

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import net from "node:net";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { connect, findInstance, mirrorSelection, PROTOCOL_VERSION } from "../examples/reasock-client.mjs";

const { port, token } = findInstance();
let failures = 0;

async function check(name, fn) {
  try {
    await fn();
    console.log(`ok   ${name}`);
  } catch (error) {
    failures++;
    console.log(`FAIL ${name}\n     ${error.message.split("\n").join("\n     ")}`);
  }
}

/** A raw socket for sending exact frames; `next()` waits for the next reply. */
async function rawSocket() {
  const ws = new WebSocket(`ws://127.0.0.1:${port}`);
  const queue = [];
  const waiters = [];
  ws.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    waiters.length ? waiters.shift()(message) : queue.push(message);
  });
  await new Promise((resolve) => ws.addEventListener("open", resolve, { once: true }));
  return {
    send: (frame) => ws.send(typeof frame === "string" ? frame : JSON.stringify(frame)),
    next: (ms = 2000) =>
      queue.length
        ? Promise.resolve(queue.shift())
        : new Promise((resolve, reject) => {
            waiters.push(resolve);
            setTimeout(() => reject(new Error("no reply")), ms);
          }),
    close: () => ws.close(),
  };
}

const errorCode = (reply) => reply.error?.code;

/** The HTTP status line ReaSock answers a WebSocket handshake with. */
function handshakeStatus(origin) {
  return new Promise((resolve, reject) => {
    const socket = net.connect(port, "127.0.0.1", () => {
      socket.write(
        "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n" +
          "Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n" +
          (origin ? `Origin: ${origin}\r\n` : "") +
          "\r\n",
      );
    });
    socket.once("data", (data) => {
      resolve(data.toString().split("\r\n")[0]);
      socket.destroy();
    });
    socket.once("error", reject);
    setTimeout(() => reject(new Error("no answer")), 2000);
  });
}

await check("web pages are refused unless their origin is allowed", async () => {
  assert.match(await handshakeStatus(null), / 101 /);
  assert.match(await handshakeStatus("https://evil.example"), / 403 /);
  // Allow one origin for a moment, restoring any existing config afterwards.
  const config = join(homedir(), ".reasock", "config.json");
  const previous = existsSync(config) ? readFileSync(config) : null;
  try {
    writeFileSync(config, JSON.stringify({ allowedOrigins: ["http://localhost:5173"] }));
    assert.match(await handshakeStatus("http://localhost:5173"), / 101 /);
    assert.match(await handshakeStatus("http://localhost:5174"), / 403 /);
  } finally {
    previous ? writeFileSync(config, previous) : rmSync(config, { force: true });
  }
});

const raw = await rawSocket();

await check("parse error is -32700 with null id", async () => {
  raw.send("{nope");
  const reply = await raw.next();
  assert.equal(errorCode(reply), -32700);
  assert.equal(reply.id, null);
});

await check("missing jsonrpc field is -32600", async () => {
  raw.send({ id: 1, method: "rpc.discover" });
  assert.equal(errorCode(await raw.next()), -32600);
});

await check("rpc.discover works before initialize", async () => {
  raw.send({ jsonrpc: "2.0", id: 2, method: "rpc.discover" });
  const reply = await raw.next();
  assert.equal(reply.jsonrpc, "2.0");
  assert.equal(reply.id, 2);
  assert.match(reply.result.openrpc, /^1\.4\./);
  assert.equal(reply.result.info.version, PROTOCOL_VERSION);
});

await check("other methods need initialize (-32001)", async () => {
  raw.send({ jsonrpc: "2.0", id: 3, method: "action.run", params: { command: 40182 } });
  assert.equal(errorCode(await raw.next()), -32001);
});

await check("unknown method is -32601", async () => {
  raw.send({ jsonrpc: "2.0", id: 4, method: "nope" });
  assert.equal(errorCode(await raw.next()), -32601);
});

await check("positional params are -32602", async () => {
  raw.send({ jsonrpc: "2.0", id: 5, method: "initialize", params: ["0.1.0"] });
  assert.equal(errorCode(await raw.next()), -32602);
});

await check("bad client name is -32602", async () => {
  raw.send({ jsonrpc: "2.0", id: 6, method: "initialize", params: { protocolVersion: PROTOCOL_VERSION, client: { name: "has space" } } });
  assert.equal(errorCode(await raw.next()), -32602);
});

await check("initialize needs this REAPER's token (-32003)", async () => {
  const init = (id, extra) => ({
    jsonrpc: "2.0", id, method: "initialize",
    params: { protocolVersion: PROTOCOL_VERSION, client: { name: "smoke" }, ...extra },
  });
  raw.send(init(20, {}));
  assert.equal(errorCode(await raw.next()), -32003);
  raw.send(init(21, { token: "0".repeat(64) }));
  assert.equal(errorCode(await raw.next()), -32003);
  raw.send(init(22, { token: token.slice(1) }));
  assert.equal(errorCode(await raw.next()), -32003);
});

await check("the instance file is readable only by this user", async () => {
  if (process.platform === "win32") return;
  const { statSync, readdirSync } = await import("node:fs");
  const dir = join(homedir(), ".reasock", "instances");
  const file = join(dir, readdirSync(dir).find((name) => name === `${findInstance().pid}.json`));
  assert.equal(statSync(file).mode & 0o777, 0o600);
  assert.equal(statSync(dir).mode & 0o777, 0o700);
});

await check("notifications get no reply", async () => {
  raw.send({ jsonrpc: "2.0", method: "rpc.discover" });
  raw.send({ jsonrpc: "2.0", id: 7, method: "nope" });
  assert.equal((await raw.next()).id, 7);
});

await check("batch gets an array of replies, skipping notifications", async () => {
  raw.send([
    { jsonrpc: "2.0", id: 8, method: "initialize", params: { protocolVersion: PROTOCOL_VERSION, client: { name: "smoke" }, token } },
    { jsonrpc: "2.0", method: "rpc.discover" },
    { jsonrpc: "2.0", id: 9, method: "initialize", params: { protocolVersion: PROTOCOL_VERSION, client: { name: "smoke" }, token } },
  ]);
  const replies = await raw.next();
  assert.ok(Array.isArray(replies));
  assert.equal(replies.length, 2);
  assert.equal(replies[0].result.protocolVersion, PROTOCOL_VERSION);
  assert.equal(errorCode(replies[1]), -32002);
});

await check("action.run of an unknown command is -32012", async () => {
  raw.send({ jsonrpc: "2.0", id: 10, method: "action.run", params: { command: "_REASOCK_nobody_nothing" } });
  assert.equal(errorCode(await raw.next()), -32012);
});

await check("action.unregister of an unknown id is -32011", async () => {
  raw.send({ jsonrpc: "2.0", id: 11, method: "action.unregister", params: { id: "nothing" } });
  assert.equal(errorCode(await raw.next()), -32011);
});

const client = await connect({ name: "smoke2", port });

await check("register, run, and receive action.triggered", async () => {
  const triggered = new Promise((resolve) => client.on("action.triggered", resolve));
  const action = await client.call("action.register", { id: "ping", label: "Smoke test" });
  assert.equal(action.command, "_REASOCK_smoke2_ping");
  await client.call("action.run", { command: action.command });
  const params = await triggered;
  assert.equal(params.id, "ping");
  assert.ok(Array.isArray(params.context.selectedItems));
});

await check("registering the same id again returns the same action", async () => {
  const first = await client.call("action.register", { id: "ping", label: "Smoke test" });
  const second = await client.call("action.register", { id: "ping", label: "Smoke test" });
  assert.deepEqual(first, second);
});

await check("another connection can't take an owned action (-32010)", async () => {
  const other = await connect({ name: "smoke2", port });
  await assert.rejects(other.call("action.register", { id: "ping", label: "Smoke test" }), { code: -32010 });
  other.close();
});

await check("action.unregister removes the action", async () => {
  await client.call("action.unregister", { id: "ping" });
  await assert.rejects(client.call("action.run", { command: "_REASOCK_smoke2_ping" }), { code: -32012 });
});

await check("events.subscribe rejects unknown events (-32602)", async () => {
  await assert.rejects(client.call("events.subscribe", { events: ["nope.happened"] }), { code: -32602 });
});

await check("subscribing sends the current state once", async () => {
  const events = client.session.capabilities.events.available;
  const received = new Map();
  for (const event of events) client.on(event, (params) => received.set(event, params));
  const result = await client.call("events.subscribe", { events });
  assert.deepEqual([...result.events].sort(), [...events].sort());
  await new Promise((resolve) => setTimeout(resolve, 300));
  assert.deepEqual([...received.keys()].sort(), [...events].sort());
  assert.equal(typeof received.get("transport.changed").playing, "boolean");
  const items = received.get("items.selectionChanged");
  assert.equal(typeof items.count, "number");
  assert.equal(items.added.length, items.count);
  assert.equal(items.truncated, false);
});

await check("transport.changed follows play and stop", async () => {
  const states = [];
  client.on("transport.changed", ({ playing }) => states.push(playing));
  await client.call("action.run", { command: 1007 }); // Transport: Play
  await new Promise((resolve) => setTimeout(resolve, 300));
  await client.call("action.run", { command: 1016 }); // Transport: Stop
  await new Promise((resolve) => setTimeout(resolve, 300));
  assert.deepEqual(states, [true, false]);
});

await check("events.unsubscribe stops notifications", async () => {
  const result = await client.call("events.unsubscribe", { events: ["transport.changed"] });
  assert.ok(!result.events.includes("transport.changed"));
  let fired = false;
  client.on("transport.changed", () => (fired = true));
  await client.call("action.run", { command: 1007 });
  await new Promise((resolve) => setTimeout(resolve, 300));
  await client.call("action.run", { command: 1016 });
  await new Promise((resolve) => setTimeout(resolve, 300));
  assert.equal(fired, false);
});

await check("params outside their documented constraints are -32602", async () => {
  await assert.rejects(client.call("action.run", { command: 0 }), { code: -32602 });
  await assert.rejects(client.call("action.run", { command: "" }), { code: -32602 });
  await assert.rejects(client.call("action.unregister", { id: "has space" }), { code: -32602 });
  await assert.rejects(client.call("action.register", { id: "ok", label: "" }), { code: -32602 });
});

await check("events must not be empty (-32602)", async () => {
  await assert.rejects(client.call("events.subscribe", { events: [] }), { code: -32602 });
  await assert.rejects(client.call("events.unsubscribe", { events: [] }), { code: -32602 });
});

await check("options are only for collection events (-32602)", async () => {
  await assert.rejects(
    client.call("events.subscribe", { events: [{ event: "transport.changed", options: { limit: 5 } }] }),
    { code: -32602 },
  );
  await assert.rejects(
    client.call("events.subscribe", { events: [{ event: "items.selectionChanged", options: { include: "some" } }] }),
    { code: -32602 },
  );
});

await check("items.listSelected and tracks.listSelected page the selection", async () => {
  const items = await client.call("items.listSelected", { limit: 5 });
  assert.equal(items.offset, 0);
  assert.ok(items.items.length <= Math.min(5, items.count));
  const tracks = await client.call("tracks.listSelected", {});
  assert.equal(tracks.tracks.length, Math.min(256, tracks.count));
  await assert.rejects(client.call("items.listSelected", { limit: 0 }), { code: -32602 });
});

await check("items.get and tracks.get return null for unknown GUIDs", async () => {
  const unknown = "{00000000-0000-0000-0000-000000000000}";
  assert.deepEqual(await client.call("items.get", { guids: [unknown] }), { items: [null] });
  assert.deepEqual(await client.call("tracks.get", { guids: [unknown] }), { tracks: [null] });
  await assert.rejects(client.call("items.get", { guids: [] }), { code: -32602 });
});

await check("items.get finds selected items by GUID", async () => {
  const { items } = await client.call("items.listSelected", { limit: 3 });
  const guids = items.map((item) => item.guid.toLowerCase());
  if (guids.length === 0) return; // nothing selected in this project
  const found = await client.call("items.get", { guids });
  assert.deepEqual(found.items, items);
});

await check("mirrorSelection matches items.listSelected", async () => {
  const mirrorClient = await connect({ name: "mirror", port });
  const mirror = await mirrorSelection(mirrorClient, "items");
  await new Promise((resolve) => setTimeout(resolve, 300));
  const listed = await mirrorClient.call("items.listSelected", { limit: 10000 });
  assert.equal(mirror.count, listed.count);
  assert.deepEqual([...mirror.entries.keys()].sort(), listed.items.map((item) => item.guid).sort());
  mirrorClient.close();
});

await check("project.get and transport.get", async () => {
  const project = await client.call("project.get");
  assert.equal(typeof project.dirty, "boolean");
  const transport = await client.call("transport.get");
  assert.equal(typeof transport.playing, "boolean");
});

await check("tracks.list pages agree with one big page", async () => {
  const all = await client.call("tracks.list", { limit: 10000 });
  const paged = [];
  for (let offset = 0; offset < all.count; offset++) {
    paged.push(...(await client.call("tracks.list", { offset, limit: 1 })).tracks);
  }
  assert.deepEqual(paged, all.tracks);
  assert.deepEqual(all.tracks.map((track) => track.number), all.tracks.map((_, index) => index + 1));
});

await check("items.list per track adds up to all items", async () => {
  const all = await client.call("items.list", { limit: 10000 });
  const { tracks } = await client.call("tracks.list", { limit: 10000 });
  let sum = 0;
  for (const track of tracks) {
    const page = await client.call("items.list", { track: track.guid, limit: 10000 });
    assert.ok(page.items.every((item) => item.trackGuid === track.guid));
    sum += page.count;
  }
  assert.equal(sum, all.count);
  await assert.rejects(client.call("items.list", { track: "{00000000-0000-0000-0000-000000000000}" }), {
    code: -32014,
  });
});

await check("takes.list matches takeCount, with one active take", async () => {
  const { items } = await client.call("items.list", { limit: 50 });
  for (const item of items) {
    const { takes } = await client.call("takes.list", { item: item.guid });
    assert.equal(takes.length, item.takeCount);
    if (takes.length > 0) {
      const active = takes.filter((take) => take.active);
      assert.equal(active.length, 1);
      assert.equal(active[0].name, item.name);
      assert.equal(active[0].path, item.path);
    }
  }
  await assert.rejects(client.call("takes.list", { item: "{00000000-0000-0000-0000-000000000000}" }), {
    code: -32014,
  });
});

await check("markers.list: regions end after they start, markers have no end", async () => {
  const { count, markers } = await client.call("markers.list", { limit: 10000 });
  assert.equal(markers.length, count);
  for (const marker of markers) {
    if (marker.region) assert.ok(marker.end > marker.position);
    else assert.equal(marker.end, null);
  }
  const positions = markers.map((marker) => marker.position);
  assert.deepEqual(positions, [...positions].sort((a, b) => a - b));
});

// The ReaScript API, checked from inside REAPER. macOS can hand a script to
// the running REAPER with `open`; elsewhere, run scripts/lua-smoke.lua by hand.
if (process.platform === "darwin") {
  const script = join(dirname(fileURLToPath(import.meta.url)), "lua-smoke.lua");
  const result = join(homedir(), ".reasock", "lua-smoke.txt");
  rmSync(result, { force: true });
  execFileSync("open", ["-a", "REAPER", script]);
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline && !(existsSync(result) && readFileSync(result, "utf8").includes("done"))) {
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  const lines = existsSync(result) ? readFileSync(result, "utf8").trim().split("\n") : [];
  for (const line of lines.filter((line) => line !== "done")) {
    console.log(line.replace(/^(ok  |FAIL) /, "$1 lua: "));
    if (line.startsWith("FAIL")) failures++;
  }
  if (!lines.includes("done")) {
    failures++;
    console.log("FAIL lua: scripts/lua-smoke.lua did not finish");
  }
}

raw.close();
client.close();
console.log(failures ? `\n${failures} failed` : "\nall passed");
process.exit(failures ? 1 : 0);
