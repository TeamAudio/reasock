//! Change tracking for collections such as the selected items or tracks.
//!
//! Notifications about a collection stay bounded however large it gets:
//! they carry the count, the GUIDs that changed, and only as many full
//! entries as the subscriber's `limit` allows. Anything left out is flagged
//! with `truncated`, and the client catches up through Ask.

use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

pub const DEFAULT_LIMIT: usize = 256;
pub const MAX_LIMIT: usize = 10_000;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Include {
    None,
    Changed,
    All,
}

/// Per-subscription options for collection events.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Options {
    pub include: Include,
    pub limit: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            include: Include::Changed,
            limit: DEFAULT_LIMIT,
        }
    }
}

impl Options {
    pub fn parse(value: Option<&Value>) -> Result<Self, String> {
        let mut options = Self::default();
        let Some(value) = value else {
            return Ok(options);
        };
        let object = value.as_object().ok_or("options must be an object")?;
        for (key, value) in object {
            match key.as_str() {
                "include" => {
                    options.include = match value.as_str() {
                        Some("none") => Include::None,
                        Some("changed") => Include::Changed,
                        Some("all") => Include::All,
                        _ => return Err("include must be \"none\", \"changed\", or \"all\"".into()),
                    }
                }
                "limit" => {
                    options.limit = value
                        .as_u64()
                        .map(|limit| limit as usize)
                        .filter(|limit| (1..=MAX_LIMIT).contains(limit))
                        .ok_or_else(|| format!("limit must be between 1 and {MAX_LIMIT}"))?;
                }
                other => return Err(format!("unknown option: {other}")),
            }
        }
        Ok(options)
    }
}

/// Entries in order, keyed by their `guid` field.
#[derive(Default, Debug)]
pub struct Collection {
    order: Vec<String>,
    by_key: HashMap<String, Value>,
}

