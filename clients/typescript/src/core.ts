// The client, independent of how REAPER is found: index.ts adds discovery
// through ~/.reasock for Node, and browser.ts takes the endpoint as given.

import {
  PROTOCOL_VERSION,
  type ActionTriggered,
  type EventName,
  type InitializeResult,
  type MethodName,
  type Methods,
  type NotificationName,
  type Notifications,
  type RegisteredAction,
  type Subscription,
} from "./generated.js";

export type Params<M extends MethodName> = Methods[M]["params"];
export type Result<M extends MethodName> = Methods[M]["result"];
/** Params are optional when every field is. */
type ParamsArgs<M extends MethodName> = {} extends Params<M> ? [params?: Params<M>] : [params: Params<M>];

/** An error response from ReaSock. `code` matches `ErrorCode` or JSON-RPC's standard codes. */
export class RpcError extends Error {
  readonly code: number;
  constructor(error: { code: number; message: string }) {
    super(error.message);
    this.name = "RpcError";
    this.code = error.code;
  }
}

/** Where a REAPER instance listens, and the token it expects. */
export interface Endpoint {
  port: number;
  token: string;
  host?: string;
}

export interface ReconnectOptions {
  /** First delay before reconnecting, doubled after each failure. Default 500 ms. */
  minDelayMs?: number;
  /** Longest delay between attempts. Default 10 s. */
  maxDelayMs?: number;
}

export interface ClientOptions {
  /** Namespaces this client's actions: 1 to 32 ASCII letters or digits. */
  name: string;
  version?: string;
  /** Where to connect, or a function that finds out (called again on reconnect). */
  endpoint: Endpoint | (() => Endpoint | Promise<Endpoint>);
  /** How long a call may take. REAPER can't answer while a modal dialog is open. Default 5 s. */
  timeoutMs?: number;
  /**
   * Reconnect after REAPER restarts or the connection drops. Actions from
   * `registerAction()` and subscriptions from `subscribe()` are restored.
   */
  reconnect?: boolean | ReconnectOptions;
  /** A WebSocket implementation, if there's no global one. */
  WebSocket?: typeof WebSocket;
}

interface Pending {
  resolve(result: unknown): void;
  reject(error: Error): void;
  timer: ReturnType<typeof setTimeout>;
}

type Handler<T> = (params: T) => void;

/** A connection to REAPER through ReaSock. Create one with `connect()`. */
export class ReaSockClient {
  /** What `initialize` returned for the current connection. */
  session!: InitializeResult;

  #options: ClientOptions;
  #socket: WebSocket | null = null;
  #nextId = 1;
  #pending = new Map<number, Pending>();
  #handlers = new Map<string, Set<Handler<any>>>();
  #connectionHandlers = new Set<(connected: boolean) => void>();
  #actions = new Map<string, { label: string; handler: Handler<ActionTriggered> }>();
  #subscriptions = new Map<EventName, Subscription>();
  #closed = false;
  #reconnecting = false;

