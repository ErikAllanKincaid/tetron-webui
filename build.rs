// `main.rs` embeds the frontend at compile time via `include_str!`. Cargo's
// own change detection doesn't know to watch files referenced that way
// (only .rs files trigger an automatic rebuild) -- without this, editing
// static/*.html/css/js and running `cargo build` can silently serve stale
// content, since the crate looks unchanged from Cargo's point of view.
//
// Also stamps the git short SHA into the binary (`GIT_SHA` env var) so
// `tetron-webui --version`/`version` can distinguish two builds that share
// the same, unbumped `Cargo.toml` version -- same pattern as tetron core's
// own `build.rs`. Falls back to `"unknown"` when git is unavailable (e.g. a
// source tarball build outside a checkout), so the build never fails for
// lack of a `.git` dir.
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=static/index.html");
    println!("cargo:rerun-if-changed=static/style.css");
    println!("cargo:rerun-if-changed=static/app.js");
    println!("cargo:rerun-if-changed=static/vendor/qrcode.js");

    let sha = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GIT_SHA={sha}");

    // Rebuild when HEAD moves so the stamp stays current. `.git/HEAD` covers
    // commits/checkouts; the packed-refs/refs paths cover branch updates.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    println!("cargo:rerun-if-changed=.git/packed-refs");
}
