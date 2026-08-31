// tetron-webui frontend. Vanilla JS on purpose -- no framework, no build
// step, matches the "single page, don't pre-import a SPA framework you
// don't need yet" decision this project's design settled on.

"use strict";

// -----------------------------------------------------------------------
// Theme toggle: flips a data-theme attribute on <html>, persisted in
// localStorage. Absence of a saved preference means "follow the OS", which
// the CSS's prefers-color-scheme media query already handles on its own --
// we only ever write an explicit value here once the user has actually
// clicked the toggle at least once.
// -----------------------------------------------------------------------

function currentTheme() {
  const saved = localStorage.getItem("tetron-webui-theme");
  if (saved) return saved;
  return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

function applyTheme(theme) {
  document.documentElement.setAttribute("data-theme", theme);
}

function toggleTheme() {
  const next = currentTheme() === "dark" ? "light" : "dark";
  localStorage.setItem("tetron-webui-theme", next);
  applyTheme(next);
}

applyTheme(currentTheme());
document.getElementById("theme-toggle").addEventListener("click", toggleTheme);

// -----------------------------------------------------------------------
// Small fetch helpers. Every mutating call goes through postJson, which
// always resolves to the parsed body (even on a daemon-level rejection --
// see api.rs's doc comment on why those are still HTTP 200) and only
// rejects on an actual network-level failure (tetron-webui itself
// unreachable, which basically never happens since you're talking to it
// right now, but worth not pretending can't fail).
// -----------------------------------------------------------------------

async function getJson(url) {
  const res = await fetch(url);
  return res.json();
}

async function postJson(url, body) {
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body || {}),
  });
  return res.json();
}

async function deleteJson(url) {
  const res = await fetch(url, { method: "DELETE" });
  return res.json();
}

// -----------------------------------------------------------------------
// Confirmation modal -- shared by every destructive action (leave, kick,
// nuke). Deliberately not a bare browser confirm(): those are too easy to
// reflexively click through, and can't show the kind of specific,
// situation-dependent detail (e.g. "this will strand N members") the
// daemon's own error messages already carry.
// -----------------------------------------------------------------------

const modal = document.getElementById("confirm-modal");
const modalTitle = document.getElementById("confirm-title");
const modalBody = document.getElementById("confirm-body");
const modalOk = document.getElementById("confirm-ok");
const modalCancel = document.getElementById("confirm-cancel");

function confirmAction(title, body, onConfirm) {
  modalTitle.textContent = title;
  modalBody.textContent = body;
  modal.classList.remove("hidden");

  const cleanup = () => {
    modal.classList.add("hidden");
    modalOk.removeEventListener("click", handleOk);
    modalCancel.removeEventListener("click", handleCancel);
  };
  const handleOk = () => { cleanup(); onConfirm(); };
  const handleCancel = () => cleanup();

  modalOk.addEventListener("click", handleOk);
  modalCancel.addEventListener("click", handleCancel);
}

// -----------------------------------------------------------------------
// Invite QR modal. Shown right after minting an invite (or creating a
// network, which auto-mints one) -- the raw invite key is only ever
// available at that moment. GET /api/networks/:name/invites (loadInvites,
// below) never returns it back: it's a single-use secret, not stored
// server-side for redisplay, so there is no "show QR" action on an
// already-listed invite row, only on a freshly minted one.
// -----------------------------------------------------------------------

const qrModal = document.getElementById("qr-modal");
const qrTitle = document.getElementById("qr-title");
const qrCode = document.getElementById("qr-code");
const qrCodeText = document.getElementById("qr-code-text");
const qrCopy = document.getElementById("qr-copy");
const qrClose = document.getElementById("qr-close");

function showInviteQr(inviteKey, title) {
  // typeNumber 0 = auto-pick the smallest QR version that fits; 'M' =
  // ~15% error correction, qrcode-generator's own default recommendation
  // for general-purpose codes (enough to survive a scuffed phone screen
  // without inflating the module count on this fairly long bs58 string).
  const qr = qrcode(0, "M");
  qr.addData(inviteKey);
  qr.make();
  qrTitle.textContent = title || "Invite key";
  qrCode.innerHTML = qr.createSvgTag(4);
  qrCodeText.textContent = inviteKey;
  qrModal.classList.remove("hidden");

  const cleanup = () => {
    qrModal.classList.add("hidden");
    qrCopy.removeEventListener("click", handleCopy);
    qrClose.removeEventListener("click", handleClose);
  };
  const handleCopy = () => {
    navigator.clipboard.writeText(inviteKey).then(() => {
      qrCopy.textContent = "Copied!";
      setTimeout(() => { qrCopy.textContent = "Copy"; }, 1200);
    });
  };
  const handleClose = () => cleanup();

  qrCopy.addEventListener("click", handleCopy);
  qrClose.addEventListener("click", handleClose);
}

// -----------------------------------------------------------------------
// Detail popover -- shown when clicking a peer's hostname or a network's
// name. Both are a superset of fields already in the polled /api/status
// JSON that just never had anywhere to be shown (endpoint_id, network_key)
// -- purely a frontend reveal, no new backend data.
// -----------------------------------------------------------------------

const detailModal = document.getElementById("detail-modal");
const detailTitle = document.getElementById("detail-title");
const detailBody = document.getElementById("detail-body");
const detailClose = document.getElementById("detail-close");

function detailRow(label, value, copyable) {
  const val = value || "(none)";
  return `<div class="detail-row">
    <span class="detail-label">${label}</span>
    <span class="detail-value mono">${val}${copyable && value ? copyBtn(value) : ""}</span>
  </div>`;
}

function showDetail(title, rows) {
  detailTitle.textContent = title;
  detailBody.innerHTML = rows.map((r) => detailRow(r.label, r.value, r.copyable)).join("");
  detailModal.classList.remove("hidden");
}

detailClose.addEventListener("click", () => detailModal.classList.add("hidden"));

// Dismiss the detail popup by clicking the backdrop (clicking anywhere
// outside the card) or pressing Escape. The popup content can outgrow the
// viewport (Config Backup's details), and its Close button sits at the
// bottom of the scroll -- without these two the user can get stuck with
// no way to close it. Scoped to the detail modal on purpose: the confirm
// modal is a deliberate two-button destructive flow and must not be
// dismissable by accident.
detailModal.addEventListener("click", (e) => {
  if (e.target === detailModal) detailModal.classList.add("hidden");
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !detailModal.classList.contains("hidden")) {
    detailModal.classList.add("hidden");
  }
});

