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
  /** Details some errors carry, such as which file `items.insert` couldn't use. */
  readonly data?: unknown;
  constructor(error: { code: number; message: string; data?: unknown }) {
    super(error.message);
    this.name = "RpcError";
    this.code = error.code;
    this.data = error.data;
  }
}

/** Not connected to REAPER: it isn't running, or the connection dropped. */
export class NotConnectedError extends Error {
  constructor(message = "not connected to REAPER") {
    super(message);
    this.name = "NotConnectedError";
  }
}

/** REAPER didn't answer in time. Usually a modal dialog is open in REAPER. */
export class TimeoutError extends Error {
  constructor(method: string) {
    super(`${method} timed out; is a REAPER dialog open?`);
    this.name = "TimeoutError";
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
  /** Stops connecting and reconnecting, like `close()`. */
  signal?: AbortSignal;
  /**
   * While waiting to (re)connect, called with a function that retries right
   * away; returns a function that stops watching. The Node entry point
   * watches `~/.reasock/instances` so a starting REAPER is found at once.
   */
  watch?: (retryNow: () => void) => () => void;
  /** A WebSocket implementation, if there's no global one. */
  WebSocket?: typeof WebSocket;
}

/** The methods of protocol 0.3.0, whose servers don't list their methods. */
const PROTOCOL_0_3_METHODS: readonly string[] = [
  "initialize",
  "rpc.discover",
  "action.register",
  "action.unregister",
  "action.run",
  "events.subscribe",
  "events.unsubscribe",
  "project.get",
  "transport.get",
  "items.list",
  "items.listSelected",
  "items.get",
  "takes.list",
  "tracks.list",
  "tracks.listSelected",
  "tracks.get",
  "markers.list",
  "debug.stats",
];

interface Pending {
  resolve(result: unknown): void;
  reject(error: Error): void;
  timer: ReturnType<typeof setTimeout>;
}

type Handler<T> = (params: T) => void;

/** A connection to REAPER through ReaSock. Create one with `connect()` or `createClient()`. */
export class ReaSockClient {
  /** What `initialize` returned for the current connection; unset until the first one. */
  session!: InitializeResult;

  #options: ClientOptions;
  #socket: WebSocket | null = null;
  #nextId = 1;
  #pending = new Map<number, Pending>();
  #handlers = new Map<string, Set<Handler<any>>>();
  #connectionHandlers = new Set<(connected: boolean) => void>();
  #actions = new Map<string, { label: string; handler: Handler<ActionTriggered>; on?: boolean }>();
  #subscriptions = new Map<EventName, Subscription>();
  /** `registerAction()` and `subscribe()` calls waiting for a connection. */
  #waiting = new Set<{ resolve(): void; reject(error: Error): void }>();
  /** Initialized and restored: what `connected` reports. */
  #ready = false;
  #closed = false;
  #reconnecting = false;
  #retryNow: (() => void) | null = null;

