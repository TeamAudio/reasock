# Python examples

A small client (`reasock.py`) and two scripts. Python 3.11+.

```sh
python3 -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt
python3 copy_paths.py        # or watch_selection.py
```

REAPER needs to be running with ReaSock installed. The client finds it, and
its connection token, through `~/.reasock/instances/`.

| Script | Shows |
| --- | --- |
| `copy_paths.py` | **Lend**: adds "Copy file paths of selected items" to REAPER's action list. Bind it to a key; the selected items' source files go to the clipboard. `--print` prints them instead. |
| `watch_selection.py` | **Tell**: keeps a local copy of the item selection, catching up through `items.listSelected` when a notification is truncated. |

## Using the client

```python
from reasock import Client, RpcError

with Client("myapp", "1.0") as reasock:
    tracks = reasock.call("tracks.list", {"limit": 100})["tracks"]
    reasock.on("transport.changed", lambda state: print("playing" if state["playing"] else "stopped"))
    reasock.call("events.subscribe", {"events": ["transport.changed"]})
    reasock.run_forever()
```

- `call()` blocks until REAPER answers and raises `RpcError` (with `.code`) on
  an error, or `TimeoutError` if REAPER is stuck behind a dialog.
- Notification handlers run on their own thread, so they can make calls.
- `selected_items(client, context)` returns every selected item, paging when
  an action context or notification was truncated.

The full API reference ships with ReaSock: run **ReaSock: Open API
documentation** in REAPER.
