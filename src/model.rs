//! Snapshots of REAPER state as protocol types. Main thread only.
//!
//! Items and tracks carry their GUIDs, which stay stable across sessions,
//! unlike REAPER's pointers and indexes.

use crate::types::{
    ExtKey, Item, Marker, Project, ProjectEdit, Take, TimeRange, Track, Transport,
};
use reaper_medium::{
    ItemAttributeKey, MasterTrackBehavior, MediaItem, MediaItemTake, MediaTrack, ProjectContext,
    ProjectRef, Reaper, TakeAttributeKey, TimeRangeType, TrackAttributeKey,
};
use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, CStr, CString};
use std::ptr::null_mut;

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
unsafe fn track_guid(track: MediaTrack) -> String {
    let reaper = Reaper::get();
    let guid = reaper.low().GetTrackGUID(track.as_ptr());
    guid.as_ref()
        .map(|guid| reaper.guid_to_string(guid).to_str().to_owned())
        .unwrap_or_default()
}

/// # Safety
/// `track` must be a valid track in the current project.
unsafe fn track_name(track: MediaTrack) -> Option<String> {
    Reaper::get().get_set_media_track_info_get_name(track, |name| name.to_str().to_owned())
}

/// # Safety
/// `track` must be a valid track in the current project.
unsafe fn track_info(track: MediaTrack) -> Track {
    let number = Reaper::get().get_media_track_info_value(track, TrackAttributeKey::TrackNumber);
    Track {
        guid: track_guid(track),
        name: track_name(track),
        number: number as i64,
    }
}

/// Optional item fields a caller asked for.
#[derive(Clone, Copy, Default)]
pub struct Extras<'a> {
    pub notes: bool,
    pub ext: Option<&'a [ExtKey]>,
}

/// Per-snapshot scratch space: a reusable path buffer, track details looked
/// up once per track rather than once per item, and the optional fields to
/// include.
struct ItemScratch<'a> {
    path: Vec<c_char>,
    tracks: HashMap<MediaTrack, (Option<String>, String)>,
    extras: Extras<'a>,
    /// `P_EXT:<key>` for each wanted key.
    ext_params: Vec<(String, CString)>,
}

impl<'a> ItemScratch<'a> {
    fn new(extras: Extras<'a>) -> Self {
        Self {
            path: vec![0; 4096],
            tracks: HashMap::new(),
            extras,
            ext_params: ext_params(extras.ext),
        }
    }

    /// # Safety
    /// `track` must be a valid track in the current project.
    unsafe fn track(&mut self, track: MediaTrack) -> &(Option<String>, String) {
        self.tracks
            .entry(track)
            .or_insert_with(|| (track_name(track), track_guid(track)))
    }
}

/// # Safety
/// `item` must be a valid item in the current project.
unsafe fn item_info(item: MediaItem, scratch: &mut ItemScratch) -> Item {
    let reaper = Reaper::get();
    let guid = item_guid(item);
    let take = reaper.get_active_take(item);
    let name = take.and_then(|take| {
        reaper.get_take_name(take, |name| name.ok().map(|name| name.to_str().to_owned()))
    });
    let path = take.and_then(|take| take_path(take, &mut scratch.path));
    let (track, track_guid) = match reaper.get_media_item_track(item) {
        Some(track) => {
            let (name, guid) = scratch.track(track).clone();
            (name, Some(guid))
        }
        None => (None, None),
    };
    Item {
        guid,
        name,
        path,
        track,
        track_guid,
        position: reaper.get_media_item_info_value(item, ItemAttributeKey::Position),
        length: reaper.get_media_item_info_value(item, ItemAttributeKey::Length),
        take_count: reaper.low().CountTakes(item.as_ptr()).max(0) as usize,
        take_guid: take.map(|take| take_guid(take)),
        start_offset: take
            .map(|take| reaper.get_media_item_take_info_value(take, TakeAttributeKey::StartOffs)),
        play_rate: take
            .map(|take| reaper.get_media_item_take_info_value(take, TakeAttributeKey::PlayRate)),
        loop_source: reaper.low().GetMediaItemInfo_Value(item.as_ptr(), c"B_LOOPSRC".as_ptr()) != 0.0,
        notes: scratch
            .extras
            .notes
            .then(|| item_string(item, c"P_NOTES").unwrap_or_default()),
        ext: scratch.extras.ext.map(|_| {
            scratch
                .ext_params
                .iter()
                .map(|(key, param)| (key.clone(), item_string(item, param)))
                .collect()
        }),
    }
}

