//! The protocol's data types.
//!
//! ReaSock works with these structs internally and turns them into JSON only
//! when a message is sent. Their doc comments are the field descriptions in
//! the published schema.

use schemars::JsonSchema;
use serde::Serialize;

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
    /// Name of the item's track.
    pub track: Option<String>,
    pub track_guid: Option<String>,
    /// Seconds from the project start.
    pub position: f64,
    /// Seconds.
    pub length: f64,
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
    pub name: Option<String>,
    /// 1-based position, as shown in REAPER.
    pub number: i64,
}

impl Keyed for Track {
    fn key(&self) -> &str {
        &self.guid
    }
}

/// Params of `transport.changed`.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Transport {
    pub playing: bool,
    pub paused: bool,
    pub recording: bool,
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

/// Result of `items.listSelected`.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ItemsPage {
    /// Total selected items.
    pub count: usize,
    pub offset: usize,
    pub items: Vec<Item>,
}

/// Result of `tracks.listSelected`.
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
                    "trackGuid": "{T}", "position": 1.5, "length": 2.0,
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
