//! Change tracking for collections such as the selected items or tracks.
//!
//! Notifications about a collection stay bounded however large it gets:
//! they carry the count, the GUIDs that changed, and only as many full
//! entries as the subscriber's `limit` allows. Anything left out is flagged
//! with `truncated`, and the client catches up through Ask.

use crate::types::{CollectionOptions, Include, Keyed};
use std::collections::{HashMap, HashSet};

pub const DEFAULT_LIMIT: usize = 256;
pub const MAX_LIMIT: usize = 10_000;

/// Per-subscription options for collection events, validated.
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

impl TryFrom<&CollectionOptions> for Options {
    type Error = String;

    fn try_from(options: &CollectionOptions) -> Result<Self, String> {
        let limit = options.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(format!("limit must be between 1 and {MAX_LIMIT}"));
        }
        Ok(Self {
            include: options.include,
            limit,
        })
    }
}

/// Entries in order, indexed by key.
#[derive(Debug)]
pub struct Collection<T> {
    entries: Vec<T>,
    index: HashMap<String, usize>,
}

impl<T: Keyed + Clone + PartialEq> Collection<T> {
    pub fn new(entries: Vec<T>) -> Self {
        let index = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.key().to_owned(), position))
            .collect();
        Self { entries, index }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    fn get(&self, key: &str) -> Option<&T> {
        self.index.get(key).map(|&position| &self.entries[position])
    }

    /// What changed from `self` to `newer`.
    pub fn diff(&self, newer: &Collection<T>) -> Delta {
        let mut delta = Delta::default();
        for entry in &newer.entries {
            match self.get(entry.key()) {
                None => delta.added.push(entry.key().to_owned()),
                Some(old) if old != entry => delta.changed.push(entry.key().to_owned()),
                Some(_) => {}
            }
        }
        for entry in &self.entries {
            if !newer.index.contains_key(entry.key()) {
                delta.removed.push(entry.key().to_owned());
            }
        }
        delta
    }

    /// Builds one subscriber's view of a change. With no `delta` this is the
    /// subscriber's first notification, so everything counts as added.
    pub fn render(&self, delta: Option<&Delta>, options: Options) -> Change<T> {
        let initial;
        let delta = match delta {
            Some(delta) => delta,
            None => {
                initial = Delta {
                    added: self.entries.iter().map(|entry| entry.key().to_owned()).collect(),
                    ..Delta::default()
                };
                &initial
            }
        };
        let lists_fit = delta.len() <= options.limit;
        let (entries, truncated) = match options.include {
            Include::None => (None, !lists_fit),
            Include::Changed if lists_fit => {
                let wanted: HashSet<&str> =
                    delta.added.iter().chain(&delta.changed).map(String::as_str).collect();
                let entries = self
                    .entries
                    .iter()
                    .filter(|entry| wanted.contains(entry.key()))
                    .cloned()
                    .collect();
                (Some(entries), false)
            }
            Include::Changed => (None, true),
            // Subscribers holding the whole collection don't need the lists.
            Include::All if self.len() <= options.limit => (Some(self.entries.clone()), false),
            Include::All => (None, true),
        };
        Change {
            count: self.len(),
            delta: lists_fit.then(|| delta.clone()),
            entries,
            truncated,
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq)]
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

