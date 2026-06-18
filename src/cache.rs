//! The on-disk snapshot that keeps the network off the render path.
//!
//! `render` reads `state.json` and prints from it *immediately*, then — if the snapshot is
//! older than `TTL` — spawns a detached `vastline refresh` to update it for next time. So the
//! prompt never blocks on vast.ai; the displayed numbers are at most `TTL + one render tick`
//! stale, which for a billing readout is fine.
//!
//! A short-lived lock file stops a burst of 10-second render ticks from spawning a pile of
//! overlapping refreshes (a "thundering herd") while one fetch is already in flight.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::api::Snapshot;
use crate::config::{now_secs, state_dir};

/// How long a snapshot is considered fresh, in seconds. Balance/burn move slowly; 60s is plenty
/// and keeps us far under any vast.ai rate limit even with several Claude sessions open. It also
/// doubles as the retry interval after a *failed* fetch (see `refresh_due`), so a broken key
/// backs off to one attempt per minute rather than one per render tick.
pub const TTL_SECS: f64 = 60.0;

/// Past this age the displayed numbers are marked "(stale)" — a 30-minute-old reading is still
/// shown (better than nothing) but flagged so it isn't mistaken for live.
pub const MAX_AGE_SECS: f64 = 30.0 * 60.0;

/// Reclaim a lock older than this — covers a refresh child that died before clearing it. Must
/// stay comfortably above the curl timeout (`api::CURL_MAX_TIME_SECS`, 8s) so a slow-but-alive
/// fetch isn't treated as crashed mid-flight.
const LOCK_GRACE_SECS: f64 = 20.0;

#[derive(Serialize, Deserialize, Clone)]
pub struct State {
    /// Unix seconds of the last *successful* fetch — drives the displayed data age / stale marker.
    pub fetched_at: f64,
    /// Unix seconds of the last fetch *attempt*, success or failure — drives refresh backoff so a
    /// persistent error doesn't re-spawn every render tick. Defaults to 0.0 for snapshots written
    /// before this field existed, which forces one refresh on the first read after upgrade.
    #[serde(default)]
    pub last_attempt: f64,
    /// Did the last fetch succeed? On failure we keep the previous numbers but flag the error.
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub running: u32,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub burn_running: f64,
    /// Storage-only $/hr over stopped instances (see `api::Snapshot::burn_stopped`).
    #[serde(default)]
    pub burn_stopped: f64,
    #[serde(default)]
    pub balance: Option<f64>,
}

impl State {
    pub fn from_snapshot(s: &Snapshot, now: f64) -> Self {
        State {
            fetched_at: now,
            last_attempt: now,
            ok: true,
            error: None,
            running: s.running,
            total: s.total,
            burn_running: s.burn_running,
            burn_stopped: s.burn_stopped,
            balance: s.balance,
        }
    }

    /// Total $/hr draining the balance now: running compute + stopped storage.
    pub fn burn_total(&self) -> f64 {
        self.burn_running + self.burn_stopped
    }

    /// Age of the last *successful* data in seconds relative to `now` (never negative).
    pub fn age(&self, now: f64) -> f64 {
        (now - self.fetched_at).max(0.0)
    }

    /// Whether a refresh is due, throttled by the last *attempt* (not the last success) so a
    /// run of failures retries at most once per `TTL`, never once per render tick.
    pub fn refresh_due(&self, now: f64) -> bool {
        (now - self.last_attempt).max(0.0) > TTL_SECS
    }

