# MCP server

Lets AI agents that speak the [Model Context Protocol](https://modelcontextprotocol.io)
(Claude Code, Claude Desktop, and others) inspect and drive REAPER through
ReaSock. Node 22+.

```sh
cd examples/mcp && npm install
```

**Claude Code:**

```sh
claude mcp add reasock -- node /path/to/reasock/examples/mcp/server.mjs
```

**Claude Desktop** (`claude_desktop_config.json`):

```json
{ "mcpServers": { "reasock": { "command": "node", "args": ["/path/to/reasock/examples/mcp/server.mjs"] } } }
```

REAPER needs to be running with ReaSock installed; the server connects when a
tool is first used, and reconnects after REAPER restarts. `npm run check`
spawns it the way an agent would and calls the read-only tools.

| Tool | What it does |
| --- | --- |
| `reaper_project` | The current project and transport state |
| `reaper_tracks` | Tracks with GUIDs, names, and numbers |
| `reaper_items` | Media items, for the project or one track |
| `reaper_selection` | Selected items and tracks |
| `reaper_takes` | An item's takes |
| `reaper_markers` | Markers and regions |
| `reaper_recent_events` | What changed since the last call: edits with their undo descriptions, selection, transport |
| `reaper_run_action` | Runs any REAPER action by command ID or name |

`reaper_run_action` is marked destructive, and its description asks the agent
to confirm with you before changing your project. MCP clients usually ask for
approval before running a tool like that anyway.

Try asking: *"What's in my REAPER project?"*, *"Which items are on the
Dialogue track, and what files do they use?"*, or *"What have I changed in the
last few minutes?"*
