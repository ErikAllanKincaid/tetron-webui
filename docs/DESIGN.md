# UI/UX design decisions

Worked through in a `LEARNINGMODE` session 2026-07-18/19 (Erik: "not experienced in UI design, my specialty is pipelines") before any code was written. Captured here since it only existed as conversation until now, and the reasoning behind each choice matters as much as the choice itself if this ever needs revisiting.

## Information hierarchy: glance → scan → dig in

The starting principle, before any layout or code: a pipeline's interface is inputs/outputs and stages; a UI's interface is a human's attention — what they see first, what they can ignore, what they need to act on. Three tiers, in decreasing importance:

1. **Glance** — the "is everything okay" answer, answerable in under a second. One line: overall up/standby state, color-coded.
2. **Scan** — "which of my networks needs a look." One row per network: name, role, active/standby, member count. Never more than a glance's worth of reading per row, regardless of how much detail exists underneath.
3. **Dig in** — full detail, only shown on request. Per-peer connection type, RTT, byte counters, admin/nuke controls. Hidden by default.

This maps directly onto the shipped layout: the sticky header is tier 1, the network list is tier 2, and each network row's expand-in-place accordion is tier 3.

## Page regions

Real HTML5 landmark elements (`<header>`, `<main>`, `<footer>`), not generic `<div>`s — self-documenting, and screen readers can jump between them by name.

- `<header>`: sticky (stays visible while scrolling), holds tier 1 + the theme toggle. Its whole job is "is everything fine" — it should never scroll out of view.
- `<main>`: tier 2 (the network list). Tier 3 lives *inside* it, as a state change on an existing row, not a separate region.
- `<footer>`: low-priority aggregate info (traffic totals).

**Deliberately no sidebar or nav bar.** Those exist to switch between different pages or major views; Phase 1–3 is still a single page, so a nav element would have nowhere to point. Adding one now would be building an affordance for navigation that doesn't exist — restraint here is a decision, not an oversight.

## Color: status encoding, not decoration

In a dashboard, color's job is communicating state, not looking nice. This is the actual reason the tier-1 line works: the dot's color does more of the "is this okay" work than the words next to it. Pattern name: **semantic color** — the same idea behind git diffs, CI badges, traffic lights. Rule: 3–4 colors max for status meaning (`--status-up` green, `--status-standby` gray, `--status-down` red, `--status-warn` amber), never reused for anything that isn't actually that status. Consistency matters more than the exact shade chosen.

## Typography

Monospace (`--mono`, via the `.mono` class) for anything tabular or identifier-like — IPs, endpoint ids, byte counts. Columns of these actually align and scan when set this way, which is why every terminal tool and admin panel does it for data even while using a regular sans-serif for labels/prose. Applied selectively, not everywhere — this is the one design instinct that directly matched Erik's existing "plain text tools" aesthetic from decades of prior UI work, just scoped to where it actually earns its keep.

## Theming: dark default, real light/dark support, not "designed around dark only"

Dark chosen as the default because it's the strong convention for this exact tool category (Grafana, GitHub's dark mode, most Rust/Go ops dashboards) and because saturated status colors visually pop more against a dark background than a light one — directly amplifying the tier-1 signal.

**The technique that makes "toggle-able, not dark-only" actually true:** every color is a named CSS custom property (`--bg`, `--text`, `--status-up`, etc.), referenced by name everywhere else in the stylesheet, never a raw hex value inline. Light mode is a second value-set under `:root[data-theme="light"]` for the exact same names — there is only ever **one** design, expressed twice, so a dark-only assumption can't sneak in anywhere deep in the CSS; if something looks wrong in light mode, it's a wrong *value*, not a structural gap. The OS-level `prefers-color-scheme` is respected on first visit, before any explicit toggle click; the toggle itself just flips the `data-theme` attribute and persists the choice in `localStorage`.

## Reading width

`main { width: 92%; max-width: 900px; margin: auto; }` — percentage width alone has no ceiling (80% of an ultrawide monitor is still uncomfortably wide), so it's combined with a hard pixel cap. Never edge-to-edge on a wide screen; never uncomfortably cramped on a narrow one.

## Long scroll, not pagination

The real dividing line: pagination earns its keep for genuinely large, browsable datasets where someone needs to jump to a specific chunk directly (search results, an inbox). Scroll is better for continuous/homogeneous content someone is browsing, where the total is bounded. Neither is quite the right frame here, though — the accordion (tier 3 hidden by default) already keeps the *rendered* page short regardless of how much data exists underneath, which is a third option: **progressive disclosure already does the job pagination would otherwise be needed for.** Realistically nobody has hundreds of tetron networks on one node; this isn't a hyperscale admin panel with a large browsable dataset.

## Confirmation UX for destructive actions

Never a bare browser `confirm()` — too easy to reflexively click through, and can't carry situation-specific detail. A real modal, shared across every destructive action (leave-as-sole-coordinator, kick, nuke), that:

- States plainly what is about to happen, including irreversibility where it applies (nuke; a force-leave that strands members).
- For nuke specifically, reflects the *actual* NUKE-CONSENSUS state (`nuke_proposals` from the status poll) rather than one generic "destroy" button — the UI offers "propose," "second `<short-id>`'s proposal," or "cancel my proposal" depending on what's really going on, matching what the CLI itself would show.

## Known rough edges (see `TODO.md`)

Status polling does a wholesale `innerHTML` replacement of the network list every ~2s, which breaks an in-progress text selection (e.g. trying to copy an IP). Found via actual use, not anticipated during design — logged with root cause and fix approach in `TODO.md` rather than fixed reflexively.
