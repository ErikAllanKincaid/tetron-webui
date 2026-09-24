//! Network-wide discovery of running tetron-messageboard instances.
//!
//! The board is a browser UI bound to a host's mesh IP; there is no registry
//! of who is hosting one. To surface a link at the top of each network
//! section, this probes every member of every network on the board's port
//! for `GET /health` and collects whichever respond.
//!
//! Probing is server-side (here), not in the browser, on purpose: the board
//! serves a locked-down `default-src 'none'; sandbox` CSP and sets no CORS
//! headers, so a `fetch()` from the webui page to a peer's `/health` would be
//! blocked cross-origin; and a webui behind an HTTPS reverse proxy fetching an
//! `http://` board would be mixed-content. Doing it here sidesteps both, and
//! one probe pass per webui (cached ~60s) beats one per browser tab.
//!
//! Reuses the `curl` shell-out pattern the Config-Backup proxy already uses
//! (no new HTTP-client dependency), one child per member, run concurrently
//! with a short timeout via a `JoinSet`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tetron_proto::ipc::IpcMessage;

use crate::ipc_client::call;

/// How long a discovery result is served before the next request re-probes.
/// Matches the design's ~60s discovery cache; the browser polls `/api/boards`
/// on its own slower cadence and this keeps the roster probe off that path.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// Per-member probe timeout. An unreachable peer (offline, no board) simply
/// times out and is treated as "no board there".
const PROBE_TIMEOUT_SECS: u64 = 2;

/// Default board port (tetron-messageboard's own `DEFAULT_PORT`). Only this
/// port is probed; a board on a non-default `TETRON_MESSAGEBOARD_PORT` is not
/// auto-discovered in v1 (its operator can share the URL by hand). A fleet
/// that standardised on a different port can point discovery at it by setting
/// the same env var for this webui.
const DEFAULT_BOARD_PORT: u16 = 28088;

fn board_port() -> u16 {
    std::env::var("TETRON_MESSAGEBOARD_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_BOARD_PORT)
}

/// One discovered board, as surfaced to the frontend.
#[derive(Serialize, Clone)]
pub struct Board {
    /// Roster hostname of the hosting node (falls back to the mesh IP).
    pub host: String,
    pub ip: String,
    /// Full URL to open in a new tab.
    pub url: String,
    /// `installed_by_coordinator` from the board's own `/health` -- the
    /// coordinator soft-gate. The frontend surfaces admin-hosted boards
    /// normally and member-hosted ones dimmed.
    pub admin: bool,
}

/// Discovered boards keyed by network name.
pub type BoardsByNetwork = HashMap<String, Vec<Board>>;

static CACHE: Mutex<Option<(Instant, BoardsByNetwork)>> = Mutex::new(None);

/// Boards per network name, served from a ~60s cache. Only networks with at
/// least one discovered board appear in the map (the frontend shows nothing
/// for a network with no board).
pub async fn get_boards() -> BoardsByNetwork {
    if let Some((at, boards)) = CACHE.lock().unwrap().as_ref()
        && at.elapsed() < CACHE_TTL
    {
        return boards.clone();
    }
    let fresh = discover().await;
    *CACHE.lock().unwrap() = Some((Instant::now(), fresh.clone()));
    fresh
}

async fn discover() -> BoardsByNetwork {
    let mut result = BoardsByNetwork::new();
    let Ok(IpcMessage::StatusResponse { networks, .. }) = call(IpcMessage::Status).await else {
        return result;
    };
    let port = board_port();

    for net in networks {
        // Every member of this network, including self: the board could be
        // hosted on this very node.
        let mut members: Vec<(String, String)> = Vec::new();
        members.push((
            net.my_ip.to_string(),
            net.my_hostname.clone().unwrap_or_else(|| net.my_ip.to_string()),
        ));
        for p in &net.peers {
            members.push((
                p.ip.to_string(),
                p.hostname.clone().unwrap_or_else(|| p.ip.to_string()),
            ));
        }

        let mut set = tokio::task::JoinSet::new();
        for (ip, host) in members {
            let network = net.network.clone();
            set.spawn(probe_one(ip, port, host, network));
        }
        let mut boards: Vec<Board> = Vec::new();
        while let Some(joined) = set.join_next().await {
            if let Ok(Some(board)) = joined {
                boards.push(board);
            }
        }

        if boards.is_empty() {
            continue;
        }
        // Admin-hosted first, then member-hosted; stable by host within each.
        boards.sort_by(|a, b| b.admin.cmp(&a.admin).then_with(|| a.host.cmp(&b.host)));
        result.insert(net.network, boards);
    }
    result
}

/// Probe one member's `/health`. Returns a `Board` only if it answers with a
/// tetron-messageboard health payload whose `network` matches the network we
/// are probing (a node in several networks hosts a board for exactly one).
async fn probe_one(ip: String, port: u16, host: String, network: String) -> Option<Board> {
    let url = format!("http://{ip}:{port}/health");
    let out = tokio::process::Command::new("curl")
        .args(["-fsS", "--max-time", &PROBE_TIMEOUT_SECS.to_string()])
        .arg(&url)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    if v.get("service").and_then(|s| s.as_str()) != Some("tetron-messageboard") {
        return None;
    }
    if v.get("network").and_then(|s| s.as_str()) != Some(network.as_str()) {
        return None;
    }
    let admin = v.get("installed_by_coordinator").and_then(|b| b.as_bool()).unwrap_or(false);
    Some(Board {
        host,
        ip: ip.clone(),
        url: format!("http://{ip}:{port}/"),
        admin,
    })
}