// -----------------------------------------------------------------------
// Status polling (Phase 1). This is also the entire "reconnect after a
// daemon restart" story: every poll independently reconnects via
// tetron-webui's own backend, so there is no persistent connection on this
// side to go stale -- a poll just fails once, then succeeds again once the
// daemon's socket reappears.
// -----------------------------------------------------------------------

// 2s was unusually aggressive for a human-watched dashboard (most
// comparable tools poll in the 5-10s range); relaxed alongside the
// Idiomorph switch above, which is what actually fixes the blink/focus-
// loss this interval used to make worse by sheer frequency.
const POLL_INTERVAL_MS = 10000;

// Tracks which networks' admin <details> are currently open (by name)
// across re-renders, so the `open` attribute in the HTML render() hands to
// Idiomorph always matches reality -- kept explicit rather than trusting
// Idiomorph to leave a DOM-native `open` state alone on its own, since the
// exact attribute-preservation semantics for an attribute we simply never
// mention either way aren't something to depend on implicitly.
const adminOpenNetworks = new Set();
// Cached invite list per network (by name), populated by loadInvites()
// below. renderInviteList() reads from here instead of always starting at
// a "Loading invites…" placeholder -- see that function's own comment.
const inviteCache = new Map();
// Which .admin-details elements already have a "toggle" listener attached
// -- a WeakSet, not a DOM attribute, since Idiomorph strips any attribute
// on a kept node that isn't part of the freshly-rendered HTML (see
// render()'s own comment on this below).
const toggleBoundDetails = new WeakSet();
// Whether the create/join panel's <details> should default open: true
// only until the user has at least one network, and only re-applied on
// that specific reachable/empty -> non-empty transition (see render()) so
// polling doesn't fight a user who reopened it themselves to add another
// network.
let wasNetworksEmpty = true;
let lastStatus = null;

function setHeader(status) {
  const dot = document.getElementById("status-dot");
  const text = document.getElementById("status-text");
  const info = document.getElementById("endpoint-info");
  const traffic = document.getElementById("header-traffic");

  dot.className = "dot";
  if (!status.reachable) {
    dot.classList.add("dot-unknown");
    text.textContent = "daemon unreachable";
    info.textContent = status.message || "";
    traffic.textContent = "";
    return;
  }
  if (status.active) {
    dot.classList.add("dot-active");
    text.textContent = "tetron is active";
  } else {
    dot.classList.add("dot-standby");
    text.textContent = "tetron is on standby";
  }
  info.textContent = `${status.endpoint_short}  ·  v${status.daemon_version}`;
  // Right next to "is it active" -- a live indicator of *how* active,
  // not tucked at the bottom of the page under the network list.
  traffic.textContent = `↑${formatBytes(status.traffic.bytes_tx)} ↓${formatBytes(status.traffic.bytes_rx)}`;
}

function formatBytes(n) {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / 1024 / 1024).toFixed(1)}MB`;
}

// Small inline "copy to clipboard" button for a value shown in mono text
// (IPs, endpoint/network short ids). Complements the selection-preserving
// render() guard below rather than replacing it: this sidesteps the
// re-render problem entirely for the common case (no text selection
// involved, nothing to lose on the next poll tick), but people may still
// want to manually select longer runs of text (e.g. a whole nuke-proposal
// banner) where the selection guard is what actually matters.
function copyBtn(value) {
  return `<button class="copy-btn" data-action="copy" data-copy="${value}" title="Copy ${value}">⧉</button>`;
}

function renderPeerRow(net, peer) {
  const conn = peer.connection;
  const status = conn
    ? `${conn.conn_type.toLowerCase()} · ${conn.rtt_ms != null ? conn.rtt_ms.toFixed(0) + "ms" : "?"} · ↑${formatBytes(conn.bytes_tx)} ↓${formatBytes(conn.bytes_rx)}`
    : "offline";
  const kickBtn =
    net.role === "admin"
      ? `<button class="btn-small btn-danger" data-action="kick" data-net="${net.short_id}" data-peer="${peer.short_id}">kick</button>`
      : "";
  const ipv6 = peer.ipv6 ? `<span class="peer-ipv6">${peer.ipv6}${copyBtn(peer.ipv6)}</span>` : "";
  // id is required for reliable Idiomorph matching across polls (network
  // + short_id scoped, since the same peer can appear in more than one
  // network's table if we ever join several networks with a shared member).
  return `<tr id="peer-${net.network}-${peer.short_id}">
    <td>${peer.role}</td>
    <td><span class="clickable-name" data-action="peer-detail" data-network="${net.network}" data-peer="${peer.endpoint_id}" title="Show full details">${peer.hostname || peer.ip}</span></td>
    <td class="mono">${peer.ip}${copyBtn(peer.ip)}${ipv6}</td>
    <td class="mono">${status}</td>
    <td>${kickBtn}</td>
  </tr>`;
}

// A pending nuke is a status fact every member should see, not an admin-only
// concern -- separate from renderNukeActions below, which gates the actual
// propose/second/cancel buttons to admins only.
function renderNukeBanner(net) {
  const proposals = net.nuke_proposals || [];
  if (proposals.length === 0) return "";
  const who = proposals.map((p) => p.short_id).join(", ");
  return `<div class="nuke-proposal-banner">Nuke proposed by ${who} (${proposals.length}/2 coordinators needed).</div>`;
}

// Admin-only: the actual destroy/second/cancel buttons. Never called for a
// plain member's own row (see renderNetworkRow) -- a member sees the banner
// above but has no way to act on it, same as they have no invite/kick button.
function renderNukeActions(net, myShortId) {
  const proposals = net.nuke_proposals || [];
  const iAlreadyProposed = proposals.some((p) => p.short_id === myShortId);
  const otherProposal = proposals.find((p) => p.short_id !== myShortId);

  if (iAlreadyProposed) {
    return `<button class="btn-small btn-secondary" data-action="nuke-cancel" data-net="${net.short_id}">cancel my proposal</button>`;
  }
  if (otherProposal) {
    return `<button class="btn-small btn-danger" data-action="nuke-second" data-net="${net.short_id}" data-second="${otherProposal.short_id}">second ${otherProposal.short_id}'s proposal</button>`;
  }
  return `<button class="btn-small btn-danger" data-action="nuke-propose" data-net="${net.short_id}">destroy network</button>`;
}

