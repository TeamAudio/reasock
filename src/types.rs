//! The protocol's data types.
//!
//! ReaSock works with these structs internally and turns them into JSON only
//! when a message is sent. Their doc comments are the field descriptions in
//! the published schema.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// ASCII letters and digits; used inside REAPER command names.
pub const NAME_PATTERN: &str = "^[A-Za-z0-9]{1,32}$";

/// Something with a stable key, so a collection of them can be diffed.
pub trait Keyed {
    fn key(&self) -> &str;
}

/// A media item. `guid` is stable across sessions.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub guid: String,
    /// Name of the active take.
    pub name: Option<String>,
    /// Source file of the active take, or null for MIDI and empty items.
    pub path: Option<String>,
    /// Name of the item's track; empty when the track has no name.
    pub track: Option<String>,
    pub track_guid: Option<String>,
    /// Seconds from the project start.
    pub position: f64,
    /// Seconds.
    pub length: f64,
    /// How many takes the item has; see `takes.list`.
    pub take_count: usize,
    /// GUID of the active take, or null for an empty item.
    pub take_guid: Option<String>,
    /// Seconds into the active take's source where the item starts playing.
    pub start_offset: Option<f64>,
    /// The active take's playback rate; 1.0 is normal speed.
    pub play_rate: Option<f64>,
    /// Whether the item repeats its source when it's longer than the source.
    /// When it does, `startOffset + length * playRate` can run past the end
    /// of the file.
    pub loop_source: bool,
    /// The item's notes. Only present when asked for with `notes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Extension data for the keys asked for with `ext`; null where a key
    /// isn't set. Only present when asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<BTreeMap<String, Option<String>>>,
}

impl Keyed for Item {
    fn key(&self) -> &str {
        &self.guid
    }
}

/// A track. `guid` is stable across sessions.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Track {
    pub guid: String,
    /// Empty when the track has no name.
    pub name: Option<String>,
    /// 1-based position, as shown in REAPER.
    pub number: i64,
}

impl Keyed for Track {
    fn key(&self) -> &str {
        &self.guid
    }
}

/// A take of a media item. `guid` is stable across sessions.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Take {
    pub guid: String,
    pub name: Option<String>,
    /// Source file, or null for MIDI and empty takes.
    pub path: Option<String>,
    /// Whether this is the item's active take.
    pub active: bool,
    /// Seconds into the source where the take starts playing.
    pub start_offset: f64,
    /// 1.0 is normal speed.
    pub play_rate: f64,
    /// Extension data for the keys asked for with `ext`; null where a key
    /// isn't set. Only present when asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<BTreeMap<String, Option<String>>>,
}

/// A marker or region.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Marker {
    /// Stable across sessions once the project is saved (REAPER 7 assigns
    /// GUIDs to markers and regions when a project is loaded).
    pub guid: Option<String>,
    /// The number shown in REAPER. Markers and regions are numbered separately.
    pub number: u32,
    pub name: String,
    pub region: bool,
    /// Seconds from the project start.
    pub position: f64,
    /// Where a region ends, in seconds; null for markers.
    pub end: Option<f64>,
}

/// Params of `transport.changed`.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Transport {
    pub playing: bool,
    pub paused: bool,
    pub recording: bool,
    /// Seconds from the project start.
    pub edit_cursor: f64,
    /// Null when nothing is selected.
    pub time_selection: Option<TimeRange>,
}

/// A span of the timeline, in seconds from the project start.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TimeRange {
    pub start: f64,
    pub end: f64,
}

/// Params of `project.changed`.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Project {
    /// Index of the project's tab.
    pub tab: Option<usize>,
    /// File name without extension, or null until the project is saved.
    pub name: Option<String>,
    /// Path of the project file, or null until the project is saved.
    pub path: Option<String>,
    /// Whether there are unsaved changes.
    pub dirty: bool,
}

/// Params of `project.edited`.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEdit {
    /// Increases with every change to the project.
    pub state_change_count: u32,
    /// Description of the most recent undo point, for example "Move media items".
    pub last_undo: Option<String>,
}

