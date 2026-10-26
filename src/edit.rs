//! Changes to the current project. Main thread only.
//!
//! Each method checks everything it was given before changing anything, so a
//! bad request leaves the project as it was, and makes its changes as one
//! undo point. Extension data gets an undo point too: REAPER's undo restores
//! items and takes whole, so data written outside an undo point disappears
//! the next time the user undoes or redoes an edit to the same item.

use crate::api::not_found;
use crate::model;
use crate::protocol::{RpcError, MEDIA_UNAVAILABLE, NOT_FOUND};
use crate::types::{
    Arrange, Empty, ExtKey, InsertFile, InsertParams, InsertPosition, Inserted, SetItemsParams,
    SetTakesParams, TrackKeyword, TrackTarget, MAX_EXT_VALUE,
};
use reaper_low::raw;
use reaper_medium::{MediaItem, Reaper};
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{c_char, CString};
use std::path::Path;
use std::ptr::null_mut;

/// The most files one `items.insert` call takes.
pub const MAX_INSERT_FILES: usize = 1000;

/// The most items or takes one `items.set` or `takes.set` call changes.
pub const MAX_CHANGES: usize = 10000;

/// How far past the end of a file a requested range may run before it's an
/// error rather than rounding (shortened to fit).
const TOLERANCE: f64 = 0.001;

/// Checks an extension data key against `EXT_KEY_PATTERN`.
pub fn check_key(key: &ExtKey) -> Result<(), RpcError> {
    let valid = (1..=128).contains(&key.0.len())
        && key
            .0
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(RpcError::invalid_params(format!(
            "ext keys must be 1 to 128 ASCII letters, digits, or _ . : - (got {:?})",
            key.0
        )))
    }
}

fn check_ext<'a>(ext: impl IntoIterator<Item = (&'a ExtKey, Option<&'a String>)>) -> Result<(), RpcError> {
    for (key, value) in ext {
        check_key(key)?;
        if value.is_some_and(|value| value.len() > MAX_EXT_VALUE) {
            return Err(RpcError::invalid_params(format!(
                "ext values must be at most {MAX_EXT_VALUE} bytes ({})",
                key.0
            )));
        }
    }
    Ok(())
}

fn c_string(value: &str, what: &str) -> Result<CString, RpcError> {
    CString::new(value).map_err(|_| RpcError::invalid_params(format!("{what} contains a NUL character")))
}

fn undo_label(undo: Option<String>, default: &str, client: &str) -> Result<CString, RpcError> {
    let label = undo.unwrap_or_else(|| format!("{default} (ReaSock: {client})"));
    if label.is_empty() {
        return Err(RpcError::invalid_params("undo must not be empty"));
    }
    c_string(&label, "undo")
}

/// A string to hand to one of REAPER's `GetSet…_String` setters, which copy
/// it and don't write through the pointer.
fn setter(value: &CString) -> *mut c_char {
    value.as_ptr() as *mut c_char
}

// ---------------------------------------------------------------------------
// items.insert

/// Media sources opened for an insert. Any not handed to a take are
/// destroyed when this is dropped, so a failed insert leaks nothing.
struct Sources(Vec<Source>);

struct Source {
    pcm: *mut raw::PCM_source,
    take_name: CString,
    start: f64,
    /// Seconds, or quarter notes when `in_qn`.
    length: f64,
    in_qn: bool,
}

impl Drop for Sources {
    fn drop(&mut self) {
        let low = Reaper::get().low();
        for source in &mut self.0 {
            if !source.pcm.is_null() {
                unsafe { low.PCM_Source_Destroy(source.pcm) };
            }
        }
    }
}

/// `MediaUnavailable`, saying which file and why.
fn media_unavailable(index: usize, path: &str, reason: &str) -> RpcError {
    RpcError::app(&MEDIA_UNAVAILABLE, format!("{reason}: {path}"))
        .with_data(json!({ "index": index, "path": path, "reason": reason }))
}

/// Invalid params in one file's entry, saying which file and why.
fn bad_file(index: usize, path: &str, reason: impl Into<String>) -> RpcError {
    let reason = reason.into();
    RpcError::invalid_params(format!("{reason}: {path}"))
        .with_data(json!({ "index": index, "path": path, "reason": reason }))
}

/// Adds which file to an error about one file's entry, if it doesn't say yet.
fn for_file(error: RpcError, index: usize, path: &str) -> RpcError {
    if error.data.is_some() {
        return error;
    }
    let reason = error.message.clone();
    error.with_data(json!({ "index": index, "path": path, "reason": reason }))
}

