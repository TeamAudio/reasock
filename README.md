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
```

On load, ReaSock listens on `127.0.0.1`, using the first free port from 9670
to 9679 (`REASOCK_PORT` overrides this). It writes
`~/.reasock/instances/<pid>.json` and logs to `~/.reasock/reasock.log`.

## Protocol (spike)

JSON text frames, loosely shaped like JSON-RPC.

| Method | Params | Result |
| --- | --- | --- |
| `hello` | `{client}` | `{reasock, reaper, connection}` |
| `action.register` | `{id, label}` | `{command, commandId}` |
| `action.unregister` | `{id}` | `{}` |
| `action.run` | `{command}`: a named command string or numeric ID | `{commandId}` |

When a registered action runs, the owning client receives:

```json
{"method": "action.triggered", "params": {"id": "hello", "command": "_REASOCK_demo_hello",
  "context": {"selectedItems": [{"path": "/…/tone.wav", "track": "tone", "position": 0, "length": 2}]}}}
```

Actions are named `_REASOCK_<client>_<id>`. A client's actions are removed
when it disconnects.

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
- **Modal dialogs pause the bridge.** The main-thread timer doesn't run while,
  for example, the update notification is open. Requests queue until it
  closes, so clients need timeouts and a "REAPER busy" state.
- Registering an action takes about one timer tick (~30 ms). A full
  register → run → triggered round trip takes ~130 ms.

## Next

- Close sockets cleanly when REAPER exits (clients currently see a reset).
- Tell: register as a control surface (`csurf_inst`) and stream events.
- Ask: a general way to call REAPER API functions.
- A ReaScript side (`ReaSock_Send` / `ReaSock_Poll`, in the style of
  reaspeech-lib) so Lua scripts can join.
- Launch-on-trigger: remember which client owns which actions and start it if
  it's offline.
