// Node entry point: finds the running REAPER and its token through
// ~/.reasock/instances. Browsers get ./browser.js instead.

import { existsSync, readFileSync, readdirSync, statSync, watch } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { ReaSockClient, type ClientOptions, type Endpoint } from "./core.js";

export * from "./generated.js";
export {
  NotConnectedError,
  ReaSockClient,
  RpcError,
  TimeoutError,
  type ClientOptions,
  type Endpoint,
  type Params,
  type ReconnectOptions,
  type Result,
} from "./core.js";
export { mirrorSelection, type MirrorOptions, type SelectionMirror } from "./mirror.js";

/** A running REAPER, as announced in its instance file. */
export interface Instance extends Endpoint {
  pid: number;
  reasockVersion: string;
  reaperVersion: string;
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

/** Every running REAPER with ReaSock, newest first. */
export function findInstances(): Instance[] {
  const dir = join(homedir(), ".reasock", "instances");
  let names: string[] = [];
  try {
    names = readdirSync(dir).filter((name) => name.endsWith(".json"));
  } catch {
    return [];
  }
  return names
    .map((name) => join(dir, name))
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)
    .map((path) => JSON.parse(readFileSync(path, "utf8")) as Instance)
    .filter((instance) => alive(instance.pid));
}

/** The most recently started REAPER with ReaSock. Throws if none is running. */
export function findInstance(): Instance {
  const [instance] = findInstances();
  if (!instance) throw new Error("no running REAPER with ReaSock found in ~/.reasock/instances");
  return instance;
}

/**
 * Connects to REAPER. Without an `endpoint`, finds the most recently started
 * REAPER (again on each reconnect, since a restarted REAPER has a new token).
 *
 *     const reasock = await connect({ name: "myapp" });
 *     const { tracks } = await reasock.call("tracks.list");
 */
export function connect(options: Omit<ClientOptions, "endpoint"> & { endpoint?: ClientOptions["endpoint"] }) {
  return ReaSockClient.connect({ ...options, endpoint: options.endpoint ?? findInstance });
}

/**
 * Returns a client right away and connects in the background: it waits for
 * REAPER to start (watching ~/.reasock/instances), then reconnects whenever
 * the connection drops. `registerAction()` and `subscribe()` wait for the
 * connection; other calls reject with `NotConnectedError` until then.
 *
 *     const reasock = createClient({ name: "myapp" });
 *     reasock.onConnectionChange((connected) => updateStatus(connected));
 *     void reasock.registerAction("send", "Send selection to My App", handleSend);
 */
export function createClient(options: Omit<ClientOptions, "endpoint"> & { endpoint?: ClientOptions["endpoint"] }) {
  return ReaSockClient.create({
    ...options,
    endpoint: options.endpoint ?? findInstance,
    watch: options.watch ?? watchInstances,
  });
}

/** Calls `retryNow` when an instance file appears; see `ClientOptions.watch`. */
function watchInstances(retryNow: () => void): () => void {
  const base = join(homedir(), ".reasock");
  const instances = join(base, "instances");
  const dir = existsSync(instances) ? instances : existsSync(base) ? base : null;
  if (!dir) return () => {};
  try {
    // REAPER writes the file and then fills it in, so wait a moment before retrying.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const watcher = watch(dir, () => {
      clearTimeout(timer);
      timer = setTimeout(retryNow, 200);
    });
    watcher.unref();
    return () => {
      clearTimeout(timer);
      watcher.close();
    };
  } catch {
    return () => {};
  }
}
