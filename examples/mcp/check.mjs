#!/usr/bin/env node
// Checks the server the way an agent would use it: spawns it over stdio with
// the MCP SDK's client, lists its tools, and calls the read-only ones against
// the running REAPER.
//
//   npm run check

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { fileURLToPath } from "node:url";

const client = new Client({ name: "reasock-mcp-check", version: "0.1.0" });
await client.connect(
  new StdioClientTransport({ command: process.execPath, args: [fileURLToPath(new URL("server.mjs", import.meta.url))] }),
);

const { tools } = await client.listTools();
console.log(`tools: ${tools.map((tool) => tool.name).join(", ")}`);

const call = async (name, args = {}) => {
  const result = await client.callTool({ name, arguments: args });
  const text = result.content[0].text;
  if (result.isError) throw new Error(`${name}: ${text}`);
  return JSON.parse(text);
};

const { project, transport } = await call("reaper_project");
console.log(`project: ${project.name ?? "(unsaved)"}, ${transport.playing ? "playing" : "stopped"}`);
const { count: tracks } = await call("reaper_tracks");
const { count: items, items: firstItems } = await call("reaper_items", { limit: 5 });
console.log(`${tracks} track(s), ${items} item(s)`);
if (firstItems[0]) {
  const { takes } = await call("reaper_takes", { item: firstItems[0].guid });
  console.log(`first item has ${takes.length} take(s)`);
}
const { count: markers } = await call("reaper_markers");
console.log(`${markers} marker(s)/region(s)`);
const selection = await call("reaper_selection");
console.log(`selected: ${selection.items.count} item(s), ${selection.tracks.count} track(s)`);
const events = await call("reaper_recent_events");
console.log(`recent events: ${events.length}`);

const bad = await client.callTool({ name: "reaper_takes", arguments: { item: "{00000000-0000-0000-0000-000000000000}" } });
console.log(`unknown item reported as an error: ${bad.isError === true}`);

await client.close();
console.log("ok");
