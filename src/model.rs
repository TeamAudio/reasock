//! Snapshots of REAPER state as JSON. Main thread only.
//!
//! Items and tracks carry their GUIDs, which stay stable across sessions,
//! unlike REAPER's pointers and indexes.

use reaper_medium::{
    MasterTrackBehavior, MediaItem, MediaTrack, ItemAttributeKey, ProjectContext, ProjectRef,
    Reaper, TrackAttributeKey,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::{c_char, CStr};

const CURRENT: ProjectContext = ProjectContext::CurrentProject;

fn from_buffer(buffer: &[c_char]) -> String {
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

pub fn reaper_version() -> String {
    let ptr = Reaper::get().low().GetAppVersion();
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
}

/// # Safety
/// `track` must be a valid track in the current project.
unsafe fn track_guid(track: MediaTrack) -> Option<String> {
    let reaper = Reaper::get();
    let guid = reaper.low().GetTrackGUID(track.as_ptr());
    guid.as_ref()
        .map(|guid| reaper.guid_to_string(guid).to_str().to_owned())
}

/// # Safety
/// `track` must be a valid track in the current project.
unsafe fn track_name(track: MediaTrack) -> Option<String> {
    Reaper::get().get_set_media_track_info_get_name(track, |name| name.to_str().to_owned())
}

/// # Safety
/// `track` must be a valid track in the current project.
unsafe fn track_json(track: MediaTrack) -> Value {
    let number = Reaper::get().get_media_track_info_value(track, TrackAttributeKey::TrackNumber);
    json!({
        "guid": track_guid(track),
        "name": track_name(track),
        "number": number as i64,
    })
}

/// Per-snapshot scratch space: a reusable path buffer, and track details
/// looked up once per track rather than once per item.
struct ItemScratch {
    path: Vec<c_char>,
    tracks: HashMap<MediaTrack, (Option<String>, Option<String>)>,
}

impl ItemScratch {
    fn new() -> Self {
        Self {
            path: vec![0; 4096],
            tracks: HashMap::new(),
        }
    }

    /// # Safety
    /// `track` must be a valid track in the current project.
    unsafe fn track(&mut self, track: MediaTrack) -> &(Option<String>, Option<String>) {
        self.tracks
            .entry(track)
            .or_insert_with(|| (track_name(track), track_guid(track)))
    }
}

/// # Safety
/// `item` must be a valid item in the current project.
unsafe fn item_json(item: MediaItem, scratch: &mut ItemScratch) -> Value {
    let reaper = Reaper::get();
    let guid = item_guid(item);
    let take = reaper.get_active_take(item);
    let name = take.and_then(|take| {
        reaper.get_take_name(take, |name| name.ok().map(|name| name.to_str().to_owned()))
    });
    let path = take
        .and_then(|take| reaper.get_media_item_take_source(take))
        .map(|source| {
            let buffer = &mut scratch.path;
            buffer[0] = 0;
            reaper.low().GetMediaSourceFileName(
                source.as_ptr(),
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            );
            from_buffer(buffer)
        })
        .filter(|path| !path.is_empty());
    let (track, track_guid) = match reaper.get_media_item_track(item) {
        Some(track) => scratch.track(track).clone(),
        None => (None, None),
    };
    json!({
        "guid": guid,
        "name": name,
        "path": path,
        "track": track,
        "trackGuid": track_guid,
        "position": reaper.get_media_item_info_value(item, ItemAttributeKey::Position),
        "length": reaper.get_media_item_info_value(item, ItemAttributeKey::Length),
    })
}

pub fn selected_items() -> Vec<Value> {
    selected_items_page(0, usize::MAX).1
}

/// The total number of selected items, and up to `limit` of them starting at
/// `offset`, in selection order.
pub fn selected_items_page(offset: usize, limit: usize) -> (usize, Vec<Value>) {
    let reaper = Reaper::get();
    let count = reaper.count_selected_media_items(CURRENT);
    let mut scratch = ItemScratch::new();
    let items = (0..count)
        .skip(offset)
        .take(limit)
        .filter_map(|index| reaper.get_selected_media_item(CURRENT, index))
        .map(|item| unsafe { item_json(item, &mut scratch) })
        .collect();
    (count as usize, items)
}

/// # Safety
/// `item` must be a valid item in the current project.
unsafe fn item_guid(item: MediaItem) -> String {
    let mut guid = [0 as c_char; 64];
    Reaper::get()
        .low()
        .GetSetMediaItemInfo_String(item.as_ptr(), c"GUID".as_ptr(), guid.as_mut_ptr(), false);
    from_buffer(&guid)
}

/// Positions of each wanted GUID in `guids`, compared case-insensitively.
fn guid_positions(guids: &[String]) -> HashMap<String, Vec<usize>> {
    let mut positions: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, guid) in guids.iter().enumerate() {
        positions.entry(guid.to_ascii_uppercase()).or_default().push(index);
    }
    positions
}

/// Looks up items in the current project by GUID; `None` where not found.
pub fn items_by_guid(guids: &[String]) -> Vec<Option<Value>> {
    let reaper = Reaper::get();
    let positions = guid_positions(guids);
    let mut found = vec![None; guids.len()];
    let mut scratch = ItemScratch::new();
    for index in 0..reaper.count_media_items(CURRENT) {
        let Some(item) = reaper.get_media_item(CURRENT, index) else {
            continue;
        };
        let guid = unsafe { item_guid(item) }.to_ascii_uppercase();
        if let Some(at) = positions.get(&guid) {
            let json = unsafe { item_json(item, &mut scratch) };
            for &position in at {
                found[position] = Some(json.clone());
            }
        }
    }
    found
}

/// Identifies the current project and its edit state. Names, paths, GUIDs,
/// and takes only change through edits, which bump the state change count.
fn project_fingerprint(out: &mut Vec<u64>) {
    let reaper = Reaper::get();
    let project = reaper
        .enum_projects(ProjectRef::Current, 0)
        .map(|current| current.project.as_ptr() as u64)
        .unwrap_or(0);
    out.push(project);
    out.push(u64::from(reaper.get_project_state_change_count(CURRENT)));
}

/// Cheap stand-in for `selected_items()`: if this is unchanged, so is the
/// snapshot. Includes position, length, and track because those change live
/// while dragging, before the edit is committed.
pub fn selected_items_fingerprint(out: &mut Vec<u64>) {
    out.clear();
    project_fingerprint(out);
    let reaper = Reaper::get();
    for index in 0..reaper.count_selected_media_items(CURRENT) {
        let Some(item) = reaper.get_selected_media_item(CURRENT, index) else {
            continue;
        };
        unsafe {
            out.push(item.as_ptr() as u64);
            out.push(reaper.get_media_item_info_value(item, ItemAttributeKey::Position).to_bits());
            out.push(reaper.get_media_item_info_value(item, ItemAttributeKey::Length).to_bits());
            out.push(reaper.get_media_item_track(item).map_or(0, |track| track.as_ptr() as u64));
        }
    }
}

/// Cheap stand-in for `selected_tracks()`.
pub fn selected_tracks_fingerprint(out: &mut Vec<u64>) {
    out.clear();
    project_fingerprint(out);
    let reaper = Reaper::get();
    let master = MasterTrackBehavior::ExcludeMasterTrack;
    for index in 0..reaper.count_selected_tracks_2(CURRENT, master) {
        if let Some(track) = reaper.get_selected_track_2(CURRENT, index, master) {
            out.push(track.as_ptr() as u64);
        }
    }
}

pub fn selected_tracks() -> Vec<Value> {
    selected_tracks_page(0, usize::MAX).1
}

/// Like `selected_items_page`, for tracks. The master track is never included.
pub fn selected_tracks_page(offset: usize, limit: usize) -> (usize, Vec<Value>) {
    let reaper = Reaper::get();
    let master = MasterTrackBehavior::ExcludeMasterTrack;
    let count = reaper.count_selected_tracks_2(CURRENT, master);
    let tracks = (0..count)
        .skip(offset)
        .take(limit)
        .filter_map(|index| reaper.get_selected_track_2(CURRENT, index, master))
        .map(|track| unsafe { track_json(track) })
        .collect();
    (count as usize, tracks)
}

/// Looks up tracks in the current project by GUID; `None` where not found.
pub fn tracks_by_guid(guids: &[String]) -> Vec<Option<Value>> {
    let reaper = Reaper::get();
    let positions = guid_positions(guids);
    let mut found = vec![None; guids.len()];
    for index in 0..reaper.count_tracks(CURRENT) {
        let Some(track) = reaper.get_track(CURRENT, index) else {
            continue;
        };
        let Some(guid) = (unsafe { track_guid(track) }) else {
            continue;
        };
        if let Some(at) = positions.get(&guid.to_ascii_uppercase()) {
            let json = unsafe { track_json(track) };
            for &position in at {
                found[position] = Some(json.clone());
            }
        }
    }
    found
}

pub fn transport() -> Value {
    let state = Reaper::get().get_play_state_ex(CURRENT);
    json!({
        "playing": state.is_playing,
        "paused": state.is_paused,
        "recording": state.is_recording,
    })
}

pub fn project() -> Value {
    let reaper = Reaper::get();
    let current = reaper.enum_projects(ProjectRef::Current, 4096);
    let tab = current.as_ref().and_then(|current| {
        (0..)
            .map_while(|index| reaper.enum_projects(ProjectRef::Tab(index), 0))
            .position(|tab| tab.project == current.project)
    });
    let path = current.and_then(|current| current.file_path);
    json!({
        "tab": tab,
        "name": path.as_ref().and_then(|path| path.file_stem()),
        "path": path.as_ref().map(|path| path.as_str()),
        "dirty": reaper.is_project_dirty(CURRENT),
    })
}

pub fn project_edit() -> Value {
    let reaper = Reaper::get();
    json!({
        "stateChangeCount": reaper.get_project_state_change_count(CURRENT),
        "lastUndo": reaper.undo_can_undo_2(CURRENT, |description| description.to_str().to_owned()),
    })
}
