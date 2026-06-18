//! Builds the single vast.ai status line from the cached snapshot, then (if configured) prefixes
//! the output of the *base* status-line command we delegate to — see `install.rs`. This is what
//! lets vastline sit "on top of" quotaline without either tool knowing about the other.
//!
//! Example line (running compute and stopped-storage burn shown as separate components, runway
//! from their total):
//!   vast  1/2 up · run $0.57/hr · store $0.01/hr · bal $47.20 · ~25h
//!
//! Degrades gracefully: no key → a one-line hint; stale/failed fetch → dimmed with a marker;
//! empty fleet → `vast  idle · bal $47.20`; everything stopped → `vast  0/1 up · store $0.01/hr …`.

use std::io::Read;
use std::process::{Command, Stdio};

use crate::burn::runway_hours;
use crate::cache::{read as read_cache, spawn_refresh_if_stale, State};
use crate::config::{now_secs, state_dir};
use crate::fmt::{fmt_hours, fmt_money, fmt_rate, runway_color, DIM, GRAY, GREEN, RED, RESET};
use crate::install::base_command;

/// Entry point for the default (no-arg) invocation: print the base line(s), then the vast line.
pub fn run_statusline() -> i32 {
    // Claude Code pipes a JSON session payload on stdin. vastline itself doesn't need it, but the
    // base command (quotaline) does — so capture it once and forward it verbatim.
    let mut stdin_payload = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin_payload);

    let mut out = String::new();
    if let Some(base) = base_command() {
        if let Some(base_out) = run_base(&base, &stdin_payload) {
            out.push_str(&base_out);
            if !base_out.ends_with('\n') {
                out.push('\n');
            }
        }
    }

    let now = now_secs();
    let dir = state_dir();
    let state = read_cache(&dir);
    // Kick a background refresh for next time if the snapshot is stale; never blocks this render.
    spawn_refresh_if_stale(&dir, state.as_ref(), now);

    out.push_str(&line(state.as_ref(), now));
    out.push('\n');

    // Single write, errors ignored — if the consumer closes the pipe early (e.g. `vastline | head`)
    // we exit quietly instead of panicking on a broken pipe mid-output.
    use std::io::Write;
    let _ = std::io::stdout().write_all(out.as_bytes());
    0
}

