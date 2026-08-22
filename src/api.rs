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
    /// tetron's `--tor` flag -- routes this network's transport over Tor
    /// instead of relay/direct.
    #[serde(default)]
    tor: bool,
}

/// `POST /api/networks`. Always creates a closed (`Restricted`) network --
/// tetron's own CLI removed the ability to create an open one
/// (`MINIMAL-013`), so there is nothing to expose a toggle for here either.
pub async fn create_network(Json(req): Json<CreateReq>) -> Response {
    let resp = call(IpcMessage::Create {
        mode: tetron_proto::GroupMode::Restricted,
        network_name: req.network_name,
        hostname: req.hostname,
        transport: req.tor.then_some(tetron_proto::TransportMode::Tor),
        subnet: req.subnet,
        nuke_consensus: req.nuke_consensus,
        force: false,
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
    /// tetron's `--tor` flag -- should mirror the coordinator's own
    /// transport if it used one.
    #[serde(default)]
    tor: bool,
}

/// Length of the random invite secret, in bytes. Mirrors `SECRET_LEN` in
/// the main tetron crate's `src/invite.rs`.
const SECRET_LEN: usize = 16;
/// Length of the blake3 integrity checksum appended to an invite payload
/// (`INVITE-CHECKSUM-001`). Mirrors `CHECKSUM_LEN` in `src/invite.rs`.
const CHECKSUM_LEN: usize = 4;
/// Payload length of an invite code before the checksum: network pubkey
/// (32 bytes) + secret. Mirrors `PAYLOAD_LEN` in `src/invite.rs`.
const PAYLOAD_LEN: usize = 32 + SECRET_LEN;
/// Total raw length of a checksummed invite code (payload + checksum).
/// Mirrors `ENCODED_LEN` in `src/invite.rs`.
const ENCODED_LEN: usize = PAYLOAD_LEN + CHECKSUM_LEN;

/// Mirrors `src/invite.rs`'s `decode_invite_code` in the main tetron crate:
/// an invite code is `bs58(network_pubkey(32 bytes) || secret(16) ||
/// blake3(payload)[..4])` (`INVITE-CHECKSUM-001`), or the legacy 48-byte
/// unchecksummed form. That function lives in a binary crate not meant to
/// be depended on as a library, so it's re-implemented here rather than
/// imported. **Keep in sync with `src/invite.rs::decode_invite_code`** --
/// this copy went stale once already (found and fixed 2026-08-06, see
/// `DO-NOT-COMMIT/TODO_DETAILS.md`) when core added the checksum and this
/// side wasn't updated; there is no compiler link between the two to catch
/// drift automatically.
fn decode_invite(code: &str) -> Result<(iroh::EndpointId, Vec<u8>), String> {
    let bytes = bs58::decode(code)
        .into_vec()
        .map_err(|e| format!("invalid invite code: {e}"))?;
    let payload = match bytes.len() {
        // Checksummed form: 48-byte payload + 4-byte checksum.
        ENCODED_LEN => {
            let (payload, csum) = bytes.split_at(PAYLOAD_LEN);
            if csum != &blake3::hash(payload).as_bytes()[..CHECKSUM_LEN] {
                return Err("invalid invite code: checksum mismatch (corrupted or mistyped)".to_string());
            }
            payload
        }
        // Legacy unchecksummed form: 48-byte payload only.
        PAYLOAD_LEN => &bytes[..],
        other => {
            return Err(format!(
                "invalid invite code: expected {ENCODED_LEN} or {PAYLOAD_LEN} bytes, got {other}"
            ));
        }
    };
    let net: [u8; 32] = payload[0..32]
        .try_into()
        .map_err(|_| "invalid invite code: malformed network key".to_string())?;
    let secret = payload[32..].to_vec();
    let network_pubkey =
        iroh::EndpointId::from_bytes(&net).map_err(|e| format!("invalid network key in invite: {e}"))?;
    Ok((network_pubkey, secret))
}

#[cfg(test)]
mod decode_invite_tests {
    use super::*;

    fn test_id(seed: u8) -> iroh::EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        iroh::SecretKey::from(key_bytes).public()
    }

    fn encode_checksummed(network_pubkey: &iroh::EndpointId, secret: &[u8]) -> String {
        let mut bytes = Vec::with_capacity(PAYLOAD_LEN + CHECKSUM_LEN);
        bytes.extend_from_slice(network_pubkey.as_bytes());
        bytes.extend_from_slice(secret);
        bytes.extend_from_slice(&blake3::hash(&bytes).as_bytes()[..CHECKSUM_LEN]);
        bs58::encode(&bytes).into_string()
    }

    fn encode_legacy(network_pubkey: &iroh::EndpointId, secret: &[u8]) -> String {
        let mut bytes = Vec::with_capacity(PAYLOAD_LEN);
        bytes.extend_from_slice(network_pubkey.as_bytes());
        bytes.extend_from_slice(secret);
        bs58::encode(&bytes).into_string()
    }

    #[test]
    fn decodes_a_checksummed_code() {
        let id = test_id(1);
        let secret = [7u8; SECRET_LEN];
        let code = encode_checksummed(&id, &secret);
        let (decoded_id, decoded_secret) = decode_invite(&code).expect("valid checksummed code");
        assert_eq!(decoded_id, id);
        assert_eq!(decoded_secret, secret);
    }

    #[test]
    fn decodes_a_legacy_unchecksummed_code() {
        let id = test_id(2);
        let secret = [9u8; SECRET_LEN];
        let code = encode_legacy(&id, &secret);
        let (decoded_id, decoded_secret) = decode_invite(&code).expect("valid legacy code");
        assert_eq!(decoded_id, id);
        assert_eq!(decoded_secret, secret);
    }

    #[test]
    fn rejects_a_checksum_mismatch() {
        let id = test_id(3);
        let secret = [1u8; SECRET_LEN];
        let mut bytes = bs58::decode(encode_checksummed(&id, &secret)).into_vec().unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF; // corrupt one checksum byte
        let code = bs58::encode(&bytes).into_string();
        let err = decode_invite(&code).unwrap_err();
        assert!(err.contains("checksum mismatch"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_wrong_length() {
        let err = decode_invite(&bs58::encode([0u8; 10]).into_string()).unwrap_err();
        assert!(err.contains("expected"), "unexpected error: {err}");
    }

    /// This is the exact bug found 2026-08-06: before this fix, a
    /// checksummed (52-byte) code decoded "successfully" by treating the
    /// trailing 4 checksum bytes as part of the secret, silently producing
    /// a wrong (20-byte instead of 16-byte) secret instead of a clean
    /// decode error.
    #[test]
    fn checksummed_code_secret_is_exactly_secret_len_bytes() {
        let id = test_id(4);
        let secret = [3u8; SECRET_LEN];
        let code = encode_checksummed(&id, &secret);
        let (_, decoded_secret) = decode_invite(&code).expect("valid checksummed code");
        assert_eq!(decoded_secret.len(), SECRET_LEN);
    }
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
        transport: req.tor.then_some(tetron_proto::TransportMode::Tor),
        invite: Some(secret),
        force: false,
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

// ---------------------------------------------------------------------
// Sync Receiver addon: live configuration, all shelled out through
// `sync_receiver.rs` to the installed `tetron-sync-receiver` binary's own
// CLI (`--json`) -- this webui never reimplements rsyncd.conf/module/
// allow-list logic itself. Same reachability-over-hard-failure shape as
// `get_status` (BAD_GATEWAY + ActionResult::err on failure, e.g. the addon
// not being installed) rather than a bare 500.
// ---------------------------------------------------------------------

pub async fn sync_receiver_status() -> Response {
    match crate::sync_receiver::status().await {
        Ok(status) => Json(status).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, ActionResult::err(e.to_string())).into_response(),
    }
}

pub async fn sync_receiver_modules_list() -> Response {
    match crate::sync_receiver::list_modules().await {
        Ok(modules) => Json(modules).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, ActionResult::err(e.to_string())).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ModuleAddReq {
    name: String,
    path: String,
}

pub async fn sync_receiver_module_add(Json(req): Json<ModuleAddReq>) -> Json<ActionResult> {
    match crate::sync_receiver::add_module(&req.name, &req.path).await {
        Ok(()) => ActionResult::ok(format!("Module '{}' saved.", req.name)),
        Err(e) => ActionResult::err(e.to_string()),
    }
}

pub async fn sync_receiver_module_remove(Path(name): Path<String>) -> Json<ActionResult> {
    match crate::sync_receiver::remove_module(&name).await {
        Ok(()) => ActionResult::ok(format!("Module '{name}' removed.")),
        Err(e) => ActionResult::err(e.to_string()),
    }
}

pub async fn sync_receiver_allow_list() -> Response {
    match crate::sync_receiver::list_allow().await {
        Ok(ips) => Json(ips).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, ActionResult::err(e.to_string())).into_response(),
    }
}

#[derive(Deserialize)]
pub struct AllowAddReq {
    ip: String,
}

pub async fn sync_receiver_allow_add(Json(req): Json<AllowAddReq>) -> Json<ActionResult> {
    match crate::sync_receiver::add_allow_ip(&req.ip).await {
        Ok(()) => ActionResult::ok(format!("'{}' allowed.", req.ip)),
        Err(e) => ActionResult::err(e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct AllowAddPeerReq {
    hostname: String,
}

pub async fn sync_receiver_allow_add_peer(Json(req): Json<AllowAddPeerReq>) -> Json<ActionResult> {
    match crate::sync_receiver::add_allow_peer(&req.hostname).await {
        Ok(()) => ActionResult::ok(format!("'{}' allowed.", req.hostname)),
        Err(e) => ActionResult::err(e.to_string()),
    }
}

pub async fn sync_receiver_allow_remove(Path(ip): Path<String>) -> Json<ActionResult> {
    match crate::sync_receiver::remove_allow(&ip).await {
        Ok(()) => ActionResult::ok(format!("'{ip}' removed.")),
        Err(e) => ActionResult::err(e.to_string()),
    }
}

pub async fn sync_receiver_enable() -> Json<ActionResult> {
    match crate::sync_receiver::enable().await {
        Ok(()) => ActionResult::ok("Sync Receiver started.".to_string()),
        Err(e) => ActionResult::err(e.to_string()),
    }
}

pub async fn sync_receiver_disable() -> Json<ActionResult> {
    match crate::sync_receiver::disable().await {
        Ok(()) => ActionResult::ok("Sync Receiver stopped.".to_string()),
        Err(e) => ActionResult::err(e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct PortReq {
    port: u16,
}

pub async fn sync_receiver_set_port(Json(req): Json<PortReq>) -> Json<ActionResult> {
    match crate::sync_receiver::set_port(req.port).await {
        Ok(()) => ActionResult::ok(format!("Port changed to {}.", req.port)),
        Err(e) => ActionResult::err(e.to_string()),
    }
}
