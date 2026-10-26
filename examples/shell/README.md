# Shell

`reasock.sh` wraps the protocol in two shell functions, for poking at REAPER
from a terminal. Needs [websocat](https://github.com/vi/websocat) and
[jq](https://jqlang.org); works in bash and zsh (on Windows, in WSL or Git
Bash).

```sh
source examples/shell/reasock.sh

reasock project.get
reasock tracks.list
reasock items.list '{"limit": 5}'
reasock takes.list '{"item": "{GUID-FROM-ITEMS-LIST}"}'
reasock action.run '{"command": 40044}'   # Transport: Play/stop

reasock_watch                             # every event, until Ctrl+C
reasock_watch items.selectionChanged      # just these
```

`reasock` sends `initialize` and the call as one JSON-RPC batch, so each call
is a single round trip: `websocat -n -1` sends one message, waits for one
reply, and exits. The port and token come from the newest file in
`~/.reasock/instances/`.

Or talk to it by hand:

```sh
instance=$(ls -t ~/.reasock/instances/*.json | head -1)
websocat "ws://127.0.0.1:$(jq -r .port "$instance")"
```

then paste JSON-RPC messages, starting with `initialize` and the token from
`$instance`.
