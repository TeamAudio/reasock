//! The protocol's methods and notifications, and the OpenRPC document
//! generated from them.
//!
//! Adding a method means adding its params and result types to `types.rs`,
//! a `Method` impl here, and an entry in `METHODS`. Dispatch, the
//! initialize gate, params validation, and the published schema all follow.
//! `rpc.discover` serves the generated document. `openrpc.json` is a published
//! copy, checked against it by a test; regenerate it with
//! `REASOCK_UPDATE_OPENRPC=1 cargo test`.

use crate::bridge;
use crate::collection;
use crate::model;
use crate::protocol::{
    self, AppError, Params, RpcError, ACTION_CONFLICT, ALREADY_INITIALIZED, APP_ERRORS,
    MEDIA_UNAVAILABLE, NOT_FOUND, NOT_INITIALIZED, REGISTRATION_FAILED, UNAUTHORIZED,
    UNKNOWN_ACTION, UNKNOWN_COMMAND,
};
use crate::server::ConnId;
use crate::types::*;
use schemars::generate::{Contract, SchemaGenerator, SchemaSettings};
use schemars::transform::{
    RemoveRefSiblings, ReplacePrefixItems, ReplaceUnevaluatedProperties, Transform,
};
use schemars::{JsonSchema, Schema};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

/// Who is calling.
pub struct Ctx {
    pub conn: ConnId,
    /// Set once the connection has called `initialize`.
    pub client: Option<String>,
}

impl Ctx {
    fn client(&self) -> &str {
        self.client.as_deref().expect("only open methods run before initialize")
    }
}

/// A method clients can call.
pub trait Method {
    const NAME: &'static str;
    const SUMMARY: &'static str;
    const DESCRIPTION: &'static str = "";
    /// Callable before `initialize`.
    const OPEN: bool = false;
    /// Application errors besides `NotInitialized`, which every method that
    /// isn't `OPEN` can return.
    const ERRORS: &'static [&'static AppError] = &[];
    type Params: DeserializeOwned + JsonSchema;
    type Result: Serialize + JsonSchema;

    fn call(ctx: &Ctx, params: Self::Params) -> Result<Self::Result, RpcError>;

    /// OpenRPC example pairings.
    fn examples() -> Value {
        json!([])
    }
}

/// A notification the server sends.
pub trait Notification {
    const NAME: &'static str;
    const SUMMARY: &'static str;
    const DESCRIPTION: &'static str;
    type Params: Serialize + JsonSchema;
}

// ---------------------------------------------------------------------------
// Methods

struct Initialize;
impl Method for Initialize {
    const NAME: &'static str = "initialize";
    const SUMMARY: &'static str = "Start a session";
    const DESCRIPTION: &'static str = "Must be the first call on a connection. WebSocket clients \
        prove they run as the same user as REAPER by sending the `token` from its instance file. \
        The server always answers with its own `protocolVersion`; if that differs from the \
        client's, the client decides whether to continue.";
    const OPEN: bool = true;
    const ERRORS: &'static [&'static AppError] = &[&ALREADY_INITIALIZED, &UNAUTHORIZED];
    type Params = InitializeParams;
    type Result = InitializeResult;

    fn call(ctx: &Ctx, params: InitializeParams) -> Result<InitializeResult, RpcError> {
        bridge::initialize(ctx.conn, params)
    }

    fn examples() -> Value {
        json!([{
            "name": "demo client",
            "params": [
                { "name": "protocolVersion", "value": protocol::PROTOCOL_VERSION },
                { "name": "client", "value": { "name": "demo", "version": "1.0.0" } },
                { "name": "token", "value": "<64 hex digits from the instance file>" },
            ],
            "result": {
                "name": "result",
                "value": {
                    "protocolVersion": protocol::PROTOCOL_VERSION,
                    "server": { "name": "reasock", "version": "0.1.0" },
                    "reaper": { "version": "7.78/macOS-arm64" },
                    "connection": 1,
                    "capabilities": {
                        "actions": { "register": true, "run": true },
                        "events": { "available": EventName::ALL },
                        "methods": methods().iter().map(|entry| entry.name).collect::<Vec<_>>(),
                    },
                },
            },
        }])
    }
}

