// Generated from openrpc.json by scripts/generate.mjs. Don't edit by hand:
// run `npm run generate` after the protocol changes.

/** The protocol version these types describe. */
export const PROTOCOL_VERSION = "0.3.0";

export interface ActionCapabilities {
  register: boolean;
  run: boolean;
}

/** Selected items when an action runs, capped like selection notifications. */
export interface ActionContext {
  /** Up to the first 256 selected items. */
  selectedItems: Item[];
  selectedItemCount: number;
  /**
   * More items are selected than `selectedItems` holds; use
   * `items.listSelected` for the rest.
   */
  truncated: boolean;
}

/** Params of `action.triggered`. */
export interface ActionTriggered {
  /** The id the action was registered with. */
  id: string;
  /** The action's named command, `_REASOCK_<client>_<id>`. */
  command: string;
  context: ActionContext;
}

export interface Capabilities {
  actions: ActionCapabilities;
  events: EventCapabilities;
}

export interface ClientInfo {
  /** Namespaces everything this connection registers. */
  name: string;
  version?: string | null;
}

/**
 * Options for collection events (`items.selectionChanged`,
 * `tracks.selectionChanged`). Notifications never carry more than `limit`
 * GUIDs or entries; anything left out is flagged with `truncated`.
 */
export interface CollectionOptions {
  /** Which full entries to send. Defaults to `changed`. */
  include?: Include;
  /** The most GUIDs or entries one notification may carry. Defaults to 256. */
  limit?: number | null;
}

/** A command ID, such as 40182, or a named command, such as `_SWS_ABOUT`. */
export type CommandRef = number | string;

/** How much of REAPER's main thread Tell is using. Times are in microseconds. */
export interface DebugStats {
  /** Length of the measurement window. */
  seconds: number;
  /** Per timer tick: snapshots, comparisons, and serializing notifications. */
  ticks: Timing;
  events: {
    "items.selectionChanged"?: EventTiming;
    "project.changed"?: EventTiming;
    "project.edited"?: EventTiming;
    "tracks.selectionChanged"?: EventTiming;
    "transport.changed"?: EventTiming;
  };
}

/** A result with nothing to report. */
export type Empty = { [key: string]: unknown };

export interface EventCapabilities {
  available: EventName[];
}

/** An event a client can subscribe to; also its notification method name. */
export type EventName = "transport.changed" | "project.changed" | "project.edited" | "items.selectionChanged" | "tracks.selectionChanged";

export interface EventTiming {
  /** The cheap change check, every tick. */
  check: Timing;
  /** Full snapshots, only when the check shows a change. */
  snapshot: Timing;
  compare: Timing;
  /** Notifications queued, counting each recipient. */
  sent: number;
  /** Size of the most recent notification. */
  lastBytes: number;
}

/** Which full entries a collection notification carries. */
export type Include = "none" | "changed" | "all";

export interface InitializeResult {
  protocolVersion: string;
  server: ServerInfo;
  reaper: ReaperInfo;
  /** This connection's number, unique while REAPER runs. */
  connection: number;
  capabilities: Capabilities;
}

/** A media item. `guid` is stable across sessions. */
export interface Item {
  guid: string;
  /** Name of the active take. */
  name: string | null;
  /** Source file of the active take, or null for MIDI and empty items. */
  path: string | null;
  /** Name of the item's track. */
  track: string | null;
  trackGuid: string | null;
  /** Seconds from the project start. */
  position: number;
  /** Seconds. */
  length: number;
  /** How many takes the item has; see `takes.list`. */
  takeCount: number;
}

/** Params of `items.selectionChanged`. */
export interface ItemsChange {
  /** How many are selected. */
  count: number;
  /**
   * GUIDs newly selected. Omitted, like `removed` and `changed`,
   * when the change is larger than the subscription's `limit`.
   */
  added?: string[] | null;
  /** GUIDs no longer selected. */
  removed?: string[] | null;
  /** GUIDs still selected whose details changed. */
  changed?: string[] | null;
  /**
   * Full entries, per the subscription's `include` option, in
   * selection order.
   */
  items?: Item[] | null;
  /**
   * Something this subscription asks for was left out because it
   * exceeded `limit`. Page through the selection to catch up.
   */
  truncated: boolean;
}

/**
 * Result of `items.get`: each entry in the same position as its GUID, or
 * null if there is no such item.
 */
export interface ItemsFound {
  items: (Item | null)[];
}

/** Result of `items.listSelected` and `items.list`. */
export interface ItemsPage {
  /** Total selected items. */
  count: number;
  offset: number;
  items: Item[];
}

/** A marker or region. */
export interface Marker {
  /**
   * Stable across sessions once the project is saved (REAPER 7 assigns
   * GUIDs to markers and regions when a project is loaded).
   */
  guid: string | null;
  /** The number shown in REAPER. Markers and regions are numbered separately. */
  number: number;
  name: string;
  region: boolean;
  /** Seconds from the project start. */
  position: number;
  /** Where a region ends, in seconds; null for markers. */
  end: number | null;
}

