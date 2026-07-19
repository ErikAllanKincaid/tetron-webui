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

// Tracks which network rows are currently expanded (by name) across
// re-renders, so polling every 2s doesn't visually collapse something the
// user just opened. Rebuilt from scratch on each poll otherwise.
const expandedNetworks = new Set();
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
    net.role === "coordinator"
      ? `<button class="btn-small btn-danger" data-action="kick" data-net="${net.short_id}" data-peer="${peer.short_id}">kick</button>`
      : "";
  return `<tr>
    <td>${peer.hostname || peer.ip}</td>
    <td class="mono">${peer.ip}</td>
    <td class="mono">${status}</td>
    <td>${kickBtn}</td>
  </tr>`;
}

function renderNukeSection(net, myShortId) {
  const proposals = net.nuke_proposals || [];
  const iAlreadyProposed = proposals.some((p) => p.short_id === myShortId);
  const otherProposal = proposals.find((p) => p.short_id !== myShortId);

  let banner = "";
  if (proposals.length > 0) {
    const who = proposals.map((p) => p.short_id).join(", ");
    banner = `<div class="nuke-proposal-banner">Nuke proposed by ${who} (${proposals.length}/2 coordinators needed).</div>`;
  }

  let actions = "";
  if (net.role === "coordinator") {
    if (iAlreadyProposed) {
      actions = `<button class="btn-small btn-secondary" data-action="nuke-cancel" data-net="${net.short_id}">cancel my proposal</button>`;
    } else if (otherProposal) {
      actions = `<button class="btn-small btn-danger" data-action="nuke-second" data-net="${net.short_id}" data-second="${otherProposal.short_id}">second ${otherProposal.short_id}'s proposal</button>`;
    } else {
      actions = `<button class="btn-small btn-danger" data-action="nuke-propose" data-net="${net.short_id}">destroy network</button>`;
    }
  }

  return `<div class="danger-zone">
    <div class="danger-zone-label">Danger zone</div>
    ${banner}
    ${actions}
  </div>`;
}

function renderNetworkRow(net, myShortId) {
  const isExpanded = expandedNetworks.has(net.name);
  const standbyBadge = !net.active ? '<span class="network-standby-badge">standby</span>' : "";
  const online = net.peers.filter((p) => p.connection).length;

  const peerRows = net.peers.map((p) => renderPeerRow(net, p)).join("");
  const peerTable = net.peers.length
    ? `<table class="peer-table"><thead><tr><th>host</th><th>ip</th><th>connection</th><th></th></tr></thead><tbody>${peerRows}</tbody></table>`
    : `<p class="muted">no other members</p>`;

  return `<div class="network-row ${isExpanded ? "expanded" : ""}" data-network="${net.name}">
    <div class="network-summary" data-action="toggle-expand" data-network="${net.name}">
      <span class="expand-arrow">▸</span>
      <span class="network-name">${net.name}</span>
      <span class="network-role">${net.role}</span>
      <span class="network-members">${online}/${net.peers.length + 1}</span>
      ${standbyBadge}
    </div>
    <div class="network-detail">
      <div class="network-meta mono">id ${net.short_id || "?"} · interface ${net.tun_name || "?"} · ${net.my_ip}</div>
      ${peerTable}
      <div class="action-row">
        <button class="btn-small btn-secondary" data-action="${net.active ? "standby" : "resume"}" data-network="${net.name}">
          ${net.active ? "standby this network" : "activate this network"}
        </button>
        <button class="btn-small" data-action="invite-create" data-network="${net.name}">mint invite</button>
        <button class="btn-small btn-secondary" data-action="leave" data-network="${net.name}">leave</button>
      </div>
      ${renderNukeSection(net, myShortId)}
    </div>
  </div>`;
}

function render(status) {
  lastStatus = status;
  setHeader(status);

  const container = document.getElementById("networks");
  const pending = document.getElementById("pending-networks");

  if (!status.reachable) {
    container.innerHTML = "";
    pending.classList.add("hidden");
    return;
  }

  if (status.networks.length === 0) {
    container.innerHTML = '<p class="muted">No networks yet -- create or join one above.</p>';
  } else {
    container.innerHTML = status.networks.map((n) => renderNetworkRow(n, status.endpoint_short)).join("");
  }

  if (status.pending_networks && status.pending_networks.length > 0) {
    pending.textContent = `Waiting for approval: ${status.pending_networks.join(", ")}`;
    pending.classList.remove("hidden");
  } else {
    pending.classList.add("hidden");
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

  if (action === "toggle-expand") {
    if (expandedNetworks.has(network)) expandedNetworks.delete(network);
    else expandedNetworks.add(network);
    if (lastStatus) render(lastStatus);
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

  if (action === "invite-create") {
    const result = await postJson(`/api/networks/${encodeURIComponent(network)}/invites`, {});
    if (result.ok) {
      alert(`Invite key (single use):\n\n${result.invite_key}`);
    } else {
      alert(`Failed to mint invite: ${result.error}`);
    }
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
    if (value !== "") data[key] = value;
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
// Boot
// -----------------------------------------------------------------------

poll();
setInterval(poll, POLL_INTERVAL_MS);