/// Opens file `index` and checks the requested range against it.
fn open(index: usize, file: &InsertFile) -> Result<Source, RpcError> {
    let low = Reaper::get().low();
    let path = Path::new(&file.path);
    if !path.is_absolute() {
        return Err(bad_file(index, &file.path, "path must be absolute"));
    }
    if !path.is_file() {
        return Err(media_unavailable(index, &file.path, "no such file"));
    }
    let c_path = c_string(&file.path, "path").map_err(|error| for_file(error, index, &file.path))?;
    let take_name = match &file.name {
        Some(name) => c_string(name, "name"),
        None => c_string(&path.file_name().unwrap_or_default().to_string_lossy(), "path"),
    }
    .map_err(|error| for_file(error, index, &file.path))?;
    let pcm = unsafe { low.PCM_Source_CreateFromFileEx(c_path.as_ptr(), false) };
    if pcm.is_null() {
        return Err(media_unavailable(index, &file.path, "REAPER can't open this file"));
    }
    let mut in_qn = false;
    let total = unsafe { low.GetMediaSourceLength(pcm, &mut in_qn) };
    match range(index, file, total, in_qn) {
        Ok((start, length)) => Ok(Source {
            pcm,
            take_name,
            start,
            length,
            in_qn,
        }),
        Err(error) => {
            unsafe { low.PCM_Source_Destroy(pcm) };
            Err(error)
        }
    }
}

/// The part of a file to use, `(start, length)`, checked against the file's
/// `total` length (in quarter notes when `in_qn`).
fn range(index: usize, file: &InsertFile, total: f64, in_qn: bool) -> Result<(f64, f64), RpcError> {
    if !(total > 0.0) {
        return Err(media_unavailable(index, &file.path, "REAPER can't read this file as media"));
    }
    if in_qn {
        if file.source_start.is_some() || file.length.is_some() {
            return Err(bad_file(index, &file.path, "sourceStart and length aren't supported for MIDI files"));
        }
        return Ok((0.0, total));
    }
    let start = file.source_start.unwrap_or(0.0);
    if !(start >= 0.0 && start < total) {
        return Err(bad_file(
            index,
            &file.path,
            format!("sourceStart must be at least 0 and less than the file's length ({total}s)"),
        ));
    }
    let length = file.length.unwrap_or(total - start);
    if !(length > 0.0) || start + length > total + TOLERANCE {
        return Err(bad_file(
            index,
            &file.path,
            format!("length must be more than 0, and sourceStart + length at most the file's length ({total}s)"),
        ));
    }
    Ok((start, length.min(total - start)))
}

/// Where new items go, before anything is created.
enum Target {
    Track(*mut raw::MediaTrack),
    /// A new track at this index.
    NewTrack(i32),
}

/// The index after the first selected track, else after the last touched
/// one, else the end: where REAPER puts new tracks.
fn new_track_index() -> i32 {
    let low = Reaper::get().low();
    unsafe {
        let anchor = Some(low.GetSelectedTrack2(null_mut(), 0, false))
            .filter(|track| !track.is_null())
            .or_else(|| Some(low.GetLastTouchedTrack()).filter(|track| !track.is_null()));
        match anchor {
            // IP_TRACKNUMBER is 1-based, so it's the index just after the track.
            Some(track) => low.GetMediaTrackInfo_Value(track, c"IP_TRACKNUMBER".as_ptr()) as i32,
            None => low.CountTracks(null_mut()),
        }
    }
}

fn target(track: &Option<TrackTarget>) -> Result<Target, RpcError> {
    let low = Reaper::get().low();
    let selected = unsafe { low.GetSelectedTrack2(null_mut(), 0, false) };
    Ok(match track {
        None | Some(TrackTarget::Keyword(TrackKeyword::Current)) => {
            let touched = low.GetLastTouchedTrack();
            if !selected.is_null() {
                Target::Track(selected)
            } else if !touched.is_null() {
                Target::Track(touched)
            } else {
                Target::NewTrack(new_track_index())
            }
        }
        Some(TrackTarget::Keyword(TrackKeyword::Selected)) => {
            if selected.is_null() {
                return Err(RpcError::app(
                    &NOT_FOUND,
                    "no track is selected; pass a track's GUID, \"current\", or \"new\"",
                ));
            }
            Target::Track(selected)
        }
        Some(TrackTarget::Keyword(TrackKeyword::New)) => Target::NewTrack(new_track_index()),
        Some(TrackTarget::Guid(guid)) => Target::Track(
            model::find_track(guid)
                .ok_or_else(|| not_found("track", guid))?
                .as_ptr(),
        ),
    })
}

