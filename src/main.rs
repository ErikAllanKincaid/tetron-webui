//! `tetron-webui`: a browser dashboard/admin console for the `tetron` daemon.
//! Genuinely optional and separate from tetron itself -- see README.md.
//!
//! Binds `127.0.0.1` only, deliberately: no remote/WAN exposure by default.
//! Anyone wanting remote access puts a reverse proxy with real TLS + auth in
//! front of this themselves; that's a decision this project shouldn't make
//! on their behalf by defaulting to a public bind address.

mod api;
mod ipc_client;
mod service;

use axum::routing::{delete, get, post};
use axum::Router;
use clap::{Parser, Subcommand};

const BIND_ADDR: &str = "127.0.0.1:7870";

#[derive(Parser)]
#[command(name = "tetron-webui")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Install and start the per-user service (systemd --user on Linux,
    /// a launchd LaunchAgent on macOS) so tetron-webui runs at login
    /// instead of needing a terminal kept open
    Install,
    /// Stop and remove the per-user service
    Uninstall,
}

// Static frontend, embedded at compile time -- no file-serving dependency,
// no build step, matches the "vanilla HTML/CSS/JS, not a SPA framework"
// decision this project's design settled on.
const INDEX_HTML: &str = include_str!("../static/index.html");
const STYLE_CSS: &str = include_str!("../static/style.css");
const APP_JS: &str = include_str!("../static/app.js");

async fn serve_index() -> axum::response::Html<&'static str> {
    axum::response::Html(INDEX_HTML)
}

async fn serve_css() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "text/css")], STYLE_CSS)
}

async fn serve_js() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], APP_JS)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Install) => return service::install(),
        Some(Command::Uninstall) => return service::uninstall(),
        None => {}
    }

    let app = Router::new()
        // Frontend
        .route("/", get(serve_index))
        .route("/style.css", get(serve_css))
        .route("/app.js", get(serve_js))
        // Phase 1: read-only status
        .route("/api/status", get(api::get_status))
        // Phase 2: low-stakes mutations
        .route("/api/networks", post(api::create_network))
        .route("/api/networks/join", post(api::join_network))
        .route("/api/networks/{name}/leave", post(api::leave_network))
        .route("/api/resume", post(api::activate))
        .route("/api/standby", post(api::deactivate))
        .route("/api/sync", post(api::sync_now))
        .route(
            "/api/networks/{name}/invites",
            get(api::invite_list).post(api::invite_create),
        )
        .route("/api/networks/{name}/invites/{invite_id}", delete(api::invite_revoke))
        // Phase 3: full admin surface
        .route("/api/networks/{net_id}/kick", post(api::kick_member))
        .route(
            "/api/networks/{name}/admin",
            get(api::admin_list).post(api::admin_add),
        )
        .route("/api/networks/{net_id}/nuke", post(api::nuke_network));

    let listener = tokio::net::TcpListener::bind(BIND_ADDR).await?;
    eprintln!("tetron-webui listening on http://{BIND_ADDR}");
    axum::serve(listener, app).await?;
    Ok(())
}
