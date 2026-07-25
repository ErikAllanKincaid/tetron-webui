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
// Status polling (Phase 1). This is also the entire "reconnect after a
// daemon restart" story: every poll independently reconnects via
// tetron-webui's own backend, so there is no persistent connection on this
// side to go stale -- a poll just fails once, then succeeds again once the
// daemon's socket reappears.
// -----------------------------------------------------------------------

const POLL_INTERVAL_MS = 2000;

// Tracks which networks' admin <details> are currently open (by name)
// across re-renders -- #networks' innerHTML is fully replaced every poll,
// which would otherwise snap a manually-opened <details> shut every 2s.
const adminOpenNetworks = new Set();
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

  dot.className = "dot";
  if (!status.reachable) {
    dot.classList.add("dot-unknown");
    text.textContent = "daemon unreachable";
    info.textContent = status.message || "";
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
}

function formatBytes(n) {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / 1024 / 1024).toFixed(1)}MB`;
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
  const ipv6 = peer.ipv6 ? `<span class="peer-ipv6">${peer.ipv6}</span>` : "";
  return `<tr>
    <td>${peer.role}</td>
    <td>${peer.hostname || peer.ip}</td>
    <td class="mono">${peer.ip}${ipv6}</td>
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
  return `<details class="admin-details" data-network="${net.network}" ${isOpen ? "open" : ""}>
    <summary>Admin</summary>
    <div class="action-row">
      <input type="text" class="invite-expires-input" data-network="${net.network}" placeholder="expires (e.g. 24h, 7d -- optional)">
      <button class="btn-small" data-action="invite-create" data-network="${net.network}">mint invite</button>
    </div>
    <div class="invite-list" data-network="${net.network}"><p class="muted">Loading invites…</p></div>
    <div class="danger-zone">
      <div class="danger-zone-label">Danger zone</div>
      ${renderNukeActions(net, myShortId)}
    </div>
  </details>`;
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

async function loadInvites(network) {
  const container = document.querySelector(`.invite-list[data-network="${CSS.escape(network)}"]`);
  if (!container) return;
  try {
    const result = await getJson(`/api/networks/${encodeURIComponent(network)}/invites`);
    if (!result.ok) {
      container.innerHTML = `<p class="muted">Could not load invites: ${result.error}</p>`;
      return;
    }
    container.innerHTML = result.invites.length
      ? result.invites.map((inv) => renderInviteRow({ network }, inv)).join("")
      : `<p class="muted">No invites minted yet.</p>`;
  } catch (e) {
    container.innerHTML = `<p class="muted">Could not load invites: ${String(e)}</p>`;
  }
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
  return `<div class="network-row" data-network="${net.network}">
    <div class="network-summary">
      <span class="network-name">${net.network}</span>
      <span class="network-role">${net.role}</span>
      <span class="network-members">${online}/${net.peers.length + 1}</span>
      ${standbyBadge}
    </div>
    <div class="network-body">
      <div class="network-meta mono">id ${net.short_id || "?"} · interface ${net.tun_name || "?"} · ${net.my_ip}${net.my_ipv6 ? ` · ${net.my_ipv6}` : ""}</div>
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

function render(status) {
  lastStatus = status;
  setHeader(status);

  const container = document.getElementById("networks");

  if (!status.reachable) {
    container.innerHTML = "";
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

  if (isEmpty) {
    container.innerHTML = '<p class="muted">No networks yet -- create or join one above.</p>';
  } else {
    container.innerHTML = status.networks.map((n) => renderNetworkRow(n, status.endpoint_short)).join("");
    // <details>' native "toggle" event does not bubble, so it cannot be
    // handled by the single delegated click listener below (the delegated
    // listener catches clicks on the <summary>'s underlying activation,
    // but that fires before the browser's own open/close default action
    // has actually run) -- attach directly per row instead. Cheap: only
    // ever as many admin rows as networks, and #networks' entire subtree
    // is already being freshly rebuilt this same tick anyway.
    container.querySelectorAll(".admin-details").forEach((details) => {
      // The whole #networks subtree is rebuilt every poll tick, so an
      // already-open admin panel is a brand-new <details open> element
      // each time -- its invite-list placeholder needs reloading here,
      // not just on a user-driven toggle (a freshly created already-open
      // element never fires "toggle").
      if (details.open) loadInvites(details.dataset.network);
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

  const footer = document.getElementById("footer");
  footer.textContent = `↑${formatBytes(status.traffic.bytes_tx)} ↓${formatBytes(status.traffic.bytes_rx)}`;
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
      alert(`Invite key (single use):\n\n${result.invite_key}`);
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
    out.textContent = `Created "${result.network}" — ${result.my_ip}${result.initial_invite_key ? ` — invite: ${result.initial_invite_key}` : ""}`;
    out.className = "form-result success";
    form.reset();
    poll();
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
  const action = addon.installed ? "uninstall" : "install";
  const label = addon.installed ? "Uninstall" : "Install";
  const btnClass = addon.installed ? "btn-secondary" : "";
  return `<div class="addon-row" data-addon="${addon.id}">
    <div class="addon-info">
      <span class="addon-name">${addon.display_name}</span>
      <span class="addon-status ${addon.installed ? "installed" : "not-installed"}">${addon.installed ? "installed" : "not installed"}</span>
      <p class="muted addon-description">${addon.description}</p>
    </div>
    <div class="addon-actions">
      <button class="btn-small ${btnClass}" data-action="${action}" data-addon="${addon.id}">${label}</button>
    </div>
    <p class="form-result"></p>
  </div>`;
}

async function pollAddons() {
  const list = document.getElementById("addons-list");
  try {
    const addons = await getJson("/api/addons");
    list.innerHTML = addons.length ? addons.map(renderAddonRow).join("") : `<p class="muted">No add-ons known.</p>`;
  } catch (e) {
    list.innerHTML = `<p class="muted">Could not load add-ons: ${String(e)}</p>`;
  }
}

document.getElementById("addons-list").addEventListener("click", async (e) => {
  const btn = e.target.closest("button[data-action]");
  if (!btn) return;
  const { action, addon } = btn.dataset;
  const row = btn.closest(".addon-row");
  const out = row.querySelector(".form-result");

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