/// Defines a selection-change notification for one kind of entry. The entries
/// field is named after the kind (`items`, `tracks`).
macro_rules! selection_change {
    ($name:ident, $entry:ty, $field:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
        pub struct $name {
            /// How many are selected.
            pub count: usize,
            /// GUIDs newly selected. Omitted, like `removed` and `changed`,
            /// when the change is larger than the subscription's `limit`.
            #[serde(skip_serializing_if = "Option::is_none")]
            pub added: Option<Vec<String>>,
            /// GUIDs no longer selected.
            #[serde(skip_serializing_if = "Option::is_none")]
            pub removed: Option<Vec<String>>,
            /// GUIDs still selected whose details changed.
            #[serde(skip_serializing_if = "Option::is_none")]
            pub changed: Option<Vec<String>>,
            /// Full entries, per the subscription's `include` option, in
            /// selection order.
            #[serde(skip_serializing_if = "Option::is_none")]
            pub $field: Option<Vec<$entry>>,
            /// Something this subscription asks for was left out because it
            /// exceeded `limit`. Page through the selection to catch up.
            pub truncated: bool,
        }

        impl From<crate::collection::Change<$entry>> for $name {
            fn from(change: crate::collection::Change<$entry>) -> Self {
                let (added, removed, changed) = match change.delta {
                    Some(delta) => (Some(delta.added), Some(delta.removed), Some(delta.changed)),
                    None => (None, None, None),
                };
                Self {
                    count: change.count,
                    added,
                    removed,
                    changed,
                    $field: change.entries,
                    truncated: change.truncated,
                }
            }
        }
    };
}

selection_change!(
    ItemsChange,
    Item,
    items,
    "Params of `items.selectionChanged`."
);
selection_change!(
    TracksChange,
    Track,
    tracks,
    "Params of `tracks.selectionChanged`."
);

/// Selected items when an action runs, capped like selection notifications.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActionContext {
    /// Up to the first 256 selected items.
    pub selected_items: Vec<Item>,
    pub selected_item_count: usize,
    /// More items are selected than `selectedItems` holds; use
    /// `items.listSelected` for the rest.
    pub truncated: bool,
}

/// Result of `items.listSelected` and `items.list`.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ItemsPage {
    /// Total selected items.
    pub count: usize,
    pub offset: usize,
    pub items: Vec<Item>,
}

/// Result of `tracks.listSelected` and `tracks.list`.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TracksPage {
    /// Total selected tracks.
    pub count: usize,
    pub offset: usize,
    pub tracks: Vec<Track>,
}

/// Result of `items.get`: each entry in the same position as its GUID, or
/// null if there is no such item.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ItemsFound {
    pub items: Vec<Option<Item>>,
}

/// Result of `tracks.get`.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TracksFound {
    pub tracks: Vec<Option<Track>>,
}

// ---------------------------------------------------------------------------
// Events

/// An event a client can subscribe to; also its notification method name.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum EventName {
    #[serde(rename = "transport.changed")]
    TransportChanged,
    #[serde(rename = "project.changed")]
    ProjectChanged,
    #[serde(rename = "project.edited")]
    ProjectEdited,
    #[serde(rename = "items.selectionChanged")]
    ItemsSelectionChanged,
    #[serde(rename = "tracks.selectionChanged")]
    TracksSelectionChanged,
}

impl EventName {
    pub const ALL: [EventName; 5] = [
        EventName::TransportChanged,
        EventName::ProjectChanged,
        EventName::ProjectEdited,
        EventName::ItemsSelectionChanged,
        EventName::TracksSelectionChanged,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EventName::TransportChanged => "transport.changed",
            EventName::ProjectChanged => "project.changed",
            EventName::ProjectEdited => "project.edited",
            EventName::ItemsSelectionChanged => "items.selectionChanged",
            EventName::TracksSelectionChanged => "tracks.selectionChanged",
        }
    }

    /// Collection events report changes to a set of entries and take options.
    pub fn is_collection(self) -> bool {
        matches!(
            self,
            EventName::ItemsSelectionChanged | EventName::TracksSelectionChanged
        )
    }
}

/// Which full entries a collection notification carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Include {
    /// GUIDs only.
    None,
    /// Entries that were added or changed.
    #[default]
    Changed,
    /// The whole collection.
    All,
}

/// Options for collection events (`items.selectionChanged`,
/// `tracks.selectionChanged`). Notifications never carry more than `limit`
/// GUIDs or entries; anything left out is flagged with `truncated`.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionOptions {
    /// Which full entries to send. Defaults to `changed`.
    #[serde(default)]
    pub include: Include,
    /// The most GUIDs or entries one notification may carry. Defaults to 256.
    #[schemars(range(min = 1, max = 10000))]
    pub limit: Option<usize>,
}