/// Adds a track at `index`, named `name` if given.
fn add_track(index: i32, name: Option<&CString>) -> *mut raw::MediaTrack {
    let low = Reaper::get().low();
    unsafe {
        low.InsertTrackAtIndex(index, true);
        let track = low.GetTrack(null_mut(), index);
        if let Some(name) = name {
            low.GetSetMediaTrackInfo_String(track, c"P_NAME".as_ptr(), setter(name), true);
        }
        track
    }
}

/// Seconds a source takes up at `position`.
fn seconds(source: &Source, position: f64) -> f64 {
    if !source.in_qn {
        return source.length;
    }
    let low = Reaper::get().low();
    unsafe {
        let start = low.TimeMap2_timeToQN(null_mut(), position);
        low.TimeMap2_QNToTime(null_mut(), start + source.length) - position
    }
}

/// Adds a take playing `source` to `item`, which then owns the source.
fn add_take(item: *mut raw::MediaItem, source: &mut Source) -> *mut raw::MediaItem_Take {
    let low = Reaper::get().low();
    unsafe {
        let take = low.AddTakeToMediaItem(item);
        low.SetMediaItemTake_Source(take, source.pcm);
        source.pcm = null_mut();
        low.SetMediaItemTakeInfo_Value(take, c"D_STARTOFFS".as_ptr(), source.start);
        low.GetSetMediaItemTakeInfo_String(take, c"P_NAME".as_ptr(), setter(&source.take_name), true);
        start_peaks(take);
        take
    }
}

/// `(param, value)` pairs to write, `P_EXT:<key>` with an empty value to
/// remove a key.
type ExtWrites = Vec<(CString, CString)>;

fn ext_writes<'a>(ext: impl IntoIterator<Item = (&'a ExtKey, Option<&'a String>)>) -> Result<ExtWrites, RpcError> {
    ext.into_iter()
        .map(|(key, value)| {
            Ok((
                c_string(&format!("P_EXT:{}", key.0), "ext key")?,
                c_string(value.map_or("", String::as_str), "ext value")?,
            ))
        })
        .collect()
}

/// Checked strings for one inserted file's notes and ext.
struct FileExtras {
    notes: Option<CString>,
    ext: ExtWrites,
}

