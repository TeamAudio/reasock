//! Tell: state-change notifications for subscribed clients.
//!
//! Each timer tick, every event type with at least one subscriber is checked
//! for changes. Expensive events first compare a cheap fingerprint and only
//! build a full snapshot when it differs. Changes go to all of the event's
//! subscribers; new subscribers get the current snapshot once. Polling
//! covers state that REAPER has no callback for (item selection, project
//! state) and coalesces bursts to at most one notification per tick.
//!
//! Selections are collections: their notifications describe what changed and
//! stay within each subscriber's limit (see `collection`).

use crate::collection::{Collection, Delta, Options};
use crate::model;
use crate::protocol;
use crate::types::{
    DebugStats, EventName, EventTiming, Item, ItemsChange, Project, ProjectEdit, Timing, Track,
    TracksChange, Transport,
};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

/// Fills `out` with a cheap fingerprint of the event's state, if it has one.
fn fingerprint(event: EventName, out: &mut Vec<u64>) -> bool {
    match event {
        EventName::ItemsSelectionChanged => model::selected_items_fingerprint(out),
        EventName::TracksSelectionChanged => model::selected_tracks_fingerprint(out),
        _ => return false,
    }
    true
}

/// The current state behind one event.
enum State {
    Transport(Transport),
    Project(Project),
    ProjectEdit(ProjectEdit),
    Items(Collection<Item>),
    Tracks(Collection<Track>),
}

impl State {
    fn snapshot(event: EventName) -> State {
        match event {
            EventName::TransportChanged => State::Transport(model::transport()),
            EventName::ProjectChanged => State::Project(model::project()),
            EventName::ProjectEdited => State::ProjectEdit(model::project_edit()),
            EventName::ItemsSelectionChanged => {
                State::Items(Collection::new(model::selected_items()))
            }
            EventName::TracksSelectionChanged => {
                State::Tracks(Collection::new(model::selected_tracks()))
            }
        }
    }

    /// Whether `self` differs from `older`, and for collections, how.
    fn compare(&self, older: Option<&State>) -> (bool, Option<Delta>) {
        let collection_delta = |delta: Delta| (!delta.is_empty(), Some(delta));
        match (older, self) {
            (Some(State::Transport(old)), State::Transport(new)) => (old != new, None),
            (Some(State::Project(old)), State::Project(new)) => (old != new, None),
            (Some(State::ProjectEdit(old)), State::ProjectEdit(new)) => (old != new, None),
            (Some(State::Items(old)), State::Items(new)) => collection_delta(old.diff(new)),
            (Some(State::Tracks(old)), State::Tracks(new)) => collection_delta(old.diff(new)),
            _ => (true, None),
        }
    }

    /// The notification for one subscriber, serialized.
    fn message(&self, event: EventName, delta: Option<&Delta>, options: Options) -> String {
        let method = event.as_str();
        match self {
            State::Transport(params) => protocol::notification(method, params),
            State::Project(params) => protocol::notification(method, params),
            State::ProjectEdit(params) => protocol::notification(method, params),
            State::Items(items) => {
                let params: ItemsChange = items.render(delta, options).into();
                protocol::notification(method, &params)
            }
            State::Tracks(tracks) => {
                let params: TracksChange = tracks.render(delta, options).into();
                protocol::notification(method, &params)
            }
        }
    }
}

/// The last state of each event type that currently has subscribers.
#[derive(Default)]
pub struct Tracker {
    last: HashMap<EventName, Watched>,
    scratch: Vec<u64>,
    pub stats: Stats,
}

struct Watched {
    fingerprint: Option<Vec<u64>>,
    state: State,
    /// For collections: what changed in the most recent poll that changed it.
    delta: Option<Delta>,
}

impl Tracker {
    /// Checks the given events and returns those that changed since the last
    /// poll. Forgets events that are no longer wanted, so a later subscriber
    /// starts fresh.
    pub fn poll(&mut self, wanted: &[EventName]) -> Vec<EventName> {
        self.last.retain(|event, _| wanted.contains(event));
        let mut changed = Vec::new();
        for &event in wanted {
            let started = Instant::now();
            let has_fingerprint = fingerprint(event, &mut self.scratch);
            let checked = Instant::now();
            let stats = self.stats.events.entry(event).or_default();
            stats.check.record(checked - started);
            let previous = self.last.get(&event);
            if let Some(watched) = previous {
                if has_fingerprint && watched.fingerprint.as_ref() == Some(&self.scratch) {
                    continue;
                }
            }

            let state = State::snapshot(event);
            let snapshotted = Instant::now();
            let (is_new, delta) = state.compare(previous.map(|watched| &watched.state));
            stats.snapshot.record(snapshotted - checked);
            stats.compare.record(snapshotted.elapsed());
            let fingerprint = has_fingerprint.then(|| self.scratch.clone());
            // Keep the last real delta when only the fingerprint moved.
            let delta = match (is_new, self.last.remove(&event)) {
                (false, Some(watched)) => watched.delta,
                _ => delta,
            };
            self.last.insert(event, Watched { fingerprint, state, delta });
            if is_new {
                changed.push(event);
            }
        }
        changed
    }

    /// The serialized notification for a polled event. `initial` is a
    /// subscriber's first notification; `options` apply to collection events.
    pub fn message(&self, event: EventName, initial: bool, options: Options) -> Option<String> {
        let watched = self.last.get(&event)?;
        let delta = if initial { None } else { watched.delta.as_ref() };
        Some(watched.state.message(event, delta, options))
    }
}

/// Timing for `debug.stats`: how much main-thread time Tell costs.
#[derive(Default)]
pub struct Stats {
    since: Option<Instant>,
    ticks: Accumulator,
    events: HashMap<EventName, EventStats>,
}

#[derive(Default)]
struct Accumulator {
    count: u64,
    total: Duration,
    max: Duration,
}

impl Accumulator {
    fn record(&mut self, elapsed: Duration) {
        self.count += 1;
        self.total += elapsed;
        self.max = self.max.max(elapsed);
    }

    fn timing(&self) -> Timing {
        let average = self.total.checked_div(self.count as u32).unwrap_or_default();
        Timing {
            count: self.count,
            avg_us: average.as_micros() as u64,
            max_us: self.max.as_micros() as u64,
        }
    }
}

#[derive(Default)]
struct EventStats {
    check: Accumulator,
    snapshot: Accumulator,
    compare: Accumulator,
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

    pub fn record_send(&mut self, event: EventName, bytes: usize) {
        let stats = self.events.entry(event).or_default();
        stats.sent += 1;
        stats.last_bytes = bytes;
    }

    pub fn report(&self) -> DebugStats {
        let events: BTreeMap<EventName, EventTiming> = self
            .events
            .iter()
            .map(|(&event, stats)| {
                let timing = EventTiming {
                    check: stats.check.timing(),
                    snapshot: stats.snapshot.timing(),
                    compare: stats.compare.timing(),
                    sent: stats.sent,
                    last_bytes: stats.last_bytes,
                };
                (event, timing)
            })
            .collect();
        DebugStats {
            seconds: self.since.map(|since| since.elapsed().as_secs_f64()).unwrap_or(0.0),
            ticks: self.ticks.timing(),
            events,
        }
    }
}