struct Discover;
impl Method for Discover {
    const NAME: &'static str = "rpc.discover";
    const SUMMARY: &'static str = "Get this document";
    const DESCRIPTION: &'static str = "Returns the OpenRPC document describing this server.";
    const OPEN: bool = true;
    type Params = NoParams;
    type Result = Map<String, Value>;

    fn call(_: &Ctx, _: NoParams) -> Result<Map<String, Value>, RpcError> {
        static DOCUMENT: OnceLock<Map<String, Value>> = OnceLock::new();
        let document = DOCUMENT.get_or_init(|| match document() {
            Value::Object(document) => document,
            _ => unreachable!("the document is an object"),
        });
        Ok(document.clone())
    }
}

struct RegisterAction;
impl Method for RegisterAction {
    const NAME: &'static str = "action.register";
    const SUMMARY: &'static str = "Add an action to REAPER's action list";
    const DESCRIPTION: &'static str = "Registers `_REASOCK_<client>_<id>` in the main section. \
        Users can bind it to keys, menus, and toolbars, and those bindings survive restarts. \
        When it runs, this connection receives `action.triggered`. Registering an id this \
        connection already owns returns the existing action. Actions are removed when the \
        connection closes. Always refer to actions by `command`; `commandId` can change between \
        REAPER sessions.";
    const ERRORS: &'static [&'static AppError] = &[&ACTION_CONFLICT, &REGISTRATION_FAILED];
    type Params = RegisterActionParams;
    type Result = RegisteredAction;

    fn call(ctx: &Ctx, params: RegisterActionParams) -> Result<RegisteredAction, RpcError> {
        bridge::register_action(ctx.conn, ctx.client(), params)
    }

    fn examples() -> Value {
        json!([{
            "name": "hello action",
            "params": [
                { "name": "id", "value": "hello" },
                { "name": "label", "value": "Hello" },
            ],
            "result": {
                "name": "result",
                "value": { "command": "_REASOCK_demo_hello", "commandId": 55867 },
            },
        }])
    }
}

struct UnregisterAction;
impl Method for UnregisterAction {
    const NAME: &'static str = "action.unregister";
    const SUMMARY: &'static str = "Remove one of this connection's actions";
    const ERRORS: &'static [&'static AppError] = &[&UNKNOWN_ACTION];
    type Params = UnregisterActionParams;
    type Result = Empty;

    fn call(ctx: &Ctx, params: UnregisterActionParams) -> Result<Empty, RpcError> {
        bridge::unregister_action(ctx.conn, params)
    }
}

struct RunAction;
impl Method for RunAction {
    const NAME: &'static str = "action.run";
    const SUMMARY: &'static str = "Run any main-section action";
    const DESCRIPTION: &'static str = "Runs a built-in action by numeric ID (for example 40182, \
        Item: Select all items) or any named action that is currently in the action list.";
    const ERRORS: &'static [&'static AppError] = &[&UNKNOWN_COMMAND];
    type Params = RunActionParams;
    type Result = RanAction;

    fn call(_: &Ctx, params: RunActionParams) -> Result<RanAction, RpcError> {
        bridge::run_action(params)
    }
}

struct Subscribe;
impl Method for Subscribe {
    const NAME: &'static str = "events.subscribe";
    const SUMMARY: &'static str = "Start receiving state-change notifications";
    const DESCRIPTION: &'static str = "Adds events to this connection's subscriptions, replacing \
        the options of any it already has. Right after subscribing (or changing options), the \
        connection receives the event once with the current state; after that, only when the \
        state changes. State is checked about 30 times per second, and bursts of changes arrive \
        as one notification per check. Collection events accept `CollectionOptions`.";
    type Params = SubscriptionParams;
    type Result = Subscriptions;

    fn call(ctx: &Ctx, params: SubscriptionParams) -> Result<Subscriptions, RpcError> {
        bridge::subscribe(ctx.conn, params)
    }

