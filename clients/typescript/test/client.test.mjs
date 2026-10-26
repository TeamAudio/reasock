import assert from "node:assert/strict";
import { test } from "node:test";
import { ErrorCode, ReaSockClient, RpcError } from "../dist/browser.js";
import { FakeReaSock } from "./fake.mjs";

const connect = (server, options = {}) =>
  ReaSockClient.connect({ name: "test", endpoint: { port: 1, token: server.token }, WebSocket: server.WebSocket, ...options });

const until = async (condition, ms = 2000) => {
  const deadline = Date.now() + ms;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error("timed out waiting");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
};

test("initializes with the protocol version and token", async () => {
  const server = new FakeReaSock();
  const client = await connect(server, { version: "1.2" });
  assert.equal(server.calls[0].method, "initialize");
  assert.deepEqual(server.calls[0].params, { protocolVersion: "0.3.0", client: { name: "test", version: "1.2" }, token: "secret" });
  assert.equal(client.session.server.name, "reasock");
  client.close();
});

test("a wrong token rejects with Unauthorized", async () => {
  const server = new FakeReaSock();
  await assert.rejects(
    ReaSockClient.connect({ name: "test", endpoint: { port: 1, token: "wrong" }, WebSocket: server.WebSocket }),
    (error) => error instanceof RpcError && error.code === ErrorCode.Unauthorized,
  );
});

test("calls resolve with results and reject with RpcError", async () => {
  const server = new FakeReaSock();
  const client = await connect(server);
  assert.deepEqual(await client.call("tracks.list", { limit: 5 }), { echo: "tracks.list", params: { limit: 5 } });
  assert.deepEqual(server.calls.at(-1).params, { limit: 5 });
  await client.call("project.get");
  assert.deepEqual(server.calls.at(-1).params, {});
  await assert.rejects(client.call("takes.list", { item: "{X}" }), (error) => error.code === ErrorCode.NotFound);
  client.close();
});

test("calls time out when REAPER doesn't answer", async () => {
  const server = new FakeReaSock();
  const client = await connect(server, { timeoutMs: 50 });
  server.hang.add("project.get");
  await assert.rejects(client.call("project.get"), /timed out/);
  client.close();
});

test("notifications reach every handler until removed", async () => {
  const server = new FakeReaSock();
  const client = await connect(server);
  const seen = [];
  const off = client.on("transport.changed", (state) => seen.push(["a", state.playing]));
  client.on("transport.changed", (state) => seen.push(["b", state.playing]));
  server.notify("transport.changed", { playing: true, paused: false, recording: false });
  off();
  server.notify("transport.changed", { playing: false, paused: false, recording: false });
  assert.deepEqual(seen, [["a", true], ["b", true], ["b", false]]);
  client.close();
});

test("registerAction dispatches action.triggered by id", async () => {
  const server = new FakeReaSock();
  const client = await connect(server);
  const fired = [];
  const action = await client.registerAction("hello", "Hello", (params) => fired.push(params.id));
  assert.equal(action.command, "_REASOCK_test_hello");
  server.notify("action.triggered", { id: "other", command: "_x", context: {} });
  server.notify("action.triggered", { id: "hello", command: "_REASOCK_test_hello", context: {} });
  assert.deepEqual(fired, ["hello"]);
  client.close();
});

test("pending calls fail when the connection drops", async () => {
  const server = new FakeReaSock();
  const client = await connect(server);
  server.hang.add("project.get");
  const call = client.call("project.get");
  server.socket.close();
  await assert.rejects(call, /closed/);
  assert.equal(client.connected, false);
});

test("reconnects, restoring actions and subscriptions, with a fresh endpoint", async () => {
  const server = new FakeReaSock();
  let endpointCalls = 0;
  const client = await ReaSockClient.connect({
    name: "test",
    endpoint: () => {
      endpointCalls++;
      return { port: 1, token: server.token };
    },
    WebSocket: server.WebSocket,
    reconnect: { minDelayMs: 10 },
  });
  await client.registerAction("hello", "Hello", () => {});
  const unsubscribe = await client.subscribe("transport.changed", { event: "items.selectionChanged", options: { include: "none" } });
  const changes = [];
  client.onConnectionChange((connected) => changes.push(connected));

  // REAPER restarts: new token, and the old connection drops.
  server.token = "restarted";
  server.actions.clear();
  server.subscriptions.clear();
  server.socket.close();
  await until(() => changes.length === 2);

  assert.deepEqual(changes, [false, true]);
  assert.equal(endpointCalls, 2);
  assert.ok(server.actions.has("hello"));
  assert.deepEqual([...server.subscriptions].sort(), ["items.selectionChanged", "transport.changed"]);
  assert.equal(client.connected, true);

  await unsubscribe();
  assert.deepEqual([...server.subscriptions], []);
  client.close();
});

test("close() stops reconnecting", async () => {
  const server = new FakeReaSock();
  const client = await connect(server, { reconnect: { minDelayMs: 10 } });
  client.close();
  await new Promise((resolve) => setTimeout(resolve, 60));
  assert.equal(server.sockets.length, 1);
});