pub fn insert(client: &str, params: InsertParams) -> Result<Inserted, RpcError> {
    let low = Reaper::get().low();
    if params.files.is_empty() || params.files.len() > MAX_INSERT_FILES {
        return Err(RpcError::invalid_params(format!(
            "files must hold 1 to {MAX_INSERT_FILES} files"
        )));
    }
    let position = match params.position {
        None | Some(InsertPosition::Keyword(_)) => None,
        Some(InsertPosition::Seconds(seconds)) if seconds >= 0.0 && seconds.is_finite() => Some(seconds),
        Some(InsertPosition::Seconds(_)) => {
            return Err(RpcError::invalid_params("position must be at least 0"));
        }
    };
    let gap = params.gap.unwrap_or(0.0);
    if !(gap >= 0.0 && gap.is_finite()) {
        return Err(RpcError::invalid_params("gap must be at least 0"));
    }
    let arrange = params.arrange.unwrap_or_default();
    let undo = undo_label(params.undo, "Insert media", client)?;
    let track_name = params.track_name.as_deref().map(|name| c_string(name, "trackName")).transpose()?;
    let extras = params
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            // Notes and ext describe the new item, and `takes` makes only one.
            if arrange == Arrange::Takes && index > 0 && (file.notes.is_some() || file.ext.is_some()) {
                return Err(bad_file(
                    index,
                    &file.path,
                    "with arrange \"takes\" there's one new item, so only the first file may have notes or ext",
                ));
            }
            let ext = file.ext.iter().flatten().map(|(key, value)| (key, Some(value)));
            let checked = check_ext(ext.clone()).and_then(|()| {
                Ok(FileExtras {
                    notes: file.notes.as_deref().map(|notes| c_string(notes, "notes")).transpose()?,
                    ext: ext_writes(ext)?,
                })
            });
            checked.map_err(|error| for_file(error, index, &file.path))
        })
        .collect::<Result<Vec<_>, RpcError>>()?;

    // Nothing changes until every check has passed.
    let target = target(&params.track)?;
    let mut sources = Sources(Vec::with_capacity(params.files.len()));
    for (index, file) in params.files.iter().enumerate() {
        sources.0.push(open(index, file)?);
    }

    let start = position.unwrap_or_else(|| unsafe { low.GetCursorPositionEx(null_mut()) });
    let select = params.select.unwrap_or(true);
    unsafe { low.Undo_BeginBlock2(null_mut()) };
    low.PreventUIRefresh(1);
    if select {
        unsafe { low.SelectAllMediaItems(null_mut(), false) };
    }
    let mut track = match target {
        Target::Track(track) => track,
        Target::NewTrack(index) => add_track(index, track_name.as_ref()),
    };
    let mut items: Vec<*mut raw::MediaItem> = Vec::new();
    let mut end = start;
    unsafe {
        match arrange {
            Arrange::Sequence | Arrange::Tracks => {
                // New tracks for `tracks` go after the target track.
                let mut next_index = low.GetMediaTrackInfo_Value(track, c"IP_TRACKNUMBER".as_ptr()) as i32;
                let mut position = start;
                for (index, (source, extras)) in sources.0.iter_mut().zip(&extras).enumerate() {
                    if arrange == Arrange::Tracks && (index > 0 || matches!(target, Target::Track(_))) {
                        track = add_track(next_index, track_name.as_ref());
                        next_index += 1;
                    }
                    let length = seconds(source, position);
                    let item = low.AddMediaItemToTrack(track);
                    low.SetMediaItemInfo_Value(item, c"D_POSITION".as_ptr(), position);
                    low.SetMediaItemInfo_Value(item, c"D_LENGTH".as_ptr(), length);
                    add_take(item, source);
                    write_item_extras(item, extras.notes.as_ref(), &extras.ext);
                    items.push(item);
                    end = end.max(position + length);
                    if arrange == Arrange::Sequence {
                        position += length + gap;
                    }
                }
            }
            Arrange::Takes => {
                // As long as the first file, whose take is the one that plays.
                let item = low.AddMediaItemToTrack(track);
                let length = seconds(&sources.0[0], start);
                low.SetMediaItemInfo_Value(item, c"D_POSITION".as_ptr(), start);
                low.SetMediaItemInfo_Value(item, c"D_LENGTH".as_ptr(), length);
                let mut first = None;
                for source in sources.0.iter_mut() {
                    let take = add_take(item, source);
                    first.get_or_insert(take);
                }
                if let Some(take) = first {
                    low.SetActiveTake(take);
                }
                write_item_extras(item, extras[0].notes.as_ref(), &extras[0].ext);
                items.push(item);
                end = start + length;
            }
        }
        if select {
            for &item in &items {
                low.SetMediaItemSelected(item, true);
            }
        }
        if params.move_cursor.unwrap_or(false) {
            low.SetEditCurPos2(null_mut(), end, false, false);
        }
    }
    low.PreventUIRefresh(-1);
    low.UpdateArrange();
    let flags = raw::UNDO_STATE_ITEMS | raw::UNDO_STATE_TRACKCFG;
    unsafe { low.Undo_EndBlock2(null_mut(), undo.as_ptr(), flags as i32) };

    Ok(Inserted {
        items: items
            .into_iter()
            .filter_map(MediaItem::new)
            .map(|item| unsafe { model::item(item) })
            .collect(),
    })
}

/// # Safety
/// `item` must be a valid item in the current project.
unsafe fn write_item_extras(item: *mut raw::MediaItem, notes: Option<&CString>, ext: &[(CString, CString)]) {
    let low = Reaper::get().low();
    if let Some(notes) = notes {
        low.GetSetMediaItemInfo_String(item, c"P_NOTES".as_ptr(), setter(notes), true);
    }
    for (param, value) in ext {
        low.GetSetMediaItemInfo_String(item, param.as_ptr(), setter(value), true);
    }
}

// ---------------------------------------------------------------------------
// Peaks

thread_local! {
    /// Takes inserted by `items.insert` whose peaks are still being built.
    /// Main thread only, like everything that touches the project.
    static BUILDING_PEAKS: RefCell<Vec<*mut raw::MediaItem_Take>> = const { RefCell::new(Vec::new()) };
}

/// Starts building peaks for a new take's source, if it has none. REAPER
/// doesn't do this for sources created through its API, and building them all
/// at once could stall REAPER on long files, so `build_peaks` continues a
/// slice at a time.
fn start_peaks(take: *mut raw::MediaItem_Take) {
    let low = Reaper::get().low();
    unsafe {
        let source = low.GetMediaItemTake_Source(take);
        if !source.is_null() && low.PCM_Source_BuildPeaks(source, 0) != 0 {
            BUILDING_PEAKS.with_borrow_mut(|takes| takes.push(take));
        }
    }
}