/// Run the captured base status-line command, feeding it the same stdin Claude Code gave us.
/// Returns its stdout, or `None` if it could not be launched.
fn run_base(command: &str, stdin_payload: &str) -> Option<String> {
    // Loop-breaker: never delegate to ourselves. If a poisoned base.json (e.g. from a path-changed
    // reinstall) points back at vastline, executing it would recurse without bound.
    if crate::install::looks_like_self(command) {
        return None;
    }
    let mut child = shell(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Feed stdin on a separate thread: if the base command emits enough stdout to fill the pipe
    // before draining its stdin, a single-threaded write_all + wait_with_output could deadlock.
    if let Some(mut sin) = child.stdin.take() {
        use std::io::Write;
        let payload = stdin_payload.to_owned();
        std::thread::spawn(move || {
            let _ = sin.write_all(payload.as_bytes());
            // sin drops here, closing the child's stdin so it sees EOF.
        });
    }
    let out = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Build a shell command that runs `command` the way Claude Code's statusLine would, so a base
/// command with arguments/quoting behaves identically when we delegate to it.
#[cfg(unix)]
fn shell(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command);
    c
}

#[cfg(not(unix))]
fn shell(command: &str) -> Command {
    let mut c = Command::new("cmd");
    c.arg("/C").arg(command);
    c
}

const LABEL: &str = "vast";

/// Render just the vast line (no base prefix). Pure given a snapshot + clock, for testing.
pub fn line(state: Option<&State>, now: f64) -> String {
    let label = format!("{DIM}{LABEL}{RESET}");

    let Some(s) = state else {
        // No cache yet — either never refreshed or no key. Keep it quiet and actionable.
        return match crate::key::resolve() {
            Some(_) => format!("{label}  {GRAY}fetching…{RESET}"),
            None => format!("{label}  {GRAY}no API key — run `vastline key set`{RESET}"),
        };
    };

    // A failed-but-never-succeeded snapshot (no good numbers to fall back on).
    if !s.ok && s.fetched_at == 0.0 {
        let msg = s.error.as_deref().unwrap_or("unavailable");
        return format!("{label}  {RED}{msg}{RESET}");
    }

    let mut segs: Vec<String> = Vec::new();

    if s.total == 0 {
        segs.push(format!("{GREEN}idle{RESET}"));
    } else {
        segs.push(format!("{}/{} up", s.running, s.total));
        // Running compute burn (includes those instances' storage), shown only when something runs.
        if s.running > 0 {
            segs.push(format!("{DIM}run{RESET} {}", fmt_rate(s.burn_running)));
        }
        // Storage still billing on stopped-but-not-destroyed instances — a separate, much smaller
        // drain. `dph_total` stays at the full rate when stopped, so this is computed from storage.
        if s.burn_stopped > 5e-4 {
            segs.push(format!("{DIM}store{RESET} {}", fmt_rate(s.burn_stopped)));
        }
    }

    if let Some(bal) = s.balance {
        let bal_col = if bal <= 0.0 { RED } else { GREEN };
        segs.push(format!(
            "{DIM}bal{RESET} {bal_col}{}{RESET}",
            fmt_money(bal)
        ));
    }

    // Runway from the TOTAL drain (running + stopped storage) — what actually empties the wallet.
    if let Some(hrs) = runway_hours(s.balance, s.burn_total()) {
        segs.push(format!(
            "{}~{}{RESET}",
            runway_color(Some(hrs)),
            fmt_hours(hrs)
        ));
    }

    let body = segs.join(&format!("{DIM} · {RESET}"));
    let mut out = format!("{label}  {body}");

    // Staleness / error markers, appended dim so the numbers stay readable.
    if !s.ok {
        out.push_str(&format!("  {RED}⚠ stale{RESET}"));
    } else if s.is_expired(now) {
        out.push_str(&format!("  {GRAY}(stale){RESET}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(s: &str) -> String {
        // Crude ANSI stripper for assertions.
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for d in chars.by_ref() {
                    if d == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn state(running: u32, total: u32, run: f64, stopped: f64, bal: Option<f64>) -> State {
        State {
            fetched_at: 1000.0,
            last_attempt: 1000.0,
            ok: true,
            error: None,
            running,
            total,
            burn_running: run,
            burn_stopped: stopped,
            balance: bal,
        }
    }

    #[test]
    fn all_running_shows_run_and_runway_from_total() {
        // 1 A100 up: store burn is 0, runway from running burn alone.
        let s = state(1, 1, 0.57, 0.0, Some(15.63));
        let got = strip(&line(Some(&s), 1010.0));
        assert_eq!(got, "vast  1/1 up · run $0.57/hr · bal $15.63 · ~27h");
    }

    #[test]
    fn stopped_instance_shows_storage_not_compute() {
        // The live-tested case: instance stopped → no compute burn, only tiny storage, long runway.
        let s = state(0, 1, 0.0, 0.0089, Some(15.62));
        let got = strip(&line(Some(&s), 1010.0));
        assert!(got.contains("0/1 up"), "{got}");
        assert!(
            !got.contains("run "),
            "no compute burn when nothing runs: {got}"
        );
        assert!(got.contains("store $0.01/hr"), "{got}");
        // 15.62 / 0.0089 ≈ 1755h ≈ 73 days — not the misleading ~27h.
        assert!(got.contains("~73d"), "{got}");
    }

    #[test]
    fn mixed_fleet_shows_both_components() {
        let s = state(1, 2, 0.57, 0.0089, Some(15.62));
        let got = strip(&line(Some(&s), 1010.0));
        assert!(
            got.contains("1/2 up · run $0.57/hr · store $0.01/hr"),
            "{got}"
        );
    }

    #[test]
    fn idle_fleet() {
        let s = state(0, 0, 0.0, 0.0, Some(47.20));
        let got = strip(&line(Some(&s), 1010.0));
        assert_eq!(got, "vast  idle · bal $47.20");
    }

    #[test]
    fn no_cache_no_key_hint() {
        // With no key configured and no cache, we hint at setup.
        let got = strip(&line(None, 1010.0));
        assert!(got.contains("vast"));
    }
}
