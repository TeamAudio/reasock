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
node scripts/smoke-test.mjs  # protocol checks against the running REAPER,
                             # plus the ReaScript API on macOS
cargo test                   # envelope tests, no REAPER needed
```

`dev-install.sh` also builds the API reference (`scripts/build_docs.py`) and
links it into REAPER's `Data/reasock/`, where the **ReaSock: Open API
documentation** action and `ReaSock_OpenDocs()` open it.

On load, ReaSock listens on `127.0.0.1`, using the first free port from 9670
to 9679 (`REASOCK_PORT` overrides this). It writes
`~/.reasock/instances/<pid>.json` (port and connection token) and logs to
`~/.reasock/reasock.log`.
`examples/reasock-client.mjs` is a small dependency-free Node client.

## Protocol

[JSON-RPC 2.0](https://www.jsonrpc.org/specification) over WebSocket text
frames, with params by name. Batches and notifications are supported. The
methods are described in [`openrpc.json`](openrpc.json)
([OpenRPC](https://open-rpc.org/) 1.4). The document is generated from the
code and served by `rpc.discover`, so it always matches the running server.

| Method | Params | Result |
| --- | --- | --- |
| `initialize` | `{protocolVersion, client: {name, version?}, capabilities?}` | `{protocolVersion, server, reaper, connection, capabilities}` |
| `rpc.discover` | | the OpenRPC document |
| `action.register` | `{id, label}` | `{command, commandId}` |
| `action.unregister` | `{id}` | `{}` |
| `action.run` | `{command}`: a named command string or numeric ID | `{commandId}` |
| `events.subscribe` | `{events: [name or {event, options}…]}` | `{events}`: all current subscriptions |
| `events.unsubscribe` | `{events: [name…]}` | `{events}` |
| `project.get` / `transport.get` | | the current project / transport state |
| `items.list` | `{offset?, limit?, track?}` | `{count, offset, items}`: all items, or one track's |
| `items.listSelected` | `{offset?, limit?}` | `{count, offset, items}` |
| `items.get` | `{guids}` | `{items}`: each an item or `null` |
| `takes.list` | `{item}` | `{item, takes}` |
| `tracks.list` / `tracks.listSelected` | `{offset?, limit?}` | `{count, offset, tracks}` |
| `tracks.get` | `{guids}` | `{tracks}` |
| `markers.list` | `{offset?, limit?}` | `{count, offset, markers}`: markers and regions |
| `debug.stats` | `{reset?}` | main-thread time spent on events |

`initialize` must come first; only `rpc.discover` is allowed before it. When a
registered action runs, the client that owns it receives a notification. These
server-to-client notifications are listed under `x-notifications` in the
OpenRPC document:

```json
{"jsonrpc": "2.0", "method": "action.triggered", "params": {"id": "hello", "command": "_REASOCK_demo_hello",
  "context": {"selectedItems": [{"guid": "{…}", "path": "/…/tone.wav", "track": "tone", "position": 0, "length": 2}],
    "selectedItemCount": 1, "truncated": false}}}
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

### Selections stay bounded

The two selection events describe *what changed*, so their size doesn't grow
with the session:

```json
{"count": 3, "added": ["{…}"], "removed": ["{…}"], "changed": ["{…}"], "items": [{…}], "truncated": false}
```

Each subscription can set `include` (`"none"`, `"changed"` (the default), or
`"all"`) to choose which full entries come along, and `limit` (default 256,
max 10,000) to cap how many GUIDs or entries a notification can carry:

```json
{"events": ["transport.changed", {"event": "items.selectionChanged", "options": {"include": "all", "limit": 50}}]}
```

When something the subscription asks for would exceed `limit`, it's left out
and `truncated` is `true`. The client then catches up with
`items.listSelected` (paged) or `items.get`. `mirrorSelection()` in
`examples/reasock-client.mjs` does all of this and keeps a local copy of the
selection up to date.

Actions are named `_REASOCK_<client>_<id>` and are removed when their
connection closes. Errors use the standard JSON-RPC codes, plus the
application codes listed in `openrpc.json` (-32001 not initialized, -32010
action conflict, -32012 unknown command, and so on).

