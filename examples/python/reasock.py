"""A small ReaSock client for Python 3.11+.

    from reasock import Client

    with Client("myapp") as reasock:
        print(reasock.call("project.get"))
        reasock.on("transport.changed", print)
        reasock.call("events.subscribe", {"events": ["transport.changed"]})
        reasock.run_forever()

Needs the `websockets` package (pip install -r requirements.txt).
"""

from __future__ import annotations

import itertools
import json
import os
import queue
import threading
from pathlib import Path
from typing import Any, Callable

from websockets.sync.client import connect as ws_connect

PROTOCOL_VERSION = "0.5.0"


class RpcError(Exception):
    """An error response from ReaSock."""

    def __init__(self, error: dict):
        super().__init__(error.get("message", "error"))
        self.code = error.get("code")


def _alive(pid: int) -> bool:
    if os.name == "nt":
        # os.kill(pid, 0) would terminate the process on Windows.
        return True
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def find_instance() -> dict:
    """The most recently started REAPER that's still running:
    {pid, port, token, reasockVersion, reaperVersion}."""
    instances = Path.home() / ".reasock" / "instances"
    files = sorted(instances.glob("*.json"), key=lambda path: path.stat().st_mtime, reverse=True)
    for path in files:
        instance = json.loads(path.read_text())
        if _alive(instance["pid"]):
            return instance
    raise RuntimeError(f"no running REAPER found in {instances}")


class Client:
    """A connection to REAPER. Calls block until answered; notification
    handlers run on their own thread, so they can make calls too."""

    def __init__(self, name: str, version: str | None = None, *, port: int | None = None,
                 token: str | None = None, timeout: float = 5.0):
        if port is None or token is None:
            instance = find_instance()
            port = port or instance["port"]
            token = token or instance["token"]
        self.timeout = timeout
        self._ws = ws_connect(f"ws://127.0.0.1:{port}", max_size=None)
        self._ids = itertools.count(1)
        self._pending: dict[int, queue.Queue] = {}
        self._lock = threading.Lock()
        self._handlers: dict[str, Callable[[dict], Any]] = {}
        self._notifications: queue.Queue = queue.Queue()
        self.closed = threading.Event()
        threading.Thread(target=self._read, name="reasock-read", daemon=True).start()
        threading.Thread(target=self._dispatch, name="reasock-events", daemon=True).start()
        client = {"name": name} | ({"version": version} if version else {})
        self.session = self.call(
            "initialize", {"protocolVersion": PROTOCOL_VERSION, "client": client, "token": token}
        )

    def call(self, method: str, params: dict | None = None) -> Any:
        """Calls a method and returns its result, or raises RpcError."""
        request_id = next(self._ids)
        reply: queue.Queue = queue.Queue(maxsize=1)
        with self._lock:
            self._pending[request_id] = reply
        self._ws.send(json.dumps(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params or {}}
        ))
        try:
            message = reply.get(timeout=self.timeout)
        except queue.Empty:
            raise TimeoutError(f"{method} timed out; is a REAPER dialog open?") from None
        finally:
            with self._lock:
                self._pending.pop(request_id, None)
        if message is None:
            raise ConnectionError("connection closed")
        if "error" in message:
            raise RpcError(message["error"])
        return message["result"]

    def on(self, method: str, handler: Callable[[dict], Any]) -> None:
        """Handles a notification, such as an event or action.triggered."""
        self._handlers[method] = handler

    def run_forever(self) -> None:
        """Blocks until the connection closes, or Ctrl+C."""
        try:
            while not self.closed.wait(0.5):
                pass
        except KeyboardInterrupt:
            pass

    def close(self) -> None:
        self._ws.close()

    def __enter__(self) -> "Client":
        return self

    def __exit__(self, *_: Any) -> None:
        self.close()

    def _read(self) -> None:
        try:
            for text in self._ws:
                messages = json.loads(text)
                for message in messages if isinstance(messages, list) else [messages]:
                    with self._lock:
                        reply = self._pending.get(message.get("id"))
                    if reply is not None:
                        reply.put(message)
                    elif "method" in message:
                        self._notifications.put(message)
        except Exception:
            pass
        finally:
            self.closed.set()
            self._notifications.put(None)
            with self._lock:
                for reply in self._pending.values():
                    reply.put(None)

    def _dispatch(self) -> None:
        while (message := self._notifications.get()) is not None:
            handler = self._handlers.get(message["method"])
            if handler:
                try:
                    handler(message.get("params", {}))
                except Exception as error:
                    print(f"{message['method']} handler failed: {error!r}")


def selected_items(client: Client, context_or_change: dict | None = None) -> list[dict]:
    """Every selected item. Uses the items in an action context or
    notification when they're complete, and pages through
    items.listSelected when they were truncated."""
    if context_or_change and not context_or_change.get("truncated"):
        return context_or_change.get("selectedItems", context_or_change.get("items", []))
    items: list[dict] = []
    while True:
        page = client.call("items.listSelected", {"offset": len(items), "limit": 1000})
        items += page["items"]
        if len(items) >= page["count"] or not page["items"]:
            return items