// Admin-only actions (mint invite, destroy network) tucked into a collapsed
// <details> -- these are the least-used, highest-consequence actions, so
// they stay out of the way until deliberately opened. Never rendered at all
// for a plain member's row: a "member" role has no invite/nuke capability
// server-side either, so there is nothing to show, not just something to
// hide -- see the daemon's own coordinator_handle gate.
function renderAdminDetails(net, myShortId, isOpen) {
  // ids are required for Idiomorph to match these nodes reliably across
  // polls (matters most for #admin-<net>, whose open/closed state and
  // #invites-<net>, whose content must survive a morph untouched -- see
  // renderInviteList below) and for #invite-expires-<net> specifically,
  // since Idiomorph's focus-restore only re-finds the focused element by
  // `id`, not by data-* attributes.
  return `<details class="admin-details" id="admin-${net.network}" data-network="${net.network}" ${isOpen ? "open" : ""}>
    <summary>Admin</summary>
    <div class="action-row">
      <input type="text" class="invite-expires-input" id="invite-expires-${net.network}" data-network="${net.network}" placeholder="expires (e.g. 24h, 7d -- optional)">
      <button class="btn-small" data-action="invite-create" data-network="${net.network}">mint invite</button>
    </div>
    ${renderInviteList(net.network)}
    <div class="danger-zone">
      <div class="danger-zone-label">Danger zone</div>
      ${renderNukeActions(net, myShortId)}
    </div>
  </details>`;
}

// Renders from `inviteCache` when available instead of always starting at
// a "Loading invites…" placeholder -- rendering the placeholder every poll
// tick (even once the real list has already loaded) was what made an open
// admin panel visibly flash back to "Loading…" and forward again every
// ~2-10s, since render() regenerates this whole subtree's HTML on every
// tick regardless of Idiomorph being in the loop (Idiomorph only avoids
// touching the DOM when the *new* HTML we hand it already matches; it
// can't know a "Loading…" string was never meant to overwrite real data).
// loadInvites() below populates the cache; only a genuine first open (via
// the toggle listener in render()) or an explicit invite create/revoke
// action ever calls it -- not the poll cycle itself.
function renderInviteList(network) {
  const cached = inviteCache.get(network);
  let body;
  if (cached === undefined) {
    body = `<p class="muted">Loading invites…</p>`;
  } else if (cached.error) {
    body = `<p class="muted">Could not load invites: ${cached.error}</p>`;
  } else {
    body = cached.length
      ? cached.map((inv) => renderInviteRow({ network }, inv)).join("")
      : `<p class="muted">No invites minted yet.</p>`;
  }
  return `<div class="invite-list" id="invites-${network}" data-network="${network}">${body}</div>`;
}

function formatEpoch(seconds) {
  if (!seconds) return "never";
  return new Date(seconds * 1000).toLocaleString();
}

function renderInviteRow(net, invite) {
  const status = invite.revoked ? "revoked" : "active";
  return `<div class="invite-row">
    <span class="mono invite-id">${invite.id}</span>
    <span class="invite-meta">expires ${formatEpoch(invite.expires_at)} · ${status}</span>
    ${invite.revoked ? "" : `<button class="btn-small btn-secondary" data-action="invite-revoke" data-network="${net.network}" data-invite="${invite.id}">revoke</button>`}
  </div>`;
}

// Populates inviteCache and immediately re-renders just the one invite-list
// element from it (via a targeted Idiomorph morph, not a full #networks
// rebuild) -- called on a genuine first open of an admin panel (the toggle
// listener in render()) or right after an invite create/revoke action, never
// from the poll cycle itself. That's what lets renderInviteList() show real
// content on every subsequent poll instead of a repeating "Loading…" flash.
async function loadInvites(network) {
  const container = document.getElementById(`invites-${network}`);
  if (!container) return;
  try {
    const result = await getJson(`/api/networks/${encodeURIComponent(network)}/invites`);
    inviteCache.set(network, result.ok ? result.invites : { error: result.error });
  } catch (e) {
    inviteCache.set(network, { error: String(e) });
  }
  Idiomorph.morph(container, renderInviteList(network), { morphStyle: "outerHTML" });
}

function renderNetworkRow(net, myShortId) {
  const standbyBadge = !net.active ? '<span class="network-standby-badge">standby</span>' : "";
  const online = net.peers.filter((p) => p.connection).length;

  const peerRows = net.peers.map((p) => renderPeerRow(net, p)).join("");
  const peerTable = net.peers.length
    ? `<table class="peer-table"><thead><tr><th>role</th><th>host</th><th>ip</th><th>connection</th><th></th></tr></thead><tbody>${peerRows}</tbody></table>`
    : `<p class="muted">no other members</p>`;

  const isAdmin = net.role === "admin";
  const isAdminOpen = adminOpenNetworks.has(net.network);

  // Members list (the "tetron status" equivalent -- the most-used thing
  // here) is always visible, never behind a click. Only the admin-only
  // actions below it are a dropdown, and only for a role that can actually
  // use them.
  return `<div class="network-row" id="network-${net.network}" data-network="${net.network}">
    <div class="network-summary">
      <span class="network-name clickable-name" data-action="network-detail" data-network="${net.network}" title="Show full details">${net.network}</span>
      <span class="network-role">${net.role}</span>
      <span class="network-members">${online}/${net.peers.length + 1}</span>
      ${standbyBadge}
    </div>
    <div class="network-body">
      <div class="network-meta mono">id ${net.short_id || "?"}${net.short_id ? copyBtn(net.short_id) : ""} · host ${net.my_hostname || net.my_ip} · interface ${net.tun_name || "?"} · ${net.my_ip}${copyBtn(net.my_ip)}${net.my_ipv6 ? ` · ${net.my_ipv6}${copyBtn(net.my_ipv6)}` : ""}</div>
      ${peerTable}
      ${renderNukeBanner(net)}
      <div class="action-row">
        <button class="btn-small btn-secondary" data-action="${net.active ? "standby" : "resume"}" data-network="${net.network}">
          ${net.active ? "standby this network" : "activate this network"}
        </button>
        <button class="btn-small btn-secondary" data-action="sync" data-network="${net.network}" title="Wake the DHT/group poller now instead of waiting for its interval">sync</button>
        <button class="btn-small btn-secondary" data-action="leave" data-network="${net.network}">leave</button>
      </div>
      ${isAdmin ? renderAdminDetails(net, myShortId, isAdminOpen) : ""}
    </div>
  </div>`;
}

