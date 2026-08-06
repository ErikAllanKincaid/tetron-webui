//! Addon-install framework: a small, data-driven registry of optional
//! tetron addons (systray today, more later) the webui can detect, fetch,
//! verify, install, and uninstall on this host.
//!
//! Reuses the exact convention already proven in `tetron`'s own
//! `contrib/install-tetron-suite.sh`: each release publishes a binary
//! asset named `<binary>-<platform>-<arch>` plus a `.sha256` sidecar whose
//! contents reference that exact filename (verify before renaming, or
//! `sha256sum -c`/`shasum -a 256 -c` fail with "No such file or
//! directory" -- a bug that script's own history already hit and fixed
//! once). Generalizing this to more than one addon only ever needs a new
//! `AddonSpec` entry below -- the fetch/verify/install/status logic is
//! already fully addon-agnostic.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::process::Command;

pub struct AddonSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub github_repo: &'static str,    /// Whether this addon is a single downloadable release binary this
    /// webui can fetch/verify/install/uninstall itself. `false` for addons
    /// that are a whole environment or a script run against a remote host
    /// (tetron-relay, tetron-testsuite) rather than a per-user local
    /// service -- these show up as a name/description/repo-link row only,
    /// with no install/uninstall action and no `is_active`/binary-path
    /// machinery invoked on their behalf (the fields below are unused and
    /// left as placeholders when this is `false`).
    pub installable: bool,
    pub binary_name: &'static str,
    /// systemd --user unit name (Linux), matching whatever the addon's own
    /// `install` subcommand registers (its own `service.rs`). Unread on
    /// non-Linux builds -- same reasoning as tetron-systray's own
    /// `service.rs` allowing dead code on its platform-gated helpers.
    #[allow(dead_code)]
    pub linux_unit: &'static str,
    /// launchd label (macOS), same source. Unread on non-macOS builds.
    #[allow(dead_code)]
    pub macos_label: &'static str,
    /// Whether this addon needs a graphical session to ever be useful
    /// (systray's per-user unit targets `graphical-session.target` and
    /// will simply never activate without one -- matches
    /// `contrib/install-tetron-suite.sh`'s own `has_display` skip so a
    /// headless install fails fast with a clear reason instead of a
    /// confusing "never became active" error 10s later).
    pub needs_display: bool,
    /// Whether this addon has an in-app instructions popup rendered from
    /// `ADDON_DETAILS` in app.js -- a "Details" button appears next to the
    /// row, opening the detail modal with step-by-step commands. Used by
    /// Config Backup (the popup points at the script this webui proxies at
    /// /addons/tetron-backup.sh, plus the direct tetron-repo raw URL as a
    /// fallback); any future addon with setup docs can set this and add a
    /// matching app.js entry.
    pub details: bool,
}

/// Where the Config Backup script lives upstream: the tetron repo's
/// `contrib/tetron-backup.sh` on `main` (a script has no release tag to
/// pin, so the ref floats -- same philosophy as the floating `tetron-proto`
/// git dependency). Overridable via `TETRON_BACKUP_RAW_URL`, both for
/// testing and so the suite can survive the repo moving off GitHub.
pub const DEFAULT_BACKUP_SCRIPT_URL: &str =
    "https://raw.githubusercontent.com/ErikAllanKincaid/tetron/main/contrib/tetron-backup.sh";

/// Effective upstream URL for the backup script: `TETRON_BACKUP_RAW_URL`
/// env override if set, else the default. Shared by the /addons/
/// tetron-backup.sh proxy (main.rs) and the addons API so the popup's
/// fallback curl command always shows the same URL the proxy actually uses.
pub fn backup_script_url() -> String {
    std::env::var("TETRON_BACKUP_RAW_URL").unwrap_or_else(|_| DEFAULT_BACKUP_SCRIPT_URL.to_string())
}

pub const ADDONS: &[AddonSpec] = &[
    AddonSpec {
        id: "systray",
        display_name: "Systray",
        description: "Menu-bar tray icon with per-network status and quick actions.",
        github_repo: "ErikAllanKincaid/tetron-systray",
        installable: true,
        binary_name: "tetron-systray",
        linux_unit: "tetron-systray",
        macos_label: "com.tetron.systray",
        needs_display: true,
        details: false,
    },
    AddonSpec {
        id: "backup",
        display_name: "Config Backup",
        description: "Passphrase-encrypted tar+age backup of this host's tetron config tree (secret_key, settings.toml, every networks/*.toml). The script lives in the tetron repo (contrib/tetron-backup.sh) and this webui proxies it at /addons/tetron-backup.sh -- no repo clone or external download involved.",
        github_repo: "ErikAllanKincaid/tetron",
        installable: false,
        binary_name: "",
        linux_unit: "",
        macos_label: "",
        needs_display: false,
        details: true,
    },
    AddonSpec {
        id: "relay",
        display_name: "Relay",
        description: "Bringup tool that turns a plain Linux server into a self-hosted iroh relay for your fleet's NAT-traversal fallback.",
        github_repo: "ErikAllanKincaid/tetron-relay",
        installable: false,
        binary_name: "",
        linux_unit: "",
        macos_label: "",
        needs_display: false,
        details: false,
    },
    AddonSpec {
        id: "testsuite",
        display_name: "Test Suite",
        description: "VM-based automated test suite that drives tetron through its own CLI to assert on real network behavior.",
        github_repo: "ErikAllanKincaid/tetron-testsuite",
        installable: false,
        binary_name: "",
        linux_unit: "",
        macos_label: "",
        needs_display: false,
        details: false,
    },
];

