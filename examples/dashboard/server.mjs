#!/usr/bin/env node
// Serves the dashboard page, the browser build of @techaudio/reasock, and
// /endpoint, which hands the page REAPER's port and token.
//
//   (cd ../../clients/typescript && npm install && npm run build)
//   node server.mjs --allow    # first time: allow this page's origin
//   open http://localhost:8765
//
// A web page can't read ~/.reasock, so this server reads it on the page's
// behalf. Two things keep that safe:
// - It listens on 127.0.0.1 and only answers requests whose Host is
//   localhost, so a hostile site can't use DNS rebinding to read /endpoint.
// - ReaSock refuses web pages unless their origin is in allowedOrigins.

import { createServer } from "node:http";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const PORT = Number(process.env.PORT ?? 8765);
const ORIGIN = `http://localhost:${PORT}`;
const here = fileURLToPath(new URL(".", import.meta.url));
const client = fileURLToPath(new URL("../../clients/typescript/dist/", import.meta.url));

if (!existsSync(join(client, "browser.js"))) {
  console.error("Build the client first: cd clients/typescript && npm install && npm run build");
  process.exit(1);
}
const { findInstance } = await import(join(client, "index.js"));

// --allow: add this page's origin to ReaSock's allowlist.
const configPath = join(homedir(), ".reasock", "config.json");
const config = existsSync(configPath) ? JSON.parse(readFileSync(configPath, "utf8")) : {};
const allowed = new Set(config.allowedOrigins ?? []);
if (process.argv.includes("--allow") && !allowed.has(ORIGIN)) {
  allowed.add(ORIGIN);
  mkdirSync(join(homedir(), ".reasock"), { recursive: true });
  writeFileSync(configPath, JSON.stringify({ ...config, allowedOrigins: [...allowed] }, null, 2) + "\n");
  console.log(`Added ${ORIGIN} to allowedOrigins in ${configPath}`);
}
if (!allowed.has(ORIGIN)) {
  console.log(`Note: ReaSock will refuse this page until ${ORIGIN} is in allowedOrigins in ${configPath}.`);
  console.log("Run again with --allow to add it.");
}

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".map": "application/json", ".css": "text/css" };

function send(response, status, body, type = "text/plain") {
  response.writeHead(status, { "content-type": `${type}; charset=utf-8`, "cache-control": "no-store" });
  response.end(body);
}

createServer((request, response) => {
  // DNS rebinding guard: only answer requests addressed to us by name.
  if (request.headers.host !== `localhost:${PORT}` && request.headers.host !== `127.0.0.1:${PORT}`) {
    return send(response, 403, "forbidden host");
  }
  const path = new URL(request.url, ORIGIN).pathname;
  if (path === "/endpoint") {
    try {
      const { port, token } = findInstance();
      return send(response, 200, JSON.stringify({ port, token }), "application/json");
    } catch (error) {
      return send(response, 503, JSON.stringify({ error: error.message }), "application/json");
    }
  }
  const [root, file] = path.startsWith("/reasock/")
    ? [client, path.slice("/reasock/".length)]
    : [here, path === "/" ? "index.html" : path.slice(1)];
  const full = normalize(join(root, file));
  if (!full.startsWith(root) || !existsSync(full)) return send(response, 404, "not found");
  send(response, 200, readFileSync(full), TYPES[extname(full)] ?? "application/octet-stream");
}).listen(PORT, "127.0.0.1", () => console.log(`Dashboard at ${ORIGIN}`));
