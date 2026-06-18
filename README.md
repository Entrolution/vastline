# vastline

A Claude Code **status line** for [vast.ai](https://vast.ai) GPU usage. It shows how many
instances are up, your **running** vs **total** burn rate, your account balance, and how long
that balance lasts at the current burn — as one extra line under whatever status line you
already run (e.g. [quotaline](https://github.com/Entrolution/quotaline)).

```
Opus · effort: high · ctx 12% (98k)        ← quotaline (delegated to, unchanged)
5h  ███████░░░░  68%  2h05m                 ← quotaline
wk  ████░░░░░░░  41%  3d4h                  ← quotaline
vast  2/3 up · run $1.84/hr · all $1.89/hr · bal $47.20 · ~25h   ← vastline
```

It reads only two vast.ai endpoints with a **read-only scoped key**, and does it off the render
path so your prompt never blocks on the network.

## What it shows

- **`2/3 up`** — running instances / total instances.
- **`run $1.84/hr`** — burn rate of the instances that are actually *running* (compute + their
  storage).
- **`all $1.89/hr`** — burn rate across *all* instances, **including storage still billing on
  stopped instances**. Shown only when it differs from running burn (i.e. you have stopped-but-
  not-destroyed instances quietly costing money).
- **`bal $47.20`** — account credit (red if ≤ 0).
- **`~25h`** — runway: how long the balance lasts at the **total** burn rate. Amber under 12h,
  red under 4h. Computed from *total* burn deliberately — an idle fleet still bleeds storage,
  and a runway based on running burn alone would read as falsely infinite while money leaks.

Degrades quietly: no key → a one-line setup hint; an empty fleet → `vast  idle · bal $47.20`;
a stale or failed refresh → the last good numbers, dimmed, with a marker.

## Why running vs total burn are both shown

vast.ai bills storage even when an instance is *stopped*. So "what am I spending right now on
compute" (running burn) and "what is actually draining my wallet" (total burn) are different
numbers, and conflating them hides idle storage cost. vastline shows both and bases the runway
on the honest one.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/Entrolution/vastline/main/install.sh | bash
```

This downloads the binary to `~/.local/bin` and runs `vastline install`, which wires it into
`~/.claude/settings.json`. If you already have a status line (like quotaline), vastline
**captures it and delegates to it** — your existing line keeps working, with the vast line added
underneath. (Windows: use `install.ps1`.)

Then add a **read-only** API key:

```sh
# Mint a least-privilege key (needs the vast CLI once; read-only scopes only):
vastai create api-key --name vastline \
  --permissions '{"api": {"instance_read": {}, "user_read": {}}}'

vastline key set      # paste the key when prompted (never touches shell history)
```

If you already use the `vastai` CLI, vastline will reuse its key automatically — `key set` is
optional.

## Uninstall

```sh
vastline uninstall            # restores your previous status line verbatim; keeps key + cache
vastline uninstall --purge    # also removes the stored key, captured base, and cache
```

Both back up `settings.json` first and restore the exact block vastline captured at install
time — so removing vastline leaves quotaline (or whatever you had) exactly as it was.

## Commands

| Command | Purpose |
|---|---|
| `vastline` | Render the status line (default; reads Claude Code's JSON on stdin). |
| `vastline refresh` | Fetch the API and rewrite the cache (run automatically in the background). |
| `vastline status` | Show the resolved key and a live fetch — for confirming a new key works. |
| `vastline key set [KEY]` | Store a read-only key (prompted/stdin if omitted; chmod 600). |
| `vastline key path` | Show which key would be used and from where. |
| `vastline key clear` | Remove vastline's stored key (leaves the vast CLI's key alone). |
| `vastline install [--refresh N]` | Wire into `settings.json` (default refresh 10s). |
| `vastline uninstall [--purge]` | Restore the previous status line; `--purge` drops key + cache. |

## How it stays off the render path

Claude Code runs the status-line command every few seconds. vastline's render **never** calls
the network: it reads a cached snapshot (`~/.claude/vastline/state.json`) and prints instantly.
If that snapshot is older than 60s it spawns a detached `vastline refresh` to update it for next
time, guarded by a short-lived lock so a burst of render ticks can't stampede the API. The only
thing that talks to vast.ai is `refresh`, via the system `curl` — so there's no TLS stack linked
into the binary, and deps stay limited to serde.

## Key resolution

First match wins:

1. `$VAST_API_KEY` — environment (CI / ephemeral shells).
2. `~/.config/vastline/vast_api_key` — what `vastline key set` writes.
3. `~/.config/vastai/vast_api_key` — the official CLI's key, reused if present.

`vastline key path` always tells you which one is in effect, so a stale env var can't silently
shadow the key you think you set.

## Configuration (env)

| Variable | Effect |
|---|---|
| `VAST_API_KEY` | API key (highest-priority source). |
| `VAST_URL` | Override the API base (default `https://console.vast.ai/api/v0`). |
| `VASTLINE_CONFIG_DIR` | Override `~/.config/vastline` (key + captured base). |
| `VASTLINE_STATE_DIR` | Override `~/.claude/vastline` (cache). |
| `CLAUDE_SETTINGS` | Override the `settings.json` path install/uninstall edit. |

## Security

vastline is designed around a **read-only scoped key**: `instance_read` + `user_read`, nothing
else. A leaked key of this scope exposes your instance list and balance — it cannot spin up
instances, spend money, or destroy anything. Don't point vastline at your account-wide master
key. The key file is written `chmod 600`.

## License

MIT — see [LICENSE](LICENSE).