#[derive(Serialize)]
pub struct AddonStatus {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub github_repo: &'static str,
    pub installable: bool,
    pub installed: bool,
    pub details: bool,
    /// Effective upstream URL for addons that proxy a script the popup
    /// points at (Config Backup); `None` for every other addon.
    pub script_url: Option<String>,
}

fn find(id: &str) -> anyhow::Result<&'static AddonSpec> {
    ADDONS
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| anyhow::anyhow!("unknown addon: {id}"))
}

fn install_dir() -> anyhow::Result<PathBuf> {
    dirs::home_dir()
        .map(|h| h.join(".local/bin"))
        .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))
}

fn binary_path(spec: &AddonSpec) -> anyhow::Result<PathBuf> {
    Ok(install_dir()?.join(spec.binary_name))
}

/// Whether the addon's per-user service is currently registered+active --
/// the same check its own `service.rs` uses internally (`systemctl --user
/// is-active` / `launchctl list`), just run here as an external process
/// since this lives in a different binary/repo than the addon itself.
#[cfg(target_os = "linux")]
async fn is_active(spec: &AddonSpec) -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", spec.linux_unit])
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
async fn is_active(spec: &AddonSpec) -> bool {
    Command::new("launchctl")
        .args(["list", spec.macos_label])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn is_active(_spec: &AddonSpec) -> bool {
    false
}

pub async fn list_status() -> Vec<AddonStatus> {
    let mut out = Vec::with_capacity(ADDONS.len());
    for spec in ADDONS {
        // A link-only addon has no unit/label to check -- skip the
        // shell-out entirely rather than querying systemd/launchd about a
        // service that was never registered under an empty name.
        let installed = spec.installable && is_active(spec).await;
        out.push(AddonStatus {
            id: spec.id,
            display_name: spec.display_name,
            description: spec.description,
            github_repo: spec.github_repo,
            installable: spec.installable,
            installed,
            details: spec.details,
            script_url: (spec.id == "backup").then(backup_script_url),
        });
    }
    out
}

fn asset_suffix() -> anyhow::Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x86_64"),
        ("linux", "aarch64") => Ok("linux-aarch64"),
        ("macos", "x86_64") => Ok("macos-x86_64"),
        ("macos", "aarch64") => Ok("macos-aarch64"),
        (os, arch) => anyhow::bail!("unsupported platform: {os}-{arch}"),
    }
}

async fn run_ok(cmd: &mut Command) -> anyhow::Result<()> {
    let status = cmd.status().await?;
    anyhow::ensure!(status.success(), "command exited with {status}");
    Ok(())
}

async fn verify_checksum(file: &Path, sumfile: &Path) -> anyhow::Result<()> {
    let dir = file
        .parent()
        .ok_or_else(|| anyhow::anyhow!("temp file has no parent directory"))?;
    let sumfile_name = sumfile
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("checksum file has no name"))?;
    let use_shasum = Command::new("sha256sum")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| !s.success())
        .unwrap_or(true);
    let mut cmd = if use_shasum {
        let mut c = Command::new("shasum");
        c.args(["-a", "256"]);
        c
    } else {
        Command::new("sha256sum")
    };
    cmd.arg("-c").arg(sumfile_name).current_dir(dir);
    run_ok(&mut cmd).await
}

async fn set_executable(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(path).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(path, perms).await?;
    }
    Ok(())
}

/// Same reasoning as `contrib/install-tetron-suite.sh`'s own
/// `has_display()`: macOS is assumed to always have one (a headless Mac
/// is rare enough not to special-case); Linux is checked explicitly.
/// **Caveat, unlike the shell script:** this only sees the webui
/// process's *own* environment, which can lack `$DISPLAY`/
/// `$WAYLAND_DISPLAY` even on a real desktop if webui itself runs as a
/// `systemd --user` service that never imported them (a common systemd
/// gotcha, independent of whether a graphical session actually exists --
/// systray's own unit targets `graphical-session.target`, a separate,
/// more reliable systemd-level signal this check doesn't have access
/// to). So this is treated as an advisory signal in `install()` below,
/// not a hard block -- a false "no display" reading must never prevent a
/// real install from proceeding.
fn has_display() -> bool {
    if std::env::consts::OS != "linux" {
        return true;
    }
    std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok()
}