impl Collection {
    pub fn new(entries: Vec<Value>) -> Self {
        let mut collection = Self::default();
        for (index, entry) in entries.into_iter().enumerate() {
            let key = entry["guid"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("#{index}"));
            collection.order.push(key.clone());
            collection.by_key.insert(key, entry);
        }
        collection
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// What changed from `self` to `newer`.
    pub fn diff(&self, newer: &Collection) -> Delta {
        Delta {
            added: newer
                .order
                .iter()
                .filter(|key| !self.by_key.contains_key(*key))
                .cloned()
                .collect(),
            removed: self
                .order
                .iter()
                .filter(|key| !newer.by_key.contains_key(*key))
                .cloned()
                .collect(),
            changed: newer
                .order
                .iter()
                .filter(|key| matches!(self.by_key.get(*key), Some(old) if *old != newer.by_key[*key]))
                .cloned()
                .collect(),
        }
    }

    fn entries<'a>(&'a self, keys: impl Iterator<Item = &'a String>) -> Vec<Value> {
        keys.filter_map(|key| self.by_key.get(key).cloned()).collect()
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct Delta {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        self.added.len() + self.removed.len() + self.changed.len()
    }
}

/// Builds notification params for one subscriber. With no `delta` this is a
/// subscriber's first notification, so everything counts as added.
pub fn render(field: &str, current: &Collection, delta: Option<&Delta>, options: Options) -> Value {
    let initial;
    let delta = match delta {
        Some(delta) => delta,
        None => {
            initial = Delta {
                added: current.order.clone(),
                ..Delta::default()
            };
            &initial
        }
    };
    let mut params = Map::new();
    params.insert("count".into(), current.len().into());

    let lists_fit = delta.len() <= options.limit;
    if lists_fit {
        params.insert("added".into(), delta.added.clone().into());
        params.insert("removed".into(), delta.removed.clone().into());
        params.insert("changed".into(), delta.changed.clone().into());
    }

    let truncated = match options.include {
        Include::None => !lists_fit,
        Include::Changed => {
            if lists_fit {
                let wanted: HashSet<&String> = delta.added.iter().chain(&delta.changed).collect();
                let entries = current.entries(current.order.iter().filter(|key| wanted.contains(key)));
                params.insert(field.into(), entries.into());
            }
            !lists_fit
        }
        // Subscribers holding the whole collection don't need the lists.
        Include::All => {
            let fits = current.len() <= options.limit;
            if fits {
                params.insert(field.into(), current.entries(current.order.iter()).into());
            }
            !fits
        }
    };
    params.insert("truncated".into(), truncated.into());
    Value::Object(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(guid: &str, position: f64) -> Value {
        json!({ "guid": guid, "position": position })
    }

    fn options(include: Include, limit: usize) -> Options {
        Options { include, limit }
    }

    #[test]
    fn parses_options() {
        assert_eq!(Options::parse(None), Ok(Options::default()));
        let parsed = Options::parse(Some(&json!({ "include": "all", "limit": 10 })));
        assert_eq!(parsed, Ok(options(Include::All, 10)));
        assert!(Options::parse(Some(&json!({ "include": "some" }))).is_err());
        assert!(Options::parse(Some(&json!({ "limit": 0 }))).is_err());
        assert!(Options::parse(Some(&json!({ "limit": 1_000_000 }))).is_err());
        assert!(Options::parse(Some(&json!({ "colour": "red" }))).is_err());
        assert!(Options::parse(Some(&json!("all"))).is_err());
    }

    #[test]
    fn diff_finds_added_removed_and_changed() {
        let old = Collection::new(vec![item("a", 0.0), item("b", 1.0), item("c", 2.0)]);
        let new = Collection::new(vec![item("b", 1.0), item("c", 5.0), item("d", 3.0)]);
        assert_eq!(
            old.diff(&new),
            Delta {
                added: vec!["d".into()],
                removed: vec!["a".into()],
                changed: vec!["c".into()],
            }
        );
        assert!(new.diff(&new).is_empty());
    }

    #[test]
    fn first_notification_lists_everything_as_added() {
        let current = Collection::new(vec![item("a", 0.0), item("b", 1.0)]);
        let params = render("items", &current, None, Options::default());
        assert_eq!(
            params,
            json!({
                "count": 2,
                "added": ["a", "b"],
                "removed": [],
                "changed": [],
                "items": [item("a", 0.0), item("b", 1.0)],
                "truncated": false,
            })
        );
    }

    #[test]
    fn changed_includes_only_added_and_changed_entries() {
        let old = Collection::new(vec![item("a", 0.0), item("b", 1.0), item("c", 2.0)]);
        let new = Collection::new(vec![item("b", 1.0), item("c", 5.0), item("d", 3.0)]);
        let delta = old.diff(&new);
        let params = render("items", &new, Some(&delta), Options::default());
        assert_eq!(params["items"], json!([item("c", 5.0), item("d", 3.0)]));
        assert_eq!(params["removed"], json!(["a"]));
        assert_eq!(params["truncated"], false);
    }

    #[test]
    fn large_changes_are_truncated() {
        let old = Collection::new((0..10).map(|i| item(&i.to_string(), 0.0)).collect());
        let new = Collection::new((0..10).map(|i| item(&i.to_string(), 1.0)).collect());
        let delta = old.diff(&new);
        for include in [Include::None, Include::Changed] {
            let params = render("items", &new, Some(&delta), options(include, 5));
            assert_eq!(params, json!({ "count": 10, "truncated": true }));
        }
    }

    #[test]
    fn all_sends_the_whole_collection_when_it_fits() {
        let old = Collection::new((0..4).map(|i| item(&i.to_string(), 0.0)).collect());
        let new = Collection::new((0..4).map(|i| item(&i.to_string(), 1.0)).collect());
        let delta = old.diff(&new);
        let params = render("items", &new, Some(&delta), options(Include::All, 4));
        assert_eq!(params["items"].as_array().unwrap().len(), 4);
        assert_eq!(params["truncated"], false);

        let params = render("items", &new, Some(&delta), options(Include::All, 3));
        assert!(params.get("items").is_none());
        assert_eq!(params["truncated"], true);
    }

    #[test]
    fn none_never_includes_entries() {
        let current = Collection::new(vec![item("a", 0.0)]);
        let params = render("items", &current, None, options(Include::None, 10));
        assert!(params.get("items").is_none());
        assert_eq!(params["added"], json!(["a"]));
    }
}
