// An in-memory ReaSock for tests: a WebSocket stand-in wired to a tiny
// JSON-RPC server that records what clients do.

const EVENTS = new Set(["project.changed", "project.edited", "transport.changed", "items.selectionChanged", "tracks.selectionChanged"]);

export class FakeReaSock {
  constructor({ token = "secret" } = {}) {
    this.token = token;
    this.sockets = [];
    this.calls = [];
    this.actions = new Set();
    this.subscriptions = new Set();
    this.hang = new Set(); // methods that never answer
    const server = this;
    this.WebSocket = class {
      constructor(url) {
        this.url = url;
        this.readyState = 0;
        this.listeners = { open: [], message: [], close: [], error: [] };
        server.sockets.push(this);
        queueMicrotask(() => {
          if (server.refuse) return this.emit("error", {});
          this.readyState = 1;
          this.emit("open", {});
        });
      }
      addEventListener(type, listener, options) {
        const wrapped = options?.once ? (event) => { this.removeEventListener(type, wrapped); listener(event); } : listener;
        this.listeners[type].push(wrapped);
      }
      removeEventListener(type, listener) {
        this.listeners[type] = this.listeners[type].filter((l) => l !== listener);
      }
      emit(type, event) {
        for (const listener of [...this.listeners[type]]) listener(event);
      }
      send(text) {
        queueMicrotask(() => server.handle(this, JSON.parse(text)));
      }
      close() {
        if (this.readyState === 3) return;
        this.readyState = 3;
        queueMicrotask(() => this.emit("close", {}));
      }
    };
  }

  get socket() {
    return this.sockets.at(-1);
  }

  reply(socket, message) {
    socket.emit("message", { data: JSON.stringify({ jsonrpc: "2.0", ...message }) });
  }

  notify(method, params) {
    this.reply(this.socket, { method, params });
  }

  handle(socket, { id, method, params }) {
    this.calls.push({ method, params });
    if (this.hang.has(method)) return;
    const ok = (result) => this.reply(socket, { id, result });
    const fail = (code, message) => this.reply(socket, { id, error: { code, message } });
    switch (method) {
      case "initialize":
        if (params.token !== this.token) return fail(-32003, "missing or wrong token");
        return ok({ protocolVersion: "0.3.0", server: { name: "reasock", version: "test" }, reaper: { version: "test" }, connection: this.sockets.length, capabilities: {} });
      case "action.register":
        if (!/^[A-Za-z0-9]{1,32}$/.test(params.id)) return fail(-32602, "invalid action id");
        this.actions.add(params.id);
        return ok({ command: `_REASOCK_test_${params.id}`, commandId: 1 });
      case "events.subscribe":
        if (params.events.some((event) => !EVENTS.has(typeof event === "string" ? event : event.event))) {
          return fail(-32602, "unknown event");
        }
        params.events.forEach((event) => this.subscriptions.add(typeof event === "string" ? event : event.event));
        return ok({ events: [...this.subscriptions] });
      case "events.unsubscribe":
        params.events.forEach((event) => this.subscriptions.delete(event));
        return ok({ events: [...this.subscriptions] });
      case "takes.list":
        return fail(-32014, "no item with that GUID");
      default:
        return ok({ echo: method, params });
    }
  }
}