/// An event name, or an event with options.
#[derive(Clone, Debug, PartialEq, JsonSchema)]
#[serde(untagged)]
pub enum Subscription {
    Event(EventName),
    WithOptions {
        event: EventName,
        options: Option<CollectionOptions>,
    },
}

// Written by hand so a bad entry gets a specific error rather than serde's
// "did not match any variant".
impl<'de> Deserialize<'de> for Subscription {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WithOptions {
            event: EventName,
            options: Option<CollectionOptions>,
        }
        match Value::deserialize(deserializer)? {
            Value::String(name) => serde_json::from_value(Value::String(name))
                .map(Subscription::Event)
                .map_err(Error::custom),
            value @ Value::Object(_) => serde_json::from_value::<WithOptions>(value)
                .map(|with| Subscription::WithOptions {
                    event: with.event,
                    options: with.options,
                })
                .map_err(Error::custom),
            _ => Err(Error::custom("expected an event name or {event, options}")),
        }
    }
}

impl Subscription {
    pub fn event(&self) -> EventName {
        match self {
            Subscription::Event(event) | Subscription::WithOptions { event, .. } => *event,
        }
    }
}

/// Params of `action.triggered`.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ActionTriggered {
    /// The id the action was registered with.
    pub id: String,
    /// The action's named command, `_REASOCK_<client>_<id>`.
    pub command: String,
    pub context: ActionContext,
}

// ---------------------------------------------------------------------------
// Method params and results

