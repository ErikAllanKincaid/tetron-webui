# tetron-webui TODO

- [x] **Rename project from `tetron-web` to `tetron-webui`**, 2026-07-19, to
  follow the naming convention used by `TripoSG-WebUI` and the broader
  "X-webui" pattern (`stable-diffusion-webui`, `text-generation-webui`).
  Erik chose lowercase `tetron-webui` over `tetron-WebUI` (matches
  Rust/Cargo package-naming convention and tetron's own lowercase binary
  naming). Done: repo directory (`~/code/tetron-web` -> `~/code/tetron-webui`),
  `Cargo.toml` (`[package] name`, `[[bin]] name`, `repository` URL),
  `Dockerfile`, README title + self-references, `src/main.rs` doc comment +
  startup log line, `static/app.js` comments + `localStorage` key, this
  TODO's own header + the two repo-link bullets below, and the
  `tetron-web`/`tetron-tray` mentions in the auto-memory file
  `feedback_ui_addons_not_bound_by_cli_minimalism.md`. Deliberately left
  untouched: `docs/PLAN.md` and `docs/IDEAS_WebUI_Systray.md`, both
  explicitly frozen historical/provenance documents ("kept here verbatim,
  mistake and all, rather than quietly edited") -- rewriting their
  `tetron-web` references would falsify the record of what was actually
  decided/built under that name at the time.

- **Text selection breaks during polling.** Root cause: `render()` in
  `static/app.js` sets `container.innerHTML = ...` on every poll cycle
  (~2s), wholesale replacing every DOM node inside `#networks` -- even
  when the underlying data hasn't actually changed. Any active text
  selection anchored inside that container (e.g. copying an IP,
  endpoint id, or invite key) gets destroyed on the next re-render,
  since the new nodes are different objects even if the rendered text
  looks identical. Fix: before re-rendering, check
  `window.getSelection()`; if there's a non-collapsed selection anchored
  inside `#networks`, skip that poll cycle's render and try again next
  tick, rather than tearing down what the user is actively selecting. A
  more thorough fix (only mutate the specific nodes whose data actually
  changed, instead of full `innerHTML` replacement) would also solve
  this and reduce flicker generally, but is meaningfully more work; the
  selection-check is the pragmatic fix for the immediate problem.

- **Copy button for IP addresses** (and other copyable values -- endpoint
  ids, invite keys). A small button next to each that calls
  `navigator.clipboard.writeText(...)` on click. Complements the
  selection-breaking fix above rather than replacing it: a button
  sidesteps the re-render problem entirely for the common case (no text
  selection involved, so nothing to lose on the next poll tick), but
  people may still want to manually select/copy other text (e.g. a whole
  nuke-proposal banner) where the selection-preservation fix still
  matters. Worth doing both.

- **Link to instructions.** Add a link (probably in the footer) to
  tetron's usage docs (`docs/HOWTO.md` in the `tetron` repo). Blocked on
  `tetron` actually being pushed/public on GitHub -- there's no real URL
  to point at yet, only a local file path.

- **Links to both GitHub repos.** Footer links to
  `https://github.com/ErikAllanKincaid/tetron` (already real) and
  `https://github.com/ErikAllanKincaid/tetron-webui` (doesn't exist yet --
  this repo has never been pushed). Do the `tetron-webui` link once that
  repo is actually created and pushed.

- **Link to download the binary.** For `tetron`: link to
  `https://github.com/ErikAllanKincaid/tetron/releases/latest` (already
  real, matches the download URL convention `tetron`'s own README/HOWTO
  already use). For `tetron-webui` itself: no CI/release pipeline exists
  yet -- would need a `release.yml` built for this repo (matching
  `tetron`'s own, `.github/workflows/release.yml`) before there's
  anything real to link to. Separate, unscoped task.