// True if the browser's current text selection is non-collapsed and
// anchored somewhere inside `el`. Used to freeze #networks' contents for
// one poll tick rather than tear the user's in-progress selection out from
// under them -- see render()'s guard below.
function selectionInside(el) {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return false;
  return el.contains(sel.getRangeAt(0).commonAncestorContainer);
}

function render(status) {
  lastStatus = status;
  setHeader(status);

  const container = document.getElementById("networks");

  if (!status.reachable) {
    if (!selectionInside(container)) container.innerHTML = "";
    return;
  }

  // Networks-first once at least one exists (see style.css's
  // body.has-networks rule) -- the create/join forms are the primary
  // thing to see on a fresh install, but a secondary thing once there's
  // an actual mesh to check the health of.
  const isEmpty = status.networks.length === 0;
  document.body.classList.toggle("has-networks", !isEmpty);

  // Only force the create/join <details> open/closed on the actual
  // empty <-> non-empty transition, not on every poll tick -- otherwise a
  // user who reopened it themselves (e.g. to add a second network) would
  // get fought every 2s.
  if (isEmpty !== wasNetworksEmpty) {
    document.getElementById("create-join-details").open = isEmpty;
    wasNetworksEmpty = isEmpty;
  }

  // Skip rebuilding #networks entirely while the user has an active text
  // selection inside it (e.g. mid-copy of an IP or endpoint id). Idiomorph
  // (below) already avoids touching most unchanged nodes, but a plain text
  // selection isn't tied to any one element the way input focus is, so it
  // has no equivalent "restore" step -- morphing a node a selection spans
  // could still disrupt it. Every other status-driven update above this
  // guard still runs; only this one subtree is frozen for this tick, and
  // the very next poll retries once the selection clears. The copy
  // buttons (copyBtn(), above) are the complementary fix for the common
  // case where nothing needs manual selection at all.
  if (selectionInside(container)) return;

  if (isEmpty) {
    container.innerHTML = '<p class="muted">No networks yet -- create or join one above.</p>';
  } else {
    // Idiomorph.morph diffs the container's *children* against this new
    // HTML and only touches DOM nodes that actually changed (matched by
    // `id` where present) -- unlike the old `innerHTML =` this replaced,
    // an unchanged network/peer/admin-panel subtree is left completely
    // alone on a poll tick, which is what actually fixes the admin panel's
    // invite-list flash and the invite-expires input losing focus while
    // typing (Idiomorph's default `restoreFocus: true` re-finds and
    // refocuses the previously-active input by `id` after morphing) --
    // `ignoreActiveValue: true` is required alongside it: our templates
    // never set a `value` attribute on the invite-expires input, so
    // without this Idiomorph would still reset the *focused* input's
    // live value to empty on every poll even with focus itself restored
    // correctly (found live-testing this fix, not just theoretical).
    Idiomorph.morph(
      container,
      status.networks.map((n) => renderNetworkRow(n, status.endpoint_short)).join(""),
      { morphStyle: "innerHTML", ignoreActiveValue: true },
    );
    // <details>' native "toggle" event does not bubble, so it cannot be
    // handled by the single delegated click listener below (the delegated
    // listener catches clicks on the <summary>'s underlying activation,
    // but that fires before the browser's own open/close default action
    // has actually run) -- attach directly per row instead. With Idiomorph
    // in the loop an already-bound <details> node persists across polls
    // (it's the same DOM node, not a fresh one every tick), so guard
    // against attaching a second listener to it on the next poll -- via a
    // JS-side WeakSet, not a `dataset`/DOM attribute marker: Idiomorph
    // strips any attribute present on the old node but absent from the
    // freshly-rendered HTML string on every morph, so a `data-*` marker
    // set only via JS (never part of the template) gets silently wiped on
    // the very next poll, defeating the guard and re-accumulating a fresh
    // duplicate listener every tick (found while testing this fix).
    container.querySelectorAll(".admin-details").forEach((details) => {
      if (toggleBoundDetails.has(details)) return;
      toggleBoundDetails.add(details);
      details.addEventListener("toggle", () => {
        if (details.open) {
          adminOpenNetworks.add(details.dataset.network);
          loadInvites(details.dataset.network);
        } else {
          adminOpenNetworks.delete(details.dataset.network);
        }
      });
    });
  }
}

async function poll() {
  try {
    const status = await getJson("/api/status");
    render(status);
  } catch (e) {
    render({ reachable: false, message: String(e) });
  }
}

// -----------------------------------------------------------------------
// Click delegation for everything rendered inside #networks -- the network
// list is fully re-rendered on every poll, so listeners are attached once
// on the stable container and dispatched by the clicked element's
// data-action, rather than re-attached per row on every re-render.
// -----------------------------------------------------------------------