/// Continues building peaks, a slice per take per timer tick. Takes that no
/// longer exist (the insert was undone, say) are dropped.
pub fn build_peaks() {
    BUILDING_PEAKS.with_borrow_mut(|takes| {
        if takes.is_empty() {
            return;
        }
        let low = Reaper::get().low();
        takes.retain(|&take| unsafe {
            let exists = low.ValidatePtr2(null_mut(), take.cast(), c"MediaItem_Take*".as_ptr());
            let source = if exists { low.GetMediaItemTake_Source(take) } else { null_mut() };
            if source.is_null() {
                return false;
            }
            if low.PCM_Source_BuildPeaks(source, 1) != 0 {
                return true;
            }
            low.PCM_Source_BuildPeaks(source, 2);
            low.UpdateItemInProject(low.GetMediaItemTake_Item(take));
            false
        });
    });
}

// ---------------------------------------------------------------------------
// items.set and takes.set

fn check_count(count: usize, what: &str) -> Result<(), RpcError> {
    if count == 0 || count > MAX_CHANGES {
        return Err(RpcError::invalid_params(format!("{what} must hold 1 to {MAX_CHANGES} entries")));
    }
    Ok(())
}

/// One entry's checked changes.
struct Changes {
    text: Option<CString>,
    ext: ExtWrites,
}

fn changes(text: Option<&str>, what: &str, ext: Option<&BTreeMap<ExtKey, Option<String>>>) -> Result<Changes, RpcError> {
    let ext = ext.into_iter().flatten().map(|(key, value)| (key, value.as_ref()));
    check_ext(ext.clone())?;
    Ok(Changes {
        text: text.map(|text| c_string(text, what)).transpose()?,
        ext: ext_writes(ext)?,
    })
}

/// Runs `apply` inside one undo point.
fn undoable(undo: &CString, apply: impl FnOnce()) {
    let low = Reaper::get().low();
    unsafe { low.Undo_BeginBlock2(null_mut()) };
    low.PreventUIRefresh(1);
    apply();
    low.PreventUIRefresh(-1);
    low.UpdateArrange();
    unsafe { low.Undo_EndBlock2(null_mut(), undo.as_ptr(), raw::UNDO_STATE_ITEMS as i32) };
}

pub fn set_items(client: &str, params: SetItemsParams) -> Result<Empty, RpcError> {
    check_count(params.items.len(), "items")?;
    let undo = undo_label(params.undo, "Set item properties", client)?;
    let checked = params
        .items
        .iter()
        .map(|item| changes(item.notes.as_deref(), "notes", item.ext.as_ref()))
        .collect::<Result<Vec<_>, RpcError>>()?;
    let found = model::find_items(params.items.iter().map(|item| item.guid.as_str()));
    let items = params
        .items
        .iter()
        .map(|item| {
            found
                .get(&item.guid.to_ascii_uppercase())
                .copied()
                .ok_or_else(|| not_found("item", &item.guid))
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    undoable(&undo, || {
        for (item, changes) in items.iter().zip(&checked) {
            unsafe { write_item_extras(item.as_ptr(), changes.text.as_ref(), &changes.ext) };
        }
    });
    Ok(Empty {})
}

pub fn set_takes(client: &str, params: SetTakesParams) -> Result<Empty, RpcError> {
    check_count(params.takes.len(), "takes")?;
    let undo = undo_label(params.undo, "Set take properties", client)?;
    let checked = params
        .takes
        .iter()
        .map(|take| changes(take.name.as_deref(), "name", take.ext.as_ref()))
        .collect::<Result<Vec<_>, RpcError>>()?;
    let found = model::find_takes(params.takes.iter().map(|take| take.guid.as_str()));
    let takes = params
        .takes
        .iter()
        .map(|take| {
            found
                .get(&take.guid.to_ascii_uppercase())
                .copied()
                .ok_or_else(|| not_found("take", &take.guid))
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let low = Reaper::get().low();
    undoable(&undo, || {
        for (take, changes) in takes.iter().zip(&checked) {
            unsafe {
                if let Some(name) = &changes.text {
                    low.GetSetMediaItemTakeInfo_String(take.as_ptr(), c"P_NAME".as_ptr(), setter(name), true);
                }
                for (param, value) in &changes.ext {
                    low.GetSetMediaItemTakeInfo_String(take.as_ptr(), param.as_ptr(), setter(value), true);
                }
            }
        }
    });
    Ok(Empty {})
}
