// Browser entry point. A web page can't read ~/.reasock, so the port and
// token come from whoever serves the page, and the page's origin has to be
// listed in allowedOrigins in ~/.reasock/config.json.

import { ReaSockClient, type ClientOptions } from "./core.js";

export * from "./generated.js";
export { ReaSockClient, RpcError, type ClientOptions, type Endpoint, type Params, type ReconnectOptions, type Result } from "./core.js";
export { mirrorSelection, type MirrorOptions, type SelectionMirror } from "./mirror.js";

/** Connects to REAPER at the given endpoint (port and token). */
export function connect(options: ClientOptions) {
  return ReaSockClient.connect(options);
}
