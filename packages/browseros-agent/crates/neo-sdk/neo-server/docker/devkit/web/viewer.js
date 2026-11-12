// The dev kit's viewer: Neo's whole screen through claw-sdk's `GET /v1/display` (RFB over a
// WebSocket, through claw-sdk's display filter), as view or control, plus the takeover flow.
//
// The browser cannot put `Authorization` on a WebSocket, so the socket goes to the gateway on
// this page's origin, which opens the upstream display with the bearer header and relays its
// close code. Close codes are claw-sdk's (4002, 1008, 1003, 1011) or the gateway's
// (4000 + HTTP status for a refused upgrade, 4502 for claw-sdk unreachable); see gateway.ts.

import RFB from "/novnc/core/rfb.js";
import { api, h, clock } from "/kit/common.js";

const RETRY_MS = 2000;

// What the owner asked for. Mirrored in the page URL, so a reload reconnects the same way, and
// passed through to /v1/display as is: claw-sdk owns the rules (a control display takes no
// follow, a view takes no release) and its refusal is shown in its own words.
const query = new URLSearchParams(location.search);
const want = {
  mode: query.get("mode") ?? (query.get("release") ? "control" : "view"),
  followTab: query.get("followTab"),
  followTask: query.get("followTask"),
  release: query.get("release"),
};

/**
 * How a display ended, by close code, and what the viewer does next: `retry` reconnects after
 * RETRY_MS, `toView` drops a control display's mode and release first (its takeover is over).
 * A refused takeover upgrade carries 4409 and the host API error in its reason.
 */
const ENDS = [
  { code: 4002, retry: true, toView: true, words: "Released: the takeover ended. Back to view." },
  { code: 4409, when: "takeover_released", retry: true, toView: true, words: "That takeover is already released. Back to view." },
  { code: 4404, retry: false, toView: true, words: "No takeover has that release id. Switched to view; connect again." },
  { code: 4400, retry: false, words: "claw-sdk refused the display query." },
  { code: 4401, retry: false, words: "claw-sdk refused the gateway's token. Is the host-token file the one the container reads?" },
  { code: 4503, retry: true, words: "Neo's display or browser is unavailable. Retrying." },
  { code: 4502, retry: true, words: "The gateway cannot reach claw-sdk (is the container restarting?). Retrying." },
  { code: 1008, retry: false, words: "claw-sdk's display filter refused a message from this viewer." },
  { code: 1003, retry: false, words: "This viewer sent a text frame; the display takes binary only." },
  { code: 1011, retry: true, words: "Neo's screen closed the connection. Retrying." },
  { code: 1006, retry: true, words: "The connection to the gateway dropped. Retrying." },
];

function describeEnd(code, reason) {
  return (
    ENDS.find((end) => end.code === code && (!end.when || reason.includes(end.when))) ?? {
      retry: false,
      words: "The display closed.",
    }
  );
}

// The display's lifecycle; `attempt` makes a stale socket's late close event a no-op.
//   idle ──connect──▶ connecting ──RFB connect──▶ live ──close──▶ ended ──(retry, 2 s)──▶ connecting
//   any ──Disconnect──▶ idle
let phase = { name: "idle" };
let attempt = 0;
let rfb = null;
let retryTimer = null;
let takeover = null;

const $ = (id) => document.getElementById(id);
const screen = $("screen");

function displayUrl() {
  const params = new URLSearchParams({ mode: want.mode });
  for (const key of ["followTab", "followTask", "release"]) {
    if (want[key]) params.set(key, want[key]);
  }
  return { ws: `ws://${location.host}/v1/display?${params}`, page: `?${params}` };
}

function teardown() {
  attempt += 1;
  clearTimeout(retryTimer);
  retryTimer = null;
  if (rfb) {
    try {
      rfb.disconnect();
    } catch {
      // Already disconnected; nothing to undo.
    }
    rfb = null;
  }
}

