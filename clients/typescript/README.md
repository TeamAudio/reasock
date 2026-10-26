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

- `call()` rejects with `RpcError`, whose `code` matches `ErrorCode`
  (`ErrorCode.NotFound`, …), or with an `Error` on timeout or disconnect.
  Calls time out after 5 s by default; REAPER can't answer while a modal
  dialog is open.
- With `reconnect: true` the client reconnects after REAPER restarts, finding
  the new token, and restores what `registerAction()` and `subscribe()` set
  up. `onConnectionChange()` reports drops and recoveries.
- `on()` returns a function that removes the handler; `subscribe()` returns
  one that unsubscribes.

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