    fn examples() -> Value {
        json!([
            {
                "name": "follow the selection",
                "params": [{ "name": "events", "value": ["items.selectionChanged"] }],
                "result": { "name": "result", "value": { "events": ["items.selectionChanged"] } },
            },
            {
                "name": "selection GUIDs only",
                "params": [{
                    "name": "events",
                    "value": [{
                        "event": "items.selectionChanged",
                        "options": { "include": "none", "limit": 1000 },
                    }],
                }],
                "result": { "name": "result", "value": { "events": ["items.selectionChanged"] } },
            },
        ])
    }
}

struct Unsubscribe;
impl Method for Unsubscribe {
    const NAME: &'static str = "events.unsubscribe";
    const SUMMARY: &'static str = "Stop receiving notifications";
    type Params = SubscriptionParams;
    type Result = Subscriptions;

    fn call(ctx: &Ctx, params: SubscriptionParams) -> Result<Subscriptions, RpcError> {
        bridge::unsubscribe(ctx.conn, params)
    }
}

struct GetProject;
impl Method for GetProject {
    const NAME: &'static str = "project.get";
    const SUMMARY: &'static str = "Get the current project";
    type Params = NoParams;
    type Result = Project;

    fn call(_: &Ctx, _: NoParams) -> Result<Project, RpcError> {
        Ok(model::project())
    }
}

struct GetTransport;
impl Method for GetTransport {
    const NAME: &'static str = "transport.get";
    const SUMMARY: &'static str = "Get the play state, edit cursor, and time selection";
    type Params = NoParams;
    type Result = Transport;

    fn call(_: &Ctx, _: NoParams) -> Result<Transport, RpcError> {
        Ok(model::transport())
    }
}

/// `offset` and validated `limit` for paged methods.
fn page(params: &PageParams) -> Result<(usize, usize), RpcError> {
    paging(params.offset, params.limit)
}

fn paging(offset: Option<usize>, limit: Option<usize>) -> Result<(usize, usize), RpcError> {
    let limit = limit.unwrap_or(collection::DEFAULT_LIMIT);
    if !(1..=collection::MAX_LIMIT).contains(&limit) {
        return Err(RpcError::invalid_params(format!(
            "limit must be between 1 and {}",
            collection::MAX_LIMIT
        )));
    }
    Ok((offset.unwrap_or(0), limit))
}

pub fn not_found(what: &str, guid: &str) -> RpcError {
    RpcError::app(&NOT_FOUND, format!("no {what} with GUID {guid}"))
}

fn guids(params: GuidsParams) -> Result<Vec<String>, RpcError> {
    if params.guids.is_empty() || params.guids.len() > collection::MAX_LIMIT {
        return Err(RpcError::invalid_params(format!(
            "guids must hold 1 to {} GUIDs",
            collection::MAX_LIMIT
        )));
    }
    Ok(params.guids)
}

struct ListItems;
impl Method for ListItems {
    const NAME: &'static str = "items.list";
    const SUMMARY: &'static str = "Get all items, or one track's, a page at a time";
    const DESCRIPTION: &'static str = "Items come in REAPER's order: by track, then position.";
    const ERRORS: &'static [&'static AppError] = &[&NOT_FOUND];
    type Params = ItemsListParams;
    type Result = ItemsPage;

    fn call(_: &Ctx, params: ItemsListParams) -> Result<ItemsPage, RpcError> {
        let (offset, limit) = paging(params.offset, params.limit)?;
        let track = match &params.track {
            Some(guid) => Some(model::find_track(guid).ok_or_else(|| not_found("track", guid))?),
            None => None,
        };
        let extras = extras(params.notes, params.ext.as_deref())?;
        let (count, items) = model::items_page(offset, limit, track, extras);
        Ok(ItemsPage { count, offset, items })
    }
}

struct ListTakes;
impl Method for ListTakes {
    const NAME: &'static str = "takes.list";
    const SUMMARY: &'static str = "Get an item's takes";
    const ERRORS: &'static [&'static AppError] = &[&NOT_FOUND];
    type Params = TakesListParams;
    type Result = Takes;

    fn call(_: &Ctx, params: TakesListParams) -> Result<Takes, RpcError> {
        extras(None, params.ext.as_deref())?;
        let item = model::find_item(&params.item).ok_or_else(|| not_found("item", &params.item))?;
        Ok(Takes {
            takes: unsafe { model::takes(item, params.ext.as_deref()) },
            item: params.item,
        })
    }
}

