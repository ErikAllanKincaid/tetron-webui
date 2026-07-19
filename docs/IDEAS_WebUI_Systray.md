> **Provenance note (added when this was copied into `tetron-web`'s own
> repo, 2026-07-19):** this is the original scoping doc, written before any
> code existed, when `tetron-web` and `tetron-tray` were still going to be
> two crates in `tetron`'s own Cargo workspace. That specific piece of the
> decision below ("Why still the same workspace, not fully separate
> repos") was **revised the next day, before implementation started**: both
> addons ended up as fully separate repos instead, one level more separate
> than this doc argued for — see `PLAN.md` in this same directory for the
> plan that was actually built from, and `tetron`'s own
> `DO-NOT-COMMIT/TODO.md` ("WebUI addon" section) for the short version of
> why that changed. Everything else below (MVP phasing, tech stack, the
> together-vs-separate-*artifact* reasoning, non-goals) is still accurate
> to what got built. Kept here verbatim, mistake and all, rather than
> quietly edited, since the reasoning that led to the correction is itself
> worth having on record.

# WebUI and Systray addons — scoping and together-vs-separate

Planning doc captured 2026-07-18, superseding the two standalone sketches in
`TODO.md` ("WebUI addon", "Systray addon"). Scope-only — no implementation
started. Written at Erik's request to answer one specific question (together
or separate) plus give both addons a real MVP boundary instead of an open
feature list.

---

## Decision: separate crates and binaries, same Cargo workspace

**Build them as two independent workspace members** (`tetron-web`,
`tetron-tray`), each its own binary, each its own install/deployment story.
Not one combined addon, not one shared binary with two modes.

**Why separate:**

- **Different tech stacks with almost no real overlap.** WebUI is an HTTP
  server + a browser frontend. Systray is a native GUI toolkit binding
  (tray icon, OS menu, desktop notifications). The only thing they actually
  share is "connect to the IPC socket and speak `IpcMessage`" — a handful of
  lines against the already-shared `tetron-proto` crate, not a reason to
  entangle the rest.
- **Different lifecycle models.** The WebUI needs to be up whenever anyone
  might load the page — that's a system service, its own systemd unit/
  launchd plist, running regardless of who's logged in (same shape as the
  `tetron` daemon itself). A tray icon is inherently a per-user desktop
  session thing — it starts when you log into a GUI session and has no
  meaning on a headless box. Forcing these into one artifact means shipping
  service-manager plumbing a tray icon has no use for, or GUI-toolkit
  dependencies a headless server has no use for.
- **Different users, not always the same person.** A homelab operator
  running tetron on a headless server wants the WebUI (remote dashboard,
  no desktop session to put a tray icon in at all). A laptop user wants
  the tray's glanceable status and one-click toggle, and may never open
  the WebUI. Coupling them forces both audiences to take a dependency they
  don't want.
- **Matches tetron's own ethos.** The whole `MINIMAL-*` series stripped
  tetron down to "do one thing well" per-component, not one artifact doing
  several loosely related things. Two small, focused optional tools fits
  that better than one addon wearing two hats.

**Why still the same workspace, not fully separate repos:** both are
genuinely thin IPC clients over `tetron-proto`'s `IpcMessage`/`MsgpackCodec`,
which already exists and versions together with the daemon's wire format.
Keeping them as workspace members (`Cargo.toml`'s `[workspace] members`,
currently just `tetron-proto`) means `cargo build --workspace` builds
everything, dependency/version bumps to the wire format are caught by the
compiler across all three binaries at once, and there's no separate-repo
version-skew problem (a `tetron-web` built against a stale `tetron-proto`
silently drifting from what the daemon actually speaks). Separate crates,
one repo, one shared wire-format dependency — not separate deployment
artifacts pretending to be one thing.

*(See the provenance note at the top of this file — this specific
paragraph is the part that was revised before anything was built.)*

