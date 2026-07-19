# HOWTO: build a browser dashboard for a local Unix-socket daemon

A generic, instructional writeup of the pattern `tetron-webui` is built on
— a thin HTTP server that proxies a browser to an existing local daemon's
IPC socket, with a static, framework-free frontend. Useful if you're
building something similar (a browser UI for a CLI tool/daemon that
otherwise has no web presence), not just historical context for this one
repo. Every claim below points at either a real file in this repo or a
real external doc, so you can go verify or copy directly rather than
taking it on faith.

Companion piece: [`tetron-systray`'s own
HOWTO](https://github.com/ErikAllanKincaid/tetron-systray/blob/main/docs/HOWTO_Build_A_Systray.md)
covers the same daemon from a menu-bar/tray angle instead — the IPC client
pattern and per-user-service deployment sections overlap heavily with this
one; read whichever matches what you're building, or both if you want the
full picture.

## The pattern, in one diagram

```
Browser --HTTP (127.0.0.1 only)--> web server --your IPC protocol--> daemon
```

The web server is a translator, not where any real logic lives. It holds
no state of its own beyond what's needed to relay a request and format a
response — the daemon is still the single source of truth, exactly as it
was before the web server existed. This matters: it means the web server
can crash, restart, or be skipped entirely (use the CLI instead) without
the daemon ever noticing or caring.

## 1. Reuse your existing wire protocol — don't invent a second one

If your daemon already has a CLI that talks to it over some IPC channel,
that protocol almost certainly already has everything a web server needs:
a request/response enum, and a framing scheme (how one message's bytes end
and the next begin). Pull that into a **shared library crate** both the
CLI and the web server depend on, rather than reimplementing wire framing
in the web server.

- Real example: [`tetron-proto`](https://github.com/ErikAllanKincaid/tetron/tree/main/tetron-proto) — the `IpcMessage` enum, plus
  [`MsgpackCodec`](https://github.com/ErikAllanKincaid/tetron/blob/main/tetron-proto/src/ipc.rs)
  (length-prefixed [msgpack](https://msgpack.org/) via
  [`rmp-serde`](https://docs.rs/rmp-serde/latest/rmp_serde/) +
  [`tokio_util::codec::LengthDelimitedCodec`](https://docs.rs/tokio-util/latest/tokio_util/codec/struct.LengthDelimitedCodec.html)),
  and `connect()`/`send()`/`recv()` helpers over a Unix socket.
- Depend on it as a **git dependency** if the daemon and web server live in
  separate repos (see this repo's own `Cargo.toml`) — floating on the
  daemon's main branch, not a pinned tag, is a deliberate choice: the real
  compatibility risk is which *binaries* are running together at runtime,
  not what was pinned at build time. Use `#[serde(default)]` on wire-type
  fields consistently so an older/newer client and daemon still
  interoperate instead of hard-failing on an unknown/missing field.

## 2. The IPC client: one connection per request

Don't hold a persistent connection to the daemon open in the web server.
Open a fresh one per request, send one message, read one reply, close it.

```rust
pub async fn call(msg: IpcMessage) -> Result<IpcMessage, String> {
    let mut stream = ipc::connect().await.map_err(|e| format!("could not reach the daemon: {e}"))?;
    ipc::send(&mut stream, msg).await.map_err(|e| format!("failed to send: {e}"))?;
    ipc::recv(&mut stream).await.map_err(|e| format!("failed to read reply: {e}"))
}
```

Real example: [`src/ipc_client.rs`](../src/ipc_client.rs) in this repo.
Why this is worth doing deliberately rather than defaulting to a
connection pool: a daemon restart is never something the web server has to
detect and specially recover from — the *next* request just reconnects
fresh and either works or reports "daemon unreachable." No reconnection
state machine, no stale-connection edge cases.

## 3. The HTTP layer: never turn a daemon-level rejection into an HTTP error

Two genuinely different failure modes exist, and collapsing them into one
HTTP status code loses information the frontend needs:

1. **Could not reach the daemon at all** (not running, socket gone). A real
   infrastructure problem.
2. **The daemon understood the request and said no** (e.g. "permission
   denied," "network not found"). Not an error in the HTTP sense — the
   request worked, the answer was just "no."

Represent both as `200 OK` with a JSON body describing which one happened
(e.g. `{"reachable": false, "message": "..."}` vs.
`{"ok": false, "error": "..."}`), and let frontend JS branch on the body,
not on HTTP status. Real example:
[`src/api.rs`](../src/api.rs)'s doc comments explain this for every
handler — the frontend gets to treat "no" as a normal, renderable answer
instead of a special error-handling path.

Framework: [`axum`](https://docs.rs/axum/latest/axum/) — a `Router`, a
handler function per route returning `Json<T>`, nothing fancier needed for
this shape of app. **Bind `127.0.0.1` only**, deliberately — no
remote/WAN exposure by default; anyone wanting remote access should put a
reverse proxy with real TLS + auth in front of this themselves. That's a
decision this kind of tool shouldn't make on a user's behalf by defaulting
to a public bind address.

## 4. The frontend: vanilla JS, no build step, no framework

For a dashboard this size (poll an endpoint, render a list, handle a
handful of button clicks), a SPA framework is overhead with no payoff —
extra dependency surface, a build step, a bundler config, none of which
buys anything a `fetch()` + `setInterval()` polling loop and
`document.createElement` don't already do directly.

- Embed the static assets at compile time with `include_str!` — no
  file-serving dependency, no separate deploy step for `static/`. Real
  example: [`src/main.rs`](../src/main.rs)'s `INDEX_HTML`/`STYLE_CSS`/`APP_JS` consts.
- Build the DOM with `document.createElement`/`.textContent`, not
  `innerHTML` string interpolation — a hostname or network name containing
  HTML-special characters can't inject markup this way. Real example:
  [`static/app.js`](../static/app.js).
- Reconnect-after-daemon-restart is free: since the client just polls on
  an interval and each poll independently reconnects (per point 2 above),
  there is no persistent connection on the frontend side to go stale
  either — a poll just fails once, then succeeds again once the daemon's
  socket reappears.

## 5. UI/UX principles that generalize beyond this one dashboard

Worked out for this repo's own frontend — the parts that aren't specific
to tetron:

- **Information hierarchy in three tiers**: glance (one line, "is
  everything okay," answerable in under a second, color-coded) → scan (one
  row per item, never more than a glance's worth of reading per row) → dig
  in (full detail, hidden by default, shown on request). Applies to any
  dashboard for a set of similar things (networks, servers, jobs, whatever
  your domain's "row" is).
- **Semantic color, not decorative color**: 3-4 colors max for status
  meaning, each one meaning exactly one thing, never reused for anything
  that isn't actually that status.
- **Real light/dark theming, not "designed dark, light bolted on later"**:
  every color as a named CSS custom property, referenced by name
  everywhere, never a raw hex value inline. Light mode is a second
  value-set for the exact same names under `:root[data-theme="light"]` —
  there is only ever one design, expressed twice, so a dark-only
  assumption can't hide anywhere in the CSS.
- **A real confirmation modal for destructive actions, never a bare
  `confirm()`** — too easy to reflexively click through, and can't carry
  situation-specific detail (what's about to happen, whether it's
  reversible).

## 6. Deploying it: pre-built binaries + a per-user service, not `cargo run`

Most people who'd use a tool like this don't have a Rust toolchain and
shouldn't need one.

- **GitHub Actions release workflow**: tag push (`vX.Y.Z`) builds binaries
  for each target platform, attaches them + sha256 checksums to a GitHub
  release. Real example:
  [`.github/workflows/release.yml`](../.github/workflows/release.yml) in
  this repo (matrix build across Linux x86_64/aarch64 + macOS
  aarch64/x86_64, using
  [`dtolnay/rust-toolchain`](https://github.com/dtolnay/rust-toolchain),
  [`Swatinem/rust-cache`](https://github.com/Swatinem/rust-cache), and
  [`softprops/action-gh-release`](https://github.com/softprops/action-gh-release)).
- **A per-user service, not a terminal kept open**: `systemd --user` on
  Linux, a launchd **LaunchAgent** on macOS (distinct from a system-wide
  LaunchDaemon — an Agent runs inside the user's own login session, no
  root needed, which is the right fit for something that talks to a
  daemon's socket as an unprivileged client). Real example:
  [`src/service.rs`](../src/service.rs) +
  [`contrib/tetron-webui.service`](../contrib/tetron-webui.service) /
  [`contrib/com.tetron.webui.plist`](../contrib/com.tetron.webui.plist) —
  `install`/`uninstall` subcommands that write the unit/plist (substituting
  the real binary path at install time), enable it, and wait for the
  server to actually come up before declaring success.
  - If the same idea applies to a **GUI** app rather than a headless HTTP
    server (a tray icon, say), the systemd side needs one more piece of
    care: `graphical-session.target` is the semantically correct
    `WantedBy=` dependency, but it's not universally activated — some
    desktop environments (Cinnamon, XFCE) never wire it up for the
    systemd user manager, only GNOME/KDE reliably do. List **both**
    `WantedBy=default.target graphical-session.target` so it starts
    either way. Found live-testing a sibling project
    ([`tetron-systray`](https://github.com/ErikAllanKincaid/tetron-systray)) —
    doesn't apply to this repo's own headless HTTP server, but will bite
    the next GUI-shaped tool that copies this pattern if not carried over.

## References

- [`axum`](https://docs.rs/axum/latest/axum/) — the HTTP framework.
- [`tokio`](https://docs.rs/tokio/latest/tokio/) — async runtime.
- [`rmp-serde`](https://docs.rs/rmp-serde/latest/rmp_serde/) — msgpack serde support.
- [`tokio-util`'s `LengthDelimitedCodec`](https://docs.rs/tokio-util/latest/tokio_util/codec/struct.LengthDelimitedCodec.html) — length-prefixed framing over a stream.
- [`clap`](https://docs.rs/clap/latest/clap/) — CLI arg parsing (used here for the `install`/`uninstall` subcommands).
- [`dirs`](https://docs.rs/dirs/latest/dirs/) — cross-platform config/home directory resolution (used for the per-user service's install paths).
- [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) / [Semantic Versioning](https://semver.org/spec/v2.0.0.html) — if you add a `CHANGELOG.md` and want release notes with more structure than GitHub's auto-generated summary (this repo doesn't have one yet — see `.github/workflows/release.yml`'s own comment on why release notes are auto-generated for now).
