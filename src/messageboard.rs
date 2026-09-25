//! Live board management for the `tetron-messageboard` addon: shells out to
//! the installed binary's own CLI, the same "point at the binary, don't
//! reimplement" pattern `sync_receiver.rs` uses. One board per tetron
//! network; a multi-network node runs several at once (each binds its own
//! network's mesh IP, so they share the default port). This drives
//! list/start/stop/restart across all of them.
//!
//! Starting and stopping a board is a per-user systemd/launchd operation the
//! binary performs itself -- no root, unlike placing the binary in
//! root-owned `/usr/local/bin` (that stays an `install-tetron-suite.sh` job,
//! see `addons.rs`).

use serde::{Deserialize, Serialize};
use tokio::process::Command;

const BINARY: &str = "/usr/local/bin/tetron-messageboard";

#[derive(Serialize, Deserialize)]
pub struct Board {
    pub network: String,
    pub port: u16,
    #[serde(default)]
    pub token: String,
    pub active: bool,
    #[serde(default)]
    pub legacy: bool,
}

/// Whether the binary has been placed on this host at all. Distinct from
/// "a board is running": the addon row shows the sudo install one-liner until
/// this is true, then the board manager.
pub fn binary_present() -> bool {
    std::path::Path::new(BINARY).exists()
}

async fn run_ok(args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new(BINARY)
        .args(args)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run tetron-messageboard: {e}"))?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

pub async fn list() -> anyhow::Result<Vec<Board>> {
    let output = Command::new(BINARY)
        .args(["list", "--json"])
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run tetron-messageboard: {e}"))?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    serde_json::from_slice(&output.stdout)
        .map_err(|e| anyhow::anyhow!("failed to parse tetron-messageboard output: {e}"))
}

pub async fn start(network: &str) -> anyhow::Result<()> {
    run_ok(&["install", "--network", network]).await
}

pub async fn stop(network: &str) -> anyhow::Result<()> {
    run_ok(&["uninstall", "--network", network]).await
}

pub async fn restart_all() -> anyhow::Result<()> {
    run_ok(&["restart-all"]).await
}
