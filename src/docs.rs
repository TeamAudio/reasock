//! The bundled API reference: `Data/reasock/reasock-api.html` in REAPER's
//! resource folder, installed by ReaPack and generated from `openrpc.json`
//! by `scripts/build_docs.py`.

use crate::log::log;
use reaper_medium::Reaper;
use std::path::PathBuf;
use std::process::Command;

pub fn path() -> PathBuf {
    Reaper::get().get_resource_path(|resource| {
        resource
            .join("Data")
            .join("reasock")
            .join("reasock-api.html")
            .into_std_path_buf()
    })
}

/// Opens the API reference in the default browser. False if it isn't
/// installed or couldn't be opened.
pub fn open() -> bool {
    let path = path();
    if !path.is_file() {
        log!("API reference not found at {}", path.display());
        return false;
    }
    let opened = if cfg!(target_os = "macos") {
        Command::new("open").arg(&path).spawn()
    } else if cfg!(target_os = "windows") {
        Command::new("explorer").arg(&path).spawn()
    } else {
        Command::new("xdg-open").arg(&path).spawn()
    };
    match opened {
        Ok(_) => true,
        Err(error) => {
            log!("couldn't open the API reference: {error}");
            false
        }
    }
}