function connect() {
  teardown();
  const mine = attempt;
  const url = displayUrl();
  history.replaceState(null, "", url.page);
  // The socket is ours, not noVNC's, so its close event (code and reason) reaches us; noVNC's
  // own disconnect event only says clean or not.
  const ws = new WebSocket(url.ws);
  ws.binaryType = "arraybuffer";
  ws.addEventListener("close", (event) => {
    if (mine === attempt) ended(event.code, event.reason);
  });
  rfb = new RFB(screen, ws, { shared: true });
  rfb.viewOnly = want.mode === "view";
  rfb.scaleViewport = true;
  // noVNC paints its own dark gray around the screen; the kit is light only.
  rfb.background = getComputedStyle(document.documentElement).getPropertyValue("--sunken").trim();
  // The display filter may refuse SetDesktopSize as an unknown message (close 1008).
  rfb.resizeSession = false;
  rfb.addEventListener("connect", () => {
    if (mine !== attempt) return;
    setPhase({ name: "live" });
    if (want.mode === "control") rfb.focus();
  });
  setPhase({ name: "connecting" });
}

function ended(code, reason) {
  rfb = null;
  const end = describeEnd(code, reason);
  if (end.toView) {
    want.mode = "view";
    want.release = null;
    if (takeover) refreshTakeover();
  }
  if (end.retry) retryTimer = setTimeout(connect, end.toView ? 0 : RETRY_MS);
  setPhase({ name: "ended", code, reason, end });
}

function disconnect() {
  teardown();
  setPhase({ name: "idle" });
}

function setPhase(next) {
  phase = next;
  render();
}

function render() {
  for (const button of $("mode").querySelectorAll("button")) {
    button.classList.toggle("on", button.dataset.mode === want.mode);
  }
  $("warning").hidden = want.mode !== "control";
  const followKind = want.followTab ? "followTab" : want.followTask ? "followTask" : "";
  $("follow-kind").value = followKind;
  if (document.activeElement !== $("follow-value")) $("follow-value").value = want.followTab ?? want.followTask ?? "";
  $("follow-kind").disabled = $("follow-value").disabled = want.mode === "control";
  $("connect").disabled = phase.name === "connecting" || phase.name === "live";
  $("disconnect").disabled = phase.name === "idle";

  const badge = $("status-badge");
  const overlay = $("overlay");
  let badgeText = phase.name;
  let badgeClass = "";
  let words = "";
  let why = "";
  if (phase.name === "idle") {
    words = "Not connected.";
  } else if (phase.name === "connecting") {
    badgeClass = "info";
    words = `Opening a ${want.mode} display...`;
  } else if (phase.name === "live") {
    badgeText = `live: ${want.mode}`;
    badgeClass = want.mode === "control" ? "warn" : "ok";
    words = want.release ? `Bound to takeover ${want.release}.` : "";
  } else {
    badgeText = phase.end.retry ? "waiting" : "closed";
    badgeClass = phase.end.retry ? "warn" : "err";
    words = phase.end.words;
    why = `close ${phase.code}${phase.reason ? `: ${phase.reason}` : ""}`;
  }
  badge.textContent = badgeText;
  badge.className = `badge ${badgeClass}`;
  $("status-words").textContent = words;
  $("status-why").textContent = why;
  overlay.hidden = phase.name === "live";
  overlay.textContent = phase.name === "live" ? "" : [words, why].filter(Boolean).join("\n");
  overlay.style.whiteSpace = "pre-line";
}

// ---------- takeover ----------

async function startTakeover() {
  const raw = $("takeover-tab").value.trim();
  if (!/^\d+$/.test(raw)) {
    renderTakeover("Enter a tab id (a number from the recent tabs list).");
    return;
  }
  // With `viewer`, claw-sdk fits the tab to this page's screen box: it brings the tab to the
  // front (Neo keeps agent tabs in the background) and sizes Neo's screen to match, until release.
  const clamp = (n) => Math.min(4096, Math.max(100, Math.round(n)));
  const viewer = { width: clamp(screen.clientWidth), height: clamp(screen.clientHeight) };
  try {
    takeover = await api("/v1/takeovers", { method: "POST", body: { tabId: Number(raw), viewer } });
  } catch (error) {
    renderTakeover(`${error.code}: ${error.message}`);
    return;
  }
  want.mode = "control";
  want.release = takeover.id;
  want.followTab = want.followTask = null;
  renderTakeover();
  connect();
}

async function releaseTakeover() {
  try {
    takeover = await api(`/v1/takeovers/${encodeURIComponent(takeover.id)}/release`, {
      method: "POST",
      body: { reason: "done" },
    });
    renderTakeover();
  } catch (error) {
    renderTakeover(`${error.code}: ${error.message}`);
  }
  // The bound display closes with 4002, and ended() takes the viewer back to view.
}