/// One subscriber's view of a change, before it's given its wire shape
/// (`types::ItemsChange`, `types::TracksChange`).
#[derive(Debug, PartialEq)]
pub struct Change<T> {
    pub count: usize,
    /// The change lists, unless they exceed the limit.
    pub delta: Option<Delta>,
    pub entries: Option<Vec<T>>,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Clone, Debug, PartialEq)]
    struct Entry {
        guid: String,
        position: f64,
    }

    impl Keyed for Entry {
        fn key(&self) -> &str {
            &self.guid
        }
    }

    fn entry(guid: &str, position: f64) -> Entry {
        Entry {
            guid: guid.into(),
            position,
        }
    }

    fn collection(entries: &[(&str, f64)]) -> Collection<Entry> {
        Collection::new(entries.iter().map(|&(guid, position)| entry(guid, position)).collect())
    }

    fn numbered(count: usize, position: f64) -> Collection<Entry> {
        Collection::new((0..count).map(|i| entry(&i.to_string(), position)).collect())
    }

    fn options(include: Include, limit: usize) -> Options {
        Options { include, limit }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn validates_options() {
        let parse = |value| {
            serde_json::from_value::<CollectionOptions>(value)
                .map_err(|error| error.to_string())
                .and_then(|options| Options::try_from(&options))
        };
        assert_eq!(parse(json!({})), Ok(Options::default()));
        assert_eq!(parse(json!({ "include": "all", "limit": 10 })), Ok(options(Include::All, 10)));
        assert!(parse(json!({ "include": "some" })).is_err());
        assert!(parse(json!({ "limit": 0 })).is_err());
        assert!(parse(json!({ "limit": 1_000_000 })).is_err());
        assert!(parse(json!({ "colour": "red" })).is_err());
        assert!(parse(json!("all")).is_err());
    }

    #[test]
    fn diff_finds_added_removed_and_changed() {
        let old = collection(&[("a", 0.0), ("b", 1.0), ("c", 2.0)]);
        let new = collection(&[("b", 1.0), ("c", 5.0), ("d", 3.0)]);
        assert_eq!(
            old.diff(&new),
            Delta {
                added: strings(&["d"]),
                removed: strings(&["a"]),
                changed: strings(&["c"]),
            }
        );
        assert!(new.diff(&new).is_empty());
    }

    #[test]
    fn first_notification_lists_everything_as_added() {
        let current = collection(&[("a", 0.0), ("b", 1.0)]);
        assert_eq!(
            current.render(None, Options::default()),
            Change {
                count: 2,
                delta: Some(Delta {
                    added: strings(&["a", "b"]),
                    ..Delta::default()
                }),
                entries: Some(vec![entry("a", 0.0), entry("b", 1.0)]),
                truncated: false,
            }
        );
    }

    #[test]
    fn changed_includes_only_added_and_changed_entries() {
        let old = collection(&[("a", 0.0), ("b", 1.0), ("c", 2.0)]);
        let new = collection(&[("b", 1.0), ("c", 5.0), ("d", 3.0)]);
        let change = new.render(Some(&old.diff(&new)), Options::default());
        assert_eq!(change.entries, Some(vec![entry("c", 5.0), entry("d", 3.0)]));
        assert_eq!(change.delta.unwrap().removed, strings(&["a"]));
        assert!(!change.truncated);
    }

    #[test]
    fn large_changes_are_truncated() {
        let old = numbered(10, 0.0);
        let new = numbered(10, 1.0);
        let delta = old.diff(&new);
        for include in [Include::None, Include::Changed] {
            let change = new.render(Some(&delta), options(include, 5));
            assert_eq!(
                change,
                Change {
                    count: 10,
                    delta: None,
                    entries: None,
                    truncated: true,
                }
            );
        }
    }

    #[test]
    fn all_sends_the_whole_collection_when_it_fits() {
        let old = numbered(4, 0.0);
        let new = numbered(4, 1.0);
        let delta = old.diff(&new);
        let change = new.render(Some(&delta), options(Include::All, 4));
        assert_eq!(change.entries.map(|entries| entries.len()), Some(4));
        assert!(!change.truncated);

        let change = new.render(Some(&delta), options(Include::All, 3));
        assert_eq!(change.entries, None);
        assert!(change.truncated);
    }

    #[test]
    fn none_never_includes_entries() {
        let current = collection(&[("a", 0.0)]);
        let change = current.render(None, options(Include::None, 10));
        assert_eq!(change.entries, None);
        assert_eq!(change.delta.unwrap().added, strings(&["a"]));
    }
}