    pub fn is_expired(&self, now: f64) -> bool {
        self.age(now) > MAX_AGE_SECS
    }
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

fn lock_path(dir: &Path) -> PathBuf {
    dir.join("refresh.lock")
}

/// Read the cached snapshot, or `None` if absent/unreadable/corrupt.
pub fn read(dir: &Path) -> Option<State> {
    let text = fs::read_to_string(state_path(dir)).ok()?;
    serde_json::from_str::<State>(&text).ok()
}

/// Atomically persist a snapshot (per-process temp + rename, like quotaline's history writer).
pub fn write(dir: &Path, state: &State) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(state).unwrap_or_default();
    let tmp = dir.join(format!("state.{}.json.tmp", std::process::id()));
    fs::write(&tmp, json)?;
    if let Err(e) = fs::rename(&tmp, state_path(dir)) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Record a failed fetch without discarding the last good numbers: keep the prior snapshot's
/// values but stamp the error and advance `last_attempt` so the next render backs off instead of
/// re-spawning immediately. `fetched_at` is intentionally NOT advanced — the displayed age still
/// reflects the last *good* data.
pub fn write_error(dir: &Path, message: &str, now: f64) {
    let mut state = read(dir).unwrap_or(State {
        fetched_at: 0.0,
        last_attempt: 0.0,
        ok: true,
        error: None,
        running: 0,
        total: 0,
        burn_running: 0.0,
        burn_stopped: 0.0,
        balance: None,
    });
    state.ok = false;
    state.error = Some(message.to_string());
    state.last_attempt = now;
    let _ = write(dir, &state);
}

/// Age of the lock file in seconds (`None` if absent/unreadable), measured against `now`.
fn lock_age(dir: &Path, now: f64) -> Option<f64> {
    let mtime = fs::metadata(lock_path(dir))
        .and_then(|m| m.modified())
        .ok()?;
    let secs = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    Some((now - secs).max(0.0))
}

/// Try to acquire the refresh lock atomically (O_EXCL via `create_new`). Returns true only for the
/// single caller that created it — concurrent renders on the same tick can't all win. A lock older
/// than `LOCK_GRACE_SECS` (a crashed/killed refresh) is reclaimed first.
fn acquire_lock(dir: &Path, now: f64) -> bool {
    let _ = fs::create_dir_all(dir);
    if let Some(age) = lock_age(dir, now) {
        if age >= LOCK_GRACE_SECS {
            let _ = fs::remove_file(lock_path(dir));
        }
    }
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path(dir))
    {
        Ok(mut f) => {
            let _ = write!(f, "{now}");
            true
        }
        Err(_) => false, // someone else holds it (or a fresh lock exists) → don't spawn
    }
}

/// Spawn a detached `vastline refresh` if a refresh is due and we win the lock. Best-effort and
/// non-blocking: failure to spawn just means we try again next render tick.
pub fn spawn_refresh_if_stale(dir: &Path, state: Option<&State>, now: f64) {
    let due = match state {
        None => true,
        Some(s) => s.refresh_due(now),
    };
    if !due {
        return;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    // Atomic acquire: only the winner spawns; the lock also serializes against an in-flight fetch.
    if !acquire_lock(dir, now) {
        return;
    }
    if Command::new(exe)
        .arg("refresh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_err()
    {
        // Don't leave an orphaned lock blocking the next attempt if the spawn itself failed.
        let _ = fs::remove_file(lock_path(dir));
    }
}

/// `vastline refresh` — fetch from the API and rewrite the cache. Synchronous by design: it is
/// what the detached child (or a manual run) executes. Always clears the lock on the way out.
pub fn run_refresh() -> i32 {
    let dir = state_dir();
    let now = now_secs();
    let _ = fs::create_dir_all(&dir);
    // (Re)stamp the lock so a manual `vastline refresh` also serializes, and a long fetch keeps
    // the herd guard alive while it runs.
    let _ = fs::write(lock_path(&dir), format!("{now}"));

    let code = match crate::key::resolve() {
        None => {
            write_error(&dir, "no API key", now);
            eprintln!("vastline: no API key (run `vastline key set`)");
            1
        }
        Some(r) => match crate::api::fetch(&r.key) {
            Ok(snap) => {
                let state = State::from_snapshot(&snap, now);
                if let Err(e) = write(&dir, &state) {
                    eprintln!("vastline: could not write cache: {e}");
                    1
                } else {
                    0
                }
            }
            Err(e) => {
                write_error(&dir, &e, now);
                eprintln!("vastline: refresh failed: {e}");
                1
            }
        },
    };

    let _ = fs::remove_file(lock_path(&dir));
    code
}
