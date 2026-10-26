//! Announces this REAPER instance to clients.
//!
//! Each running REAPER writes `~/.reasock/instances/<pid>.json`. Clients scan
//! that directory, skip entries whose process is gone, and connect to the port
//! with the token from the same file.
//!
//! The token proves a client can read your files, which a web page or another
//! user on the machine can't: on macOS and Linux the file and its folders are
//! readable only by you, and on Windows your profile folder already is.

use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Instance<'a> {
    pid: u32,
    port: u16,
    reasock_version: &'a str,
    reaper_version: &'a str,
    token: &'a str,
}

/// This REAPER's connection token: 256 random bits, hex-encoded.
pub fn token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("the OS random source is available");
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    })
}

/// Compares in constant time, so response timing reveals nothing about the token.
pub fn token_matches(presented: &str) -> bool {
    let expected = token().as_bytes();
    let presented = presented.as_bytes();
    presented.len() == expected.len()
        && presented
            .iter()
            .zip(expected)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// Creates `dir` readable only by the current user (where the OS supports it).
fn private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Writes a file readable only by the current user, replacing any old one.
fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let _ = fs::remove_file(path);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(contents)
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
    if let Some(instances) = path.parent() {
        if let Some(base) = instances.parent() {
            private_dir(base)?;
        }
        private_dir(instances)?;
    }
    let instance = Instance {
        pid: std::process::id(),
        port,
        reasock_version: env!("CARGO_PKG_VERSION"),
        reaper_version,
        token: token(),
    };
    write_private(&path, &serde_json::to_vec_pretty(&instance)?)?;
    Ok(path)
}

pub fn remove() {
    if let Some(path) = instance_path() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_256_random_bits_in_hex() {
        assert_eq!(token().len(), 64);
        assert!(token().chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(token(), token());
    }

    #[test]
    fn only_the_exact_token_matches() {
        assert!(token_matches(token()));
        assert!(!token_matches(""));
        assert!(!token_matches(&token()[1..]));
        let mut wrong = token().to_owned();
        wrong.replace_range(63.., if token().ends_with('0') { "1" } else { "0" });
        assert!(!token_matches(&wrong));
    }
}
