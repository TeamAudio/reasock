//! Announces this REAPER instance to clients.
//!
//! Each running REAPER writes `~/.reasock/instances/<pid>.json`. Clients scan
//! that directory, skip entries whose process is gone, and connect to the port.

use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Instance<'a> {
    pid: u32,
    port: u16,
    reasock_version: &'a str,
    reaper_version: &'a str,
}

pub fn base_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".reasock"))
}

fn instance_path() -> Option<PathBuf> {
    Some(
        base_dir()?
            .join("instances")
            .join(format!("{}.json", std::process::id())),
    )
}

pub fn write(port: u16, reaper_version: &str) -> std::io::Result<PathBuf> {
    let path = instance_path().ok_or_else(|| std::io::Error::other("no home directory"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let instance = Instance {
        pid: std::process::id(),
        port,
        reasock_version: env!("CARGO_PKG_VERSION"),
        reaper_version,
    };
    std::fs::write(&path, serde_json::to_vec_pretty(&instance)?)?;
    Ok(path)
}

pub fn remove() {
    if let Some(path) = instance_path() {
        let _ = std::fs::remove_file(path);
    }
}
