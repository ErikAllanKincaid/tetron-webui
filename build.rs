// `main.rs` embeds the frontend at compile time via `include_str!`. Cargo's
// own change detection doesn't know to watch files referenced that way
// (only .rs files trigger an automatic rebuild) -- without this, editing
// static/*.html/css/js and running `cargo build` can silently serve stale
// content, since the crate looks unchanged from Cargo's point of view.
fn main() {
    println!("cargo:rerun-if-changed=static/index.html");
    println!("cargo:rerun-if-changed=static/style.css");
    println!("cargo:rerun-if-changed=static/app.js");
}
