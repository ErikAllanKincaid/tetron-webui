# tetron-web

A browser dashboard and admin console for [tetron](https://github.com/ErikAllanKincaid/tetron), a P2P mesh VPN. Talks to the existing `tetron` daemon over its Unix-socket IPC protocol — no daemon changes required.

**Optional and separate from tetron on purpose.** tetron itself follows the Unix "do one thing well" philosophy and stays CLI-only by default; `tetron-web` is a genuinely separate, opt-in product for anyone who wants a browser interface, not a bundled or default-installed component. Nothing about tetron's own behavior changes whether this exists or not.

## Running it

```bash
cargo run
```

Then open `http://127.0.0.1:7870`. Requires a running `tetron` daemon (`sudo tetron up`) reachable at its usual Unix socket. Read-only status works for any local user; mutating actions (create/join/leave/kick/nuke/etc.) require the browsing user to be tetron's configured operator (`sudo tetron set-operator <user>`) or root — same authorization model the CLI uses.

## What it does

- Live dashboard: which networks you're on, who's connected, traffic stats.
- Network lifecycle: create, join, leave.
- Invites: mint, list, revoke.
- Coordinator actions: kick a member, grant/list co-coordinators, destroy a network (with the same consensus/force safeguards the CLI has).

## Architecture

```
Browser --HTTP (127.0.0.1 only)--> tetron-web --msgpack/Unix socket--> tetron daemon
```

No daemon-side changes. Depends on `tetron-proto` (tetron's shared wire-protocol crate) as a git dependency, floating on `main` rather than pinned to a release tag — see the comment in `Cargo.toml` for why.

## Design history

- [`docs/DESIGN.md`](docs/DESIGN.md) — the UI/UX decisions (information hierarchy, theming, layout, confirmation UX) and the reasoning behind each.
- [`docs/PLAN.md`](docs/PLAN.md) — the implementation plan this was built from.
- [`docs/IDEAS_WebUI_Systray.md`](docs/IDEAS_WebUI_Systray.md) — the original scoping doc for this and `tetron-tray` (not yet built).
- [`TODO.md`](TODO.md) — known rough edges and planned follow-ups.

## License

MPL-2.0, matching tetron itself.
