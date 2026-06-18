//! vastline — a Claude Code status line for vast.ai GPU usage.
//!
//!   vastline                       render the status line (default; reads Claude's JSON stdin)
//!   vastline refresh               fetch the API and rewrite the cache (run by the bg refresh)
//!   vastline status                show resolved key + a live fetch, for debugging
//!   vastline key set [KEY]         store a read-only API key (prompted/stdin if omitted)
//!   vastline key path              show which key would be used and from where
//!   vastline key clear             remove vastline's stored key
//!   vastline install [--refresh N] wire into ~/.claude/settings.json (delegates to any existing
//!                                  status line, e.g. quotaline)
//!   vastline uninstall [--purge]   restore the previous status line; --purge also drops key+cache
//!
//! Network I/O happens only in `refresh` (via curl); the render path reads a cache, so the
//! prompt never blocks on vast.ai.

mod api;
mod burn;
mod cache;
mod config;
mod fmt;
mod install;
mod json;
mod key;
mod render;

const USAGE: &str = "\
vastline — Claude Code status line for vast.ai usage

USAGE:
  vastline                        render the status line (reads Claude Code's JSON on stdin)
  vastline refresh                fetch the vast.ai API and rewrite the cache
  vastline status                 show the resolved API key and a live fetch (debug)
  vastline key set [KEY]          store a read-only API key (prompted if KEY omitted)
  vastline key path               show which key would be used and from where
  vastline key clear              remove vastline's stored key
  vastline install [--refresh N]  wire into ~/.claude/settings.json (default refresh 10s)
  vastline uninstall [--purge]    restore the previous status line; --purge drops key+cache
";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(String::as_str) {
        None => render::run_statusline(),
        Some("refresh") => cache::run_refresh(),
        Some("status") => status(),
        Some("key") => match args.get(2).map(String::as_str) {
            Some("set") => key::set(args.get(3).map(String::as_str)),
            Some("path") | Some("show") => key::show(),
            Some("clear") | Some("rm") => key::clear(),
            _ => {
                eprintln!("usage: vastline key <set [KEY]|path|clear>");
                2
            }
        },
        Some("install") => {
            if has_flag(&args, "--refresh") && flag_u64(&args, "--refresh").is_none() {
                eprintln!("vastline: --refresh needs a positive integer (seconds); using 10s");
            }
            let refresh = flag_u64(&args, "--refresh").unwrap_or(10);
            install::install(refresh)
        }
        Some("uninstall") => install::uninstall(has_flag(&args, "--purge")),
        Some("-h") | Some("--help") | Some("help") => {
            print!("{USAGE}");
            0
        }
        Some(other) => {
            eprintln!("vastline: unknown command '{other}'\n");
            eprint!("{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

/// `vastline status` — a human-readable diagnostic: where the key came from and what a live
/// fetch returns right now. Bypasses the cache so it's useful for confirming a new key works.
fn status() -> i32 {
    match key::resolve() {
        None => {
            println!("api key: NONE");
            println!("  set one with: vastline key set");
            println!("  mint read-only: {}", key::MINT_CMD);
            1
        }
        Some(r) => {
            println!(
                "api key: {} (from {})",
                key::mask(&r.key),
                r.source.describe()
            );
            print!("fetching… ");
            match api::fetch(&r.key) {
                Ok(s) => {
                    println!("ok");
                    println!("  instances:    {} running / {} total", s.running, s.total);
                    println!(
                        "  burn running: {} (running compute)",
                        fmt::fmt_rate(s.burn_running)
                    );
                    println!(
                        "  burn stopped: {} (storage on stopped instances)",
                        fmt::fmt_rate(s.burn_stopped)
                    );
                    println!("  burn total:   {}", fmt::fmt_rate(s.burn_total()));
                    match s.balance {
                        Some(b) => println!("  balance:      {}", fmt::fmt_money(b)),
                        None => println!("  balance:      (not returned — check user_read scope)"),
                    }
                    if let Some(h) = burn::runway_hours(s.balance, s.burn_total()) {
                        println!("  runway:       ~{} at total burn", fmt::fmt_hours(h));
                    }
                    0
                }
                Err(e) => {
                    println!("FAILED");
                    eprintln!("  {e}");
                    1
                }
            }
        }
    }
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn flag_u64(args: &[String], name: &str) -> Option<u64> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).and_then(|s| s.parse::<u64>().ok())
}