struct ListTracks;
impl Method for ListTracks {
    const NAME: &'static str = "tracks.list";
    const SUMMARY: &'static str = "Get all tracks, a page at a time";
    const DESCRIPTION: &'static str = "In order. The master track is not included.";
    type Params = PageParams;
    type Result = TracksPage;

    fn call(_: &Ctx, params: PageParams) -> Result<TracksPage, RpcError> {
        let (offset, limit) = page(&params)?;
        let (count, tracks) = model::tracks_page(offset, limit);
        Ok(TracksPage { count, offset, tracks })
    }
}

struct ListMarkers;
impl Method for ListMarkers {
    const NAME: &'static str = "markers.list";
    const SUMMARY: &'static str = "Get markers and regions, a page at a time";
    type Params = PageParams;
    type Result = MarkersPage;

    fn call(_: &Ctx, params: PageParams) -> Result<MarkersPage, RpcError> {
        let (offset, limit) = page(&params)?;
        let (count, markers) = model::markers_page(offset, limit);
        Ok(MarkersPage { count, offset, markers })
    }
}

struct ListSelectedItems;
impl Method for ListSelectedItems {
    const NAME: &'static str = "items.listSelected";
    const SUMMARY: &'static str = "Get the selected items, a page at a time";
    type Params = SelectedItemsParams;
    type Result = ItemsPage;

    fn call(_: &Ctx, params: SelectedItemsParams) -> Result<ItemsPage, RpcError> {
        let (offset, limit) = paging(params.offset, params.limit)?;
        let extras = extras(params.notes, params.ext.as_deref())?;
        let (count, items) = model::selected_items_page(offset, limit, extras);
        Ok(ItemsPage { count, offset, items })
    }
}

struct GetItems;
impl Method for GetItems {
    const NAME: &'static str = "items.get";
    const SUMMARY: &'static str = "Look up items by GUID";
    const DESCRIPTION: &'static str = "Searches the current project.";
    type Params = ItemsGetParams;
    type Result = ItemsFound;

    fn call(_: &Ctx, params: ItemsGetParams) -> Result<ItemsFound, RpcError> {
        let extras = extras(params.notes, params.ext.as_deref())?;
        let guids = guids(GuidsParams { guids: params.guids })?;
        Ok(ItemsFound {
            items: model::items_by_guid(&guids, extras),
        })
    }
}

/// The optional item fields asked for, with their keys checked.
fn extras(notes: Option<bool>, ext: Option<&[ExtKey]>) -> Result<model::Extras<'_>, RpcError> {
    if let Some(keys) = ext {
        if keys.len() > 64 {
            return Err(RpcError::invalid_params("ext can ask for at most 64 keys"));
        }
        keys.iter().try_for_each(crate::edit::check_key)?;
    }
    Ok(model::Extras {
        notes: notes.unwrap_or(false),
        ext,
    })
}

struct SetItems;
impl Method for SetItems {
    const NAME: &'static str = "items.set";
    const SUMMARY: &'static str = "Change items' notes and extension data";
    const DESCRIPTION: &'static str = "Applies every change as one undo point. Every GUID is \
        checked first, so if one isn't found, nothing changes. Extension data is saved in the \
        project, so writing it marks the project as having unsaved changes: write it in response \
        to something the user did, not whenever a project opens. Like any edit, it's undone by \
        undo.";
    const ERRORS: &'static [&'static AppError] = &[&NOT_FOUND];
    type Params = SetItemsParams;
    type Result = Empty;

    fn call(ctx: &Ctx, params: SetItemsParams) -> Result<Empty, RpcError> {
        crate::edit::set_items(ctx.client(), params)
    }

    fn examples() -> Value {
        json!([{
            "name": "tag an item",
            "params": [
                { "name": "items", "value": [{
                    "guid": "{6D1A4C1E-8E0B-4D2A-9B0F-2A3C4D5E6F70}",
                    "ext": { "myapp.id": "8f2c", "myapp.old": null },
                }] },
            ],
            "result": { "name": "result", "value": {} },
        }])
    }
}