/// `(key, "P_EXT:<key>")` for each key.
fn ext_params(keys: Option<&[ExtKey]>) -> Vec<(String, CString)> {
    keys.unwrap_or_default()
        .iter()
        .filter_map(|key| {
            let param = CString::new(format!("P_EXT:{}", key.0)).ok()?;
            Some((key.0.clone(), param))
        })
        .collect()
}

/// A string attribute of an item, read through the pointer REAPER returns,
/// so no buffer can be too small. Empty counts as unset.
///
/// # Safety
/// `item` must be a valid item in the current project.
unsafe fn item_string(item: MediaItem, param: &CStr) -> Option<String> {
    let ptr = Reaper::get()
        .low()
        .GetSetMediaItemInfo(item.as_ptr(), param.as_ptr(), null_mut()) as *const c_char;
    c_string(ptr)
}

/// Like `item_string`, for a take.
///
/// # Safety
/// `take` must be a valid take in the current project.
unsafe fn take_string(take: MediaItemTake, param: &CStr) -> Option<String> {
    let ptr = Reaper::get()
        .low()
        .GetSetMediaItemTakeInfo(take.as_ptr(), param.as_ptr(), null_mut()) as *const c_char;
    c_string(ptr)
}

/// # Safety
/// `ptr` must be null or a NUL-terminated string.
unsafe fn c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(CStr::from_ptr(ptr).to_string_lossy().into_owned()).filter(|value| !value.is_empty())
}

/// # Safety
/// `item` must be a valid item in the current project.
pub unsafe fn item(item: MediaItem) -> Item {
    item_info(item, &mut ItemScratch::new(Extras::default()))
}

pub fn selected_items() -> Vec<Item> {
    selected_items_page(0, usize::MAX, Extras::default()).1
}

