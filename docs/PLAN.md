> **Provenance note (added when this was copied into `tetron-web`'s own
> repo, 2026-07-19):** written during a `Plan Mode` session in the `tetron`
> repo, before `tetron-web` existed at all. Two things changed between this
> plan being approved and what actually got built, both worth flagging
> rather than silently editing away:
>
> 1. **Repo location.** This plan (and the "Workspace change" section
>    below) still describes `tetron-web`/`tetron-tray` as members of
>    `tetron`'s own Cargo workspace, isolated from `reconcile.py`'s default
>    gate via `default-members`. That was revised the same day, before
>    implementation started: `tetron-web` became this fully separate repo
>    instead, with `tetron-proto` pulled in as a **git dependency floating
>    on `tetron`'s `main` branch**, not a path dependency. See
>    `IDEAS_WebUI_Systray.md` in this same directory for the reasoning
>    that led to that correction.
> 2. **Scope actually built.** This plan only covers **Phase 1** of
>    `tetron-web` (read-only dashboard). Erik later said "build as much as
>    you can... all the phases" and handed off full autonomy — what
>    actually shipped in the first commit was all three phases (read-only,
>    low-stakes mutations, full admin surface with confirmation UX), live
>    tested against real daemons across two machines including a full
>    NUKE-CONSENSUS propose/second/destroy cycle. The spec-first workflow
>    described below (`spec/design_spec.py` Requirement classes,
>    `reconcile.py`, `libspec link`) is `tetron`'s own convention and was
>    **not** followed here — this repo has no spec-driven gate of its own
>    (yet; could be added later if wanted, nothing stops it).
>
> The `tetron-tray` section below (Part 2) is unstarted — kept here as-is
> since it's still the plan for when that work actually begins, just not
> yet executed.

# WebUI + Systray addons — Phase 1 implementation plan

## Context

tetron's CLI is the only client today. Two optional addon clients have been sketched in `DO-NOT-COMMIT/IDEAS_WebUI_Systray.md` (the authoritative design doc, already decided): `tetron-web` (a browser dashboard for headless/remote administration) and `tetron-tray` (a native menu-bar icon for glanceable status + one-click toggle on a machine with an active desktop session). Erik confirmed he uses a laptop as his daily driver (so tray has real day-to-day value) but wants the WebUI built first since it is the more influential of the two — its roadmap grows toward full CLI parity (create/join/invite management/kick/admin-add/nuke), while tray is permanently capped at status + a couple of safe self-service actions by deliberate design (matches Tailscale/Mullvad/ProtonVPN's own tray icons — destructive actions never live in a menu-bar click). Systray follows second, for the smaller, faster, immediately-gratifying win.