document.getElementById("networks").addEventListener("click", async (e) => {
  const el = e.target.closest("[data-action]");
  if (!el) return;
  const action = el.dataset.action;
  const network = el.dataset.network;

  if (action === "copy") {
    const value = el.dataset.copy;
    navigator.clipboard.writeText(value).then(() => {
      const original = el.textContent;
      el.textContent = "✓";
      setTimeout(() => { el.textContent = original; }, 900);
    });
    return;
  }

  if (action === "network-detail") {
    const net = lastStatus?.networks.find((n) => n.network === network);
    if (!net) return;
    const rows = [
      { label: "role", value: net.role },
      { label: "hostname", value: net.my_hostname || net.my_ip },
      { label: "interface", value: net.tun_name },
      { label: "members", value: String(net.member_count) },
      { label: "my ip", value: net.my_ip, copyable: true },
      { label: "my ipv6", value: net.my_ipv6, copyable: true },
    ];
    // network_key is admin-gated, matching `tetron status`'s own text-view
    // convention (AGENTS.md's "Network/peer identifier resolution" section)
    // -- a plain member can't act on it anyway (nuke/kick both need it, and
    // both are already admin-only actions).
    if (net.role === "admin") {
      rows.push({ label: "network key", value: net.network_key, copyable: true });
    }
    showDetail(net.network, rows);
    return;
  }

  if (action === "peer-detail") {
    const net = lastStatus?.networks.find((n) => n.network === network);
    const peer = net?.peers.find((p) => p.endpoint_id === el.dataset.peer);
    if (!peer) return;
    const conn = peer.connection;
    const connSummary = conn
      ? `${conn.conn_type.toLowerCase()} · ${conn.rtt_ms != null ? conn.rtt_ms.toFixed(0) + "ms" : "?"} · ↑${formatBytes(conn.bytes_tx)} ↓${formatBytes(conn.bytes_rx)}`
      : "offline";
    showDetail(peer.hostname || peer.ip, [
      { label: "role", value: peer.role },
      { label: "endpoint id", value: peer.endpoint_id, copyable: true },
      { label: "ip", value: peer.ip, copyable: true },
      { label: "ipv6", value: peer.ipv6, copyable: true },
      { label: "connection", value: connSummary },
    ]);
    return;
  }

  if (action === "resume") {
    await postJson("/api/resume", { network });
    poll();
    return;
  }
  if (action === "standby") {
    await postJson("/api/standby", { network });
    poll();
    return;
  }
  if (action === "sync") {
    await postJson("/api/sync", { network });
    poll();
    return;
  }

  if (action === "invite-create") {
    const expiresInput = document.querySelector(`.invite-expires-input[data-network="${CSS.escape(network)}"]`);
    const expires = expiresInput && expiresInput.value ? expiresInput.value : undefined;
    const result = await postJson(`/api/networks/${encodeURIComponent(network)}/invites`, { expires });
    if (result.ok) {
      showInviteQr(result.invite_key, `Invite key for "${network}" (single use)`);
      if (expiresInput) expiresInput.value = "";
      loadInvites(network);
    } else {
      alert(`Failed to mint invite: ${result.error}`);
    }
    return;
  }

  if (action === "invite-revoke") {
    const inviteId = el.dataset.invite;
    const result = await deleteJson(`/api/networks/${encodeURIComponent(network)}/invites/${encodeURIComponent(inviteId)}`);
    if (!result.ok) alert(`Failed to revoke invite: ${result.error}`);
    loadInvites(network);
    return;
  }

  if (action === "leave") {
    confirmAction(
      `Leave "${network}"?`,
      "If you are the only coordinator, tetron will try to promote every reachable member first. It only refuses if someone is offline right now and can't be reached.",
      async () => {
        const result = await postJson(`/api/networks/${encodeURIComponent(network)}/leave`, { force: false });
        if (!result.ok) {
          confirmAction(
            "Leave failed",
            `${result.error}\n\nForce leave anyway? This is NOT reversible for whoever gets stranded.`,
            async () => {
              await postJson(`/api/networks/${encodeURIComponent(network)}/leave`, { force: true });
              poll();
            }
          );
        } else {
          poll();
        }
      }
    );
    return;
  }

  if (action === "kick") {
    const netId = el.dataset.net;
    const peer = el.dataset.peer;
    confirmAction(
      `Kick ${peer}?`,
      "They will be removed from the roster and disconnected. They cannot re-join without a new invite key.",
      async () => {
        await postJson(`/api/networks/${encodeURIComponent(netId)}/kick`, { peer });
        poll();
      }
    );
    return;
  }

  if (action === "nuke-propose" || action === "nuke-second") {
    const netId = el.dataset.net;
    const second = el.dataset.second;
    confirmAction(
      "Destroy this network?",
      "This is NOT reversible. Every member will be disconnected and the network can never be used again. With a single coordinator this happens immediately; with multiple coordinators this proposes/seconds, and only executes once two distinct coordinators agree within 24h.",
      async () => {
        await postJson(`/api/networks/${encodeURIComponent(netId)}/nuke`, {
          force: true, // the modal above IS the confirmation --force exists to skip
          second: second || null,
        });
        poll();
      }
    );
    return;
  }

  if (action === "nuke-cancel") {
    const netId = el.dataset.net;
    await postJson(`/api/networks/${encodeURIComponent(netId)}/nuke`, { cancel: true });
    poll();
    return;
  }
});

// -----------------------------------------------------------------------
// Create / join forms
// -----------------------------------------------------------------------

document.querySelectorAll(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".tab-panel").forEach((p) => p.classList.add("hidden"));
    btn.classList.add("active");
    document.querySelector(`.tab-panel[data-tab="${btn.dataset.tab}"]`).classList.remove("hidden");
  });
});

function formToObject(form) {
  const data = {};
  new FormData(form).forEach((value, key) => {
    if (value === "") return;
    const el = form.elements[key];
    // Send actual JSON numbers for <input type="number">, not the string
    // FormData always yields -- the backend's Option<u32>-typed fields
    // (e.g. nuke_consensus) fail to deserialize from a JSON string.
    // Checkboxes only appear in FormData at all when checked, with a
    // string value ("on") that a bool field can't deserialize -- convert
    // to a real boolean. Unchecked boxes never reach this callback, which
    // is fine: the backend's #[serde(default)] fields treat "key absent"
    // as false.
    if (el && el.type === "number") data[key] = Number(value);
    else if (el && el.type === "checkbox") data[key] = el.checked;
    else data[key] = value;
  });
  return data;
}

document.getElementById("create-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = e.target;
  const result = await postJson("/api/networks", formToObject(form));
  const out = form.querySelector(".form-result");
  if (result.ok) {
    out.textContent = `Created "${result.network}" — ${result.my_ip}`;
    out.className = "form-result success";
    form.reset();
    poll();
    if (result.initial_invite_key) {
      showInviteQr(result.initial_invite_key, `Invite key for "${result.network}" (single use)`);
    }
  } else {
    out.textContent = result.error;
    out.className = "form-result error";
  }
});

document.getElementById("join-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = e.target;
  const result = await postJson("/api/networks/join", formToObject(form));
  const out = form.querySelector(".form-result");
  if (result.ok) {
    out.textContent = `Joined "${result.network}" — ${result.my_ip}`;
    out.className = "form-result success";
    form.reset();
    poll();
  } else {
    out.textContent = result.error;
    out.className = "form-result error";
  }
});

// -----------------------------------------------------------------------
// Add-ons (host-level install framework, not part of the network status
// poll -- addon state only ever changes in response to an explicit install/
// uninstall click here, so it's fetched once at boot and again after any
// such action, not on the same 2s interval as network status.
// -----------------------------------------------------------------------

