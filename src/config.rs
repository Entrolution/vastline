//! Filesystem locations and small shared helpers. vastline keeps two things on disk:
//!
//!   * a config dir (`~/.config/vastline/`) — the stored API key and the captured "base"
//!     status-line block we delegate to (see `install.rs`);
//!   * a state dir (`~/.claude/vastline/`) — the cached API snapshot (see `cache.rs`).
//!
//! Both can be relocated with env vars so tests and odd setups never touch the real ones.

use std::env;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const VAST_API_BASE_DEFAULT: &str = "https://console.vast.ai/api/v0";

pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Config dir: `$VASTLINE_CONFIG_DIR`, else `~/.config/vastline`.
pub fn config_dir() -> PathBuf {
    if let Some(d) = env::var_os("VASTLINE_CONFIG_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    match home_dir() {
        Some(h) => h.join(".config").join("vastline"),
        None => PathBuf::from(".config-vastline"),
    }
}

/// State/cache dir: `$VASTLINE_STATE_DIR`, else `~/.claude/vastline` (next to quotaline's).
pub fn state_dir() -> PathBuf {
    if let Some(d) = env::var_os("VASTLINE_STATE_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    match home_dir() {
        Some(h) => h.join(".claude").join("vastline"),
        None => PathBuf::from(".state"),
    }
}

/// The vast.ai API base, overridable with `$VAST_URL` (the same var the official CLI honours).
pub fn api_base() -> String {
    match env::var("VAST_URL") {
        Ok(u) if !u.trim().is_empty() => u.trim().trim_end_matches('/').to_string(),
        _ => VAST_API_BASE_DEFAULT.to_string(),
    }
}

/// Current Unix time in seconds (fractional). 0 if the clock predates the epoch.
pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
