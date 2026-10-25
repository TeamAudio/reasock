//! Tell: state-change notifications for subscribed clients.
//!
//! Each timer tick, every event type with at least one subscriber is
//! snapshotted and compared with the previous snapshot. Changes go to all of
//! its subscribers; new subscribers get the current snapshot once. Polling
//! covers state that REAPER has no callback for (item selection, project
//! state) and coalesces bursts to at most one notification per tick.

use crate::model;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Every event a client can subscribe to; also the notification method names.
pub const EVENTS: &[&str] = &[
    "transport.changed",
    "project.changed",
    "project.edited",
    "items.selectionChanged",
    "tracks.selectionChanged",
];

pub fn lookup(name: &str) -> Option<&'static str> {
    EVENTS.iter().copied().find(|event| *event == name)
}

fn snapshot(event: &str) -> Value {
    match event {
        "transport.changed" => model::transport(),
        "project.changed" => model::project(),
        "project.edited" => model::project_edit(),
        "items.selectionChanged" => json!({ "selectedItems": model::selected_items() }),
        "tracks.selectionChanged" => json!({ "selectedTracks": model::selected_tracks() }),
        other => unreachable!("unknown event {other}"),
    }
}

/// The last snapshot of each event type that currently has subscribers.
#[derive(Default)]
pub struct Tracker {
    last: HashMap<&'static str, Value>,
    pub stats: Stats,
}

/// Timing for `debug.stats`: how much main-thread time Tell costs.
#[derive(Default)]
pub struct Stats {
    since: Option<Instant>,
    ticks: Timing,
    events: HashMap<&'static str, EventStats>,
}

#[derive(Default)]
struct Timing {
    count: u64,
    total: Duration,
    max: Duration,
}

impl Timing {
    fn record(&mut self, elapsed: Duration) {
        self.count += 1;
        self.total += elapsed;
        self.max = self.max.max(elapsed);
    }

    fn to_json(&self) -> Value {
        let average = self.total.checked_div(self.count as u32).unwrap_or_default();
        json!({
            "count": self.count,
            "avgUs": average.as_micros() as u64,
            "maxUs": self.max.as_micros() as u64,
        })
    }
}

#[derive(Default)]
struct EventStats {
    snapshot: Timing,
    compare: Timing,
    sent: u64,
    last_bytes: usize,
}

impl Stats {
    /// Time spent on events in one timer tick, including serializing and
    /// queueing notifications.
    pub fn record_tick(&mut self, elapsed: Duration) {
        self.since.get_or_insert_with(Instant::now);
        self.ticks.record(elapsed);
    }

    pub fn record_send(&mut self, event: &'static str, bytes: usize) {
        let stats = self.events.entry(event).or_default();
        stats.sent += 1;
        stats.last_bytes = bytes;
    }

    pub fn to_json(&self) -> Value {
        let events: serde_json::Map<String, Value> = self
            .events
            .iter()
            .map(|(event, stats)| {
                let value = json!({
                    "snapshot": stats.snapshot.to_json(),
                    "compare": stats.compare.to_json(),
                    "sent": stats.sent,
                    "lastBytes": stats.last_bytes,
                });
                (event.to_string(), value)
            })
            .collect();
        json!({
            "seconds": self.since.map(|since| since.elapsed().as_secs_f64()).unwrap_or(0.0),
            "ticks": self.ticks.to_json(),
            "events": events,
        })
    }
}

pub struct Snapshot {
    pub params: Value,
    pub changed: bool,
}

impl Tracker {
    /// Snapshots the given events, reporting which changed since last time.
    /// Forgets events that are no longer wanted, so a later subscriber starts
    /// fresh.
    pub fn poll(&mut self, wanted: &[&'static str]) -> HashMap<&'static str, Snapshot> {
        self.last.retain(|event, _| wanted.contains(event));
        wanted
            .iter()
            .map(|&event| {
                let started = Instant::now();
                let params = snapshot(event);
                let snapshotted = Instant::now();
                let changed = self.last.get(event) != Some(&params);
                let stats = self.stats.events.entry(event).or_default();
                stats.snapshot.record(snapshotted - started);
                stats.compare.record(snapshotted.elapsed());
                if changed {
                    self.last.insert(event, params.clone());
                }
                (event, Snapshot { params, changed })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_is_documented() {
        let document: Value = serde_json::from_str(crate::protocol::OPENRPC).unwrap();
        let documented: Vec<&str> = document["x-notifications"]
            .as_array()
            .unwrap()
            .iter()
            .map(|notification| notification["name"].as_str().unwrap())
            .collect();
        for event in EVENTS {
            assert!(documented.contains(event), "{event} missing from x-notifications");
        }
        let names = document["components"]["schemas"]["EventName"]["enum"].clone();
        assert_eq!(names, json!(EVENTS));
    }
}
