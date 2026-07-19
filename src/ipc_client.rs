//! Thin wrapper around `tetron_proto::ipc`'s connect/send/recv helpers.
//!
//! Every HTTP handler in `api.rs` needs the same three steps: open a fresh
//! connection to the daemon's Unix socket, send one `IpcMessage`, and read
//! back exactly one reply. tetron's own CLI opens a new connection per
//! command rather than keeping one persistent connection alive -- we follow
//! that same convention here, since it means a daemon restart is never
//! something this process has to detect and recover from: the *next* request
//! just reconnects fresh and either works or reports "daemon unreachable".

use tetron_proto::ipc::{self, IpcMessage};

/// Send one `IpcMessage` to the daemon and return its reply.
///
/// This only handles the *transport* layer -- did we manage to talk to the
/// daemon at all. It deliberately does NOT unwrap `IpcMessage::Error` into
/// an `Err` here, because "the daemon understood the request and rejected
/// it" (e.g. permission denied, network not found) is a different kind of
/// failure than "we could not reach the daemon at all", and callers usually
/// want to show those two cases differently to the user. Callers match on
/// the returned `IpcMessage` themselves.
pub async fn call(msg: IpcMessage) -> Result<IpcMessage, String> {
    let mut stream = ipc::connect()
        .await
        .map_err(|e| format!("could not reach the tetron daemon: {e}"))?;
    ipc::send(&mut stream, msg)
        .await
        .map_err(|e| format!("failed to send request to daemon: {e}"))?;
    ipc::recv(&mut stream)
        .await
        .map_err(|e| format!("failed to read daemon response: {e}"))
}

/// Convenience for the extremely common case of "I expect either `Ok` or
/// `Error` back, and just want the message string either way, tagged with
/// which one it was". Used by every mutating endpoint (create, join, leave,
/// kick, nuke, admin add, invite create/revoke, up/down) — none of them
/// return interesting structured data on success, just a human-readable
/// confirmation string, so this collapses the repetitive match into one call.
pub async fn call_expect_ok(msg: IpcMessage) -> Result<String, String> {
    match call(msg).await? {
        IpcMessage::Ok { message } => Ok(message),
        IpcMessage::Error { message } => Err(message),
        other => Err(format!("unexpected daemon response: {other:?}")),
    }
}
