//! `tetron-webui`: a browser dashboard/admin console for the `tetron` daemon.
//! Genuinely optional and separate from tetron itself -- see README.md.
//!
//! Binds `127.0.0.1` only, deliberately: no remote/WAN exposure by default.
//! Anyone wanting remote access puts a reverse proxy with real TLS + auth in
//! front of this themselves; that's a decision this project shouldn't make
//! on their behalf by defaulting to a public bind address.

mod addons;
mod api;
mod ipc_client;
mod service;

use axum::routing::{delete, get, post};
use axum::Router;
use clap::{Parser, Subcommand};

const BIND_ADDR: &str = "127.0.0.1:7870";

/// Full version string: the crate version plus the git short SHA stamped in
/// by `build.rs` (e.g. `0.8.4 (a1b2c3d4)`). The SHA distinguishes two builds
/// that share the same, unbumped crate version -- same pattern as tetron
/// core's own `FULL_VERSION`.
const FULL_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_SHA"), ")");

#[derive(Parser)]
#[command(name = "tetron-webui", version = FULL_VERSION)]
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
    /// Print the tetron-webui version
    #[command(visible_alias = "ver")]
    Version,
}

// Static frontend, embedded at compile time -- no file-serving dependency,
// no build step, matches the "vanilla HTML/CSS/JS, not a SPA framework"
// decision this project's design settled on.
const INDEX_HTML: &str = include_str!("../static/index.html");
const STYLE_CSS: &str = include_str!("../static/style.css");
const APP_JS: &str = include_str!("../static/app.js");
// Vendored third-party QR encoder (MIT, kazuhikoarase/qrcode-generator) --
// see static/vendor/qrcode.js's own header for provenance.
const QRCODE_JS: &str = include_str!("../static/vendor/qrcode.js");
// Reuses tetron-systray's own icon unmodified -- see static/favicon.svg's
// own header for provenance.
const FAVICON_SVG: &str = include_str!("../static/favicon.svg");

async fn serve_index() -> axum::response::Html<&'static str> {
    axum::response::Html(INDEX_HTML)
}

async fn serve_css() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "text/css")], STYLE_CSS)
}

async fn serve_js() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], APP_JS)
}

async fn serve_qrcode_js() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], QRCODE_JS)
}

async fn serve_favicon() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "image/svg+xml")], FAVICON_SVG)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Install) => return service::install(),
        Some(Command::Uninstall) => return service::uninstall(),
        Some(Command::Version) => {
            println!("tetron-webui {FULL_VERSION}");
            return Ok(());
        }
        None => {}
    }

    let app = Router::new()
        // Frontend
        .route("/", get(serve_index))
        .route("/style.css", get(serve_css))
        .route("/app.js", get(serve_js))
        .route("/vendor/qrcode.js", get(serve_qrcode_js))
        .route("/favicon.svg", get(serve_favicon))
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
        .route("/api/networks/{net_id}/nuke", post(api::nuke_network))
        // Addon-install framework
        .route("/api/addons", get(api::addons_list))
        .route("/api/addons/{id}/install", post(api::addon_install))
        .route("/api/addons/{id}/uninstall", post(api::addon_uninstall));

    let listener = tokio::net::TcpListener::bind(BIND_ADDR).await?;
    eprintln!("tetron-webui listening on http://{BIND_ADDR}");
    axum::serve(listener, app).await?;
    Ok(())
}