**The one intentional coupling point, and it's a runtime nicety, not a
build dependency:** the tray's "open full status" action can shell out to
open a browser at the WebUI's local URL, *if* the WebUI happens to be
installed and reachable. Neither tool requires the other to exist or run.
`tetron-tray` never links against `tetron-web`'s code; it just knows the
convention (`http://127.0.0.1:<port>`) and no-ops or falls back if nothing's
listening there.

---

## Shared foundation (already built, nothing new needed here)

Both addons are unprivileged clients over the same socket the CLI already
uses:

```
Client ──msgpack over Unix socket──> /var/run/tetron/tetron.sock (mode 0666)
                                              │ SO_PEERCRED per-request auth
                                              ▼
                                        tetron daemon (root)
```

- Wire format: `IpcMessage` enum + `MsgpackCodec`, both in `tetron-proto`,
  already stable and used by the CLI today.
- Auth: socket is `0666` (any local user connects), but `check_authorized()`
  gates mutating calls on root or the configured `operator_uid` via
  `SO_PEERCRED`. Neither addon needs its own auth layer — it inherits
  whatever the invoking user is already authorized for.
- Both addons "log in" as whichever local user runs them. No separate
  credential system, no separate user/session model.

---

## WebUI (`tetron-web`)

### Goal

A localhost dashboard for the deeper admin surface — everything `tetron`
the CLI can do, reachable from a browser, primarily for headless/remote
boxes where there's no desktop session to put a tray icon in.

### Non-goals

- No remote/WAN exposure by default. Binds `127.0.0.1` only; a reverse
  proxy with TLS+auth is the user's problem if they want remote access,
  not this tool's.
- No WebSocket/streaming layer for v1 — polling `Status` every few seconds
  is enough (this is a dashboard, not a real-time monitoring tool).
- No new auth/session system — SO_PEERCRED + the socket's existing
  authorization model is the whole story.

### Architecture

```
Browser ──HTTP (127.0.0.1 only)──> tetron-web (unprivileged, own process)
                                          │
                                          │ msgpack over Unix socket
                                          ▼
                                    tetron.sock ──SO_PEERCRED──> daemon (root)
```

### Tech stack (proposed)

- **Backend:** `axum` — tokio-based (matches the daemon's existing async
  runtime family), minimal, well-maintained, no reason to reach for
  anything heavier.
- **Frontend:** server-rendered HTML + a small vanilla-JS polling loop
  (`fetch` + `setInterval`), not a SPA framework. Consistent with the
  project's demonstrated dependency discipline (`MINIMAL-015` stripped
  `indicatif`/`crossterm` from the CLI specifically to avoid exactly this
  kind of frontend-framework weight). A templating crate (`askama` or
  `minijinja`) if hand-written `format!` strings get unwieldy — decide once
  actual page count is known, don't pre-import one.
- **Deployment:** own binary, own systemd unit/launchd plist (same shape
  as the daemon's), own install step. Two binaries total for someone who
  wants both the daemon and the dashboard.

### MVP phasing

1. **Read-only dashboard.** `Status` polling only — networks, peers,
   connection info, mirrors `tetron status --json` almost directly. Zero
   mutating IPC calls. Lowest risk, fastest to ship, immediately useful on
   its own, and validates the whole plumbing (auth, socket reconnect on
   daemon restart, polling) before anything destructive is exposed.
2. **Low-stakes mutations.** Create network, mint/list/revoke invites,
   leave network. Everything here is either non-destructive or
   easily-undoable (a burned invite just needs re-minting).
3. **Full admin surface.** Kick, admin add/list, nuke. These need real
   confirmation UX (a web modal, not a single click) — the CLI's `--force`
   flag and NUKE-CONSENSUS's two-coordinator requirement are the daemon-side
   safety nets; the UI shouldn't make it easier to bypass the caution those
   were built for.

### Known operational wrinkle (already flagged in the old TODO note)

The Unix socket disappears and reappears across a daemon restart
(`sudo tetron restart`, or a crash-and-respawn under systemd). `tetron-web`
needs to detect a dropped connection and retry/reconnect rather than dying
or silently going stale — this is the same reconnect problem the CLI's
`ipc::connect()` doesn't have to solve today because it's a one-shot
process per command, not a long-running service. Worth its own small
design pass when this is actually built, not solved by this doc.

*(Resolved in practice, differently than expected: each `/api/status` poll
opens a fresh connection rather than keeping one alive, so a daemon
restart just makes the next poll fail-then-succeed once the socket
reappears — no explicit reconnect logic needed. See `PLAN.md`.)*

---

## Systray (`tetron-tray`)

### Goal

Glanceable status and the one action people do constantly — modeled on how
Tailscale/Mullvad/ProtonVPN tray icons work. Not an admin console.

### Non-goals

- **No destructive or trust-changing actions in the menu, ever.** Kick,
  admin add, nuke are deliberately excluded — a stray click on a menu item
  is a much easier accident than a typed CLI command or a WebUI
  confirmation dialog. Point to the CLI or WebUI for those.
- No Windows build. tetron doesn't build for Windows today (`config_dir()`
  only handles Linux/macOS) — not a new decision, just inherited scope.
