# ReaSock examples

Everything here is MIT-licensed (see `LICENSE`), so copy freely into your
own projects.

| Example | Language | Shows |
| --- | --- | --- |
| [`hello.mjs`](hello.mjs) | Node | Registering an action and receiving it |
| [`inspector.mjs`](inspector.mjs) | Node | Following every event as you work |
| [`reasock-client.mjs`](reasock-client.mjs) | Node | A dependency-free client with `mirrorSelection()` |
| [`python/`](python/) | Python | A client library; copying selected items' paths with a hotkey; following the selection |
| [`mcp/`](mcp/) | Node | An MCP server so AI agents (Claude Code, Claude Desktop) can inspect and drive REAPER |
| [`lua/`](lua/) | Lua (ReaScript) | Following the selection from a script inside REAPER |

Clients outside REAPER find it through `~/.reasock/instances/<pid>.json`,
which holds the port and the connection token that `initialize` needs.
Scripts inside REAPER use `reaper.ReaSock_Open` instead and need no token.