function renderAddonRow(addon) {
  const hasPopup = !!ADDON_DETAILS[addon.id];
  // A *script* popup addon (Config Backup: a bare script fetched from
  // contrib/ on main, no release tag) links straight to that folder
  // instead of the repo root -- `script_url` is only ever set for that
  // addon (see addons.rs::AddonStatus), so it is the reliable signal, not
  // `hasPopup` alone (Sync Receiver also has a popup, but is a normal
  // versioned release binary, not a script in contrib/).
  const isScriptPopup = !!addon.script_url;
  const repoLine = isScriptPopup
    ? `<p class="addon-repo"><a class="addon-link" href="https://github.com/${addon.github_repo}/tree/main/contrib" target="_blank" rel="noopener">${addon.github_repo}/tree/main/contrib</a></p>`
    : `<p class="addon-repo"><a class="addon-link" href="https://github.com/${addon.github_repo}" target="_blank" rel="noopener">${addon.github_repo}</a></p>`;

  // Link-only addons (a script run against a remote host, or a whole VM
  // environment) have no local install/uninstall action -- just the name,
  // description, and a link out to the repo for their own setup steps.
  // Addons with `details: true` additionally get a button that opens the
  // in-app instructions popup (ADDON_DETAILS below).
  if (!addon.installable) {
    const actions = addon.details
      ? `<div class="addon-actions"><button class="btn-small" data-action="details" data-addon="${addon.id}">Details</button></div>`
      : "";
    return `<div class="addon-row" data-addon="${addon.id}">
      <div class="addon-info">
        <span class="addon-name">${addon.display_name}</span>
        <p class="muted addon-description">${addon.description}</p>
        ${repoLine}
      </div>
      ${actions}
    </div>`;
  }

  // Config Backup: a single persistent "Backup" button in both install
  // states (outline throughout) -- one click installs the script if
  // needed and opens the popup (see the "backup" branch in the click
  // handler below). Unlike every other installable addon, it never shows
  // a plain Install/Uninstall toggle on the row.
  if (isScriptPopup) {
    return `<div class="addon-row" data-addon="${addon.id}">
      <div class="addon-info">
        <span class="addon-name">${addon.display_name}</span>
        <span class="addon-status ${addon.installed ? "installed" : "not-installed"}">${addon.installed ? "installed" : "not installed"}</span>
        <p class="muted addon-description">${addon.description}</p>
        ${repoLine}
      </div>
      <div class="addon-actions">
        <button class="btn-small btn-secondary" data-action="backup" data-addon="${addon.id}">Backup</button>
      </div>
      <p class="form-result"></p>
    </div>`;
  }

  // Every other installable addon: the normal Install/Uninstall toggle
  // (reference look: Systray's outline "Uninstall" button). An addon that
  // also has a live config popup (`details: true` but not a script, e.g.
  // Sync Receiver) gets a second "Configure" button once installed --
  // there is nothing to configure before the binary exists on this host.
  const action = addon.installed ? "uninstall" : "install";
  const label = addon.installed ? "Uninstall" : "Install";
  const btnClass = addon.installed ? "btn-secondary" : "";
  const configureBtn = hasPopup && addon.installed
    ? `<button class="btn-small btn-secondary" data-action="configure" data-addon="${addon.id}">Configure</button>`
    : "";
  return `<div class="addon-row" data-addon="${addon.id}">
    <div class="addon-info">
      <span class="addon-name">${addon.display_name}</span>
      <span class="addon-status ${addon.installed ? "installed" : "not-installed"}">${addon.installed ? "installed" : "not installed"}</span>
      <p class="muted addon-description">${addon.description}</p>
      ${repoLine}
    </div>
    <div class="addon-actions">
      ${configureBtn}
      <button class="btn-small ${btnClass}" data-action="${action}" data-addon="${addon.id}">${label}</button>
    </div>
    <p class="form-result"></p>
  </div>`;
}

// In-app instructions popups for addons with `details: true` (see
// src/addons.rs). Body is a function of the addon status so the
// script-fetch commands can be built from the browser's current origin
// (whoever is viewing the popup -- directly on 127.0.0.1:7870 or through
// a reverse proxy -- gets a curl command that reaches this webui from
// where they are) and from the effective upstream URL the server reports
// (script_url: TETRON_BACKUP_RAW_URL override or the tetron-repo default).
const ADDON_DETAILS = {
  backup: {
    title: "Config Backup",
    body: (addon) => {
      const origin = window.location.origin;
      const scriptPath = "~/.local/bin/tetron-backup.sh";
      const backupCmd = `sudo ${scriptPath}`;
      const restoreCmd = `sudo ${scriptPath} --restore /path/to/backup.tar.age`;
      // The script is proxied by this webui from the tetron repo
      // (contrib/tetron-backup.sh), so the manual fetch needs no repo
      // clone either. Manual fetch only appears as a fallback -- the
      // normal path is the Install button on the row above.
      const installBlock = addon.installed
        ? `<p class="muted">Installed at <code>~/.local/bin/tetron-backup.sh</code> (no root needed). <button class="btn-small btn-secondary" data-action="backup-uninstall">Uninstall script</button></p>`
        : `<p class="muted">Not installed yet — click <strong>Install</strong> on the row above: one click, no root needed, fetches the script into <code>~/.local/bin/tetron-backup.sh</code>.</p>
        <h4>Fallback (no webui install button?)</h4>
        ${copyBlock(`curl -fsSL ${origin}/addons/tetron-backup.sh -o tetron-backup.sh && chmod +x tetron-backup.sh`)}
        <p class="muted">If this webui can not reach the tetron repo (e.g. no internet on this host), fetch the script directly instead:</p>
        ${copyBlock(`curl -fsSL ${addon.script_url} -o tetron-backup.sh && chmod +x tetron-backup.sh`)}`;
      return `
        <p class="muted">Passphrase-encrypted tar+age backup of this host's tetron config tree. Requires <code>age</code> (age-encryption.org) — the script prints install hints if it is missing.</p>
        ${installBlock}
        <h4>1. Back up</h4>
        ${copyBlock(backupCmd)}
        <p class="muted">Prompts for a passphrase twice; writes <code>tetron-backup-&lt;host&gt;-&lt;date&gt;.tar.age</code> in the current directory. A custom path works too: <code>sudo ${scriptPath} /path/to/backup.tar.age</code>. Verify a backup with <code>age -d backup.tar.age | tar -tzf -</code>.</p>
        <h4>2. Restore</h4>
        ${copyBlock(restoreCmd)}
        <p class="muted">Stops the tetron daemon, restores the config tree (Linux <code>/etc/tetron</code>, macOS <code>/var/root/Library/Application Support/tetron</code>), and starts the daemon again.</p>
        <p class="muted"><strong>Passphrase is the only key:</strong> lose it, lose the backup. Covers <code>secret_key</code>, <code>settings.toml</code>, and every <code>networks/*.toml</code>.</p>`;
    },
  },
  "sync-receiver": {
    title: "Sync Receiver",
    // Real content is filled in by renderSyncReceiverPanel() right after
    // the modal opens (see the "configure" click branch below) -- unlike
    // Config Backup's body(), this one needs live data (status/modules/
    // allow-list) fetched from the addon's own binary, not just static
    // instructions, so body() only ever renders a loading placeholder.
    body: () => `<p class="muted">Loading configuration…</p>`,
  },
};

