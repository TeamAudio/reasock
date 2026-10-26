// Node entry point: finds the running REAPER and its token through
// ~/.reasock/instances. Browsers get ./browser.js instead.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { ReaSockClient, type ClientOptions, type Endpoint } from "./core.js";

export * from "./generated.js";
export { ReaSockClient, RpcError, type ClientOptions, type Endpoint, type Params, type ReconnectOptions, type Result } from "./core.js";
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
