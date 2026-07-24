//! HTTP handlers -- one per browser-facing action. Each one builds an
//! `IpcMessage`, sends it via `ipc_client`, and translates the daemon's
//! reply into a small JSON shape the frontend understands.
//!
//! Response shape convention used throughout: mutating endpoints return
//! `{"ok": true, "message": "..."}` on success or `{"ok": false, "error":
//! "..."}` on failure -- including the case where the daemon understood the
//! request and rejected it (e.g. "permission denied", "network not found").
//! We always answer with HTTP 200 for a *reached and understood* request,
//! even a rejected one, and only use a non-200 status when we truly could
//! not talk to the daemon at all. This keeps the frontend's error handling
//! simple: check `ok`, don't juggle HTTP status codes for daemon-level
//! outcomes that are really just "no" rather than a network-level failure.

use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use axum::{Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use tetron_proto::ipc::IpcMessage;

use crate::ipc_client::{call, call_expect_ok};

/// Shared "did it work" envelope for every mutating endpoint.
#[derive(Serialize)]
pub struct ActionResult {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl ActionResult {
    fn ok(message: String) -> Json<Self> {
        Json(Self {
            ok: true,
            message: Some(message),
            error: None,
        })
    }
    fn err(error: String) -> Json<Self> {
        Json(Self {
            ok: false,
            message: None,
            error: Some(error),
        })
    }
}

/// Runs an `IpcMessage` that's expected to come back as a plain `Ok`/`Error`
/// (i.e. every mutating action except the few with structured success data
/// handled separately below) and wraps it into the `ActionResult` envelope.
async fn run_action(msg: IpcMessage) -> Json<ActionResult> {
    match call_expect_ok(msg).await {
        Ok(message) => ActionResult::ok(message),
        Err(error) => ActionResult::err(error),
    }
}

// ---------------------------------------------------------------------
// Status (Phase 1 -- read-only)
// ---------------------------------------------------------------------

/// `GET /api/status`. Polled by the browser every few seconds. Never
/// returns an HTTP error for "daemon not running" -- that's a completely
/// normal state (e.g. before `sudo tetron install`), so it's represented as
/// `{"reachable": false}` in a 200 response, not a 5xx. The frontend
/// branches on `reachable`, not on HTTP status.
///
/// Catch-up note (tetron fell behind several unrelated releases before this
/// pass): `StatusResponse.pending_networks` no longer exists on the daemon
/// side (tetron's `LIVE-001` replaced live join-approval with invite-only
/// admission, removing the pending-join queue entirely) -- dropped here and
/// from the frontend's "waiting for approval" banner, which could never
/// fire again either way.
pub async fn get_status() -> Json<serde_json::Value> {
    let resp = match call(IpcMessage::Status).await {
        Ok(r) => r,
        Err(e) => return Json(serde_json::json!({"reachable": false, "message": e})),
    };
    let IpcMessage::StatusResponse {
        endpoint_id,
        active,
        daemon_version,
        networks,
        packets_rx,
        packets_tx,
        bytes_rx,
        bytes_tx,
        drops,
        fragmented_ipv4,
        fragmented_ipv6,
    } = resp
    else {
        // Status should never come back as anything but StatusResponse or
        // Error (already handled inside `call`'s caller convention elsewhere
        // for mutating actions) -- but Status itself has no Error path in
        // practice since it's always allowed. Treat anything unexpected the
        // same as unreachable, rather than panicking on a malformed reply.
        return Json(serde_json::json!({"reachable": false, "message": "unexpected daemon response"}));
    };

    // Enrich each network with its short id (first 10 hex chars of the
    // network key, same truncation `tetron status` itself uses) -- kick and
    // nuke both need it, and it's simpler to compute it once here than to
    // duplicate the slicing logic in JavaScript.
    let networks: Vec<serde_json::Value> = networks
        .into_iter()
        .map(|n| {
            let short_id = n.network_key.as_ref().map(|k| k.chars().take(10).collect::<String>());
            serde_json::json!({
                // STATUS-NETWORK-FIELD-001 (tetron): NetworkStatus.name is
                // being phased out in favor of .network (identical value
                // during the fleet upgrade window; see tetron's
                // DO-NOT-COMMIT/TODO.md checklist). Read + re-expose the new
                // field name here so this API's own contract doesn't
                // perpetuate the same ambiguity one layer out.
                "network": n.network,
                "role": format!("{}", n.role),
                "my_ip": n.my_ip,
                "my_ipv6": n.my_ipv6,
                "my_hostname": n.my_hostname,
                "network_key": n.network_key,
                "short_id": short_id,
                "member_count": n.member_count,
                "tun_name": n.tun_name,
                "active": n.active,
                "nuke_proposals": n.nuke_proposals,
                "peers": n.peers.into_iter().map(|p| serde_json::json!({
                    "endpoint_id": p.endpoint_id.to_string(),
                    "short_id": p.endpoint_id.to_string().chars().take(10).collect::<String>(),
                    "ip": p.ip,
                    "ipv6": p.ipv6,
                    "hostname": p.hostname,
                    "connection": p.connection,
                    "role": if p.is_coordinator { "admin" } else { "member" },
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    Json(serde_json::json!({
        "reachable": true,
        "endpoint_id": endpoint_id.to_string(),
        "endpoint_short": endpoint_id.to_string().chars().take(10).collect::<String>(),
        "active": active,
        "daemon_version": daemon_version,
        "networks": networks,
        "traffic": {
            "packets_rx": packets_rx, "packets_tx": packets_tx,
            "bytes_rx": bytes_rx, "bytes_tx": bytes_tx,
        },
        // Catch-up from tetron's MTU-DIAG-001: per-DropReason breakdown plus
        // successful-fragmentation counts, previously invisible through this
        // API. No frontend display for these yet -- exposed here so a future
        // pass can surface them without another API-shape catch-up.
        "drops": drops,
        "fragmented_ipv4": fragmented_ipv4,
        "fragmented_ipv6": fragmented_ipv6,
    }))
}

// ---------------------------------------------------------------------
// Phase 2 -- low-stakes mutations (create / join / leave / up / down)
// ---------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateReq {
    #[serde(default)]
    network_name: Option<String>,
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    subnet: Option<String>,
    /// NUKE-CONSENSUS proposer threshold (tetron's `--nuke-consensus`,
    /// catch-up from `NUKE-CONSENSUS-THRESHOLD-001`). `None` uses tetron's
    /// own default of 2.
    #[serde(default)]
    nuke_consensus: Option<u32>,
}

/// `POST /api/networks`. Always creates a closed (`Restricted`) network --
/// tetron's own CLI removed the ability to create an open one
/// (`MINIMAL-013`), so there is nothing to expose a toggle for here either.
pub async fn create_network(Json(req): Json<CreateReq>) -> Response {
    let resp = call(IpcMessage::Create {
        mode: tetron_proto::GroupMode::Restricted,
        network_name: req.network_name,
        hostname: req.hostname,
        transport: None,
        subnet: req.subnet,
        nuke_consensus: req.nuke_consensus,
    })
    .await;
    match resp {
        Ok(IpcMessage::Created {
            network,
            network_key,
            my_ip,
            my_ipv6,
            warning,
            initial_invite_key,
            subnet,
        }) => Json(serde_json::json!({
            "ok": true,
            "network": network,
            "network_key": network_key.to_string(),
            "my_ip": my_ip,
            "my_ipv6": my_ipv6,
            "warning": warning,
            "initial_invite_key": initial_invite_key,
            "subnet": subnet,
        }))
        .into_response(),
        Ok(IpcMessage::Error { message }) => ActionResult::err(message).into_response(),
        Ok(other) => ActionResult::err(format!("unexpected daemon response: {other:?}")).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, ActionResult::err(e)).into_response(),
    }
}

#[derive(Deserialize)]
pub struct JoinReq {
    /// The bs58 invite code as copy-pasted by the user, e.g. from another
    /// member running `tetron invite <net> create`. Decoded server-side
    /// (see `decode_invite` below) rather than in JavaScript, since the
    /// decode logic depends on `bs58` + `iroh::EndpointId`, both already
    /// available on the Rust side and not worth re-implementing in JS.
    invite_code: String,
    #[serde(default)]
    alias: Option<String>,
    #[serde(default)]
    hostname: Option<String>,
}

/// Mirrors `src/invite.rs`'s `decode_invite_code` in the main tetron crate:
/// an invite code is `bs58(network_pubkey(32 bytes) || secret)`. That
/// function lives in a binary crate not meant to be depended on as a
/// library, so it's re-implemented here rather than imported.
fn decode_invite(code: &str) -> Result<(iroh::EndpointId, Vec<u8>), String> {
    let bytes = bs58::decode(code)
        .into_vec()
        .map_err(|e| format!("invalid invite code: {e}"))?;
    if bytes.len() <= 32 {
        return Err(format!(
            "invalid invite code: expected more than 32 bytes, got {}",
            bytes.len()
        ));
    }
    let net: [u8; 32] = bytes[0..32]
        .try_into()
        .map_err(|_| "invalid invite code: malformed network key".to_string())?;
    let secret = bytes[32..].to_vec();
    let network_pubkey =
        iroh::EndpointId::from_bytes(&net).map_err(|e| format!("invalid network key in invite: {e}"))?;
    Ok((network_pubkey, secret))
}

/// `POST /api/networks/join`.
pub async fn join_network(Json(req): Json<JoinReq>) -> Response {
    let (network_key, secret) = match decode_invite(&req.invite_code) {
        Ok(v) => v,
        Err(e) => return ActionResult::err(e).into_response(),
    };
    let resp = call(IpcMessage::Join {
        network_key: network_key.to_string(),
        alias: req.alias,
        hostname: req.hostname,
        transport: None,
        invite: Some(secret),
    })
    .await;
    match resp {
        Ok(IpcMessage::Joined { network, my_ip, my_ipv6, warning }) => Json(serde_json::json!({
            "ok": true, "network": network, "my_ip": my_ip, "my_ipv6": my_ipv6, "warning": warning,
        }))
        .into_response(),
        Ok(IpcMessage::Error { message }) => ActionResult::err(message).into_response(),
        Ok(other) => ActionResult::err(format!("unexpected daemon response: {other:?}")).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, ActionResult::err(e)).into_response(),
    }
}

#[derive(Deserialize)]
pub struct LeaveReq {
    #[serde(default)]
    force: bool,
}

/// `POST /api/networks/:name/leave`. `:name` is the local display name (the
/// same value that keys `self.networks` daemon-side), NOT the network's
/// short id -- matching `tetron leave`'s own CLI argument.
pub async fn leave_network(Path(name): Path<String>, Json(req): Json<LeaveReq>) -> Json<ActionResult> {
    run_action(IpcMessage::Leave { network: name, force: req.force }).await
}

#[derive(Deserialize)]
pub struct ResumeStandbyReq {
    #[serde(default)]
    hostname: Option<String>,
    /// `None` means daemon-wide (every joined network); `Some(name)` scopes
    /// to just that one network (STANDBY-PER-NETWORK).
    #[serde(default)]
    network: Option<String>,
}

/// `POST /api/resume`.
pub async fn activate(Json(req): Json<ResumeStandbyReq>) -> Json<ActionResult> {
    run_action(IpcMessage::Resume { hostname: req.hostname, network: req.network }).await
}

/// `POST /api/standby`.
pub async fn deactivate(Json(req): Json<ResumeStandbyReq>) -> Json<ActionResult> {
    run_action(IpcMessage::Standby { network: req.network }).await
}

#[derive(Deserialize)]
pub struct SyncReq {
    /// `None` triggers every joined network's poller; `Some(name)` scopes to
    /// just that one (SYNC-001).
    #[serde(default)]
    network: Option<String>,
}

/// `POST /api/sync`. Manually wakes the DHT/group poller instead of waiting
/// for its configured interval -- causes no local mutation, same
/// any-local-user authorization tier as `Status` (AUTHZ-001/SYNC-001 on the
/// daemon side).
pub async fn sync_now(Json(req): Json<SyncReq>) -> Json<ActionResult> {
    run_action(IpcMessage::Sync { network: req.network }).await
}

// ---------------------------------------------------------------------
// Invites (Phase 2)
// ---------------------------------------------------------------------

#[derive(Deserialize)]
pub struct InviteCreateReq {
    /// Human-readable duration ("24h", "7d"), parsed daemon-side. `None`
    /// uses whatever default expiry the daemon applies.
    #[serde(default)]
    expires: Option<String>,
}

/// `POST /api/networks/:name/invites`.
pub async fn invite_create(Path(name): Path<String>, Json(req): Json<InviteCreateReq>) -> Response {
    match call(IpcMessage::InviteCreate { network: name, expires: req.expires }).await {
        Ok(IpcMessage::InviteCreated { invite_key, invite_id, expires_at }) => Json(serde_json::json!({
            "ok": true, "invite_key": invite_key, "invite_id": invite_id, "expires_at": expires_at,
        }))
        .into_response(),
        Ok(IpcMessage::Error { message }) => ActionResult::err(message).into_response(),
        Ok(other) => ActionResult::err(format!("unexpected daemon response: {other:?}")).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, ActionResult::err(e)).into_response(),
    }
}

/// `GET /api/networks/:name/invites`.
pub async fn invite_list(Path(name): Path<String>) -> Response {
    match call(IpcMessage::InviteList { network: name }).await {
        Ok(IpcMessage::InviteListResponse { invites }) => {
            Json(serde_json::json!({"ok": true, "invites": invites})).into_response()
        }
        Ok(IpcMessage::Error { message }) => ActionResult::err(message).into_response(),
        Ok(other) => ActionResult::err(format!("unexpected daemon response: {other:?}")).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, ActionResult::err(e)).into_response(),
    }
}

/// `DELETE /api/networks/:name/invites/:invite_id`.
pub async fn invite_revoke(Path((name, invite_id)): Path<(String, String)>) -> Json<ActionResult> {
    run_action(IpcMessage::InviteRevoke { network: name, invite_id }).await
}

// ---------------------------------------------------------------------
// Phase 3 -- full admin surface (kick / admin add+list / nuke)
// ---------------------------------------------------------------------

#[derive(Deserialize)]
pub struct KickReq {
    peer: String,
}

/// `POST /api/networks/:net_id/kick`. `:net_id` is the network's own short
/// id (its public key prefix, as shown by `tetron status`'s `network_key`
/// line -- renamed from `id` by tetron's `CLI-VOCAB-005`), NOT the local
/// display name -- matching `tetron kick`'s own CLI argument. `peer` must
/// likewise be a short id or endpoint id, never a hostname -- kick is
/// destructive and needs a cryptographic identity, not a mutable,
/// spoofable one. The frontend's confirmation dialog is the enforcement
/// point for "are you sure"; this handler just passes the request through.
pub async fn kick_member(Path(net_id): Path<String>, Json(req): Json<KickReq>) -> Json<ActionResult> {
    run_action(IpcMessage::Kick { network_key: net_id, endpoint_id: req.peer }).await
}

#[derive(Deserialize)]
pub struct AdminAddReq {
    peer: String,
}

/// `POST /api/networks/:name/admin`. `:name` is the local display name here
/// (matching `tetron admin <name> add <peer>`) -- admin add is additive,
/// not destructive, so (unlike kick/nuke) it resolves by the friendlier
/// local name and accepts a hostname for `peer` too.
pub async fn admin_add(Path(name): Path<String>, Json(req): Json<AdminAddReq>) -> Json<ActionResult> {
    run_action(IpcMessage::AdminAdd { network: name, peer: req.peer }).await
}

/// `GET /api/networks/:name/admin`.
pub async fn admin_list(Path(name): Path<String>) -> Response {
    match call(IpcMessage::AdminList { network: name }).await {
        Ok(IpcMessage::AdminListResponse { admins }) => {
            Json(serde_json::json!({"ok": true, "admins": admins})).into_response()
        }
        Ok(IpcMessage::Error { message }) => ActionResult::err(message).into_response(),
        Ok(other) => ActionResult::err(format!("unexpected daemon response: {other:?}")).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, ActionResult::err(e)).into_response(),
    }
}

#[derive(Deserialize)]
pub struct NukeReq {
    #[serde(default)]
    force: bool,
    #[serde(default)]
    cancel: bool,
    #[serde(default)]
    second: Option<String>,
}

/// `POST /api/networks/:net_id/nuke`. `:net_id` is the network's own short
/// id, same as kick. With a single coordinator this destroys immediately;
/// with two or more it participates in NUKE-CONSENSUS (propose / second /
/// cancel) exactly as the CLI does -- the frontend surfaces
/// `nuke_proposals` from the status poll so the confirmation dialog can
/// show "propose" vs "second an existing proposal" correctly instead of
/// presenting one generic "destroy" button that hides what will actually
/// happen.
pub async fn nuke_network(Path(net_id): Path<String>, Json(req): Json<NukeReq>) -> Json<ActionResult> {
    run_action(IpcMessage::Nuke { network_key: net_id, force: req.force, cancel: req.cancel, second: req.second }).await
}

// ---------------------------------------------------------------------
// Addon-install framework
// ---------------------------------------------------------------------

/// `GET /api/addons`. Unlike every other handler in this file, this never
/// touches the daemon's IPC socket at all -- addon install/status is a
/// host-level concern (a service registered outside tetron itself), not a
/// daemon one.
pub async fn addons_list() -> Json<Vec<crate::addons::AddonStatus>> {
    Json(crate::addons::list_status().await)
}

/// `POST /api/addons/:id/install`. Can take several seconds (download +
/// verify + the addon's own service registration) -- the frontend shows a
/// "working" state for the duration rather than assuming this is instant
/// like the other action endpoints.
pub async fn addon_install(Path(id): Path<String>) -> Json<ActionResult> {
    match crate::addons::install(&id).await {
        Ok(message) => ActionResult::ok(message),
        Err(e) => ActionResult::err(e.to_string()),
    }
}

pub async fn addon_uninstall(Path(id): Path<String>) -> Json<ActionResult> {
    match crate::addons::uninstall(&id).await {
        Ok(message) => ActionResult::ok(message),
        Err(e) => ActionResult::err(e.to_string()),
    }
}