Research already done (two Explore agents + one Plan agent, 2026-07-18) confirmed the entire client-side foundation already exists and is directly reusable: `tetron-proto/src/ipc.rs` has the full `IpcMessage` enum, the `MsgpackCodec` wire framing, and ready-to-use `connect()`/`send()`/`recv()`/`socket_path()` helpers, with a genuinely light dependency footprint (no daemon-only TUN/DNS/blob/crypto deps leak in). A new crate needs only `tetron-proto = { path = "../tetron-proto" }` to get the whole client stack. This plan only implements **Phase 1** of each addon (per the ideas doc's own phasing) — the smallest useful, buildable slice, not the full multi-phase roadmap.

**Order of operations, confirmed:** `tetron-web` Phase 1 first (spec entry → implement → `reconcile.py` green → commit → `libspec link`, this project's unbroken spec-first convention), then `tetron-tray` Phase 1 the same way. No technical dependency runs between them — this is a value-sequencing choice, not a build constraint.

## Workspace change (do this once, before either addon)

`Cargo.toml`'s `[workspace]` currently reads `members = ["tetron-proto"]`. `reconcile.py` runs bare `cargo build`/`cargo clippy --all-targets -- -D warnings`/`cargo test` (no `-p` flag), which builds every workspace member by default — so adding `tetron-web`/`tetron-tray` as members would pull them into the same gate that verifies the core daemon on every commit, including unrelated ones. `tray-icon` in particular needs GTK3 + AppIndicator dev headers on Linux at build time, a genuinely new system-dependency class for this repo.

**Confirmed: isolate them.** Add:
```toml
[workspace]
members = ["tetron-proto", "tetron-web", "tetron-tray"]
default-members = ["."]
```
Bare `cargo build`/`clippy`/`test` (what `reconcile.py` runs) then only cover the root `tetron` package (which still transitively covers `tetron-proto`). Addons build explicitly via `cargo build -p tetron-web` / `-p tetron-tray` / `--workspace`. Verify empirically after adding: run `cargo build --quiet` from repo root and confirm `target/debug/` does not gain `tetron-web`/`tetron-tray` binaries.

Add both crates to the workspace in the same commit that adds the first addon (`tetron-web`), so `tetron-tray`'s crate can be scaffolded as an empty/stub member later without a second workspace-file edit — or add `tetron-tray` to `members` only when its own work actually starts, whichever reads cleaner at the time; either is fine since `default-members` already isolates both from the default gate regardless of listing order.

## Part 1 — `tetron-web` Phase 1 (build first)

**Scope:** read-only dashboard, `Status` polling only, zero mutating IPC calls. Validates plumbing before anything destructive is ever exposed through a browser.

### Spec entry (this project's convention — do first)

Add a `Requirement` class to `spec/design_spec.py` (e.g. `WebUiReadOnlyDashboard`, `REQUIREMENT-ID: WEBUI-001`) describing: new `tetron-web` binary, read-only `Status`-only dashboard, `127.0.0.1`-bound, no daemon changes. `reconcile.py`'s existing checks do not need new logic for this — it is a structural/design requirement verified by reading the diff, not a new automatable constraint.

### Files to create

- `tetron-web/Cargo.toml`:
  ```toml
  [package]
  name = "tetron-web"
  version = "0.1.0"
  edition = "2024"
  rust-version = "1.91"
  license = "MPL-2.0"
  description = "Read-only browser dashboard for tetron, over the daemon IPC socket"

  [[bin]]
  name = "tetron-web"
  path = "src/main.rs"

  [dependencies]
  tetron-proto = { path = "../tetron-proto" }
  axum = "0.8"
  tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
  serde_json = "1"
  ```
  (Treat the `tokio` feature list as a starting guess — prune/adjust against real compiler errors.)

- `tetron-web/src/main.rs` — two routes: `GET /` returns a static HTML shell (`html::INDEX_HTML`), `GET /api/status` returns JSON built by polling the daemon once per request:
  ```rust
  #[tokio::main]
  async fn main() -> anyhow::Result<()> {
      let app = axum::Router::new()
          .route("/", axum::routing::get(index))
          .route("/api/status", axum::routing::get(api_status));
      let listener = tokio::net::TcpListener::bind("127.0.0.1:7870").await?;
      eprintln!("tetron-web listening on http://127.0.0.1:7870");
      axum::serve(listener, app).await?;
      Ok(())
  }
  ```

- `tetron-web/src/status.rs` — `pub async fn poll_daemon() -> serde_json::Value`: opens `tetron_proto::ipc::connect()`, sends `IpcMessage::Status`, matches the response the same way `ipc_status()` does in `src/cli/status.rs` (reuse that exact match shape — `StatusResponse { .. }` / `Error { message }` / unexpected variant). Any failure (connect error, `Error` response, unexpected variant) collapses to `{"reachable": false, "message": "..."}` rather than an HTTP error — the browser branches on `reachable` client-side, not HTTP status. This is also the entire "reconnect after daemon restart" story: each poll independently reconnects, so there is no persistent-connection state to go stale.

- `tetron-web/src/html.rs` — `pub const INDEX_HTML: &str = r#"..."#`: static page + inline `<script>` doing `fetch('/api/status')` every ~2s via `setInterval`, updating the DOM via `document.createElement`/`.textContent` (not `innerHTML` string interpolation, so a hostname with HTML-special characters cannot inject markup). Zero templating crate, zero SPA framework — matches the ideas doc's explicit "vanilla-JS polling loop" decision for a single-page Phase 1.

### Dashboard content (source: `StatusResponse`/`NetworkStatus`/`PeerStatus` in `tetron-proto/src/ipc.rs`, mirror `print_network()` in `src/cli/status.rs` field-for-field)

- Header: `endpoint_id` (short form), `active` (up/standby), `daemon_version`.
- Per network (`NetworkStatus`): `name`, `role`, `my_ip`/`my_ipv6`, `my_hostname`, `member_count`, `tun_name`, `active` (per-network standby, `STANDBY-PER-NETWORK`), `nuke_proposals` (surfaced as text only, no action buttons).
- Per peer (`PeerStatus`): `hostname` or `ip`; `connection` present → `conn_type`/`rtt_ms`/`bytes_tx`/`bytes_rx`, else "offline".
- Top-level traffic counters (`packets_rx`/`packets_tx`/`bytes_rx`/`bytes_tx`) and `pending_networks`.

### Running (Phase 1, no install step)

`cargo run -p tetron-web`, open `http://127.0.0.1:7870`. No systemd unit, no `tetron install --with-web` flag — explicitly out of scope for Phase 1.

### Verification

1. Baseline: run `tetron status` in a terminal, note endpoint short id, networks/roles, peers, traffic counters.
2. `cargo run -p tetron-web`; confirm the startup line prints the bind address.
3. Open the browser; confirm every field from step 1 matches exactly and updates on its own every ~2s with no manual reload.
4. While `tetron-web` keeps running, `sudo tetron restart` the daemon; confirm the dashboard shows unreachable within one poll cycle and recovers automatically once the socket reappears — no `tetron-web` restart needed.
5. `grep -n "IpcMessage::" tetron-web/src/*.rs` — confirm only `IpcMessage::Status` is ever sent, zero mutating calls.
6. `reconcile.py` green (root package only, per the `default-members` isolation — confirm `cargo build -p tetron-web` and `cargo clippy -p tetron-web -- -D warnings` also pass on their own, since those are not covered by the default gate but should still be clean).

### Commit

Spec entry + implementation in one commit (or spec-then-implement as two, matching whatever granularity this project's other single-requirement commits use), `reconcile.py` green, `libspec link`.

## Part 2 — `tetron-tray` Phase 1 (build second, not started yet)

**Scope:** icon reflects up/active vs. down/standby vs. daemon-unreachable (via `Status` polling), tooltip shows active network count + peer count, click toggles `Up`/`Down`.

### Before starting: verify Linux build prerequisites

`tray-icon`/`tao` need GTK3 dev headers + an AppIndicator package on Linux (`libgtk-3-dev` and `libayatana-appindicator3-dev` or `libappindicator3-dev`, distro-dependent) at build time. Check with `pkg-config --exists gtk+-3.0` before `cargo build -p tetron-tray`; install via the distro package manager if missing. This is a genuinely new system-dependency class for this repo (everything else is pure Rust + `tun`/`rtnetlink`).

### Spec entry

Add a `Requirement` class to `spec/design_spec.py` (e.g. `TrayStatusAndToggle`, `REQUIREMENT-ID: TRAY-001`) describing: new `tetron-tray` binary, status-reflecting icon + tooltip, click-to-toggle `Up`/`Down`, no daemon changes, Linux + macOS only (no Windows — matches `config_dir()`'s existing platform scope in `src/config.rs`).

*(This assumes tetron-tray lives in the tetron repo's own spec-driven workspace, per the original plan. Given tetron-web ended up in its own separate repo with no spec/reconcile.py convention, tetron-tray will likely follow the same pattern — a separate repo, not a tetron-proto path dependency — when that work actually starts. Revisit this section then.)*

### Files to create

- `tetron-tray/Cargo.toml`:
  ```toml
  [package]
  name = "tetron-tray"
  version = "0.1.0"
  edition = "2024"
  rust-version = "1.91"
  license = "MPL-2.0"
  description = "Native system-tray status + toggle for tetron, over the daemon IPC socket"

  [[bin]]
  name = "tetron-tray"
  path = "src/main.rs"

  [dependencies]
  tetron-proto = { path = "../tetron-proto" }
  tray-icon = "0.24"
  tao = "0.35"
  tokio = { version = "1", features = ["rt", "time"] }
  anyhow = "1"
  ```
  (Versions confirmed current via docs.rs at plan time — `tray-icon` 0.24.1, `tao` 0.35.3, still the recommended pairing per `tray-icon`'s own docs, using `TrayIconEvent::set_event_handler()` + an `EventLoopProxy` to forward clicks into the host event loop. Re-check exact API shapes against `cargo doc --open -p tray-icon -p tao` before writing code — the snippets below are directionally correct, reconstructed from a docs summary, not verbatim-verified against source.)

- `tetron-tray/src/main.rs` — owns the `tao` event loop on the main thread. A background thread runs its own single-threaded tokio runtime driving `poll::run()`, forwarding `TrayUserEvent::Poll(...)` into the event loop via `EventLoopProxy`. `TrayIconEvent::set_event_handler` forwards left-clicks as `TrayUserEvent::ToggleClicked`. On click, spawn a short-lived thread + one-shot tokio runtime to call `toggle::send()` and post `TrayUserEvent::ToggleResult(...)` back — keeps the toggle round-trip off both the GUI thread and the polling loop's runtime, consistent with this codebase's existing "one connection per request" convention.

  Add a minimal one-item context menu ("Toggle up/down") as a click-reliability fallback alongside bare left-click — `tray-icon` on Linux (StatusNotifierItem/AppIndicator) has historically inconsistent left-click "activate" support across desktop environments; the menu item triggers the identical toggle action, no new IPC calls.

- `tetron-tray/src/poll.rs` — `pub async fn run(proxy: EventLoopProxy<TrayUserEvent>)`: loop of `ipc::connect()` → `Status` → `recv()` → build `PollResult` (mirror `ipc_status()`'s match shape from `src/cli/status.rs`; any failure collapses to `PollResult::Unreachable`) → `proxy.send_event(...)` → `tokio::time::sleep(Duration::from_secs(3))`.

- `tetron-tray/src/toggle.rs` — `pub async fn send(go_up: bool) -> Result<String, String>`: sends `IpcMessage::Up { hostname: None, network: None }` or `IpcMessage::Down { network: None }` (matching `src/cli/service.rs`'s existing `Up` call shape). On `IpcMessage::Error { message }`, returns `Err(message)` — this surfaces the daemon's real permission-denied text verbatim when the running user is not the operator (`check_authorized` in `src/daemon/mod.rs`: *"permission denied: this user is not authorized to control tetron... Grant access with: sudo tetron set-operator \<user\>"*), so the tray shows the actual actionable hint rather than a generic error, and does not crash or hang on denial.

- `tetron-tray/src/icon.rs` — `pub fn make(state: State) -> Result<tray_icon::Icon>` with `State::Active`/`Standby`/`Unreachable`: procedurally generate a flat-color filled circle as an RGBA byte buffer (32×32, simple distance-from-center test), pass to `Icon::from_rgba(...)`. Zero new asset files, zero image-decoding dependency — tetron ships no square tray-shaped icon asset today.

### Authorization note (surface this, do not silently assume it works)

A `tetron-tray` process running as a normal (non-root, non-operator) user gets read-only `Status` polling for free, but the toggle click will fail with a permission-denied error until that user is granted operator status (`sudo tetron set-operator <uid>`, or automatically already granted if they ran `sudo tetron up`/`install` themselves). Phase 1 must handle this gracefully (show the error in the tooltip via `toggle::send`'s `Err` path above), not crash.

### Running (Phase 1, no install step)

`cargo run -p tetron-tray`, inside an actual GUI session (a headless SSH shell has no tray host to attach to). No systemd/launchd unit, no login-item/autostart registration — explicitly out of scope for Phase 1.

### Verification

1. Confirm build prerequisites (`pkg-config --exists gtk+-3.0` + appindicator package on Linux) before first build.
2. `cargo run -p tetron-tray` in a real desktop session; confirm an icon appears.
3. With the daemon stopped/unreachable, confirm icon+tooltip reflect "unreachable" within one poll interval.
4. `sudo tetron up`; confirm the icon flips to active/standby matching `tetron status` output from a terminal at the same moment.
5. Click (or use the fallback menu item) to toggle; confirm icon+tooltip update, then independently confirm via `tetron status` in a terminal that the daemon's actual state changed.
6. Run as a non-operator user, click toggle, confirm the tray shows the permission-denied hint gracefully (no crash/hang) and `tetron status` confirms the daemon's state is unchanged.
7. `reconcile.py` green for the root package (isolated via `default-members`); separately confirm `cargo build -p tetron-tray` and `cargo clippy -p tetron-tray -- -D warnings` pass on their own.

### Commit

Spec entry + implementation, `reconcile.py` green, `libspec link` — same discipline as Part 1.

## Explicitly out of scope for this plan (do not build)

- Any mutating WebUI feature beyond Phase 1 (create/join/invite/kick/admin-add/nuke) — Phase 2/3 of the ideas doc, not this plan. *(Note: built anyway per Erik's later "all the phases" instruction — see the provenance note at the top of this file.)*
- Any tray menu item beyond status + toggle (per-network submenu, copy-invite-key, notifications, WebUI hand-off) — Phase 2/3.
- systemd/launchd units, login-item/autostart registration, or any `tetron install --with-web`/`--with-tray` convenience flag for either addon.
- Kick, admin add, and nuke will never be added to the tray menu at any phase — this is a permanent design constraint, not a Phase 1 limitation.