- No standalone daemon-management (install/start/stop) — that's `sudo
  tetron install`/`start`/`stop`'s job, needs root, out of place in a
  per-user tray app.

### Architecture

Same IPC client pattern as the WebUI (`IpcMessage` + `MsgpackCodec` from
`tetron-proto`), but a native GUI process instead of an HTTP server — no
network listener of any kind, no browser involved.

### Tech stack (proposed)

- **Tray/menu:** `tray-icon` crate — cross-platform (Linux
  `StatusNotifierItem`/AppIndicator, macOS `NSStatusBar`).
- **Event loop:** `tray-icon` doesn't run its own event loop; needs pairing
  with `tao` (the winit fork commonly used alongside it) to actually pump
  OS events. Confirm this is still the recommended pairing when
  implementation starts — crate ecosystem in this space moves.
- **Notifications:** a cross-platform desktop-notification crate (e.g.
  `notify-rust`) rather than hand-rolling per-OS notification APIs.
- **Deployment:** own binary, no systemd/launchd unit by default — a tray
  app is a per-user login-item (autostart entry in the desktop
  environment), not a system service.

### MVP phasing

1. **Status + toggle.** Icon reflects up/active, down/standby, or daemon
   unreachable (via `Status` polling). Tooltip: active network count, peer
   count. Click toggles `Up`/`Down` — the single most common action.
2. **Per-network detail.** Submenu per joined network: member count, click
   through to peer hostnames/IPs. Copy invite key
   (`InviteList` + clipboard) for quick sharing without opening a terminal.
3. **Notifications + WebUI handoff.** OS-native notifications for peer
   online/offline, an `activate()` warning (e.g. failed to bring TUN up),
   daemon becoming unreachable. "Open full status" launches the WebUI in
   a browser if it's installed and reachable; otherwise a no-op or a hint
   to run `tetron status`. "Leave network" stays available (lightweight,
   self-service, not other-affecting) — kick/admin/nuke stay excluded per
   the non-goals above, permanently, not just for v1.

---

## Open questions before either gets built

- **Which first, if not both at once?** Not answered here since it wasn't
  asked — but worth noting the effort/value shapes differ: the systray's
  MVP (phase 1) is small and self-contained; the WebUI's full value
  (the "deeper admin surface" it exists for) doesn't land until phase 3.
  If only one gets built soon, that asymmetry is worth weighing.

  *(Answered in practice: WebUI first, "as it is the most influential,"
  then systray "for immediate gratification" — Erik's call, 2026-07-18.)*

- **`tetron-web`'s reconnect behavior** across a daemon restart needs a
  real design pass, not just "poll and hope" (see the wrinkle noted above).
- **Packaging cost**: each addon is a genuinely separate install step for
  users (own binary, own service-or-login-item registration) — worth
  deciding whether `tetron install` ever grows an `--with-web`/`--with-tray`
  convenience flag, or these stay fully opt-in manual installs indefinitely.
  Not urgent; flagging so it doesn't get decided by accident later.
