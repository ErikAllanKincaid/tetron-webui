//! Addon-install framework: a small, data-driven registry of optional
//! tetron addons (systray today, more later) the webui can detect and
//! uninstall on this host.
//!
//! Release-binary addons (systray) live at root-owned `/usr/local/bin`,
//! same as `tetron`/`tetron-webui`, so this webui -- running unprivileged
//! -- can no longer fetch and place the binary itself; `install()` points
//! at `contrib/install-tetron-suite.sh` instead, the same installer
//! `tetron`/`tetron-webui` are themselves installed with. Script addons
//! (Config Backup) are unaffected: no service, no root, still installed
//! directly into `~/.local/bin`.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::process::Command;

pub struct AddonSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub github_repo: &'static str,
    /// Whether this addon is a single release binary/script this webui can
    /// detect and uninstall (and, for script addons, install too -- see
    /// `script` below). `false` for addons that are a whole environment or
    /// a script run against a remote host (tetron-relay, tetron-testsuite)
    /// rather than a per-user local service -- these show up as a
    /// name/description/repo-link row only, with no install/uninstall
    /// action and no `is_active`/binary-path machinery invoked on their
    /// behalf (the fields below are unused and left as placeholders when
    /// this is `false`).
    pub installable: bool,
    pub binary_name: &'static str,
    /// Whether this addon's artifact is a bare script fetched from
    /// `backup_script_url()` (Config Backup) rather than a release binary:
    /// install = curl the raw file into `~/.local/bin`, chmod, done -- no
    /// checksum sidecar, no `install` subcommand, no service to register
    /// and no `is_active` poll. No root needed. Release-binary addons
    /// (`script: false`, e.g. systray) install to root-owned
    /// `/usr/local/bin` instead (see `install_dir`) -- this webui can
    /// detect and uninstall them, but installing needs
    /// `contrib/install-tetron-suite.sh` run by hand (see `install()`).
    pub script: bool,
    /// systemd --user unit name (Linux), matching whatever the addon's own
    /// `install` subcommand registers (its own `service.rs`). Unread on
    /// non-Linux builds -- same reasoning as tetron-systray's own
    /// `service.rs` allowing dead code on its platform-gated helpers.
    #[allow(dead_code)]
    pub linux_unit: &'static str,
    /// launchd label (macOS), same source. Unread on non-macOS builds.
    #[allow(dead_code)]
    pub macos_label: &'static str,
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
        script: false,
        details: false,
    },
    AddonSpec {
        id: "backup",
        display_name: "Config Backup",
        description: "Passphrase-encrypted tar+age backup tetron config tree (secret_key, settings.toml, every networks/*.toml).",
        github_repo: "ErikAllanKincaid/tetron",
        installable: true,
        binary_name: "tetron-backup.sh",
        linux_unit: "",
        macos_label: "",
        script: true,
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
        script: false,
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
        script: false,
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

/// Script addons (Config Backup) install unprivileged into `~/.local/bin`,
/// unchanged -- no service, no `$PATH`-reachability concern the way a
/// `systemd --user`-backed binary has. Release-binary addons (systray) now
/// live at root-owned `/usr/local/bin`, matching `tetron`/`tetron-webui`
/// and `contrib/install-tetron-suite.sh`'s own convention -- this used to
/// be `~/.local/bin` here too, which let this webui and the shell
/// installer each place a binary at a different path, leaving two
/// independent copies on a machine that used both.
fn install_dir(spec: &AddonSpec) -> anyhow::Result<PathBuf> {
    if spec.script {
        dirs::home_dir()
            .map(|h| h.join(".local/bin"))
            .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))
    } else {
        Ok(PathBuf::from("/usr/local/bin"))
    }
}

