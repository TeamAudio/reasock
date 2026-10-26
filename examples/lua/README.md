# Lua (ReaScript) examples

Scripts inside REAPER use ReaSock through a local connection, with the same
methods and events as outside clients and no token:
`reaper.ReaSock_Open`, `ReaSock_Call`, `ReaSock_Send`, `ReaSock_Poll`, and
`ReaSock_Close`.

| Script | Shows |
| --- | --- |
| `inspector-panel.lua` | A dockable [ReaImGui](https://github.com/cfillion/reaimgui) window with the project, transport, last edit, and the selected items and tracks, updated live. Needs ReaImGui 0.10+. |
| `follow-selection.lua` | Prints the selection to the ReaScript console as it changes. |

Both come with the ReaSock package from ReaPack (Actions list: search
"ReaSock"). Run them from the repo with **Actions → ReaScript: Load...**.

REAPER's Lua has no JSON library, so the scripts load `json.lua` (rxi's,
MIT) from the same folder.