/// Params of a method that takes none.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// A result with nothing to report.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Empty {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub client: ClientInfo,
    /// Required on WebSocket connections: the `token` from this REAPER's
    /// instance file (`~/.reasock/instances/<pid>.json`), which only the
    /// current user can read.
    pub token: Option<String>,
    /// Reserved for features the client opts into. Currently ignored.
    #[allow(dead_code)]
    pub capabilities: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClientInfo {
    /// Namespaces everything this connection registers.
    #[schemars(regex(pattern = NAME_PATTERN))]
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub server: ServerInfo,
    pub reaper: ReaperInfo,
    /// This connection's number, unique while REAPER runs.
    pub connection: u64,
    pub capabilities: Capabilities,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReaperInfo {
    /// For example "7.78/macOS-arm64".
    pub version: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Capabilities {
    pub actions: ActionCapabilities,
    pub events: EventCapabilities,
    /// Every method this server supports. Added in protocol 0.4.0: when it's
    /// missing, the server supports only the methods of protocol 0.3.0.
    pub methods: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ActionCapabilities {
    pub register: bool,
    pub run: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EventCapabilities {
    pub available: Vec<EventName>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterActionParams {
    /// Unique within this connection.
    #[schemars(regex(pattern = NAME_PATTERN))]
    pub id: String,
    /// Shown in the action list as `ReaSock (<client>): <label>`.
    #[schemars(length(min = 1))]
    pub label: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredAction {
    /// Named command, stable across sessions: `_REASOCK_<client>_<id>`.
    pub command: String,
    /// Numeric ID for this REAPER session only.
    pub command_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnregisterActionParams {
    #[schemars(regex(pattern = NAME_PATTERN))]
    pub id: String,
}

/// A command ID, such as 40182, or a named command, such as `_SWS_ABOUT`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum CommandRef {
    Id(#[schemars(range(min = 1))] u32),
    Name(#[schemars(length(min = 1))] String),
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunActionParams {
    pub command: CommandRef,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RanAction {
    pub command_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionParams {
    #[schemars(length(min = 1))]
    pub events: Vec<Subscription>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Subscriptions {
    /// All of this connection's subscriptions.
    pub events: Vec<EventName>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageParams {
    /// Defaults to 0.
    pub offset: Option<usize>,
    /// Defaults to 256.
    #[schemars(range(min = 1, max = 10000))]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemsListParams {
    /// Defaults to 0.
    pub offset: Option<usize>,
    /// Defaults to 256.
    #[schemars(range(min = 1, max = 10000))]
    pub limit: Option<usize>,
    /// GUID of a track, to list only its items.
    pub track: Option<String>,
    /// Include each item's notes.
    pub notes: Option<bool>,
    /// Include these extension data keys.
    #[schemars(length(max = 64))]
    pub ext: Option<Vec<ExtKey>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectedItemsParams {
    /// Defaults to 0.
    pub offset: Option<usize>,
    /// Defaults to 256.
    #[schemars(range(min = 1, max = 10000))]
    pub limit: Option<usize>,
    /// Include each item's notes.
    pub notes: Option<bool>,
    /// Include these extension data keys.
    #[schemars(length(max = 64))]
    pub ext: Option<Vec<ExtKey>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemsGetParams {
    /// Compared case-insensitively.
    #[schemars(length(min = 1, max = 10000))]
    pub guids: Vec<String>,
    /// Include each item's notes.
    pub notes: Option<bool>,
    /// Include these extension data keys.
    #[schemars(length(max = 64))]
    pub ext: Option<Vec<ExtKey>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TakesListParams {
    /// GUID of the item.
    pub item: String,
    /// Include these extension data keys.
    #[schemars(length(max = 64))]
    pub ext: Option<Vec<ExtKey>>,
}

/// An extension data key: up to 128 ASCII letters, digits, and `_ . : -`.
/// Start keys with your app's name, such as `myapp.id`, so they don't clash
/// with other programs'. REAPER saves the data in the project.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ExtKey(#[schemars(regex(pattern = EXT_KEY_PATTERN))] pub String);

pub const EXT_KEY_PATTERN: &str = "^[A-Za-z0-9_.:-]{1,128}$";

/// The largest extension data value, in bytes.
pub const MAX_EXT_VALUE: usize = 65536;

/// Changes to one item. Fields left out are left alone.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemChanges {
    pub guid: String,
    /// Replaces the item's notes.
    pub notes: Option<String>,
    /// Sets each key to its value (at most 64 KiB), or removes it when null.
    pub ext: Option<BTreeMap<ExtKey, Option<String>>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetItemsParams {
    #[schemars(length(min = 1, max = 10000))]
    pub items: Vec<ItemChanges>,
    /// The undo point's description. Defaults to "Set item properties
    /// (ReaSock: <client>)".
    #[schemars(length(min = 1))]
    pub undo: Option<String>,
}

/// Changes to one take. Fields left out are left alone.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TakeChanges {
    pub guid: String,
    /// Renames the take.
    pub name: Option<String>,
    /// Sets each key to its value (at most 64 KiB), or removes it when null.
    pub ext: Option<BTreeMap<ExtKey, Option<String>>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetTakesParams {
    #[schemars(length(min = 1, max = 10000))]
    pub takes: Vec<TakeChanges>,
    /// The undo point's description. Defaults to "Set take properties
    /// (ReaSock: <client>)".
    #[schemars(length(min = 1))]
    pub undo: Option<String>,
}

/// Result of `takes.list`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Takes {
    /// GUID of the item.
    pub item: String,
    /// In REAPER's order.
    pub takes: Vec<Take>,
}

/// Result of `markers.list`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MarkersPage {
    /// Total markers and regions.
    pub count: usize,
    pub offset: usize,
    /// In position order.
    pub markers: Vec<Marker>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GuidsParams {
    /// Compared case-insensitively.
    #[schemars(length(min = 1, max = 10000))]
    pub guids: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InsertParams {
    /// In the order they're placed.
    #[schemars(length(min = 1, max = 1000))]
    pub files: Vec<InsertFile>,
    /// Where the files start: seconds from the project start, or `cursor` for
    /// the edit cursor. Defaults to `cursor`.
    pub position: Option<InsertPosition>,
    /// Which track: a track's GUID; `current` for the first selected track,
    /// else the last touched one, else a new track (what REAPER's own insert
    /// uses); `selected` for the first selected track only; or `new`. New
    /// tracks go after the first selected track, else after the last touched
    /// one, else at the end. Defaults to `current`.
    pub track: Option<TrackTarget>,
    /// How several files are laid out. Defaults to `sequence`.
    pub arrange: Option<Arrange>,
    /// Seconds between files in a `sequence`. Defaults to 0.
    #[schemars(range(min = 0))]
    pub gap: Option<f64>,
    /// Name for tracks this call creates.
    pub track_name: Option<String>,
    /// The undo point's description. Defaults to "Insert media (ReaSock:
    /// <client>)".
    #[schemars(length(min = 1))]
    pub undo: Option<String>,
    /// Select the new items, and only them, as REAPER's own insert does.
    /// Defaults to true.
    pub select: Option<bool>,
    /// Move the edit cursor to the end of the last new item. Defaults to
    /// false.
    pub move_cursor: Option<bool>,
}

/// A media file to insert, or part of one.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InsertFile {
    /// Absolute path of an audio, video, or MIDI file.
    #[schemars(length(min = 1))]
    pub path: String,
    /// Seconds into the file where the item starts playing. Defaults to 0.
    /// Not supported for MIDI files.
    #[schemars(range(min = 0))]
    pub source_start: Option<f64>,
    /// Seconds of the file to use, more than 0. Defaults to the rest of the
    /// file after `sourceStart`. A range up to 1 ms past the end of the file
    /// is shortened to fit. Not supported for MIDI files.
    #[schemars(range(min = 0))]
    pub length: Option<f64>,
    /// The take's name. Defaults to the file's name.
    pub name: Option<String>,
    /// The new item's notes. With `arrange: takes`, only the first file may
    /// have notes or ext, since there's one new item.
    pub notes: Option<String>,
    /// Extension data for the new item (at most 64 KiB per value), saved as
    /// part of the insert's undo point.
    pub ext: Option<BTreeMap<ExtKey, String>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Arrange {
    /// One after another on one track.
    #[default]
    Sequence,
    /// Each file on its own new track (after the target track), all at the
    /// same position.
    Tracks,
    /// All files as takes of one item. The first file's take is active, and
    /// the item is as long as it.
    Takes,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum InsertPosition {
    Keyword(PositionKeyword),
    Seconds(#[schemars(range(min = 0))] f64),
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PositionKeyword {
    /// The edit cursor.
    Cursor,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum TrackTarget {
    Keyword(TrackKeyword),
    /// A track's GUID.
    Guid(#[schemars(length(min = 1))] String),
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TrackKeyword {
    /// The first selected track, else the last touched one, else a new track.
    Current,
    /// The first selected track only.
    Selected,
    /// A new track.
    New,
}

/// Result of `items.insert`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Inserted {
    /// The new items, in the order of `files`.
    pub items: Vec<Item>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugStatsParams {
    /// Start a new measurement window after reading.
    pub reset: Option<bool>,
}

/// How much of REAPER's main thread Tell is using. Times are in microseconds.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugStats {
    /// Length of the measurement window.
    pub seconds: f64,
    /// Per timer tick: snapshots, comparisons, and serializing notifications.
    pub ticks: Timing,
    pub events: BTreeMap<EventName, EventTiming>,
    /// Per method: calls, and their time on the main thread (not counting
    /// serializing the reply).
    pub methods: BTreeMap<String, Timing>,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    pub count: u64,
    pub avg_us: u64,
    pub max_us: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventTiming {
    /// The cheap change check, every tick.
    pub check: Timing,
    /// Full snapshots, only when the check shows a change.
    pub snapshot: Timing,
    pub compare: Timing,
    /// Notifications queued, counting each recipient.
    pub sent: u64,
    /// Size of the most recent notification.
    pub last_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::{Change, Delta};
    use serde_json::json;

    fn item() -> Item {
        Item {
            guid: "{A}".into(),
            name: Some("kick".into()),
            path: None,
            track: Some("Drums".into()),
            track_guid: Some("{T}".into()),
            position: 1.5,
            length: 2.0,
            take_count: 1,
            take_guid: Some("{K}".into()),
            start_offset: Some(0.25),
            play_rate: Some(1.0),
            loop_source: false,
            notes: None,
            ext: None,
        }
    }

    #[test]
    fn items_change_wire_shape() {
        let change: ItemsChange = Change {
            count: 1,
            delta: Some(Delta {
                added: vec!["{A}".into()],
                ..Delta::default()
            }),
            entries: Some(vec![item()]),
            truncated: false,
        }
        .into();
        assert_eq!(
            serde_json::to_value(&change).unwrap(),
            json!({
                "count": 1,
                "added": ["{A}"],
                "removed": [],
                "changed": [],
                "items": [{
                    "guid": "{A}", "name": "kick", "path": null, "track": "Drums",
                    "trackGuid": "{T}", "position": 1.5, "length": 2.0, "takeCount": 1,
                    "takeGuid": "{K}", "startOffset": 0.25, "playRate": 1.0,
                    "loopSource": false,
                }],
                "truncated": false,
            })
        );
    }

    #[test]
    fn truncated_change_omits_lists_and_entries() {
        let change: TracksChange = Change::<Track> {
            count: 9000,
            delta: None,
            entries: None,
            truncated: true,
        }
        .into();
        assert_eq!(
            serde_json::to_value(&change).unwrap(),
            json!({ "count": 9000, "truncated": true })
        );
    }
}