  private constructor(options: ClientOptions) {
    this.#options = options;
    this.on("action.triggered", (params) => this.#actions.get(params.id)?.handler(params));
    options.signal?.addEventListener("abort", () => this.close(), { once: true });
  }

  /** Connects and initializes. Rejects if REAPER can't be reached or refuses. */
  static async connect(options: ClientOptions): Promise<ReaSockClient> {
    const client = new ReaSockClient(options);
    await client.#open();
    return client;
  }

  /**
   * Returns a client right away and connects in the background, waiting for
   * REAPER to start if it isn't running, then reconnecting as needed.
   */
  static create(options: ClientOptions): ReaSockClient {
    const client = new ReaSockClient({ ...options, reconnect: options.reconnect || true });
    void client.#reconnect(true);
    return client;
  }

  /** Whether the connection is currently open. */
  get connected(): boolean {
    return this.#ready && this.#socket?.readyState === 1;
  }

  /**
   * Whether the connected server has a method. Servers before protocol 0.4.0
   * don't list their methods, so for them it's the methods of protocol 0.3.0.
   * False while not connected.
   */
  supports(method: MethodName | (string & {})): boolean {
    if (!this.connected) return false;
    return (this.session.capabilities.methods ?? PROTOCOL_0_3_METHODS).includes(method);
  }

  /**
   * Calls a method. Rejects with `RpcError`, `TimeoutError`, or
   * `NotConnectedError` (immediately, when not connected).
   */
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
   * that unsubscribes them. While a reconnecting client is disconnected, this
   * waits for the connection.
   */
  async subscribe(...subscriptions: Subscription[]): Promise<() => Promise<void>> {
    await this.#untilConnected();
    // Remembered only once REAPER accepts them: one it refused would fail
    // every reconnect.
    await this.call("events.subscribe", { events: subscriptions });
    for (const subscription of subscriptions) {
      this.#subscriptions.set(eventOf(subscription), subscription);
    }
    return async () => {
      const events = subscriptions.map(eventOf);
      events.forEach((event) => this.#subscriptions.delete(event));
      if (this.connected) await this.call("events.unsubscribe", { events });
    };
  }

  /**
   * Adds an action to REAPER's action list and calls `handler` whenever it
   * runs (restored after a reconnect, so hotkeys keep working). While a
   * reconnecting client is disconnected, this waits for the connection.
   *
   * With `toggle: true` the action has an on/off state, shown as a checkmark
   * in menus and a lit toolbar button. It starts off; change it with
   * `setActionState()`, usually from `handler`.
   */
  async registerAction(
    id: string,
    label: string,
    handler: Handler<ActionTriggered>,
    options: { toggle?: boolean } = {},
  ): Promise<RegisteredAction> {
    await this.#untilConnected();
    const toggle = options.toggle ?? false;
    const registered = await this.call("action.register", toggle ? { id, label, toggle } : { id, label });
    this.#actions.set(id, toggle ? { label, handler, on: this.#actions.get(id)?.on ?? false } : { label, handler });
    return registered;
  }

  /** Turns a toggle action on or off (restored after a reconnect). */
  async setActionState(id: string, on: boolean): Promise<void> {
    await this.call("action.setState", { id, on });
    const action = this.#actions.get(id);
    if (action) action.on = on;
  }

  /** Closes the connection for good: no reconnecting. */
  close(): void {
    this.#closed = true;
    this.#socket?.close();
    this.#retryNow?.();
    for (const waiter of this.#waiting) waiter.reject(new NotConnectedError("the client was closed"));
    this.#waiting.clear();
  }

  /** Resolves once connected; waits only if this client will (re)connect. */
  #untilConnected(): Promise<void> {
    if (this.connected || this.#closed || !this.#options.reconnect) return Promise.resolve();
    return new Promise((resolve, reject) => this.#waiting.add({ resolve, reject }));
  }

  async #open(): Promise<void> {
    const endpoint =
      typeof this.#options.endpoint === "function" ? await this.#options.endpoint() : this.#options.endpoint;
    const WebSocketImpl = this.#options.WebSocket ?? globalThis.WebSocket;
    if (!WebSocketImpl) throw new Error("no WebSocket implementation; pass one as `WebSocket`");
    const socket = new WebSocketImpl(`ws://${endpoint.host ?? "127.0.0.1"}:${endpoint.port}`);
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener("open", () => resolve(), { once: true });
      socket.addEventListener(
        "error",
        () => reject(new NotConnectedError(`can't connect to REAPER on port ${endpoint.port}`)),
        { once: true },
      );
    });
    this.#socket = socket;
    socket.addEventListener("message", (event) => this.#receive(String(event.data)));
    socket.addEventListener("close", () => this.#lost(socket));

    const client = this.#options.version
      ? { name: this.#options.name, version: this.#options.version }
      : { name: this.#options.name };
    try {
      this.session = (await this.#request("initialize", {
        protocolVersion: PROTOCOL_VERSION,
        client,
        token: endpoint.token,
      })) as InitializeResult;
      for (const [id, { label, on }] of this.#actions) {
        if (on === undefined) {
          await this.call("action.register", { id, label });
        } else {
          await this.call("action.register", { id, label, toggle: true });
          if (on) await this.call("action.setState", { id, on });
        }
      }
      if (this.#subscriptions.size > 0) {
        await this.call("events.subscribe", { events: [...this.#subscriptions.values()] });
      }
    } catch (error) {
      // Refused (a wrong token, say): don't leave the socket open.
      this.#socket = null;
      socket.close();
      throw error;
    }
    this.#ready = true;
    for (const waiter of this.#waiting) waiter.resolve();
    this.#waiting.clear();
  }

  #request(method: string, params: unknown): Promise<unknown> {
    const socket = this.#socket;
    if (!socket || socket.readyState !== 1) return Promise.reject(new NotConnectedError());
    const id = this.#nextId++;
    const timeoutMs = this.#options.timeoutMs ?? 5000;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new TimeoutError(method));
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
    this.#ready = false;
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(new NotConnectedError("connection to REAPER closed"));
    }
    this.#pending.clear();
    this.#connectionHandlers.forEach((handler) => handler(false));
    if (!this.#closed && this.#options.reconnect) void this.#reconnect();
  }

  /** Tries to connect until it works or the client is closed; `now` skips the first delay. */
  async #reconnect(now = false): Promise<void> {
    if (this.#reconnecting) return;
    this.#reconnecting = true;
    const settings = typeof this.#options.reconnect === "object" ? this.#options.reconnect : {};
    const minDelay = settings.minDelayMs ?? 500;
    let delay = minDelay;
    let wait = now ? 0 : delay;
    const stopWatching = this.#options.watch?.(() => this.#retryNow?.());
    try {
      while (!this.#closed) {
        await new Promise<void>((resolve) => {
          const timer = setTimeout(resolve, wait);
          this.#retryNow = () => {
            clearTimeout(timer);
            resolve();
          };
        });
        this.#retryNow = null;
        if (this.#closed) break;
        try {
          await this.#open();
          this.#connectionHandlers.forEach((handler) => handler(true));
          return;
        } catch {
          delay = Math.min(delay * 2, settings.maxDelayMs ?? 10_000);
          wait = delay;
        }
      }
    } finally {
      stopWatching?.();
      this.#retryNow = null;
      this.#reconnecting = false;
    }
  }
}

function eventOf(subscription: Subscription): EventName {
  return typeof subscription === "string" ? subscription : subscription.event;
}