fn binary_path(spec: &AddonSpec) -> anyhow::Result<PathBuf> {
    Ok(install_dir(spec)?.join(spec.binary_name))
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
        // A script addon is "installed" when the file exists in
        // ~/.local/bin -- there is no service to poll. Everything else
        // skips the shell-out entirely when link-only (no unit/label to
        // query about a service that was never registered).
        let installed = if spec.script {
            binary_path(spec).map(|p| p.exists()).unwrap_or(false)
        } else {
            spec.installable && is_active(spec).await
        };
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

async fn run_ok(cmd: &mut Command) -> anyhow::Result<()> {
    let status = cmd.status().await?;
    anyhow::ensure!(status.success(), "command exited with {status}");
    Ok(())
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

/// Script addons (Config Backup) install themselves via `install_script`
/// below -- curl the raw file from the effective `backup_script_url()`
/// into `~/.local/bin`, chmod, done. No checksum sidecar (the file has
/// none upstream), no service, no root.
///
/// Release-binary addons (systray) install to root-owned
/// `/usr/local/bin` (see `install_dir`), which this webui -- running
/// unprivileged -- cannot write to itself. Rather than prompt for a
/// password through a web form (a real anti-pattern: never ask for root
/// credentials over HTTP), this points at
/// `contrib/install-tetron-suite.sh`, the same installer `tetron`/
/// `tetron-webui` themselves are installed with, which the user runs at
/// a real terminal with their own sudo.
pub async fn install(id: &str) -> anyhow::Result<String> {
    let spec = find(id)?;
    anyhow::ensure!(
        spec.installable,
        "{} is not a locally installable addon -- see https://github.com/{} for setup instructions",
        spec.display_name,
        spec.github_repo
    );
    if spec.script {
        return install_script(spec).await;
    }
    anyhow::bail!(
        "{} installs to /usr/local/bin, which needs root -- this webui can't do that for you. \
         Run this in a terminal: curl -fsSL https://raw.githubusercontent.com/ErikAllanKincaid/tetron/main/contrib/install-tetron-suite.sh \
         | bash -s -- --install-{id}",
        spec.display_name
    );
}

/// The Config Backup path: fetch the raw script from the effective
/// `backup_script_url()` (the same URL the `/addons/tetron-backup.sh`
/// proxy serves) into `~/.local/bin`, chmod 755, done. No root, no
/// checksum sidecar, no service. Download goes to a temp file first so a
/// failed/partial fetch can never clobber an already-installed copy --
/// install only overwrites once the fetch succeeded.
async fn install_script(spec: &AddonSpec) -> anyhow::Result<String> {
    let url = backup_script_url();
    let dest = binary_path(spec)?;
    let tmpdir = tempfile::Builder::new().prefix("tetron-webui-script-").tempdir()?;
    let tmp_path = tmpdir.path().join(spec.binary_name);
    run_ok(Command::new("curl").args(["-fsSL", "-o"]).arg(&tmp_path).arg(&url))
        .await
        .map_err(|e| anyhow::anyhow!("failed to download script from {url}: {e}"))?;
    let meta = tokio::fs::metadata(&tmp_path).await?;
    anyhow::ensure!(
        meta.len() > 0,
        "downloaded script from {url} is empty -- aborting install"
    );
    tokio::fs::create_dir_all(install_dir(spec)?).await?;
    tokio::fs::copy(&tmp_path, &dest).await
        .map_err(|e| anyhow::anyhow!("failed to install to {}: {e}", dest.display()))?;
    set_executable(&dest).await?;
    Ok(format!(
        "{} installed at {} (no root needed).",
        spec.display_name,
        dest.display()
    ))
}

/// Uninstall one addon. Script addons (Config Backup): remove the file
/// from `~/.local/bin` and nothing else -- there is no service to tear
/// down. Everything else delegates to `uninstall_service`, which
/// stops+deregisters the addon's own per-user service (its own `uninstall`
/// subcommand -- `systemctl --user disable --now` / `launchctl unload` plus
/// removing the unit/plist and, on macOS, the `.app` bundle it copied
/// itself into) and then tries to remove the binary at `dest`.
///
/// **The binary-removal step is best-effort, expected to routinely fail:**
/// the addon's own `uninstall` only tears down what it registered (the
/// service); `dest` for a release-binary addon is now root-owned
/// `/usr/local/bin/<binary>` (see `install_dir`), which this unprivileged
/// webui cannot delete. A failed removal does not undo the service
/// teardown that already succeeded -- it is reported distinctly ("stopped,
/// but could not remove the binary -- delete it manually") rather than
/// surfaced as a failure of the whole action.
pub async fn uninstall(id: &str) -> anyhow::Result<String> {
    let spec = find(id)?;
    anyhow::ensure!(
        spec.installable,
        "{} is not a locally installable addon -- see https://github.com/{} for setup instructions",
        spec.display_name,
        spec.github_repo
    );
    if spec.script {
        let dest = binary_path(spec)?;
        anyhow::ensure!(dest.exists(), "{} is not installed", spec.display_name);
        match tokio::fs::remove_file(&dest).await {
            Ok(()) => Ok(format!("{} script removed from {}.", spec.display_name, dest.display())),
            Err(e) => Ok(format!(
                "{} script at {} could not be removed: {e}. Delete it manually if you want a full cleanup.",
                spec.display_name,
                dest.display()
            )),
        }
    } else {
        uninstall_service(spec).await
    }
}

async fn uninstall_service(spec: &AddonSpec) -> anyhow::Result<String> {
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