// Fetches status/modules/allow-list from the Sync Receiver addon's own
// CLI (via tetron-webui's /api/sync-receiver/* proxy routes,
// src/sync_receiver.rs) and renders the live config panel. Every mutating
// control inside it (see the detailBody submit/click listeners further
// down) re-calls this after a successful action instead of patching the
// DOM in place -- simpler, and cheap enough given how rarely this popup
// is open at all.
async function renderSyncReceiverPanel() {
  detailBody.innerHTML = `<p class="muted">Loading configuration…</p>`;
  try {
    const [status, modules, allow] = await Promise.all([
      getJson("/api/sync-receiver/status"),
      getJson("/api/sync-receiver/modules"),
      getJson("/api/sync-receiver/allow"),
    ]);
    if (status.ok === false) throw new Error(status.error);
    if (modules.ok === false) throw new Error(modules.error);
    if (allow.ok === false) throw new Error(allow.error);
    detailBody.innerHTML = syncReceiverPanelHtml(status, modules, allow);
  } catch (e) {
    detailBody.innerHTML = `<p class="muted">Could not load configuration: ${String(e.message || e)}</p>`;
  }
}

function syncReceiverPanelHtml(status, modules, allow) {
  const moduleRows = modules.length
    ? modules.map((m) => `<tr><td>${m.name}</td><td><code>${m.path}</code></td><td><button class="btn-small btn-secondary" data-action="sr-module-remove" data-name="${m.name}">Remove</button></td></tr>`).join("")
    : `<tr><td colspan="3" class="muted">No modules configured yet.</td></tr>`;
  const allowRows = allow.length
    ? allow.map((ip) => `<tr><td><code>${ip}</code></td><td><button class="btn-small btn-secondary" data-action="sr-allow-remove" data-ip="${ip}">Remove</button></td></tr>`).join("")
    : `<tr><td colspan="2" class="muted">No IPs allowed yet -- every connection is denied.</td></tr>`;

  return `
    <p class="muted">Service: <strong>${status.active ? "running" : "stopped"}</strong> on port <code>${status.port}</code>.
      <button class="btn-small ${status.active ? "btn-secondary" : ""}" data-action="sr-toggle" data-active="${status.active}">${status.active ? "Stop" : "Start"}</button>
    </p>

    <h4>Port</h4>
    <form class="tab-panel" data-action="sr-set-port">
      <input type="number" name="port" min="1025" max="65535" value="${status.port}" required>
      <button type="submit" class="btn-small">Change port</button>
    </form>
    <p class="muted" style="color: var(--status-down);">Must match the port configured on the phone (tetron-mobile-sync's Settings screen) -- if they don't match, backups fail with a socket error. Changing this restarts the service if it's running.</p>

    <h4>Modules</h4>
    <table class="peer-table"><tbody>${moduleRows}</tbody></table>
    <p class="muted">A module is one folder that can serve several phones: each phone writes into its own <code>&lt;module&gt;/&lt;device-label&gt;/</code> subfolder, created by the phone. You do not need a module per device. Modules are upload-only and accept one transfer at a time.</p>
    <form class="tab-panel" data-action="sr-module-add">
      <input type="text" name="name" placeholder="name (e.g. photos)" required>
      <input type="text" name="path" placeholder="/home/user/Pictures/phone-backup" required>
      <button type="submit" class="btn-small">Add module</button>
    </form>

    <h4>Allowed mesh IPs</h4>
    <table class="peer-table"><tbody>${allowRows}</tbody></table>
    <form class="tab-panel" data-action="sr-allow-add-peer">
      <input type="text" name="hostname" placeholder="peer hostname (from the mesh roster)" required>
      <button type="submit" class="btn-small">Allow peer</button>
    </form>
    <form class="tab-panel" data-action="sr-allow-add-ip">
      <input type="text" name="ip" placeholder="or a raw mesh IP" required>
      <button type="submit" class="btn-small">Allow IP</button>
    </form>
    <p class="form-result"></p>
  `;
}

function showSyncReceiverError(message) {
  const out = detailBody.querySelector(".form-result");
  if (out) {
    out.textContent = message;
    out.className = "form-result error";
  }
}

// A copyable command block for the instructions popup. The copy handler is
// the delegated listener on detailBody below (the #networks delegate can't
// see inside the modal).
function copyBlock(cmd) {
  return `<div class="copy-block"><pre class="copy-pre">${cmd}</pre><button class="copy-btn" data-action="copy" data-copy="${cmd}" title="Copy command">⧉</button></div>`;
}

// Same copy behavior as the #networks delegate, scoped to the detail modal
// so copy buttons inside instructions popups work. Also handles the
// popup's own Uninstall script button (Config Backup): uninstalls, then
// re-renders the popup in the not-installed state and refreshes the rows.
detailBody.addEventListener("click", async (e) => {
  const un = e.target.closest("[data-action='backup-uninstall']");
  if (un) {
    un.disabled = true;
    const result = await postJson("/api/addons/backup/uninstall", {});
    const current = lastAddons.find((a) => a.id === "backup") || {};
    detailBody.innerHTML = ADDON_DETAILS.backup.body({ ...current, installed: false });
    if (!result.ok) {
      const note = document.createElement("p");
      note.className = "form-result error";
      note.textContent = result.error;
      detailBody.appendChild(note);
    }
    setTimeout(pollAddons, 1500);
    return;
  }
  const modRemove = e.target.closest("[data-action='sr-module-remove']");
  if (modRemove) {
    modRemove.disabled = true;
    const result = await deleteJson(`/api/sync-receiver/modules/${encodeURIComponent(modRemove.dataset.name)}`);
    if (result.ok) renderSyncReceiverPanel();
    else { modRemove.disabled = false; showSyncReceiverError(result.error); }
    return;
  }

  const allowRemove = e.target.closest("[data-action='sr-allow-remove']");
  if (allowRemove) {
    allowRemove.disabled = true;
    const result = await deleteJson(`/api/sync-receiver/allow/${encodeURIComponent(allowRemove.dataset.ip)}`);
    if (result.ok) renderSyncReceiverPanel();
    else { allowRemove.disabled = false; showSyncReceiverError(result.error); }
    return;
  }

  const toggle = e.target.closest("[data-action='sr-toggle']");
  if (toggle) {
    toggle.disabled = true;
    const active = toggle.dataset.active === "true";
    const result = await postJson(`/api/sync-receiver/${active ? "disable" : "enable"}`, {});
    if (result.ok) renderSyncReceiverPanel();
    else { toggle.disabled = false; showSyncReceiverError(result.error); }
    return;
  }

  const el = e.target.closest("[data-action='copy']");
  if (!el) return;
  navigator.clipboard.writeText(el.dataset.copy).then(() => {
    const original = el.textContent;
    el.textContent = "✓";
    setTimeout(() => { el.textContent = original; }, 900);
  });
});

