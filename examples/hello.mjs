#!/usr/bin/env node
// Registers a "Hello" action in REAPER and logs whatever arrives when it runs.
//
//   node examples/hello.mjs              # wait for you to trigger the action
//   node examples/hello.mjs --self-test  # select all items, trigger it, exit

import { readdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const selfTest = process.argv.includes("--self-test");

function findInstance() {
  const dir = join(homedir(), ".reasock", "instances");
  let names = [];
  try {
    names = readdirSync(dir).filter((name) => name.endsWith(".json"));
  } catch {}
  const alive = names
    .map((name) => JSON.parse(readFileSync(join(dir, name), "utf8")))
    .filter(({ pid }) => {
      try {
        process.kill(pid, 0);
        return true;
      } catch {
        return false;
      }
    });
  if (alive.length === 0) throw new Error(`no running REAPER found in ${dir}`);
  return alive[alive.length - 1];
}

const instance = findInstance();
const ws = new WebSocket(`ws://127.0.0.1:${instance.port}`);
const pending = new Map();
let nextId = 1;

function call(method, params = {}) {
  const id = nextId++;
  ws.send(JSON.stringify({ id, method, params }));
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
}

ws.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);
  if (message.id != null && pending.has(message.id)) {
    const { resolve, reject } = pending.get(message.id);
    pending.delete(message.id);
    message.error ? reject(new Error(message.error.message)) : resolve(message.result);
    return;
  }
  if (message.method === "action.triggered") {
    console.log(`→ ${message.params.command} fired`);
    for (const item of message.params.context.selectedItems) {
      console.log(`   ${item.track ?? "(unnamed track)"} @ ${item.position.toFixed(3)}s: ${item.path ?? "(no file)"}`);
    }
    if (message.params.context.selectedItems.length === 0) console.log("   (no items selected)");
    if (selfTest) ws.close();
  }
});

ws.addEventListener("open", async () => {
  const info = await call("hello", { client: "demo" });
  console.log(`connected to REAPER ${info.reaper} (ReaSock ${info.reasock}) on port ${instance.port}`);
  const action = await call("action.register", { id: "hello", label: "Hello" });
  console.log(`registered ${action.command} (command ${action.commandId})`);

  if (selfTest) {
    await call("action.run", { command: 40182 }); // Item: Select all items
    await call("action.run", { command: action.command });
  } else {
    console.log('waiting... find "ReaSock (demo): Hello" in the Actions list (Ctrl+C to quit)');
  }
});

ws.addEventListener("close", () => process.exit(0));
ws.addEventListener("error", (event) => {
  console.error("websocket error", event.message ?? "");
  process.exit(1);
});