/// Fetch+verify+install one addon: download the release asset + its
/// `.sha256` sidecar into a private temp dir, verify, copy to
/// `~/.local/bin`, then run the addon's own `install` subcommand to
/// register its per-user service -- the same end state a manual
/// `contrib/install-tetron-suite.sh` run would leave.
pub async fn install(id: &str) -> anyhow::Result<String> {
    let spec = find(id)?;
    anyhow::ensure!(
        spec.installable,
        "{} is not a locally installable addon -- see https://github.com/{} for setup instructions",
        spec.display_name,
        spec.github_repo
    );
    let display_caveat = spec.needs_display && !has_display();
    let suffix = asset_suffix()?;
    let asset = format!("{}-{suffix}", spec.binary_name);
    let base = format!("https://github.com/{}/releases/latest/download/{asset}", spec.github_repo);

    let tmpdir = tempfile::Builder::new().prefix("tetron-webui-addon-").tempdir()?;
    let asset_path = tmpdir.path().join(&asset);
    let sum_path = tmpdir.path().join(format!("{asset}.sha256"));

    run_ok(Command::new("curl").args(["-fsSL", "-o"]).arg(&asset_path).arg(&base)).await
        .map_err(|e| anyhow::anyhow!("failed to download {asset}: {e}"))?;
    run_ok(Command::new("curl").args(["-fsSL", "-o"]).arg(&sum_path).arg(format!("{base}.sha256"))).await
        .map_err(|e| anyhow::anyhow!("failed to download {asset}.sha256: {e}"))?;
    verify_checksum(&asset_path, &sum_path).await
        .map_err(|e| anyhow::anyhow!("checksum verification failed: {e}"))?;

    let dest = binary_path(spec)?;
    tokio::fs::create_dir_all(install_dir()?).await?;
    tokio::fs::copy(&asset_path, &dest).await
        .map_err(|e| anyhow::anyhow!("failed to install to {}: {e}", dest.display()))?;
    set_executable(&dest).await?;

    let install_failed_msg = |e: anyhow::Error| {
        if display_caveat {
            anyhow::anyhow!(
                "{} downloaded but its own install step failed: {e} \
                 (no $DISPLAY/$WAYLAND_DISPLAY seen in webui's own environment -- \
                 if this host does have a desktop session, webui's service may need \
                 `systemctl --user import-environment DISPLAY WAYLAND_DISPLAY`; \
                 otherwise install this on a machine with one instead)",
                spec.display_name
            )
        } else {
            anyhow::anyhow!("{} downloaded but its own install step failed: {e}", spec.display_name)
        }
    };

    run_ok(Command::new(&dest).arg("install")).await.map_err(install_failed_msg)?;

    // The addon's own `install` exit code isn't proof it's actually
    // running -- found live: a crash-looping GUI process (no display)
    // can still exit 0 from `install` if systemd briefly reports "active"
    // between restart attempts, right when that subcommand's own
    // wait-for-active poll happens to land. Re-check after a beat with
    // our own independent poll before declaring success. Materially
    // closes the race on Linux (systemctl --user is-active is a live
    // check); on macOS this inherits tetron-systray's own documented
    // `is_active()` limitation (`launchctl list` only proves the job is
    // loaded, not still running a few seconds later) -- a real remaining
    // gap there, not one this re-check can paper over from the outside.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    if !is_active(spec).await {
        return Err(install_failed_msg(anyhow::anyhow!(
            "service was installed but is not running (may be crash-looping -- check its logs)"
        )));
    }

    Ok(format!("{} installed and running.", spec.display_name))
}

/// Stops+deregisters the addon's own per-user service (its own `uninstall`
/// subcommand -- `systemctl --user disable --now` / `launchctl unload` plus
/// removing the unit/plist and, on macOS, the `.app` bundle it copied
/// itself into) and then removes the binary this webui installed at `dest`.
///
/// **The binary-removal step is the actual point of this being a *webui*
/// concern rather than just delegating entirely to the addon's own
/// subcommand:** the addon's `uninstall` only knows how to tear down what
/// it registered (the service), not the file webui itself placed at
/// `~/.local/bin/<binary>` -- it has no way to know it should delete its
/// own currently-running executable out from under itself. Found live
/// (2026-07-25, xps-17-9720): the service genuinely does stop and the tray
/// icon genuinely does disappear -- `run_ok` below was never the bug -- but
/// the ~4MB binary was left behind afterward, which reads as "Uninstall
/// didn't actually uninstall it" even though the running addon is gone.
/// Best-effort: a failure to delete the file (e.g. permissions) does not
/// undo the service teardown that already succeeded, so it is reported
/// distinctly rather than surfaced as a full failure of the whole action.
pub async fn uninstall(id: &str) -> anyhow::Result<String> {
    let spec = find(id)?;
    anyhow::ensure!(
        spec.installable,
        "{} is not a locally installable addon -- see https://github.com/{} for setup instructions",
        spec.display_name,
        spec.github_repo
    );
    let dest = binary_path(spec)?;
    anyhow::ensure!(dest.exists(), "{} is not installed", spec.display_name);
    run_ok(Command::new(&dest).arg("uninstall")).await?;
    match tokio::fs::remove_file(&dest).await {
        Ok(()) => Ok(format!("{} uninstalled.", spec.display_name)),
        Err(e) => Ok(format!(
            "{} service stopped, but its binary at {} could not be removed: {e}. Delete it manually if you want a full cleanup.",
            spec.display_name,
            dest.display()
        )),
    }
}
