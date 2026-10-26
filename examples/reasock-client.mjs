// Tiny dependency-free ReaSock client for Node 22+ (built-in WebSocket).
//
//   const reasock = await connect({ name: "demo" });
//   reasock.on("action.triggered", (params) => ...);
//   await reasock.call("action.register", { id: "hello", label: "Hello" });

import { readdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export const PROTOCOL_VERSION = "0.4.0";

/**
 * Returns the most recently announced REAPER instance that is still running:
 * `{pid, port, token, reasockVersion, reaperVersion}`.
 */
export function findInstance() {
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

export class RpcError extends Error {
  constructor({ code, message, data }) {
    super(message);
    this.code = code;
    this.data = data;
  }
}

/**
 * Connects, calls `initialize`, and resolves to a client. The port and token
 * come from the running REAPER's instance file unless given.
 */
export async function connect({ name, version, port, token, timeoutMs = 5000 }) {
  if (port === undefined || token === undefined) {
    const instance = findInstance();
    port ??= instance.port;
    token ??= instance.token;
  }
  const ws = new WebSocket(`ws://127.0.0.1:${port}`);
  const pending = new Map();
  const handlers = new Map();
  let nextId = 1;

  ws.addEventListener("message", (event) => {
    const messages = [].concat(JSON.parse(event.data));
    for (const message of messages) {
      if (message.id != null && pending.has(message.id)) {
        const { resolve, reject, timer } = pending.get(message.id);
        clearTimeout(timer);
        pending.delete(message.id);
        message.error ? reject(new RpcError(message.error)) : resolve(message.result);
      } else if (message.method) {
        handlers.get(message.method)?.(message.params);
      }
    }
  });
  ws.addEventListener("close", () => {
    for (const { reject, timer } of pending.values()) {
      clearTimeout(timer);
      reject(new Error("connection closed"));
    }
    pending.clear();
  });

  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve, { once: true });
    ws.addEventListener("error", () => reject(new Error(`cannot connect to port ${port}`)), { once: true });
  });

  const client = {
    ws,
    /** Calls a method. Rejects with RpcError, or after timeoutMs (REAPER may be showing a modal dialog). */
    call(method, params = {}) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          pending.delete(id);
          reject(new Error(`${method} timed out; is a REAPER dialog open?`));
        }, timeoutMs);
        pending.set(id, { resolve, reject, timer });
        ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
      });
    },
    on(method, handler) {
      handlers.set(method, handler);
    },
    close() {
      ws.close();
    },
  };

  client.session = await client.call("initialize", {
    protocolVersion: PROTOCOL_VERSION,
    client: version ? { name, version } : { name },
    token,
  });
  return client;
}

/**
 * Keeps a local copy of the selected items (kind "items") or tracks (kind
 * "tracks"), keyed by GUID. Applies each selectionChanged notification, and
 * when one is truncated, catches up by paging through `<kind>.listSelected`.
 * `onChange(mirror)` runs after every update.
 */
export async function mirrorSelection(client, kind = "items", { limit = 256, onChange = () => {} } = {}) {
  const mirror = { count: 0, entries: new Map(), resyncs: 0 };
  let queue = Promise.resolve();

  async function resync() {
    const entries = new Map();
    let count = 0;
    for (let offset = 0; offset === 0 || offset < count; offset += 1000) {
      const page = await client.call(`${kind}.listSelected`, { offset, limit: 1000 });
      count = page.count;
      for (const entry of page[kind]) entries.set(entry.guid, entry);
      if (page[kind].length === 0) break;
    }
    mirror.entries = entries;
    mirror.count = count;
    mirror.resyncs++;
  }

  function apply(params) {
    if (params.truncated) return resync();
    for (const guid of params.removed) mirror.entries.delete(guid);
    for (const entry of params[kind] ?? []) mirror.entries.set(entry.guid, entry);
    mirror.count = params.count;
  }

  client.on(`${kind}.selectionChanged`, (params) => {
    // Apply in order, even while a resync is in flight.
    queue = queue.then(() => apply(params)).then(() => onChange(mirror));
  });
  await client.call("events.subscribe", {
    events: [{ event: `${kind}.selectionChanged`, options: { include: "changed", limit } }],
  });
  return mirror;
}
