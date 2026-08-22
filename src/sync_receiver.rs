//! Live configuration for the `tetron-sync-receiver` addon: shells out to
//! the installed binary's own CLI (with `--json`, its single global flag --
//! `tetron-sync-receiver`'s own `src/main.rs`) for every read/write action.
//! This module never reimplements `rsyncd.conf`/module/allow-list logic --
//! that binary's CLI is the single source of truth, exactly the same
//! "point at the binary, don't reimplement" pattern `addons.rs` already
//! uses for install/uninstall and `is_active`.
//!
//! Same root-owned `/usr/local/bin` install location every release-binary
//! addon uses (see `addons.rs::install_dir`) -- this is not configurable
//! per-addon today, so it is hardcoded here too rather than plumbed through
//! as a parameter for a single caller.

use serde::{Deserialize, Serialize};
use tokio::process::Command;

const BINARY: &str = "/usr/local/bin/tetron-sync-receiver";

#[derive(Serialize, Deserialize)]
pub struct ReceiverStatus {
    pub active: bool,
    pub port: u16,
    pub module_count: usize,
    pub allow_count: usize,
}

#[derive(Serialize, Deserialize)]
pub struct Module {
    pub name: String,
    pub path: String,
}

async fn run_json<T: for<'de> Deserialize<'de>>(args: &[&str]) -> anyhow::Result<T> {
    let mut full_args = vec!["--json"];
    full_args.extend_from_slice(args);
    let output = Command::new(BINARY)
        .args(&full_args)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run tetron-sync-receiver: {e}"))?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    );
    serde_json::from_slice(&output.stdout)
        .map_err(|e| anyhow::anyhow!("failed to parse tetron-sync-receiver output: {e}"))
}

async fn run_ok(args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new(BINARY)
        .args(args)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run tetron-sync-receiver: {e}"))?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    );
    Ok(())
}

pub async fn status() -> anyhow::Result<ReceiverStatus> {
    run_json(&["receiver", "status"]).await
}

pub async fn list_modules() -> anyhow::Result<Vec<Module>> {
    run_json(&["module", "list"]).await
}

pub async fn add_module(name: &str, path: &str) -> anyhow::Result<()> {
    run_ok(&["module", "add", name, path]).await
}

pub async fn remove_module(name: &str) -> anyhow::Result<()> {
    run_ok(&["module", "remove", name]).await
}

pub async fn list_allow() -> anyhow::Result<Vec<String>> {
    run_json(&["allow", "list"]).await
}

pub async fn add_allow_ip(ip: &str) -> anyhow::Result<()> {
    run_ok(&["allow", "add", ip]).await
}

pub async fn add_allow_peer(hostname: &str) -> anyhow::Result<()> {
    run_ok(&["allow", "add-peer", hostname]).await
}

pub async fn remove_allow(ip: &str) -> anyhow::Result<()> {
    run_ok(&["allow", "remove", ip]).await
}

pub async fn enable() -> anyhow::Result<()> {
    run_ok(&["receiver", "enable"]).await
}

pub async fn disable() -> anyhow::Result<()> {
    run_ok(&["receiver", "disable"]).await
}

/// Restarts the service if it's currently active (the receiver's own
/// `receiver port` subcommand handles that -- see that repo's
/// `service::restart_if_active`). Whoever configured the phone side
/// (tetron-mobile-sync's Settings screen) must set the exact same port,
/// or transfers fail with a socket error -- same warning shown there.
pub async fn set_port(port: u16) -> anyhow::Result<()> {
    run_ok(&["receiver", "port", &port.to_string()]).await
}