struct SetTakes;
impl Method for SetTakes {
    const NAME: &'static str = "takes.set";
    const SUMMARY: &'static str = "Rename takes and change their extension data";
    const DESCRIPTION: &'static str = "Works like `items.set`, for takes.";
    const ERRORS: &'static [&'static AppError] = &[&NOT_FOUND];
    type Params = SetTakesParams;
    type Result = Empty;

    fn call(ctx: &Ctx, params: SetTakesParams) -> Result<Empty, RpcError> {
        crate::edit::set_takes(ctx.client(), params)
    }
}

struct ListSelectedTracks;
impl Method for ListSelectedTracks {
    const NAME: &'static str = "tracks.listSelected";
    const SUMMARY: &'static str = "Get the selected tracks, a page at a time";
    const DESCRIPTION: &'static str = "The master track is never included.";
    type Params = PageParams;
    type Result = TracksPage;

    fn call(_: &Ctx, params: PageParams) -> Result<TracksPage, RpcError> {
        let (offset, limit) = page(&params)?;
        let (count, tracks) = model::selected_tracks_page(offset, limit);
        Ok(TracksPage { count, offset, tracks })
    }
}

struct GetTracks;
impl Method for GetTracks {
    const NAME: &'static str = "tracks.get";
    const SUMMARY: &'static str = "Look up tracks by GUID";
    const DESCRIPTION: &'static str = "Searches the current project.";
    type Params = GuidsParams;
    type Result = TracksFound;

    fn call(_: &Ctx, params: GuidsParams) -> Result<TracksFound, RpcError> {
        Ok(TracksFound {
            tracks: model::tracks_by_guid(&guids(params)?),
        })
    }
}

struct InsertItems;
impl Method for InsertItems {
    const NAME: &'static str = "items.insert";
    const SUMMARY: &'static str = "Place media files on the timeline";
    const DESCRIPTION: &'static str = "Creates items from files, or parts of files, as a single \
        undo point. Every file is checked before anything changes, so if one can't be used, \
        nothing is inserted; a `MediaUnavailable` error's `data` says which file and why: \
        `{index, path, reason}`. New items get no fades.";
    const ERRORS: &'static [&'static AppError] = &[&NOT_FOUND, &MEDIA_UNAVAILABLE];
    type Params = InsertParams;
    type Result = Inserted;

    fn call(ctx: &Ctx, params: InsertParams) -> Result<Inserted, RpcError> {
        crate::edit::insert(ctx.client(), params)
    }

    fn examples() -> Value {
        json!([{
            "name": "part of a file at the edit cursor",
            "params": [
                { "name": "files", "value": [
                    { "path": "/Sounds/door-slam.wav", "sourceStart": 1.25, "length": 0.8 },
                ] },
            ],
            "result": {
                "name": "result",
                "value": { "items": [{
                    "guid": "{6D1A4C1E-8E0B-4D2A-9B0F-2A3C4D5E6F70}",
                    "name": "door-slam.wav",
                    "path": "/Sounds/door-slam.wav",
                    "track": "FX",
                    "trackGuid": "{0E1F2A3B-4C5D-6E7F-8091-A2B3C4D5E6F7}",
                    "position": 12.5,
                    "length": 0.8,
                    "takeCount": 1,
                    "takeGuid": "{9A8B7C6D-5E4F-4031-8293-A4B5C6D7E8F9}",
                    "startOffset": 1.25,
                    "playRate": 1.0,
                    "loopSource": true,
                }] },
            },
        }])
    }
}

struct GetDebugStats;
impl Method for GetDebugStats {
    const NAME: &'static str = "debug.stats";
    const SUMMARY: &'static str = "Main-thread time spent on events and calls";
    const DESCRIPTION: &'static str = "Reports how much of REAPER's main thread ReaSock is \
        using: per timer tick and per event for notifications, and per method for calls.";
    type Params = DebugStatsParams;
    type Result = DebugStats;

    fn call(_: &Ctx, params: DebugStatsParams) -> Result<DebugStats, RpcError> {
        Ok(bridge::debug_stats(params))
    }
}

// ---------------------------------------------------------------------------
// Notifications

struct ActionTriggeredNotification;
impl Notification for ActionTriggeredNotification {
    const NAME: &'static str = "action.triggered";
    const SUMMARY: &'static str = "One of this connection's actions ran";
    const DESCRIPTION: &'static str = "Sent to the connection that registered the action, \
        whether it ran from a key, a menu, the action list, or `action.run`.";
    type Params = ActionTriggered;
}

