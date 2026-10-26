#!/usr/bin/env node
// An MCP server that lets AI agents inspect and drive REAPER through ReaSock.
//
//   claude mcp add reasock -- node /path/to/reasock/examples/mcp/server.mjs
//
// It speaks MCP over stdio, so nothing but protocol may go to stdout; logs go
// to stderr. REAPER is reached lazily, and again after it restarts.

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { connect } from "../reasock-client.mjs";

const EVENTS = ["project.changed", "project.edited", "transport.changed", "items.selectionChanged", "tracks.selectionChanged"];
const RECENT_LIMIT = 100;

let reasock = null;
const recent = [];

/** A connected client, connecting (or reconnecting) when needed. */
async function reaper() {
  if (reasock && reasock.ws.readyState === WebSocket.OPEN) return reasock;
  try {
    reasock = await connect({ name: "mcp", version: "0.1.0" });
  } catch (error) {
    reasock = null;
    throw new Error(`Can't reach REAPER (${error.message}). Is REAPER running with ReaSock installed?`);
  }
  // Remember what happens in REAPER, for reaper_recent_events.
  for (const event of EVENTS) {
    reasock.on(event, (params) => {
      if (event.endsWith("selectionChanged")) {
        params = { count: params.count, added: params.added?.length, removed: params.removed?.length, changed: params.changed?.length };
      }
      recent.push({ at: new Date().toISOString(), event, ...params });
      if (recent.length > RECENT_LIMIT) recent.shift();
    });
  }
  await reasock.call("events.subscribe", {
    events: EVENTS.map((event) => (event.endsWith("selectionChanged") ? { event, options: { include: "none", limit: 50 } } : event)),
  });
  console.error(`reasock-mcp: connected to REAPER ${reasock.session.reaper.version}`);
  return reasock;
}

/** Runs a tool body, returning its result as JSON text, or the error. */
async function tool(body) {
  try {
    const result = await body(await reaper());
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  } catch (error) {
    return { content: [{ type: "text", text: `Error: ${error.message}` }], isError: true };
  }
}

const readOnly = { readOnlyHint: true, openWorldHint: false };
const paging = {
  offset: z.number().int().min(0).optional().describe("Skip this many (default 0)"),
  limit: z.number().int().min(1).max(1000).optional().describe("At most this many (default 100)"),
};
const page = ({ offset, limit }) => ({ offset: offset ?? 0, limit: limit ?? 100 });

const server = new McpServer({ name: "reasock", version: "0.1.0" });

server.registerTool(
  "reaper_project",
  {
    title: "Current REAPER project",
    description: "The current project (name, path, unsaved changes) and the transport state.",
    annotations: readOnly,
  },
  () => tool(async (r) => ({ project: await r.call("project.get"), transport: await r.call("transport.get") })),
);

server.registerTool(
  "reaper_tracks",
  {
    title: "List tracks",
    description: "Tracks in order, with GUIDs (stable across sessions), names, and 1-based numbers. The master track isn't included.",
    inputSchema: paging,
    annotations: readOnly,
  },
  (args) => tool((r) => r.call("tracks.list", page(args))),
);

server.registerTool(
  "reaper_items",
  {
    title: "List media items",
    description: "Media items in the project, or on one track: GUID, take name, source file, track, position and length in seconds, take count.",
    inputSchema: { ...paging, track: z.string().optional().describe("GUID of a track, to list only its items") },
    annotations: readOnly,
  },
  ({ track, ...args }) => tool((r) => r.call("items.list", { ...page(args), ...(track ? { track } : {}) })),
);

server.registerTool(
  "reaper_selection",
  {
    title: "Selected items and tracks",
    description: "What the user has selected in REAPER right now.",
    inputSchema: paging,
    annotations: readOnly,
  },
  (args) => tool(async (r) => ({ items: await r.call("items.listSelected", page(args)), tracks: await r.call("tracks.listSelected", page(args)) })),
);

server.registerTool(
  "reaper_takes",
  {
    title: "List an item's takes",
    description: "Every take of a media item, with source files, start offsets, play rates, and which take is active.",
    inputSchema: { item: z.string().describe("GUID of the item") },
    annotations: readOnly,
  },
  ({ item }) => tool((r) => r.call("takes.list", { item })),
);

server.registerTool(
  "reaper_markers",
  {
    title: "List markers and regions",
    description: "Markers and regions in position order: number, name, position, and for regions their end.",
    inputSchema: paging,
    annotations: readOnly,
  },
  (args) => tool((r) => r.call("markers.list", page(args))),
);

server.registerTool(
  "reaper_recent_events",
  {
    title: "What happened in REAPER recently",
    description:
      "Changes since the last call: edits (with their undo descriptions), selection changes, transport changes, and project switches. The first call also includes the current state of each. Use it to see what the user has been doing. Returns the events and forgets them.",
    annotations: readOnly,
  },
  () => tool(async () => recent.splice(0)),
);

server.registerTool(
  "reaper_run_action",
  {
    title: "Run a REAPER action",
    description:
      "Runs an action as if chosen from REAPER's action list: a numeric command ID (for example 40182, \"Item: Select all items\") or a named command (for example \"_SWS_ABOUT\"). Actions can edit, delete, or render; confirm with the user before running anything that changes their project.",
    inputSchema: { command: z.union([z.number().int().min(1), z.string().min(1)]).describe("Command ID or named command") },
    annotations: { readOnlyHint: false, destructiveHint: true, openWorldHint: false },
  },
  ({ command }) => tool((r) => r.call("action.run", { command })),
);

server.registerTool(
  "reaper_insert_media",
  {
    title: "Place media files in REAPER",
    description:
      "Places audio, video, or MIDI files (or parts of them) on REAPER's timeline as one undoable step: at the edit cursor by default, on the selected (else last touched) track, one after another. Paths must be absolute. Use it when the user asks to put sounds into their project.",
    inputSchema: {
      files: z
        .array(
          z.object({
            path: z.string().min(1).describe("Absolute path of the file"),
            sourceStart: z.number().min(0).optional().describe("Seconds into the file to start from"),
            length: z.number().positive().optional().describe("Seconds of the file to use"),
          }),
        )
        .min(1)
        .max(100),
      position: z
        .union([z.number().min(0), z.enum(["cursor", "play"])])
        .optional()
        .describe('Seconds from the project start, "cursor" (default), or "play" for the play position'),
      track: z.string().optional().describe('A track GUID, "current" (default), "selected", or "new"'),
      arrange: z.enum(["sequence", "tracks", "takes"]).optional().describe("How several files are laid out"),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
  },
  (params) =>
    tool(async (r) => {
      try {
        return await r.call("items.insert", params);
      } catch (error) {
        // Say which file didn't work, when ReaSock says.
        throw error.data ? new Error(`${error.data.path}: ${error.data.reason}`) : error;
      }
    }),
);

await server.connect(new StdioServerTransport());
console.error("reasock-mcp: ready");