## ReaScript

Scripts inside REAPER use the same protocol through a local connection, so
every method and event works the same way it does for WebSocket clients:

```lua
local conn = reaper.ReaSock_Open("myscript")            -- 0 if it failed
local reply = reaper.ReaSock_Call(conn, "tracks.list", '{"limit": 10}')
reaper.ReaSock_Send(conn, message)                      -- any JSON-RPC text
local message = reaper.ReaSock_Poll(conn)               -- "" when nothing is waiting
reaper.ReaSock_Close(conn)
```

- `ReaSock_Open` opens and initializes a connection as the given client.
- `ReaSock_Call` runs one method and returns the JSON-RPC response.
- `ReaSock_Send` and `ReaSock_Poll` carry everything else: replies,
  subscribed events, and `action.triggered` for actions the script registered.
  Poll from a `reaper.defer` loop.
- A connection that goes 10 seconds without a Call or Poll is closed, along
  with its actions and subscriptions, so scripts that stop without closing
  don't leave anything behind.

`examples/lua/follow-selection.lua` prints the selection as it changes.
It uses `examples/lua/json.lua` (rxi, MIT), since REAPER's Lua has no JSON
library.

## Releasing

Releases follow XY Pad's flow, with two human gates (a PR, then publishing a
draft):

1. Describe changes as they land in `releases/pending/<name>.{patch,minor,major}.md`.
2. Run **Prepare release** (Actions tab). It bumps the version in `Cargo.toml`
   from the largest pending level (or takes an explicit version), compiles
   `releases/changes-X.Y.Z.md`, and opens a `release-prep` PR.