/** Result of `markers.list`. */
export interface MarkersPage {
  /** Total markers and regions. */
  count: number;
  offset: number;
  /** In position order. */
  markers: Marker[];
}

/** Params of `project.changed`. */
export interface Project {
  /** Index of the project's tab. */
  tab: number | null;
  /** File name without extension, or null until the project is saved. */
  name: string | null;
  /** Path of the project file, or null until the project is saved. */
  path: string | null;
  /** Whether there are unsaved changes. */
  dirty: boolean;
}

/** Params of `project.edited`. */
export interface ProjectEdit {
  /** Increases with every change to the project. */
  stateChangeCount: number;
  /** Description of the most recent undo point, for example "Move media items". */
  lastUndo: string | null;
}

export interface RanAction {
  commandId: number;
}

export interface ReaperInfo {
  /** For example "7.78/macOS-arm64". */
  version: string;
}

export interface RegisteredAction {
  /** Named command, stable across sessions: `_REASOCK_<client>_<id>`. */
  command: string;
  /** Numeric ID for this REAPER session only. */
  commandId: number;
}

export interface ServerInfo {
  name: string;
  version: string;
}

/** An event name, or an event with options. */
export type Subscription = EventName | {
  event: EventName;
  options?: CollectionOptions | null;
};

export interface Subscriptions {
  /** All of this connection's subscriptions. */
  events: EventName[];
}

/** A take of a media item. `guid` is stable across sessions. */
export interface Take {
  guid: string;
  name: string | null;
  /** Source file, or null for MIDI and empty takes. */
  path: string | null;
  /** Whether this is the item's active take. */
  active: boolean;
  /** Seconds into the source where the take starts playing. */
  startOffset: number;
  /** 1.0 is normal speed. */
  playRate: number;
}

/** Result of `takes.list`. */
export interface Takes {
  /** GUID of the item. */
  item: string;
  /** In REAPER's order. */
  takes: Take[];
}

export interface Timing {
  count: number;
  avgUs: number;
  maxUs: number;
}

/** A track. `guid` is stable across sessions. */
export interface Track {
  guid: string;
  name: string | null;
  /** 1-based position, as shown in REAPER. */
  number: number;
}

/** Params of `tracks.selectionChanged`. */
export interface TracksChange {
  /** How many are selected. */
  count: number;
  /**
   * GUIDs newly selected. Omitted, like `removed` and `changed`,
   * when the change is larger than the subscription's `limit`.
   */
  added?: string[] | null;
  /** GUIDs no longer selected. */
  removed?: string[] | null;
  /** GUIDs still selected whose details changed. */
  changed?: string[] | null;
  /**
   * Full entries, per the subscription's `include` option, in
   * selection order.
   */
  tracks?: Track[] | null;
  /**
   * Something this subscription asks for was left out because it
   * exceeded `limit`. Page through the selection to catch up.
   */
  truncated: boolean;
}

/** Result of `tracks.get`. */
export interface TracksFound {
  tracks: (Track | null)[];
}

/** Result of `tracks.listSelected` and `tracks.list`. */
export interface TracksPage {
  /** Total selected tracks. */
  count: number;
  offset: number;
  tracks: Track[];
}

/** Params of `transport.changed`. */
export interface Transport {
  playing: boolean;
  paused: boolean;
  recording: boolean;
}