struct TransportChanged;
impl Notification for TransportChanged {
    const NAME: &'static str = "transport.changed";
    const SUMMARY: &'static str = "The play state, edit cursor, or time selection changed";
    const DESCRIPTION: &'static str = "Doesn't fire for the play position moving during \
        playback. Subscribe with `events.subscribe`.";
    type Params = Transport;
}

struct ProjectChanged;
impl Notification for ProjectChanged {
    const NAME: &'static str = "project.changed";
    const SUMMARY: &'static str =
        "A different project is current, or the current one was saved or became dirty";
    const DESCRIPTION: &'static str = "Fires on tab switches, opening, saving, save-as, and the \
        first edit after a save. Subscribe with `events.subscribe`.";
    type Params = Project;
}

struct ProjectEdited;
impl Notification for ProjectEdited {
    const NAME: &'static str = "project.edited";
    const SUMMARY: &'static str = "An undoable change happened in the current project";
    const DESCRIPTION: &'static str = "Subscribe with `events.subscribe`.";
    type Params = ProjectEdit;
}

struct ItemsSelectionChanged;
impl Notification for ItemsSelectionChanged {
    const NAME: &'static str = "items.selectionChanged";
    const SUMMARY: &'static str = "The item selection changed, or a selected item changed";
    const DESCRIPTION: &'static str = "Fires when items are selected or deselected, and when a \
        selected item is moved, trimmed, slip-edited, or renamed (including live, while \
        dragging). The first \
        notification after subscribing lists every selected item as added. When `truncated` is \
        true, catch up with `items.listSelected`. Subscribe with `events.subscribe`.";
    type Params = ItemsChange;
}

struct TracksSelectionChanged;
impl Notification for TracksSelectionChanged {
    const NAME: &'static str = "tracks.selectionChanged";
    const SUMMARY: &'static str = "The track selection changed, or a selected track changed";
    const DESCRIPTION: &'static str = "Works like `items.selectionChanged`; catch up with \
        `tracks.listSelected`. The master track is never included. Subscribe with \
        `events.subscribe`.";
    type Params = TracksChange;
}

// ---------------------------------------------------------------------------
// Registry and dispatch

type Generators = (SchemaGenerator, SchemaGenerator);

/// A registered method, with its types erased.
pub struct Entry {
    pub name: &'static str,
    open: bool,
    call: fn(&Ctx, Value) -> Result<Value, RpcError>,
    describe: fn(&mut Generators) -> Value,
}

fn entry<M: Method>() -> Entry {
    Entry {
        name: M::NAME,
        open: M::OPEN,
        call: call_method::<M>,
        describe: describe_method::<M>,
    }
}

fn call_method<M: Method>(ctx: &Ctx, params: Value) -> Result<Value, RpcError> {
    let params: M::Params =
        serde_json::from_value(params).map_err(|error| RpcError::invalid_params(error.to_string()))?;
    let result = M::call(ctx, params)?;
    serde_json::to_value(result).map_err(|error| RpcError::internal(error.to_string()))
}

/// Every method, in the order they're documented.
pub fn methods() -> &'static [Entry] {
    static METHODS: OnceLock<Vec<Entry>> = OnceLock::new();
    METHODS.get_or_init(|| {
        vec![
            entry::<Initialize>(),
            entry::<Discover>(),
            entry::<RegisterAction>(),
            entry::<UnregisterAction>(),
            entry::<RunAction>(),
            entry::<Subscribe>(),
            entry::<Unsubscribe>(),
            entry::<GetProject>(),
            entry::<GetTransport>(),
            entry::<ListItems>(),
            entry::<ListSelectedItems>(),
            entry::<GetItems>(),
            entry::<InsertItems>(),
            entry::<SetItems>(),
            entry::<SetTakes>(),
            entry::<ListTakes>(),
            entry::<ListTracks>(),
            entry::<ListSelectedTracks>(),
            entry::<GetTracks>(),
            entry::<ListMarkers>(),
            entry::<GetDebugStats>(),
        ]
    })
}

