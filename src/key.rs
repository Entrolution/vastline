//! API-key management. The whole point of vastline is a *read-only scoped* key, so this module
//! resolves, stores, and clears it — and always tells the user exactly which source won, so a
//! stale env var or leftover file can never silently shadow the key they think they set.
//!
//! Resolution order (first hit wins):
//!   1. `$VAST_API_KEY`                     — env, for CI / ephemeral shells
//!   2. `~/.config/vastline/vast_api_key`   — what `vastline key set` writes
//!   3. `~/.config/vastai/vast_api_key`     — reuse the official CLI's key if present
//!
//! The recommended key is scoped to read-only; `install`/`status` print the exact mint command.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use crate::config::{config_dir, home_dir};

/// The exact command that mints the least-privilege key vastline needs.
pub const MINT_CMD: &str = r#"vastai create api-key --name vastline --permissions '{"api": {"instance_read": {}, "user_read": {}}}'"#;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    VastlineFile,
    VastaiFile,
}

impl Source {
    pub fn describe(self) -> &'static str {
        match self {
            Source::Env => "$VAST_API_KEY (environment)",
            Source::VastlineFile => "~/.config/vastline/vast_api_key",
            Source::VastaiFile => "~/.config/vastai/vast_api_key (vast CLI)",
        }
    }
}

pub struct Resolved {
    pub key: String,
    pub source: Source,
}

fn vastline_key_path() -> PathBuf {
    config_dir().join("vast_api_key")
}

fn vastai_key_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".config").join("vastai").join("vast_api_key"))
}

fn read_key_file(p: &PathBuf) -> Option<String> {
    let raw = fs::read_to_string(p).ok()?;
    let k = raw.trim().to_string();
    if k.is_empty() {
        None
    } else {
        Some(k)
    }
}

/// Resolve the key from the first available source, or `None` if nothing is configured.
pub fn resolve() -> Option<Resolved> {
    if let Ok(k) = std::env::var("VAST_API_KEY") {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return Some(Resolved {
                key: k,
                source: Source::Env,
            });
        }
    }
    if let Some(k) = read_key_file(&vastline_key_path()) {
        return Some(Resolved {
            key: k,
            source: Source::VastlineFile,
        });
    }
    if let Some(p) = vastai_key_path() {
        if let Some(k) = read_key_file(&p) {
            return Some(Resolved {
                key: k,
                source: Source::VastaiFile,
            });
        }
    }
    None
}

/// `vastline key set [KEY]` — store the key (0600) in vastline's config dir. With no argument,
/// read it from stdin so it never lands in shell history.
pub fn set(arg: Option<&str>) -> i32 {
    let key = match arg {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            let mut buf = String::new();
            let read = if atty_stdin() {
                // Interactive: read a single line so a paste + Enter returns immediately
                // (read_to_string would block until Ctrl-D / EOF, which the prompt can't ask for).
                eprint!("paste vast.ai read-only API key (then Enter): ");
                let _ = std::io::Write::flush(&mut std::io::stderr());
                std::io::stdin().read_line(&mut buf).map(|_| ())
            } else {
                // Piped (`echo $KEY | vastline key set`): consume the whole stream.
                std::io::stdin().read_to_string(&mut buf).map(|_| ())
            };
            if read.is_err() {
                eprintln!("error: could not read key from stdin");
                return 1;
            }
            buf.trim().to_string()
        }
    };
    if key.is_empty() {
        eprintln!("error: empty key — nothing stored");
        return 1;
    }
    let dir = config_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("error: cannot create {}: {e}", dir.display());
        return 1;
    }
    let path = vastline_key_path();
    if let Err(e) = write_key_file(&path, &key) {
        eprintln!("error: cannot write {}: {e}", path.display());
        return 1;
    }
    chmod_600(&path); // narrows an already-existing file that create() left at its old mode
    println!("stored key → {}", path.display());
    println!("tip: this should be a read-only key. Mint one with:\n  {MINT_CMD}");
    0
}

/// `vastline key clear` — remove vastline's stored key (leaves the vast CLI's key alone).
pub fn clear() -> i32 {
    let path = vastline_key_path();
    match fs::remove_file(&path) {
        Ok(()) => {
            println!("removed {}", path.display());
            0
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("nothing to do: {} not found", path.display());
            0
        }
        Err(e) => {
            eprintln!("error: cannot remove {}: {e}", path.display());
            1
        }
    }
}

/// `vastline key path` — report what would be used and from where.
pub fn show() -> i32 {
    match resolve() {
        Some(r) => {
            let masked = mask(&r.key);
            println!("key in use: {masked}");
            println!("source:     {}", r.source.describe());
            0
        }
        None => {
            println!("no API key configured.");
            println!("set one with:  vastline key set");
            println!("mint read-only: {MINT_CMD}");
            1
        }
    }
}

/// Mask a key for display: keep a short prefix, hide the rest. Anything short enough that a prefix
/// would reveal a meaningful fraction (≤ 12 chars, e.g. a test key) is fully starred.
pub fn mask(key: &str) -> String {
    let n = key.chars().count();
    if n <= 12 {
        return "*".repeat(n);
    }
    let prefix: String = key.chars().take(6).collect();
    format!("{prefix}…({n} chars)")
}

fn atty_stdin() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

/// Write the key file. On unix it is *created* with mode 0600 so the secret is never momentarily
/// world-readable (the old `fs::write` + chmod left a race window where it existed as 0644).
#[cfg(unix)]
fn write_key_file(path: &PathBuf, key: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(format!("{key}\n").as_bytes())
}

#[cfg(not(unix))]
fn write_key_file(path: &PathBuf, key: &str) -> std::io::Result<()> {
    fs::write(path, format!("{key}\n"))
}

#[cfg(unix)]
fn chmod_600(path: &PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn chmod_600(_path: &PathBuf) {
    // Windows ACLs differ; the file lands under the user profile, which is already per-user.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking() {
        assert_eq!(mask(""), "");
        assert_eq!(mask("abcd"), "****");
        assert_eq!(mask("0123456789ab"), "************"); // 12 chars → fully starred
        assert_eq!(mask("0123456789abcdef"), "012345…(16 chars)");
    }
}