/** Every method, with its params and result. */
export interface Methods {
  /**
   * Start a session
   * 
   * Must be the first call on a connection. WebSocket clients prove they run as the same user as REAPER by sending the `token` from its instance file. The server always answers with its own `protocolVersion`; if that differs from the client's, the client decides whether to continue.
   */
  "initialize": {
    params: {
      protocolVersion: string;
      client: ClientInfo;
      /**
       * Required on WebSocket connections: the `token` from this REAPER's
       * instance file (`~/.reasock/instances/<pid>.json`), which only the
       * current user can read.
       */
      token?: string | null;
      /** Reserved for features the client opts into. Currently ignored. */
      capabilities?: { [key: string]: unknown } | null;
    };
    result: InitializeResult;
  };
  /**
   * Get this document
   * 
   * Returns the OpenRPC document describing this server.
   */
  "rpc.discover": {
    params: Record<string, never>;
    result: { [key: string]: unknown };
  };
  /**
   * Add an action to REAPER's action list
   * 
   * Registers `_REASOCK_<client>_<id>` in the main section. Users can bind it to keys, menus, and toolbars, and those bindings survive restarts. When it runs, this connection receives `action.triggered`. Registering an id this connection already owns returns the existing action. Actions are removed when the connection closes. Always refer to actions by `command`; `commandId` can change between REAPER sessions.
   */
  "action.register": {
    params: {
      /** Unique within this connection. */
      id: string;
      /** Shown in the action list as `ReaSock (<client>): <label>`. */
      label: string;
    };
    result: RegisteredAction;
  };
  /** Remove one of this connection's actions */
  "action.unregister": {
    params: {
      id: string;
    };
    result: Empty;
  };
  /**
   * Run any main-section action
   * 
   * Runs a built-in action by numeric ID (for example 40182, Item: Select all items) or any named action that is currently in the action list.
   */
  "action.run": {
    params: {
      command: CommandRef;
    };
    result: RanAction;
  };
  /**
   * Start receiving state-change notifications
   * 
   * Adds events to this connection's subscriptions, replacing the options of any it already has. Right after subscribing (or changing options), the connection receives the event once with the current state; after that, only when the state changes. State is checked about 30 times per second, and bursts of changes arrive as one notification per check. Collection events accept `CollectionOptions`.
   */
  "events.subscribe": {
    params: {
      events: Subscription[];
    };
    result: Subscriptions;
  };
  /** Stop receiving notifications */
  "events.unsubscribe": {
    params: {
      events: Subscription[];
    };
    result: Subscriptions;
  };
  /** Get the current project */
  "project.get": {
    params: Record<string, never>;
    result: Project;
  };
  /** Get the play, pause, and record state */
  "transport.get": {
    params: Record<string, never>;
    result: Transport;
  };
  /**
   * Get all items, or one track's, a page at a time
   * 
   * Items come in REAPER's order: by track, then position.
   */
  "items.list": {
    params: {
      /** Defaults to 0. */
      offset?: number | null;
      /** Defaults to 256. */
      limit?: number | null;
      /** GUID of a track, to list only its items. */
      track?: string | null;
    };
    result: ItemsPage;
  };
  /** Get the selected items, a page at a time */
  "items.listSelected": {
    params: {
      /** Defaults to 0. */
      offset?: number | null;
      /** Defaults to 256. */
      limit?: number | null;
    };
    result: ItemsPage;
  };
  /**
   * Look up items by GUID
   * 
   * Searches the current project.
   */
  "items.get": {
    params: {
      /** Compared case-insensitively. */
      guids: string[];
    };
    result: ItemsFound;
  };
  /** Get an item's takes */
  "takes.list": {
    params: {
      /** GUID of the item. */
      item: string;
    };
    result: Takes;
  };
  /**
   * Get all tracks, a page at a time
   * 
   * In order. The master track is not included.
   */
  "tracks.list": {
    params: {
      /** Defaults to 0. */
      offset?: number | null;
      /** Defaults to 256. */
      limit?: number | null;
    };
    result: TracksPage;
  };
  /**
   * Get the selected tracks, a page at a time
   * 
   * The master track is never included.
   */
  "tracks.listSelected": {
    params: {
      /** Defaults to 0. */
      offset?: number | null;
      /** Defaults to 256. */
      limit?: number | null;
    };
    result: TracksPage;
  };
  /**
   * Look up tracks by GUID
   * 
   * Searches the current project.
   */
  "tracks.get": {
    params: {
      /** Compared case-insensitively. */
      guids: string[];
    };
    result: TracksFound;
  };
  /** Get markers and regions, a page at a time */
  "markers.list": {
    params: {
      /** Defaults to 0. */
      offset?: number | null;
      /** Defaults to 256. */
      limit?: number | null;
    };
    result: MarkersPage;
  };
  /**
   * Main-thread time spent on events
   * 
   * Reports how much of REAPER's main thread Tell is using, per timer tick and per event.
   */
  "debug.stats": {
    params: {
      /** Start a new measurement window after reading. */
      reset?: boolean | null;
    };
    result: DebugStats;
  };
}

/** Every notification the server sends, with its params. */
export interface Notifications {
  /** One of this connection's actions ran */
  "action.triggered": ActionTriggered;
  /** Play, pause, or record state changed */
  "transport.changed": Transport;
  /** A different project is current, or the current one was saved or became dirty */
  "project.changed": Project;
  /** An undoable change happened in the current project */
  "project.edited": ProjectEdit;
  /** The item selection changed, or a selected item changed */
  "items.selectionChanged": ItemsChange;
  /** The track selection changed, or a selected track changed */
  "tracks.selectionChanged": TracksChange;
}

export type MethodName = keyof Methods;
export type NotificationName = keyof Notifications;

/** Application error codes, beyond JSON-RPC's standard ones. */
export const ErrorCode = {
  NotInitialized: -32001,
  AlreadyInitialized: -32002,
  Unauthorized: -32003,
  ActionConflict: -32010,
  UnknownAction: -32011,
  UnknownCommand: -32012,
  RegistrationFailed: -32013,
  NotFound: -32014,
} as const;