/// Every notification, with its name, in the order they're documented.
fn notifications() -> Vec<(&'static str, fn(&mut Generators) -> Value)> {
    fn of<N: Notification>() -> (&'static str, fn(&mut Generators) -> Value) {
        (N::NAME, describe_notification::<N>)
    }
    vec![
        of::<ActionTriggeredNotification>(),
        of::<TransportChanged>(),
        of::<ProjectChanged>(),
        of::<ProjectEdited>(),
        of::<ItemsSelectionChanged>(),
        of::<TracksSelectionChanged>(),
    ]
}

/// Runs a client's call.
pub fn call(conn: ConnId, method: &str, params: Params) -> Result<Value, RpcError> {
    let entry = methods()
        .iter()
        .find(|entry| entry.name == method)
        .ok_or_else(|| RpcError::method_not_found(method))?;
    let client = bridge::client(conn);
    if !entry.open && client.is_none() {
        return Err(RpcError::not_initialized());
    }
    (entry.call)(&Ctx { conn, client }, params.into_value())
}

// ---------------------------------------------------------------------------
// OpenRPC

fn generator(contract: Contract) -> SchemaGenerator {
    SchemaSettings::draft07()
        .with(|settings| {
            settings.definitions_path = "/components/schemas".into();
            settings.contract = contract;
        })
        .into_generator()
}

/// OpenRPC uses JSON Schema draft 7.
fn draft07(schema: &mut Schema) {
    ReplaceUnevaluatedProperties::default().transform(schema);
    RemoveRefSiblings::default().transform(schema);
    ReplacePrefixItems::default().transform(schema);
}

/// OpenRPC content descriptors for each field of a params struct.
fn content_descriptors<T: JsonSchema>(generator: &mut SchemaGenerator) -> Value {
    let schema = T::json_schema(generator);
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let descriptors: Vec<Value> = properties
        .into_iter()
        .map(|(name, property)| {
            let mut property = Schema::try_from(property).expect("property schemas are schemas");
            let description = property.remove("description");
            draft07(&mut property);
            let mut descriptor = Map::new();
            descriptor.insert("name".into(), name.clone().into());
            if let Some(description) = description {
                descriptor.insert("description".into(), description);
            }
            if required.contains(&name.as_str()) {
                descriptor.insert("required".into(), true.into());
            }
            descriptor.insert("schema".into(), property.to_value());
            Value::Object(descriptor)
        })
        .collect();
    descriptors.into()
}

fn result_descriptor<T: JsonSchema>(generator: &mut SchemaGenerator) -> Value {
    let mut schema = generator.subschema_for::<T>();
    draft07(&mut schema);
    json!({ "name": "result", "schema": schema })
}

fn error_refs(errors: impl IntoIterator<Item = &'static AppError>) -> Value {
    errors
        .into_iter()
        .map(|error| json!({ "$ref": format!("#/components/errors/{}", error.name) }))
        .collect::<Vec<_>>()
        .into()
}

fn describe_method<M: Method>((params, results): &mut Generators) -> Value {
    let mut method = Map::new();
    method.insert("name".into(), M::NAME.into());
    method.insert("summary".into(), M::SUMMARY.into());
    if !M::DESCRIPTION.is_empty() {
        method.insert("description".into(), M::DESCRIPTION.into());
    }
    method.insert("paramStructure".into(), "by-name".into());
    method.insert("params".into(), content_descriptors::<M::Params>(params));
    method.insert("result".into(), result_descriptor::<M::Result>(results));
    let gate = (!M::OPEN).then_some(&NOT_INITIALIZED);
    let errors: Vec<&'static AppError> = gate.into_iter().chain(M::ERRORS.iter().copied()).collect();
    if !errors.is_empty() {
        method.insert("errors".into(), error_refs(errors));
    }
    let examples = M::examples();
    if examples.as_array().is_some_and(|examples| !examples.is_empty()) {
        method.insert("examples".into(), examples);
    }
    Value::Object(method)
}

