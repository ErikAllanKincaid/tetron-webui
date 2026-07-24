# tetron-webui

A browser dashboard and admin console for [tetron](https://github.com/ErikAllanKincaid/tetron), a P2P mesh VPN. Talks to the existing `tetron` daemon over its Unix-socket IPC protocol — no daemon changes required.

**Optional and separate from tetron on purpose.** tetron itself follows the Unix "do one thing well" philosophy and stays CLI-only by default; `tetron-webui` is a genuinely separate, opt-in product for anyone who wants a browser interface, not a bundled or default-installed component. Nothing about tetron's own behavior changes whether this exists or not.

![tetron-webui dashboard](images/tetron-webui_screenshot.png)

## Running it

**Primary path: download a pre-built binary.**

```bash
# First install tetron daemon.
curl -Lo tetron https://github.com/ErikAllanKincaid/tetron/releases/latest/download/tetron-linux-x86_64
chmod +x tetron
sudo install tetron /usr/local/bin/tetron
sudo tetron install

# Linux x86_64 -- see the releases page for aarch64 / macOS binaries:
# https://github.com/ErikAllanKincaid/tetron-webui/releases/latest
curl -Lo tetron-webui https://github.com/ErikAllanKincaid/tetron-webui/releases/latest/download/tetron-webui-linux-x86_64
chmod +x tetron-webui
sudo install tetron-webui /usr/local/bin/tetron-webui

tetron-webui install     # sets up + starts a per-user service, no sudo needed for this step
```

Installs a `systemd --user` unit on Linux (`~/.config/systemd/user/tetron-webui.service`) or a launchd **LaunchAgent** on macOS (`~/Library/LaunchAgents/com.tetron.webui.plist`, distinct from a system-wide LaunchDaemon — this runs inside your login session, not root). `Restart=on-failure`/`KeepAlive` means it comes back automatically if it crashes. `install` points the service at whatever binary you ran it from — install it somewhere permanent first (as above) if you want the service to survive that binary being deleted. `tetron-webui uninstall` stops and removes it. Verified end to end on real hardware on both platforms: install, crash-recovery (`kill -9` the process, confirmed the service manager restarts it within seconds), and uninstall all leave the machine clean.

Then open `http://127.0.0.1:7870`. Requires a running `tetron` daemon (`sudo tetron install`) reachable at its usual Unix socket. Read-only status works for any local user; mutating actions (create/join/leave/kick/nuke/etc.) require the browsing user to be tetron's configured operator (`sudo tetron set-operator <user>`) or root — same authorization model the CLI uses.

### Building from source / development

```bash
cargo build --release   # or: cargo run, for a foreground dev run
```

Only needed if you're changing the code, or a pre-built binary isn't published for your platform yet.

## What it does

- Live dashboard: which networks you're on, per-member role/host/IP/IPv6/connection health, traffic stats.
- Network lifecycle: create (including the nuke-consensus threshold), join, leave.
- Per-network resume/standby, and a manual sync button to wake the DHT/group poller immediately.
- Invites: mint, list, revoke.
- Coordinator actions: kick a member, grant/list co-coordinators, destroy a network (with the same consensus/force safeguards the CLI has).
- Add-ons panel: detect, install, and uninstall optional tetron add-ons directly from the dashboard — [`tetron-systray`](https://github.com/ErikAllanKincaid/tetron-systray) (a menu-bar tray icon) is the first one, and this is now its recommended install path. Verified end to end on real hardware, both platforms: a fresh install renders a working tray icon on Linux and macOS, and re-installing over an already-running instance (e.g. an upgrade) cleanly restarts it instead of leaving the old binary in memory.

## Architecture

```
Browser --HTTP (127.0.0.1 only)--> tetron-webui --msgpack/Unix socket--> tetron daemon
```

No daemon-side changes. Depends on `tetron-proto` (tetron's shared wire-protocol crate) as a git dependency, floating on `main` rather than pinned to a release tag — see the comment in `Cargo.toml` for why.

## Building your own version of this

**[`docs/HOWTO_Build_A_WebUI.md`](docs/HOWTO_Build_A_WebUI.md)** — a generic,
instructional writeup of the pattern this is built on (a thin HTTP server
proxying a browser to a local daemon's IPC socket, with a static
framework-free frontend), with real references and worked examples from
this repo's own source. Useful if you're building something similar, not
just historical context for this one project.

## License

MPL-2.0, matching tetron itself.