// Sync Receiver's three config forms (add module, allow peer, allow IP) --
// delegated the same way as detailBody's click listener above, since the
// forms are re-created from scratch every renderSyncReceiverPanel() call.
detailBody.addEventListener("submit", async (e) => {
  const form = e.target.closest("form[data-action]");
  if (!form) return;
  e.preventDefault();
  const { action } = form.dataset;
  const data = Object.fromEntries(new FormData(form).entries());
  const submitBtn = form.querySelector("button[type='submit']");
  submitBtn.disabled = true;

  let result;
  if (action === "sr-module-add") {
    result = await postJson("/api/sync-receiver/modules", { name: data.name, path: data.path });
  } else if (action === "sr-allow-add-peer") {
    result = await postJson("/api/sync-receiver/allow/peer", { hostname: data.hostname });
  } else if (action === "sr-allow-add-ip") {
    result = await postJson("/api/sync-receiver/allow", { ip: data.ip });
  } else if (action === "sr-set-port") {
    result = await postJson("/api/sync-receiver/port", { port: Number(data.port) });
  } else {
    submitBtn.disabled = false;
    return;
  }

  if (result.ok) renderSyncReceiverPanel();
  else {
    submitBtn.disabled = false;
    showSyncReceiverError(result.error);
  }
});

// Last addon statuses from /api/addons, kept so the Details handler can
// pass the full addon object (script_url etc.) to the popup builder.
let lastAddons = [];

async function pollAddons() {
  const list = document.getElementById("addons-list");
  try {
    const addons = await getJson("/api/addons");
    lastAddons = addons;
    list.innerHTML = addons.length ? addons.map(renderAddonRow).join("") : `<p class="muted">No add-ons known.</p>`;
  } catch (e) {
    list.innerHTML = `<p class="muted">Could not load add-ons: ${String(e)}</p>`;
  }
}

document.getElementById("addons-list").addEventListener("click", async (e) => {
  const btn = e.target.closest("button[data-action]");
  if (!btn) return;
  const { action, addon } = btn.dataset;

  // Instructions popup (addons with `details: true`, e.g. Config Backup) --
  // no backend call, purely a frontend reveal of static content.
  if (action === "details") {
    const detail = ADDON_DETAILS[addon];
    if (!detail) return;
    const status = lastAddons.find((a) => a.id === addon) || {};
    detailTitle.textContent = detail.title;
    detailBody.innerHTML = detail.body(status);
    detailModal.classList.remove("hidden");
    return;
  }

  const row = btn.closest(".addon-row");
  const out = row.querySelector(".form-result");

  // "Backup" button (installable addons with an instructions popup, e.g.
  // Config Backup): install the script first if it is missing, then open
  // the popup either way -- one button, both things. Uninstall is offered
  // inside the popup instead of on the row, so the row stays the single
  // right-aligned Systray-style button.
  if (action === "backup") {
    const detail = ADDON_DETAILS[addon];
    if (!detail) return;
    const current = lastAddons.find((a) => a.id === addon) || {};
    let installed = !!current.installed;
    if (!installed) {
      btn.disabled = true;
      const originalLabel = btn.textContent;
      btn.textContent = "Installing…";
      out.textContent = "";
      out.className = "form-result";
      const result = await postJson(`/api/addons/${encodeURIComponent(addon)}/install`, {});
      if (!result.ok) {
        out.textContent = result.error;
        out.className = "form-result error";
        btn.disabled = false;
        btn.textContent = originalLabel;
        return;
      }
      out.textContent = result.message;
      out.className = "form-result success";
      installed = true;
      setTimeout(pollAddons, 1500);
    }
    detailTitle.textContent = detail.title;
    detailBody.innerHTML = detail.body({ ...current, installed });
    detailModal.classList.remove("hidden");
    return;
  }

  // "Configure" button (an already-installed addon with a live config
  // popup, e.g. Sync Receiver): open the popup and immediately kick off
  // the real fetch -- unlike "backup"/"details" above, body() alone is
  // just a loading placeholder here.
  if (action === "configure") {
    const detail = ADDON_DETAILS[addon];
    if (!detail) return;
    detailTitle.textContent = detail.title;
    detailBody.innerHTML = detail.body();
    detailModal.classList.remove("hidden");
    if (addon === "sync-receiver") renderSyncReceiverPanel();
    return;
  }

  btn.disabled = true;
  const originalLabel = btn.textContent;
  btn.textContent = action === "install" ? "Installing…" : "Uninstalling…";
  out.textContent = "";
  out.className = "form-result";

  const result = await postJson(`/api/addons/${encodeURIComponent(addon)}/${action}`, {});
  if (result.ok) {
    out.textContent = result.message;
    out.className = "form-result success";
    // Leave the success message on screen briefly before refreshing --
    // an immediate pollAddons() would wipe this element instantly, since
    // it lives inside the very row pollAddons() rebuilds from scratch.
    setTimeout(pollAddons, 1500);
  } else {
    out.textContent = result.error;
    out.className = "form-result error";
    btn.disabled = false;
    btn.textContent = originalLabel;
  }
});

// -----------------------------------------------------------------------
// Boot
// -----------------------------------------------------------------------

poll();
pollAddons();
setInterval(poll, POLL_INTERVAL_MS);