3. Merging that PR tags `vX.Y.Z` and starts **Release** on the tag: it builds
   all five platforms, signs and notarizes the macOS binaries (Developer ID),
   signs the Windows DLL (Azure Artifact Signing, through the `release`
   environment's federated credential), and attaches everything to a
   **draft** GitHub release.
4. Publishing the draft opens a PR in `TeamAudio/reascripts` with
   `Extensions/ReaSock.ext` and the updated index, and a PR in
   `TeamAudio/techaudio-web` with the release notes.

`scripts/release.py`, `scripts/reapack.py`, and `scripts/build_docs.py` run
the same steps locally.

Windows signing authenticates to Azure through a federated credential on the
`techaudio-ci-signing` app registration, so no client secret is stored. GitHub
sends this repo's OIDC subject in its ID-based form,
`repo:TeamAudio@95143723/reasock@1433670296:environment:release`, and Azure
needs that exact string. A credential written as
`repo:TeamAudio/reasock:environment:release` fails with `AADSTS700213`.

## Adding to the protocol

The protocol's types and methods live in Rust, and everything else is derived
from them:

1. Add the params and result structs to `src/types.rs`. Their doc comments
   become descriptions, and `#[schemars(...)]` attributes document constraints
   (patterns, ranges, lengths). Enforce those constraints in the handler too:
   they are documented, not checked automatically.
2. Add a `Method` impl in `src/api.rs` with its name, summary, errors, and
   `call`, and list it in `methods()`. Notifications work the same way with
   `Notification` and `notifications()`.
3. Run `REASOCK_UPDATE_OPENRPC=1 cargo test` to regenerate `openrpc.json`, then
   review the diff. `cargo test` fails while the published document is out of
   date.

Dispatch, the `initialize` gate, and `-32602` errors for malformed params come
from the registry. Internally, ReaSock works with these structs and serializes
them only when it sends something.

## Security

ReaSock listens on `127.0.0.1` only, so other machines can't reach it.

Clients prove they run as the same user as REAPER: `initialize` needs the
`token` from the instance file, `~/.reasock/instances/<pid>.json`, a random
256-bit value that changes every time REAPER starts. On macOS and Linux the
file and its folders are readable only by you; on Windows, your profile
folder already is. Other users on the machine and web pages can't read it.
Scripts inside REAPER (`ReaSock_Open`) need no token.

Web pages need a second check: a page can open a WebSocket to `127.0.0.1`
from any site, and browsers don't apply cross-origin rules to WebSockets. So
ReaSock refuses any handshake that carries an `Origin` header (which browsers
always send, and pages can't forge) unless that origin is allowed in
`~/.reasock/config.json`:

```json
{ "allowedOrigins": ["http://localhost:5173"] }
```

Native clients (Node, Python, an Electron app's main process) send no
`Origin` and aren't affected. The file is re-read for every connection.
Refused attempts are logged to `~/.reasock/reasock.log`. Don't allow `null`:
sandboxed frames from any site send it.

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
- **Clients see REAPER close.** When REAPER quits, every WebSocket client
  gets close code 1001 ("REAPER is closing") rather than a reset, so clients
  can tell a shutdown from a crash.
- **Modal dialogs pause the bridge.** The main-thread timer doesn't run while,
  for example, the update notification is open. Requests queue until it
  closes, so clients need timeouts and a "REAPER busy" state.
- **Live edits and committed edits arrive separately.** While an item is being
  dragged, `items.selectionChanged` streams its position (about 7 updates per
  drag, thanks to the once-per-tick merging), and `project.edited` fires once
  ("Move media items") when the mouse is released. Switching project tabs
  updates every event at once, and the state-change count is per project.
  Save As with copied media updates item paths to the copies.
- **Polling cost grows with the selection, so check cheaply first.** Building
  a full item snapshot costs several microseconds per selected item (GUID
  formatting, names, paths, and JSON). Before the fix, watching 16,384 selected
  items took ~39 ms per tick, more than the whole 33 ms tick, and REAPER's
  timers dropped to 17 per second. Now each tick compares a cheap fingerprint
  (project, state change count, and each item's pointer, position, length, and
  track) and builds the snapshot only when that differs:

  | Selected items | Idle cost per tick, before | After |
  | --- | --- | --- |
  | 256 | 2.3 ms | 0.2 ms |
  | 1,024 | 7.9 ms | 0.7 ms |
  | 4,096 | 9.6 ms | 2.7 ms |
  | 16,384 | 39 ms | 8.9 ms |

  Notifications used to carry the whole selection: 16,384 items made a 3 MB
  notification on every change. They now carry only the change and are capped
  by the subscriber's `limit`, so large changes arrive as a small `truncated`
  notice. Building the snapshot after a large change is still proportional to
  the selection (~27 ms for a change adding 8,192 items to a 16,384-item
  selection). That's internal and can be optimized later without touching the
  protocol, for example by hashing items instead of building their JSON.
  (Release builds, `scripts/bench-events.mjs`.)
- `action.run` with 40004 (File: Quit REAPER) quits cleanly when the project
  has no unsaved changes, which makes automated test runs easy.
- Registering an action takes about one timer tick (~30 ms). A full
  register → run → triggered round trip takes ~130 ms.

## Next

- Enforce documented param constraints automatically once they multiply:
  `garde` attributes are read by schemars, so one attribute could document
  and enforce a constraint. Full JSON Schema validation (`jsonschema`) was
  measured at +58 crates and roughly 4x the extension's size, so it's ruled
  out.
- Tell: more events (edit cursor, time selection, markers, tempo). Fast,
  per-track changes such as volume and FX parameters probably belong on a
  control surface (`csurf_inst`) rather than polling.
- Ask: more of REAPER's model (FX, envelopes, routing) following the paged,
  GUID-keyed pattern of `items.list` and `items.get`.
- Launch-on-trigger: remember which client owns which actions and start it if
  it's offline.

## License

ReaSock is free software under the GNU Lesser General Public License, version
3 or later (`COPYING.LESSER`, which builds on the GPL in `COPYING`). Changes to
ReaSock itself stay open; programs and scripts that talk to it can use any
license.

The protocol description (`openrpc.json`) and everything in `examples/` are
MIT-licensed, so you can copy them into your own clients. `examples/lua/json.lua`
is rxi's json.lua (MIT). The API reference embeds Karla (SIL OFL 1.1) and
Roboto Slab (Apache 2.0); their licenses are in `docs/assets/`.
