//! User settings in `~/.reasock/config.json`.
//!
//! ```json
//! { "allowedOrigins": ["http://localhost:5173"] }
//! ```
//!
//! Read on demand rather than at startup, so edits apply to the next
//! connection without restarting REAPER.

use crate::log::log;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Web origins allowed to connect, compared exactly
    /// (scheme://host[:port]).
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

pub fn load() -> Config {
    let Some(path) = crate::discovery::base_dir().map(|dir| dir.join("config.json")) else {
        return Config::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Config::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        log!("ignoring {}: {error}", path.display());
        Config::default()
    })
}

/// Whether a WebSocket handshake may proceed, given its `Origin` header.
///
/// Browsers always send `Origin` and pages can't forge it, while native
/// clients normally send none. Without this check, any web page could
/// connect to 127.0.0.1 and drive REAPER, because browsers don't apply
/// cross-origin rules to WebSockets.
pub fn origin_allowed(origin: Option<&str>, allowed: &[String]) -> bool {
    match origin {
        None => true,
        Some(origin) => allowed.iter().any(|entry| entry.eq_ignore_ascii_case(origin)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| entry.to_string()).collect()
    }

    #[test]
    fn native_clients_send_no_origin() {
        assert!(origin_allowed(None, &[]));
    }

    #[test]
    fn web_origins_need_an_exact_entry() {
        let list = allowed(&["http://localhost:5173"]);
        assert!(!origin_allowed(Some("https://evil.example"), &[]));
        assert!(origin_allowed(Some("http://localhost:5173"), &list));
        assert!(origin_allowed(Some("HTTP://LOCALHOST:5173"), &list));
        assert!(!origin_allowed(Some("http://localhost:5174"), &list));
        assert!(!origin_allowed(Some("http://localhost:5173.evil.example"), &list));
        assert!(!origin_allowed(Some("null"), &list));
    }

    #[test]
    fn parses_config() {
        let config: Config =
            serde_json::from_str(r#"{"allowedOrigins": ["http://localhost:5173"]}"#).unwrap();
        assert_eq!(config.allowed_origins, ["http://localhost:5173"]);
        let empty: Config = serde_json::from_str("{}").unwrap();
        assert!(empty.allowed_origins.is_empty());
    }
}
