# Browser dashboard

A live dashboard in a browser tab: the project, transport (with a Play/Stop
button), recent edits, and the selected items and tracks. It uses the
browser build of [`@techaudio/reasock`](../../clients/typescript/), and shows
how a web page connects safely.

```sh
(cd ../../clients/typescript && npm install && npm run build)
node server.mjs --allow
open http://localhost:8765
```

## How a page gets in

ReaSock guards against web pages in two ways, and this example meets both:

1. **Origin.** ReaSock refuses pages unless their origin is listed in
   `allowedOrigins` in `~/.reasock/config.json`. `--allow` adds
   `http://localhost:8765` for you; without it, the page explains what's
   missing.
2. **Token.** A page can't read `~/.reasock/instances/` to find the port and
   token, so `server.mjs` reads them and serves them at `/endpoint`. The page
   asks again whenever it reconnects, because a restarted REAPER has a new
   token.

`/endpoint` hands out the token, so the server protects it:

- It listens on `127.0.0.1` only.
- It answers only requests addressed to `localhost:8765` or
  `127.0.0.1:8765`. That stops *DNS rebinding*, where a hostile site points
  its own domain at 127.0.0.1 to read local servers as same-origin.

Set `PORT` to use another port (and allow that origin instead).
