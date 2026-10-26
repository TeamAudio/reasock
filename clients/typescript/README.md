# @techaudio/reasock

A typed client for [ReaSock](https://github.com/TeamAudio/reasock), the
WebSocket bridge between REAPER and other programs. Every method, param,
result, and event is typed, generated from ReaSock's OpenRPC description.
No runtime dependencies; Node 22+ or any browser.

```sh
npm install @techaudio/reasock
```

REAPER needs the ReaSock extension, from the Tech Audio ReaPack repository
(`https://github.com/TeamAudio/reascripts/raw/main/index.xml`).

```ts
import { connect, mirrorSelection } from "@techaudio/reasock";

// Finds the running REAPER and its token through ~/.reasock/instances.
const reasock = await connect({ name: "myapp", reconnect: true });

const { tracks } = await reasock.call("tracks.list");
const { items } = await reasock.call("items.list", { track: tracks[0]!.guid, limit: 50 });

// Tell: follow changes.
reasock.on("transport.changed", ({ playing }) => console.log(playing ? "playing" : "stopped"));
await reasock.subscribe("transport.changed");

// Lend: add an action to REAPER's action list (bind it to a key there).
await reasock.registerAction("export", "Export selection", ({ context }) => {
  console.log(context.selectedItems.map((item) => item.path));
});

// A local copy of the selection that stays current.
const selection = await mirrorSelection(reasock, "items", {
  onChange: ({ count }) => console.log(`${count} selected`),
});
```

- `call()` rejects with one of three errors:
  - `RpcError`: ReaSock refused the call. Its `code` matches `ErrorCode`
    (`ErrorCode.NotFound`, …), and some errors carry `data`, such as which
    file `items.insert` couldn't use.
  - `TimeoutError`: no answer in time, 5 s by default. REAPER can't answer
    while a modal dialog is open, so this usually means "REAPER is busy".
  - `NotConnectedError`: REAPER isn't running or the connection dropped.
    Calls fail with it at once instead of waiting for the timeout.
- With `reconnect: true` the client reconnects after REAPER restarts, finding
  the new token, and restores what `registerAction()` and `subscribe()` set
  up. `onConnectionChange()` reports drops and recoveries. The client stays
  with the REAPER it connected to; when that one goes away it reconnects to
  the most recently started one.
- `registerAction(id, label, handler, { toggle: true })` makes an action with
  an on/off state (a checkmark in REAPER's menus); `setActionState(id, on)`
  sets it, and it's restored after a reconnect too.
- `supports("items.insert")` says whether the connected ReaSock has a method,
  so an app can hide features an older ReaSock lacks.
- `on()` returns a function that removes the handler; `subscribe()` returns
  one that unsubscribes.

## Apps that start before REAPER

`connect()` fails if REAPER isn't running. A long-running app can use
`createClient()` instead: it returns a client at once and connects in the
background, picking up REAPER as soon as it starts (it watches
`~/.reasock/instances`), then reconnects whenever needed.
`registerAction()` and `subscribe()` wait for the connection, so they can be
set up right away; other calls fail with `NotConnectedError` until then.
`close()`, or aborting the `signal` option, stops it.

```ts
import { createClient } from "@techaudio/reasock";

const reasock = createClient({ name: "myapp" });
reasock.onConnectionChange((connected) => showStatus(connected ? "REAPER connected" : "waiting for REAPER"));
void reasock.registerAction("send", "Send selected items to My App", ({ context }) => send(context.selectedItems));
```

Only one connection at a time can use a client name, and REAPER stores
hotkeys by action name (`_REASOCK_<name>_<id>`). So a development build
running beside an installed one needs a different name, and hotkeys bound to
one build's actions won't trigger the other's.

## In the browser

Web pages can't read `~/.reasock`, so whatever serves the page passes the
port and token along, and the page's origin must be listed in
`allowedOrigins` in `~/.reasock/config.json`. Bundlers pick the browser
build automatically; it's also at `@techaudio/reasock/browser`.

```ts
import { connect } from "@techaudio/reasock/browser";
const reasock = await connect({ name: "dashboard", endpoint: { port, token } });
```

## Versions

The package is released with ReaSock and shares its version number. Its
types describe the protocol of that release (`PROTOCOL_VERSION`).

The full API reference ships with ReaSock: run **ReaSock: Open API
documentation** in REAPER. MIT licensed.