async function refreshTakeover() {
  if (!takeover) return;
  try {
    takeover = await api(`/v1/takeovers/${encodeURIComponent(takeover.id)}`);
  } catch {
    // Released over an hour ago, or Neo restarted: the card keeps what it last knew.
  }
  renderTakeover();
}

function renderTakeover(problem) {
  const card = $("takeover-card");
  const rows = takeover
    ? [
        ["id", takeover.id],
        ["tab", takeover.tabId],
        ["state", takeover.state && takeover.state + (takeover.releaseReason ? ` (${takeover.releaseReason})` : "")],
        ["lock", takeover.lockId],
        ["fit", takeover.fitId],
        ["expires", takeover.expiresAt && clock(takeover.expiresAt)],
      ].filter(([, value]) => value !== undefined)
    : [];
  card.replaceChildren(
    h(
      "div",
      null,
      problem ? h("div", { class: "notice err", style: "margin-top:6px" }, problem) : null,
      takeover
      ? h(
          "div",
          { style: "margin-top:8px" },
          h("dl", { class: "kv" }, rows.map(([k, v]) => [h("dt", null, k), h("dd", null, String(v))])),
          takeover.state === "active"
            ? h("button", { class: "primary", style: "margin-top:8px", onclick: releaseTakeover }, "Release")
            : null,
        )
      : null,
    ),
  );
}

// ---------- recent tabs ----------

/** Tabs the agent's latest operations touched, newest first, labelled with their last page. */
async function loadRecentTabs() {
  let trace;
  try {
    trace = await api("/v1/trace?tail=200");
  } catch (error) {
    $("recent").textContent = `${error.code}: ${error.message}`;
    return;
  }
  const tabs = new Map();
  for (const op of trace.operations) {
    const tab = op.tabId ?? op.page?.tabId;
    if (tab === undefined) continue;
    tabs.delete(tab);
    tabs.set(tab, op.page ?? {});
  }
  const recent = [...tabs].reverse().slice(0, 15);
  $("recent-tabs").replaceChildren(
    ...recent.map(([tab, page]) => h("option", { value: String(tab), label: page.title || page.url || "" })),
  );
  $("recent").replaceChildren(
    recent.length === 0
      ? "No operations yet."
      : h(
          "div",
          { style: "display:flex;flex-direction:column;gap:4px" },
          recent.map(([tab, page]) =>
            h(
              "div",
              { class: "row", style: "flex-wrap:nowrap" },
              h("code", null, String(tab)),
              h(
                "span",
                { class: "muted", style: "flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap", title: page.url ?? "" },
                page.title || page.url || "",
              ),
              h("button", { title: "Keep this tab in front (view)", onclick: () => setFollow("followTab", String(tab)) }, "follow"),
              h("button", { title: "Put it in the takeover box", onclick: () => ($("takeover-tab").value = String(tab)) }, "pick"),
            ),
          ),
        ),
    h("div", { style: "margin-top:6px" }, h("a", { href: "#", onclick: (e) => (e.preventDefault(), loadRecentTabs()) }, "refresh")),
  );
}

function setFollow(kind, value) {
  want.followTab = kind === "followTab" && value ? value : null;
  want.followTask = kind === "followTask" && value ? value : null;
  if (want.mode === "view") connect();
  else render();
}

// ---------- wiring ----------

$("mode").addEventListener("click", (event) => {
  const mode = event.target.dataset?.mode;
  if (!mode || mode === want.mode) return;
  want.mode = mode;
  if (mode === "view") want.release = null;
  else want.followTab = want.followTask = null;
  connect();
});
$("follow-kind").addEventListener("change", () => setFollow($("follow-kind").value, $("follow-value").value.trim()));
$("follow-value").addEventListener("change", () => setFollow($("follow-kind").value || "followTask", $("follow-value").value.trim()));
$("connect").addEventListener("click", connect);
$("disconnect").addEventListener("click", disconnect);
$("takeover-start").addEventListener("click", startTakeover);

if (want.release) {
  takeover = { id: want.release };
  refreshTakeover();
}
loadRecentTabs();
connect();