/// The total number of selected items, and up to `limit` of them starting at
/// `offset`, in selection order.
pub fn selected_items_page(offset: usize, limit: usize, extras: Extras) -> (usize, Vec<Item>) {
    let reaper = Reaper::get();
    let count = reaper.count_selected_media_items(CURRENT);
    let mut scratch = ItemScratch::new(extras);
    let items = (0..count)
        .skip(offset)
        .take(limit)
        .filter_map(|index| reaper.get_selected_media_item(CURRENT, index))
        .map(|item| unsafe { item_info(item, &mut scratch) })
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
pub fn items_by_guid(guids: &[String], extras: Extras) -> Vec<Option<Item>> {
    let reaper = Reaper::get();
    let positions = guid_positions(guids);
    let mut found = vec![None; guids.len()];
    let mut scratch = ItemScratch::new(extras);
    for index in 0..reaper.count_media_items(CURRENT) {
        let Some(item) = reaper.get_media_item(CURRENT, index) else {
            continue;
        };
        let guid = unsafe { item_guid(item) }.to_ascii_uppercase();
        if let Some(at) = positions.get(&guid) {
            let info = unsafe { item_info(item, &mut scratch) };
            for &position in at {
                found[position] = Some(info.clone());
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
/// snapshot. Includes position, length, track, and the active take's offset
/// and rate because those change live while dragging, before the edit is
/// committed.
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
            let take = reaper.get_active_take(item);
            out.push(take.map_or(0, |take| take.as_ptr() as u64));
            if let Some(take) = take {
                let offset = reaper.get_media_item_take_info_value(take, TakeAttributeKey::StartOffs);
                let rate = reaper.get_media_item_take_info_value(take, TakeAttributeKey::PlayRate);
                out.push(offset.to_bits());
                out.push(rate.to_bits());
            }
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

pub fn selected_tracks() -> Vec<Track> {
    selected_tracks_page(0, usize::MAX).1
}

/// Like `selected_items_page`, for tracks. The master track is never included.
pub fn selected_tracks_page(offset: usize, limit: usize) -> (usize, Vec<Track>) {
    let reaper = Reaper::get();
    let master = MasterTrackBehavior::ExcludeMasterTrack;
    let count = reaper.count_selected_tracks_2(CURRENT, master);
    let tracks = (0..count)
        .skip(offset)
        .take(limit)
        .filter_map(|index| reaper.get_selected_track_2(CURRENT, index, master))
        .map(|track| unsafe { track_info(track) })
        .collect();
    (count as usize, tracks)
}

/// Looks up tracks in the current project by GUID; `None` where not found.
pub fn tracks_by_guid(guids: &[String]) -> Vec<Option<Track>> {
    let reaper = Reaper::get();
    let positions = guid_positions(guids);
    let mut found = vec![None; guids.len()];
    for index in 0..reaper.count_tracks(CURRENT) {
        let Some(track) = reaper.get_track(CURRENT, index) else {
            continue;
        };
        let guid = unsafe { track_guid(track) };
        if let Some(at) = positions.get(&guid.to_ascii_uppercase()) {
            let info = unsafe { track_info(track) };
            for &position in at {
                found[position] = Some(info.clone());
            }
        }
    }
    found
}

pub fn transport() -> Transport {
    let reaper = Reaper::get();
    let state = reaper.get_play_state_ex(CURRENT);
    let edit_cursor = reaper.get_cursor_position_ex(CURRENT).map_or(0.0, |position| position.get());
    let time_selection = reaper
        .get_set_loop_time_range_2_get(CURRENT, TimeRangeType::TimeSelection)
        .map(|range| TimeRange {
            start: range.start.get(),
            end: range.end.get(),
        })
        .filter(|range| range.end > range.start);
    Transport {
        playing: state.is_playing,
        paused: state.is_paused,
        recording: state.is_recording,
        edit_cursor,
        time_selection,
    }
}

pub fn project() -> Project {
    let reaper = Reaper::get();
    let current = reaper.enum_projects(ProjectRef::Current, 4096);
    let tab = current.as_ref().and_then(|current| {
        (0..)
            .map_while(|index| reaper.enum_projects(ProjectRef::Tab(index), 0))
            .position(|tab| tab.project == current.project)
    });
    let path = current.and_then(|current| current.file_path);
    Project {
        tab,
        name: path.as_ref().and_then(|path| path.file_stem()).map(str::to_owned),
        path: path.map(|path| path.into_string()),
        dirty: reaper.is_project_dirty(CURRENT),
    }
}

pub fn project_edit() -> ProjectEdit {
    let reaper = Reaper::get();
    ProjectEdit {
        state_change_count: reaper.get_project_state_change_count(CURRENT),
        last_undo: reaper.undo_can_undo_2(CURRENT, |description| description.to_str().to_owned()),
    }
}

/// # Safety
/// `take` must be a valid take in the current project.
unsafe fn take_guid(take: MediaItemTake) -> String {
    let mut guid = [0 as c_char; 64];
    Reaper::get().low().GetSetMediaItemTakeInfo_String(
        take.as_ptr(),
        c"GUID".as_ptr(),
        guid.as_mut_ptr(),
        false,
    );
    from_buffer(&guid)
}

/// # Safety
/// `take` must be a valid take in the current project.
unsafe fn take_path(take: MediaItemTake, buffer: &mut [c_char]) -> Option<String> {
    let reaper = Reaper::get();
    let source = reaper.get_media_item_take_source(take)?;
    buffer[0] = 0;
    reaper
        .low()
        .GetMediaSourceFileName(source.as_ptr(), buffer.as_mut_ptr(), buffer.len() as i32);
    Some(from_buffer(buffer)).filter(|path| !path.is_empty())
}

/// All tracks in order (the master track is not included).
pub fn tracks_page(offset: usize, limit: usize) -> (usize, Vec<Track>) {
    let reaper = Reaper::get();
    let count = reaper.count_tracks(CURRENT);
    let tracks = (0..count)
        .skip(offset)
        .take(limit)
        .filter_map(|index| reaper.get_track(CURRENT, index))
        .map(|track| unsafe { track_info(track) })
        .collect();
    (count as usize, tracks)
}

pub fn find_track(guid: &str) -> Option<MediaTrack> {
    let reaper = Reaper::get();
    (0..reaper.count_tracks(CURRENT))
        .filter_map(|index| reaper.get_track(CURRENT, index))
        .find(|&track| unsafe { track_guid(track) }.eq_ignore_ascii_case(guid))
}

/// The items with these GUIDs, keyed by upper-case GUID, in one pass over
/// the project.
pub fn find_items<'a>(guids: impl IntoIterator<Item = &'a str>) -> HashMap<String, MediaItem> {
    let reaper = Reaper::get();
    let mut wanted: HashMap<String, Option<MediaItem>> =
        guids.into_iter().map(|guid| (guid.to_ascii_uppercase(), None)).collect();
    for index in 0..reaper.count_media_items(CURRENT) {
        let Some(item) = reaper.get_media_item(CURRENT, index) else {
            continue;
        };
        if let Some(slot) = wanted.get_mut(&unsafe { item_guid(item) }.to_ascii_uppercase()) {
            *slot = Some(item);
        }
    }
    wanted.into_iter().filter_map(|(guid, item)| Some((guid, item?))).collect()
}

/// The takes with these GUIDs, keyed by upper-case GUID, in one pass over
/// the project.
pub fn find_takes<'a>(guids: impl IntoIterator<Item = &'a str>) -> HashMap<String, MediaItemTake> {
    let reaper = Reaper::get();
    let mut wanted: HashMap<String, Option<MediaItemTake>> =
        guids.into_iter().map(|guid| (guid.to_ascii_uppercase(), None)).collect();
    for index in 0..reaper.count_media_items(CURRENT) {
        let Some(item) = reaper.get_media_item(CURRENT, index) else {
            continue;
        };
        unsafe {
            for take_index in 0..reaper.low().CountTakes(item.as_ptr()) {
                let Some(take) = MediaItemTake::new(reaper.low().GetTake(item.as_ptr(), take_index))
                else {
                    continue;
                };
                if let Some(slot) = wanted.get_mut(&take_guid(take).to_ascii_uppercase()) {
                    *slot = Some(take);
                }
            }
        }
    }
    wanted.into_iter().filter_map(|(guid, take)| Some((guid, take?))).collect()
}

pub fn find_item(guid: &str) -> Option<MediaItem> {
    let reaper = Reaper::get();
    (0..reaper.count_media_items(CURRENT))
        .filter_map(|index| reaper.get_media_item(CURRENT, index))
        .find(|&item| unsafe { item_guid(item) }.eq_ignore_ascii_case(guid))
}

/// All items in the project, or only those on `track`, in REAPER's order
/// (by track, then position).
pub fn items_page(
    offset: usize,
    limit: usize,
    track: Option<MediaTrack>,
    extras: Extras,
) -> (usize, Vec<Item>) {
    let reaper = Reaper::get();
    let mut scratch = ItemScratch::new(extras);
    let (count, item_at): (u32, Box<dyn Fn(u32) -> Option<MediaItem>>) = match track {
        Some(track) => (
            unsafe { reaper.count_track_media_items(track) },
            Box::new(move |index| unsafe { reaper.get_track_media_item(track, index) }),
        ),
        None => (
            reaper.count_media_items(CURRENT),
            Box::new(|index| reaper.get_media_item(CURRENT, index)),
        ),
    };
    let items = (0..count)
        .skip(offset)
        .take(limit)
        .filter_map(|index| item_at(index))
        .map(|item| unsafe { item_info(item, &mut scratch) })
        .collect();
    (count as usize, items)
}

/// # Safety
/// `item` must be a valid item in the current project.
pub unsafe fn takes(item: MediaItem, ext: Option<&[ExtKey]>) -> Vec<Take> {
    let reaper = Reaper::get();
    let active = reaper.get_active_take(item);
    let mut path = vec![0 as c_char; 4096];
    let ext_params = ext_params(ext);
    (0..reaper.low().CountTakes(item.as_ptr()))
        .filter_map(|index| MediaItemTake::new(reaper.low().GetTake(item.as_ptr(), index)))
        .map(|take| {
            Take {
                guid: take_guid(take),
                name: reaper.get_take_name(take, |name| name.ok().map(|name| name.to_str().to_owned())),
                path: take_path(take, &mut path),
                active: Some(take) == active,
                start_offset: reaper.get_media_item_take_info_value(take, TakeAttributeKey::StartOffs),
                play_rate: reaper.get_media_item_take_info_value(take, TakeAttributeKey::PlayRate),
                ext: ext.map(|_| {
                    ext_params
                        .iter()
                        .map(|(key, param)| (key.clone(), take_string(take, param)))
                        .collect::<BTreeMap<_, _>>()
                }),
            }
        })
        .collect()
}

/// Markers and regions in position order.
pub fn markers_page(offset: usize, limit: usize) -> (usize, Vec<Marker>) {
    let reaper = Reaper::get();
    let count = reaper.count_project_markers(CURRENT).total_count;
    let markers = (0..count)
        .skip(offset)
        .take(limit)
        .filter_map(|index| {
            reaper.enum_project_markers_3(CURRENT, index, |marker| {
                let marker = marker?;
                Some(Marker {
                    guid: marker_guid(index),
                    number: marker.id.get(),
                    name: marker.name.to_str().to_owned(),
                    region: marker.region_end_position.is_some(),
                    position: marker.position.get(),
                    end: marker.region_end_position.map(|end| end.get()),
                })
            })
        })
        .collect();
    (count as usize, markers)
}

/// The GUID of the marker or region at enumeration `index`, if REAPER has one.
fn marker_guid(index: u32) -> Option<String> {
    let key = std::ffi::CString::new(format!("MARKER_GUID:{index}")).ok()?;
    let mut guid = [0 as c_char; 64];
    let found = unsafe {
        Reaper::get().low().GetSetProjectInfo_String(
            std::ptr::null_mut(),
            key.as_ptr(),
            guid.as_mut_ptr(),
            false,
        )
    };
    Some(from_buffer(&guid)).filter(|guid| found && !guid.is_empty())
}