  private constructor(options: ClientOptions) {
    this.#options = options;
    this.on("action.triggered", (params) => this.#actions.get(params.id)?.handler(params));
  }

  /** Connects and initializes. Rejects if REAPER can't be reached or refuses. */
  static async connect(options: ClientOptions): Promise<ReaSockClient> {
    const client = new ReaSockClient(options);
    await client.#open();
    return client;
  }

  /** Whether the connection is currently open. */
  get connected(): boolean {
    return this.#socket?.readyState === 1;
  }

  /** Calls a method. Rejects with `RpcError`, or an `Error` on timeout or disconnect. */
  call<M extends MethodName>(method: M, ...[params]: ParamsArgs<M>): Promise<Result<M>> {
    return this.#request(method, params ?? {}) as Promise<Result<M>>;
  }

  /** Handles a notification. Returns a function that removes the handler. */
  on<N extends NotificationName>(event: N, handler: Handler<Notifications[N]>): () => void {
    const handlers = this.#handlers.get(event) ?? new Set();
    handlers.add(handler);
    this.#handlers.set(event, handlers);
    return () => handlers.delete(handler);
  }

  /** Called with `false` when the connection drops and `true` when it's back. */
  onConnectionChange(handler: (connected: boolean) => void): () => void {
    this.#connectionHandlers.add(handler);
    return () => this.#connectionHandlers.delete(handler);
  }

  /**
   * Subscribes to events (restored after a reconnect). Returns a function
   * that unsubscribes them.
   */
  async subscribe(...subscriptions: Subscription[]): Promise<() => Promise<void>> {
    for (const subscription of subscriptions) {
      this.#subscriptions.set(eventOf(subscription), subscription);
    }
    await this.call("events.subscribe", { events: subscriptions });
    return async () => {
      const events = subscriptions.map(eventOf);
      events.forEach((event) => this.#subscriptions.delete(event));
      if (this.connected) await this.call("events.unsubscribe", { events });
    };
  }

  /**
   * Adds an action to REAPER's action list and calls `handler` whenever it
   * runs (restored after a reconnect, so hotkeys keep working).
   */
  async registerAction(id: string, label: string, handler: Handler<ActionTriggered>): Promise<RegisteredAction> {
    this.#actions.set(id, { label, handler });
    return this.call("action.register", { id, label });
  }

  /** Closes the connection for good: no reconnecting. */
  close(): void {
    this.#closed = true;
    this.#socket?.close();
  }

  async #open(): Promise<void> {
    const endpoint =
      typeof this.#options.endpoint === "function" ? await this.#options.endpoint() : this.#options.endpoint;
    const WebSocketImpl = this.#options.WebSocket ?? globalThis.WebSocket;
    if (!WebSocketImpl) throw new Error("no WebSocket implementation; pass one as `WebSocket`");
    const socket = new WebSocketImpl(`ws://${endpoint.host ?? "127.0.0.1"}:${endpoint.port}`);
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener("open", () => resolve(), { once: true });
      socket.addEventListener("error", () => reject(new Error(`can't connect to REAPER on port ${endpoint.port}`)), {
        once: true,
      });
    });
    this.#socket = socket;
    socket.addEventListener("message", (event) => this.#receive(String(event.data)));
    socket.addEventListener("close", () => this.#lost(socket));

    const client = this.#options.version
      ? { name: this.#options.name, version: this.#options.version }
      : { name: this.#options.name };
    this.session = await this.call("initialize", {
      protocolVersion: PROTOCOL_VERSION,
      client,
      token: endpoint.token,
    });
    for (const [id, { label }] of this.#actions) {
      await this.call("action.register", { id, label });
    }
    if (this.#subscriptions.size > 0) {
      await this.call("events.subscribe", { events: [...this.#subscriptions.values()] });
    }
  }

  #request(method: string, params: unknown): Promise<unknown> {
    const socket = this.#socket;
    if (!socket || socket.readyState !== 1) return Promise.reject(new Error("not connected to REAPER"));
    const id = this.#nextId++;
    const timeoutMs = this.#options.timeoutMs ?? 5000;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new Error(`${method} timed out; is a REAPER dialog open?`));
      }, timeoutMs);
      this.#pending.set(id, { resolve, reject, timer });
      socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    });
  }

  #receive(text: string): void {
    const messages: any[] = [JSON.parse(text)].flat();
    for (const message of messages) {
      const pending = typeof message.id === "number" ? this.#pending.get(message.id) : undefined;
      if (pending) {
        this.#pending.delete(message.id);
        clearTimeout(pending.timer);
        if (message.error) pending.reject(new RpcError(message.error));
        else pending.resolve(message.result);
      } else if (typeof message.method === "string") {
        for (const handler of this.#handlers.get(message.method) ?? []) {
          try {
            handler(message.params);
          } catch (error) {
            console.error(`ReaSock: ${message.method} handler failed`, error);
          }
        }
      }
    }
  }

  #lost(socket: WebSocket): void {
    if (socket !== this.#socket) return;
    this.#socket = null;
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(new Error("connection to REAPER closed"));
    }
    this.#pending.clear();
    this.#connectionHandlers.forEach((handler) => handler(false));
    if (!this.#closed && this.#options.reconnect) void this.#reconnect();
  }

  async #reconnect(): Promise<void> {
    if (this.#reconnecting) return;
    this.#reconnecting = true;
    const settings = typeof this.#options.reconnect === "object" ? this.#options.reconnect : {};
    let delay = settings.minDelayMs ?? 500;
    while (!this.#closed) {
      await new Promise((resolve) => setTimeout(resolve, delay));
      try {
        await this.#open();
        this.#reconnecting = false;
        this.#connectionHandlers.forEach((handler) => handler(true));
        return;
      } catch {
        this.#socket?.close();
        this.#socket = null;
        delay = Math.min(delay * 2, settings.maxDelayMs ?? 10_000);
      }
    }
    this.#reconnecting = false;
  }
}

function eventOf(subscription: Subscription): EventName {
  return typeof subscription === "string" ? subscription : subscription.event;
}
