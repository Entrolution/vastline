//! The vast.ai client. Network I/O is delegated to the system `curl` so the binary needs no
//! TLS stack (deps stay at serde, matching quotaline). This module is only ever reached from
//! the `refresh` command — never from the render path — so a slow API can't stall the prompt.
//!
//! Two endpoints, both GET with `Authorization: Bearer <key>`:
//!   * `/instances/`      → `{ "instances": [ { actual_status, dph_total, ... }, ... ] }`
//!   * `/users/current/`  → `{ "credit": <float>, ... }`
//!
//! The needed scopes are `instance_read` + `user_read` (see `key::MINT_CMD`).

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::config::api_base;
use crate::json::{as_f64_loose, f64_at, nested, str_at};

/// Per-request timeout handed to curl. Generous — vast.ai can be slow — but bounded so a wedged
/// network can't leave a `refresh` child hanging forever.
const CURL_MAX_TIME_SECS: u32 = 8;

/// What a single GET produced: parsed JSON, or a human-readable error.
///
/// The bearer token is fed to curl on **stdin** via `-H @-`, never on the command line — so the
/// secret never appears in `ps`/`/proc/<pid>/cmdline` where other local users could read it. The
/// URL is fine on argv (it carries no secret).
fn get_json(url: &str, key: &str) -> Result<Value, String> {
    let mut child = Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            &CURL_MAX_TIME_SECS.to_string(),
            "-H",
            "@-", // read the Authorization header from stdin (keeps the key off argv)
            "-H",
            "Accept: application/json",
            url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run curl: {e}"))?;

    // `-H @-` reads header lines verbatim from stdin; write the one header, then close stdin
    // (dropping the handle) so curl proceeds.
    if let Some(mut sin) = child.stdin.take() {
        let _ = sin.write_all(format!("Authorization: Bearer {key}\n").as_bytes());
    }

    let out = child
        .wait_with_output()
        .map_err(|e| format!("curl did not complete: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = stderr.lines().last().unwrap_or("request failed").trim();
        return Err(format!("curl: {msg}"));
    }
    serde_json::from_slice::<Value>(&out.stdout).map_err(|e| format!("bad JSON from {url}: {e}"))
}

/// A flattened snapshot of the account, ready to cache. All money is USD.
pub struct Snapshot {
    pub running: u32,
    pub total: u32,
    /// Sum of `dph_total` over instances that are actually running (compute + their storage).
    pub burn_running: f64,
    /// Sum of `dph_total` over *all* instances — includes storage still billing while stopped.
    pub burn_all: f64,
    /// Account credit, if the user endpoint returned it.
    pub balance: Option<f64>,
}

/// Fetch and flatten both endpoints. Returns the first error encountered.
pub fn fetch(key: &str) -> Result<Snapshot, String> {
    let base = api_base();
    let instances = get_json(&format!("{base}/instances/"), key)?;
    let user = get_json(&format!("{base}/users/current/"), key)?;
    Ok(flatten(&instances, &user))
}

/// True when an instance's status string means "currently running and billing compute".
fn is_running(status: Option<&str>) -> bool {
    matches!(status, Some(s) if s.eq_ignore_ascii_case("running"))
}

/// Pull the per-hour cost off an instance, tolerating the field being absent.
fn instance_dph(inst: &Value) -> f64 {
    inst.get("dph_total")
        .and_then(as_f64_loose)
        .unwrap_or(0.0)
        .max(0.0)
}

/// Combine the two responses into a `Snapshot`. Pure, so it is unit-testable on fixtures.
pub fn flatten(instances: &Value, user: &Value) -> Snapshot {
    let list = nested(instances, &["instances"])
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut running = 0u32;
    let mut burn_running = 0.0;
    let mut burn_all = 0.0;
    for inst in &list {
        let dph = instance_dph(inst);
        burn_all += dph;
        // `actual_status` is the live state; `cur_state` is the requested one — prefer actual.
        let status = str_at(inst, &["actual_status"]).or_else(|| str_at(inst, &["cur_state"]));
        if is_running(status) {
            running += 1;
            burn_running += dph;
        }
    }

    // Balance lives under a few possible names depending on account/endpoint version.
    let balance = f64_at(user, &["credit"])
        .or_else(|| f64_at(user, &["balance"]))
        .or_else(|| f64_at(user, &["credit_balance"]));

    Snapshot {
        running,
        total: list.len() as u32,
        burn_running,
        burn_all,
        balance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flatten_mixed_fleet() {
        let instances = json!({
            "instances": [
                { "actual_status": "running", "dph_total": 1.20 },
                { "actual_status": "running", "dph_total": 0.64 },
                { "actual_status": "exited",  "dph_total": 0.05 }, // stopped: storage only
            ]
        });
        let user = json!({ "credit": 47.20 });
        let s = flatten(&instances, &user);
        assert_eq!(s.running, 2);
        assert_eq!(s.total, 3);
        assert!((s.burn_running - 1.84).abs() < 1e-9);
        assert!((s.burn_all - 1.89).abs() < 1e-9);
        assert_eq!(s.balance, Some(47.20));
    }

    #[test]
    fn flatten_empty_and_missing_fields() {
        let s = flatten(&json!({ "instances": [] }), &json!({}));
        assert_eq!(s.running, 0);
        assert_eq!(s.total, 0);
        assert_eq!(s.burn_running, 0.0);
        assert_eq!(s.balance, None);
    }

    #[test]
    fn flatten_falls_back_to_cur_state_and_balance_alias() {
        let instances = json!({ "instances": [ { "cur_state": "running", "dph_total": "0.50" } ] });
        let user = json!({ "balance": 10.0 });
        let s = flatten(&instances, &user);
        assert_eq!(s.running, 1);
        assert!((s.burn_running - 0.50).abs() < 1e-9);
        assert_eq!(s.balance, Some(10.0));
    }
}
