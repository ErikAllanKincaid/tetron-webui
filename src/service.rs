//! Per-user service install/uninstall: a `systemd --user` unit on Linux, a
//! launchd **LaunchAgent** on macOS. Deliberately per-user, not system-wide
//! -- unlike tetron's own daemon this needs no root (just an unprivileged
//! Unix-socket client to the already-running daemon) and only makes sense
//! running inside a login session (a LaunchAgent runs in the user's
//! session; a LaunchDaemon, what tetron's own daemon uses, does not and
//! can't draw to a display -- irrelevant here since this is a headless
//! HTTP server, but the distinction is why this is deliberately NOT modeled
//! on tetron's own `contrib/com.tetron.vpn.plist`).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

fn run_cmd(program: &str, args: &[&str]) {
    match Command::new(program).args(args).status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("warning: `{program}` exited with {status}"),
        Err(e) => eprintln!("warning: failed to run `{program}`: {e}"),
    }
}

/// Used for best-effort teardown before a fresh macOS launchd load; unused
/// on Linux (same `#[allow(dead_code)]` reasoning as tetron's own `service.rs`).
#[allow(dead_code)]
fn run_cmd_quiet(program: &str, args: &[&str]) {
    let _ = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(target_os = "linux")]
fn unit_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("could not determine config directory")?
        .join("systemd/user");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("tetron-webui.service"))
}

#[cfg(target_os = "macos")]
fn plist_path() -> Result<PathBuf> {
    let dir = dirs::home_dir()
        .context("could not determine home directory")?
        .join("Library/LaunchAgents");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("com.tetron.webui.plist"))
}

#[cfg(target_os = "macos")]
fn log_path() -> Result<PathBuf> {
    let dir = dirs::home_dir()
        .context("could not determine home directory")?
        .join("Library/Logs");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("tetron-webui.log"))
}

/// `tetron-webui install`: write the unit/plist (substituting the path of
/// the binary currently running, so the service execs the same binary the
/// user invoked -- same idempotent-on-every-install pattern tetron's own
/// `ensure_service_installed` uses), enable it, and wait for the HTTP
/// server to actually come up before declaring success.
pub fn install() -> Result<()> {
    println!("installing tetron-webui {}", crate::FULL_VERSION);
    let exe = std::env::current_exe()
        .context("failed to determine current executable path")?
        .to_string_lossy()
        .into_owned();

    #[cfg(target_os = "linux")]
    {
        let path = unit_path()?;
        let unit =
            include_str!("../contrib/tetron-webui.service").replace("/usr/local/bin/tetron-webui", &exe);
        std::fs::write(&path, unit).with_context(|| format!("failed to write {}", path.display()))?;
        run_cmd("systemctl", &["--user", "daemon-reload"]);
        run_cmd("systemctl", &["--user", "enable", "tetron-webui"]);
        // `enable --now` is a no-op restart on an already-active unit, so a
        // reinstall over a running instance (e.g. upgrading the binary in
        // place) would never actually pick up the new binary -- explicit
        // restart instead (same fix as tetron-systray's own service.rs).
        run_cmd("systemctl", &["--user", "restart", "tetron-webui"]);
    }

    #[cfg(target_os = "macos")]
    {
        let path = plist_path()?;
        let log = log_path()?.to_string_lossy().into_owned();
        let plist = include_str!("../contrib/com.tetron.webui.plist")
            .replace("/usr/local/bin/tetron-webui", &exe)
            .replace("/tmp/tetron-webui.log", &log);
        std::fs::write(&path, plist).with_context(|| format!("failed to write {}", path.display()))?;
        // Tear down any previously loaded job (e.g. one pointing at a stale
        // binary path) before loading the freshly written plist -- same
        // reasoning as tetron's own install_and_start_service.
        run_cmd_quiet("launchctl", &["unload", &path.to_string_lossy()]);
        run_cmd("launchctl", &["load", "-w", &path.to_string_lossy()]);
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    anyhow::bail!("per-user service install not supported on this platform");

    eprintln!("waiting for tetron-webui to come up…");
    if wait_for_http(Duration::from_secs(10)) {
        println!("tetron-webui service installed and running on http://127.0.0.1:7870");
        Ok(())
    } else {
        anyhow::bail!(
            "service was installed but tetron-webui never became reachable on http://127.0.0.1:7870.\n\
             Check the service logs (journalctl --user -u tetron-webui on Linux, or the log path in the plist on macOS)."
        );
    }
}

/// `tetron-webui uninstall`: stop, disable, and remove the unit/plist.
pub fn uninstall() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let path = unit_path()?;
        if path.exists() {
            run_cmd("systemctl", &["--user", "disable", "--now", "tetron-webui"]);
            std::fs::remove_file(&path)?;
            run_cmd("systemctl", &["--user", "daemon-reload"]);
            println!("Removed systemd --user service.");
        } else {
            println!("Service not installed.");
        }
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let path = plist_path()?;
        if path.exists() {
            run_cmd("launchctl", &["unload", "-w", &path.to_string_lossy()]);
            std::fs::remove_file(&path)?;
            println!("Removed launchd LaunchAgent.");
        } else {
            println!("Service not installed.");
        }
        return Ok(());
    }

    #[allow(unreachable_code)]
    {
        anyhow::bail!("per-user service uninstall not supported on this platform");
    }
}

fn wait_for_http(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if std::net::TcpStream::connect("127.0.0.1:7870").is_ok() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
