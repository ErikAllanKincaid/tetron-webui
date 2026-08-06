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
use axum::response::IntoResponse;
use axum::Router;
use clap::{Parser, Subcommand};

/// Default port the web server listens on. Override via `TETRON_WEBUI_PORT`
/// env var (both the server and downstream consumers like tetron-systray
/// read the same var), or via `tetron-webui install --port <n>`.
pub(crate) const DEFAULT_PORT: u16 = 7870;

/// Full version string: the crate version plus the git short SHA stamped in
/// by `build.rs` (e.g. `0.8.4 (a1b2c3d4)`). The SHA distinguishes two builds
/// that share the same, unbumped crate version -- same pattern as tetron
/// core's own `FULL_VERSION`.
pub(crate) const FULL_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_SHA"), ")");

/// Resolve the web server's listen port. Reads `TETRON_WEBUI_PORT` from the
/// environment; falls back to `DEFAULT_PORT` if unset or unparsable.
pub(crate) fn resolve_port() -> u16 {
    std::env::var("TETRON_WEBUI_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT)
}

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
    Install {
        /// Port to bind the web server on. Also sets `TETRON_WEBUI_PORT`
        /// in the service unit so downstream consumers (tetron-systray)
        /// discover the correct URL.
        #[arg(short = 'p', long, env = "TETRON_WEBUI_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
    },
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
// Vendored third-party DOM diffing (Zero-Clause BSD, bigskysoftware/idiomorph)
// -- see static/vendor/idiomorph.js's own header for provenance.
const IDIOMORPH_JS: &str = include_str!("../static/vendor/idiomorph.js");

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

async fn serve_idiomorph_js() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], IDIOMORPH_JS)
}

// --- Config-Backup script proxy -------------------------------------------------
//
// The script lives in the tetron repo (contrib/tetron-backup.sh) and is
// proxied here so the Details popup can hand the user a `curl` command
// against this webui instead of a repo clone. `curl` shell-out matches the
// addon-download machinery in addons.rs (no new HTTP dependency). Fetched
// from `backup_script_url()` (TETRON_BACKUP_RAW_URL override, default the
// tetron repo's main branch) and cached in memory for 10 minutes; when the
// upstream fetch fails, the last known good copy is served if one exists
// (stale is better than a dead endpoint when GitHub is unreachable), and a
// brand-new webui with no cache returns 502 naming the direct URL.

const BACKUP_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(600);
static BACKUP_CACHE: std::sync::Mutex<Option<(std::time::Instant, String)>> =
    std::sync::Mutex::new(None);

async fn fetch_backup_script(url: &str) -> anyhow::Result<String> {
    let out = tokio::process::Command::new("curl")
        .args(["-fsSL"])
        .arg(url)
        .output()
        .await?;
    anyhow::ensure!(out.status.success(), "curl exited with {}", out.status);
    Ok(String::from_utf8(out.stdout)?)
}

fn serve_backup_ok(body: String) -> axum::response::Response {
    (
        axum::http::StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "text/x-shellscript"),
            (axum::http::header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

async fn serve_backup_script() -> axum::response::Response {
    let url = addons::backup_script_url();
    let cached = BACKUP_CACHE.lock().unwrap().clone();
    if let Some((fetched_at, body)) = &cached
        && fetched_at.elapsed() < BACKUP_CACHE_TTL
    {
        return serve_backup_ok(body.clone());
    }
    match fetch_backup_script(&url).await {
        Ok(body) => {
            *BACKUP_CACHE.lock().unwrap() = Some((std::time::Instant::now(), body.clone()));
            serve_backup_ok(body)
        }
        Err(e) => {
            if let Some((_, body)) = cached {
                return serve_backup_ok(body);
            }
            (
                axum::http::StatusCode::BAD_GATEWAY,
                [
                    (axum::http::header::CONTENT_TYPE, "text/plain"),
                    (axum::http::header::CACHE_CONTROL, "no-cache"),
                ],
                format!(
                    "could not fetch the backup script from {url}: {e}\n\
                     fetch it directly with: curl -fsSL '{url}' -o tetron-backup.sh"
                ),
            )
                .into_response()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Install { port }) => return service::install(port),
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
        .route("/vendor/idiomorph.js", get(serve_idiomorph_js))
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
        .route("/api/addons/{id}/uninstall", post(api::addon_uninstall))
        // Config-Backup addon: the script itself, fetched by the Details
        // popup's curl command
        .route("/addons/tetron-backup.sh", get(serve_backup_script));

    let port = resolve_port();
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("tetron-webui listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