fn describe_notification<N: Notification>((_, results): &mut Generators) -> Value {
    let mut params_type = results.subschema_for::<N::Params>();
    draft07(&mut params_type);
    json!({
        "name": N::NAME,
        "summary": N::SUMMARY,
        "description": N::DESCRIPTION,
        "paramStructure": "by-name",
        "params": content_descriptors::<N::Params>(results),
        // The named type of the whole params object, for typed clients.
        "x-params": params_type,
    })
}

/// The ReaScript functions, for the API reference.
fn reascript() -> Value {
    crate::lua::FUNCTIONS
        .iter()
        .map(|function| {
            let params: Vec<Value> = function
                .params
                .iter()
                .map(|(kind, name)| json!({ "name": name, "type": kind }))
                .collect();
            json!({
                "name": function.name,
                "returns": function.returns,
                "params": params,
                "description": function.help,
            })
        })
        .collect::<Vec<_>>()
        .into()
}

/// The OpenRPC document for the protocol.
pub fn document() -> Value {
    // Params describe what clients send; results and notifications describe
    // what the server sends. That matters for optional fields.
    let mut generators = (generator(Contract::Deserialize), generator(Contract::Serialize));
    let methods: Vec<Value> = methods()
        .iter()
        .map(|entry| (entry.describe)(&mut generators))
        .collect();
    let notifications: Vec<Value> = notifications()
        .iter()
        .map(|(_, describe)| describe(&mut generators))
        .collect();

    let mut schemas = generators.0.take_definitions(true);
    for (name, schema) in generators.1.take_definitions(true) {
        if let Some(existing) = schemas.get(&name) {
            assert_eq!(
                existing, &schema,
                "{name} is described differently as a param and as a result"
            );
        }
        schemas.insert(name, schema);
    }
    schemas.sort_keys();
    let errors: Map<String, Value> = APP_ERRORS
        .iter()
        .map(|error| {
            let value = json!({ "code": error.code, "message": error.message });
            (error.name.to_owned(), value)
        })
        .collect();

    json!({
        "openrpc": "1.4.1",
        "info": {
            "title": "ReaSock",
            "version": protocol::PROTOCOL_VERSION,
            "description": "JSON-RPC 2.0 over WebSocket between REAPER and outside programs. \
                Each running REAPER announces itself in `~/.reasock/instances/<pid>.json`. \
                After connecting, a client must call `initialize`, with the `token` from that \
                file, before anything else except `rpc.discover`. Params are always by name. Messages the server sends to clients \
                on its own (registered actions running, and subscribed events) are listed under \
                `x-notifications`. Scripts inside REAPER use the same protocol through the \
                ReaScript functions listed under `x-reascript`.",
            "license": { "name": "MIT" },
        },
        "servers": [{
            "name": "local",
            "url": "ws://127.0.0.1:{port}",
            "summary": "A REAPER instance on this machine",
            "variables": {
                "port": {
                    "default": "9670",
                    "description": "Read the actual port from the instance file.",
                },
            },
        }],
        "methods": methods,
        "x-notifications": notifications,
        "x-reascript": reascript(),
        "components": { "schemas": schemas, "errors": errors },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openrpc_json_is_current() {
        let generated = serde_json::to_string_pretty(&document()).unwrap() + "\n";
        if std::env::var_os("REASOCK_UPDATE_OPENRPC").is_some() {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/openrpc.json");
            std::fs::write(path, &generated).unwrap();
            return;
        }
        // Tolerate a CRLF checkout; .gitattributes should prevent one.
        let published = include_str!("../openrpc.json").replace("\r\n", "\n");
        assert!(
            generated == published,
            "openrpc.json is out of date; run REASOCK_UPDATE_OPENRPC=1 cargo test"
        );
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = methods().iter().map(|entry| entry.name).collect();
        names.extend(notifications().iter().map(|(name, _)| *name));
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn every_event_has_a_notification() {
        let names: Vec<&str> = notifications().iter().map(|(name, _)| *name).collect();
        for event in EventName::ALL {
            assert!(names.contains(&event.as_str()), "{} has no notification", event.as_str());
        }
    }

    #[test]
    fn every_app_error_is_used() {
        let document = document();
        let text = document["methods"].to_string();
        for error in APP_ERRORS {
            assert!(
                text.contains(&format!("#/components/errors/{}", error.name)),
                "{} is never returned",
                error.name
            );
        }
    }
}
