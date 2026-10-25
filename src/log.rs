//! Minimal append-only log at `~/.reasock/reasock.log`.
//!
//! REAPER's console window pops open whenever an extension writes to it, so
//! routine diagnostics go to a file instead.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn write(message: &str) {
    let mut guard = FILE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        let Some(dir) = crate::discovery::base_dir() else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        *guard = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("reasock.log"))
            .ok();
    }
    if let Some(file) = guard.as_mut() {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let _ = writeln!(file, "{millis} [{}] {message}", std::process::id());
    }
}

macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::write(&format!($($arg)*))
    };
}

pub(crate) use log;
