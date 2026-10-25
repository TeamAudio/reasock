#!/usr/bin/env node
// Checks the JSON-RPC protocol against a running REAPER with ReaSock loaded.
// Registers and runs a throwaway action; does not touch the project.
//
//   node scripts/smoke-test.mjs

import assert from "node:assert/strict";
import { connect, findInstance, PROTOCOL_VERSION } from "../examples/reasock-client.mjs";

const { port } = findInstance();
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

await check("notifications get no reply", async () => {
  raw.send({ jsonrpc: "2.0", method: "rpc.discover" });
  raw.send({ jsonrpc: "2.0", id: 7, method: "nope" });
  assert.equal((await raw.next()).id, 7);
});

await check("batch gets an array of replies, skipping notifications", async () => {
  raw.send([
    { jsonrpc: "2.0", id: 8, method: "initialize", params: { protocolVersion: PROTOCOL_VERSION, client: { name: "smoke" } } },
    { jsonrpc: "2.0", method: "rpc.discover" },
    { jsonrpc: "2.0", id: 9, method: "initialize", params: { protocolVersion: PROTOCOL_VERSION, client: { name: "smoke" } } },
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

raw.close();
client.close();
console.log(failures ? `\n${failures} failed` : "\nall passed");
process.exit(failures ? 1 : 0);
