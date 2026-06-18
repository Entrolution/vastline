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

/// A flattened snapshot of the account, ready to cache. All money is USD per hour (except balance).
pub struct Snapshot {
    pub running: u32,
    pub total: u32,
    /// Sum of `dph_total` over instances that are actually running (compute + their storage).
    pub burn_running: f64,
    /// Sum of the storage-only rate over *stopped* instances. vast.ai keeps `dph_total` at the
    /// full running rate even when an instance is exited, so a stopped instance is NOT billed its
    /// compute rate — only storage. The honest total drain is `burn_running + burn_stopped`.
    pub burn_stopped: f64,
    /// Account credit, if the user endpoint returned it.
    pub balance: Option<f64>,
}

impl Snapshot {
    /// Total $/hr actually draining the balance right now: running compute (incl. their storage)
    /// plus storage on stopped-but-not-destroyed instances.
    pub fn burn_total(&self) -> f64 {
        self.burn_running + self.burn_stopped
    }
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

/// Hours per month vast.ai uses to amortise the monthly storage rate (30 × 24).
const HOURS_PER_MONTH: f64 = 720.0;

/// The full running per-hour cost of an instance (`dph_total`: compute + storage + bandwidth),
/// tolerating the field being absent.
fn instance_dph(inst: &Value) -> f64 {
    inst.get("dph_total")
        .and_then(as_f64_loose)
        .unwrap_or(0.0)
        .max(0.0)
}

/// The per-hour storage cost still billing while an instance is *stopped*. `dph_total` must NOT be
/// used here — vast.ai keeps it at the full running rate even when the instance is exited. Prefer
/// the precomputed `storage_total_cost` ($/hr); fall back to `storage_cost` ($/GB/month) ×
/// `disk_space` (GB) ÷ 720.
fn stopped_storage_dph(inst: &Value) -> f64 {
    if let Some(c) = inst.get("storage_total_cost").and_then(as_f64_loose) {
        return c.max(0.0);
    }
    let rate = inst
        .get("storage_cost")
        .and_then(as_f64_loose)
        .unwrap_or(0.0);
    let gb = inst.get("disk_space").and_then(as_f64_loose).unwrap_or(0.0);
    (rate * gb / HOURS_PER_MONTH).max(0.0)
}

/// Combine the two responses into a `Snapshot`. Pure, so it is unit-testable on fixtures.
pub fn flatten(instances: &Value, user: &Value) -> Snapshot {
    let list = nested(instances, &["instances"])
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut running = 0u32;
    let mut burn_running = 0.0;
    let mut burn_stopped = 0.0;
    for inst in &list {
        // `actual_status` is the live state; `cur_state` is the requested one — prefer actual.
        let status = str_at(inst, &["actual_status"]).or_else(|| str_at(inst, &["cur_state"]));
        if is_running(status) {
            running += 1;
            burn_running += instance_dph(inst); // dph_total already includes this one's storage
        } else {
            burn_stopped += stopped_storage_dph(inst); // storage only — NOT dph_total
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
        burn_stopped,
        balance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flatten_mixed_fleet() {
        // A stopped instance keeps its full dph_total (vast.ai quirk) but only bills storage —
        // so burn_stopped must come from storage_total_cost, NOT dph_total.
        let instances = json!({
            "instances": [
                { "actual_status": "running", "dph_total": 1.20 },
                { "actual_status": "running", "dph_total": 0.64 },
                { "actual_status": "exited",  "dph_total": 0.57, "storage_total_cost": 0.01 },
            ]
        });
        let user = json!({ "credit": 47.20 });
        let s = flatten(&instances, &user);
        assert_eq!(s.running, 2);
        assert_eq!(s.total, 3);
        assert!((s.burn_running - 1.84).abs() < 1e-9);
        assert!(
            (s.burn_stopped - 0.01).abs() < 1e-9,
            "stopped uses storage, not dph_total"
        );
        assert!((s.burn_total() - 1.85).abs() < 1e-9);
        assert_eq!(s.balance, Some(47.20));
    }

    #[test]
    fn stopped_instance_ignores_dph_total() {
        // The real bug live-testing caught: a stopped A100 still reports dph_total≈0.57, but the
        // honest drain is storage only (~0.009/hr).
        let instances = json!({
            "instances": [
                { "actual_status": "exited", "dph_total": 0.5689, "storage_total_cost": 0.0089 },
            ]
        });
        let s = flatten(&instances, &json!({ "credit": 15.0 }));
        assert_eq!(s.running, 0);
        assert_eq!(s.total, 1);
        assert_eq!(s.burn_running, 0.0);
        assert!((s.burn_stopped - 0.0089).abs() < 1e-9);
    }

    #[test]
    fn stopped_storage_falls_back_to_rate_times_disk() {
        // No storage_total_cost → compute from storage_cost ($/GB/month) × disk_space ÷ 720.
        let instances = json!({
            "instances": [
                { "actual_status": "exited", "storage_cost": 0.20, "disk_space": 32.0 },
            ]
        });
        let s = flatten(&instances, &json!({}));
        assert!((s.burn_stopped - (0.20 * 32.0 / 720.0)).abs() < 1e-9);
    }

    #[test]
    fn flatten_empty_and_missing_fields() {
        let s = flatten(&json!({ "instances": [] }), &json!({}));
        assert_eq!(s.running, 0);
        assert_eq!(s.total, 0);
        assert_eq!(s.burn_running, 0.0);
        assert_eq!(s.burn_stopped, 0.0);
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
