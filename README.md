# ReaSock

A REAPER extension that opens a local WebSocket so outside programs can talk to
REAPER, without each one writing its own extension.

The idea has three directions of traffic:

- **Ask**: a client calls into REAPER (run actions; later, the wider API).
- **Tell**: REAPER pushes events to subscribers (not built yet).
- **Lend**: a client registers actions in REAPER's action list. Users can bind
  them to hotkeys and menus, and triggering one sends the context back to that
  client.

This is a spike. It proves Lend end to end, plus the smallest Ask (`action.run`).

## Try it

```sh
./scripts/dev-install.sh     # build + symlink into REAPER's UserPlugins
# restart REAPER
node examples/hello.mjs      # registers "ReaSock (demo): Hello"
node examples/hello.mjs --self-test
node scripts/smoke-test.mjs  # protocol checks against the running REAPER
cargo test                   # envelope tests, no REAPER needed
```

On load, ReaSock listens on `127.0.0.1`, using the first free port from 9670
to 9679 (`REASOCK_PORT` overrides this). It writes
`~/.reasock/instances/<pid>.json` and logs to `~/.reasock/reasock.log`.
`examples/reasock-client.mjs` is a small dependency-free Node client.

## Protocol

[JSON-RPC 2.0](https://www.jsonrpc.org/specification) over WebSocket text
frames, with params by name. Batches and notifications are supported. The
methods are described in [`openrpc.json`](openrpc.json)
([OpenRPC](https://open-rpc.org/) 1.4), which the server also returns from
`rpc.discover`.

| Method | Params | Result |
| --- | --- | --- |
| `initialize` | `{protocolVersion, client: {name, version?}, capabilities?}` | `{protocolVersion, server, reaper, connection, capabilities}` |
| `rpc.discover` | | the OpenRPC document |
| `action.register` | `{id, label}` | `{command, commandId}` |
| `action.unregister` | `{id}` | `{}` |
| `action.run` | `{command}`: a named command string or numeric ID | `{commandId}` |
| `events.subscribe` | `{events: [name…]}` | `{events}`: all current subscriptions |
| `events.unsubscribe` | `{events: [name…]}` | `{events}` |

`initialize` must come first; only `rpc.discover` is allowed before it. When a
registered action runs, the client that owns it receives a notification. These
server-to-client notifications are listed under `x-notifications` in the
OpenRPC document:

```json
{"jsonrpc": "2.0", "method": "action.triggered", "params": {"id": "hello", "command": "_REASOCK_demo_hello",
  "context": {"selectedItems": [{"path": "/…/tone.wav", "track": "tone", "position": 0, "length": 2}]}}}
```

### Events (Tell)

| Notification | When |
| --- | --- |
| `transport.changed` | play, pause, or record state changes |
| `project.changed` | another tab becomes current, or the project is opened, saved, or becomes dirty |
| `project.edited` | an undoable change happens; includes the undo description |
| `items.selectionChanged` | the selection changes, or a selected item moves, is trimmed, or is renamed |
| `tracks.selectionChanged` | the track selection changes, or a selected track is renamed or moved |

A new subscriber gets the current state of each event right away, then only
changes. ReaSock snapshots subscribed state once per timer tick (~30 Hz) and
compares it with the previous snapshot, so a burst of changes arrives as one
notification per tick. Polling also covers state REAPER has no callback for,
such as item selection and project state. Items and tracks carry GUIDs, which
stay stable across sessions. `examples/inspector.mjs` prints everything as it
happens.

Actions are named `_REASOCK_<client>_<id>` and are removed when their
connection closes. Errors use the standard JSON-RPC codes, plus the
application codes listed in `openrpc.json` (-32001 not initialized, -32010
action conflict, -32012 unknown command, and so on).

## Spike findings (REAPER 7.78, macOS arm64)

- **Actions can be registered at runtime.** They appear in the action list
  right away and work from the list, from hotkeys, and from `Main_OnCommand`.
- **Use `custom_action`, not `command_id` + `gaccel`.** After an action is
  unregistered and registered again, the gaccel path gives it a new command ID
  with no name (`ReverseNamedCommandLookup` returns `""`). REAPER then saves
  the hotkey to `reaper-kb.ini` as a blank `KEY … _ 0`, which is lost on
  restart. `custom_action` keeps both the ID and the name stable.
- **Custom actions report through `hookcommand2`** (hotkeys, the action list),
  while `Main_OnCommand` goes through `hookcommand`. Register both. Each
  trigger arrives exactly once.
- **Hotkeys survive the owner going away.** While the client is offline, the
  bound key does nothing (or "find shortcut" reports an "unavailable registered
  action"). When the client registers again, even after a REAPER restart, the
  same key works again. The binding is stored by name, so command IDs don't
  matter, and clients should always use the `_REASOCK_…` name.
- **REAPER keeps resolving the names of unregistered actions.**
  `NamedCommandLookup` still returns an ID, and `Main_OnCommand` on it silently
  does nothing. `action.run` therefore checks `kbd_getTextFromCmd` and rejects
  commands that aren't in the action list.
- **Modal dialogs pause the bridge.** The main-thread timer doesn't run while,
  for example, the update notification is open. Requests queue until it
  closes, so clients need timeouts and a "REAPER busy" state.
- **Live edits and committed edits arrive separately.** While an item is being
  dragged, `items.selectionChanged` streams its position (about 7 updates per
  drag, thanks to the once-per-tick merging), and `project.edited` fires once
  ("Move media items") when the mouse is released. Switching project tabs
  updates every event at once, and the state-change count is per project.
  Save As with copied media updates item paths to the copies.
- `action.run` with 40004 (File: Quit REAPER) quits cleanly when the project
  has no unsaved changes, which makes automated test runs easy.
- Registering an action takes about one timer tick (~30 ms). A full
  register → run → triggered round trip takes ~130 ms.

## Next

- Close sockets cleanly when REAPER exits (clients currently see a reset).
- Tell: more events (edit cursor, time selection, markers, tempo). Fast,
  per-track changes such as volume and FX parameters probably belong on a
  control surface (`csurf_inst`) rather than polling.
- Ask: a general way to call REAPER API functions.
- A ReaScript side (`ReaSock_Send` / `ReaSock_Poll`, in the style of
  reaspeech-lib) so Lua scripts can join.
- Launch-on-trigger: remember which client owns which actions and start it if
  it's offline.
