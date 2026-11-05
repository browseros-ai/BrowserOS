// The leak-test fixture: one script for every frame of every instance (leak_plan.py's
// Instance.slice is its contract).
//
// Each document learns who it is from its path (/fx/<instance>/<location>), fetches its
// slice (<path>.json, no-store) and renders it: one block per field region with the
// attributes and label words the slice gives, its child frames, and in MAIN the open and
// the closed shadow root. A server echo document (after a submit) carries its slice inline.
// Values exist only in slices and in what the owner types, never in a URL or in run code.
//
// Every field reports its value to the top document, which renders the echoes:
//
//   any frame ── postMessage {fx: 'value', id, value} ──▶ top: FX.values ─▶ <p class=echo>
//
// That is how real pages move a value across frames (cooperative postMessage); what claw-sdk
// sees is only the resulting text in the top document.
//
// The oracle (leak_inside.py) only calls window.__fx in a tab's top frame, in the page's own
// main world. For a proof the top asks every descendant window (window.frames, which
// cross-origin windows expose) and merges their answers:
//
//   __fx.state() ── {fx: 'state', nonce} ──▶ each frame ── {fx: 'stateReply', regions} ─▶ top
//
// Answers are FNV-1a digests and lengths, never values.

"use strict";

const FX = {
  slice: null,
  values: new Map(), // top only: field region id -> current value, across every frame
  fields: new Map(), // region id -> element in this document (closed-root fields too)
  closed: null, // the closed ShadowRoot, reachable only through this closure
  facts: new Map(), // region id -> the dialog text or download name this page produced
  replies: new Map(), // top only: nonce -> callback for stateReply / focused
};

function fnv1a(text) {
  let h = 0x811c9dc5;
  for (const b of new TextEncoder().encode(text)) {
    h ^= b;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
}

function el(tag, attrs = {}, text = null) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  if (text !== null) node.textContent = text;
  return node;
}

function regionsWhere(test) {
  return FX.slice.regions.filter(test);
}

// "ECHO <anchor> {{value}}": the {{ }} delimiters survive JSON and HTML escaping, so the
// judge reads a short value's slot exactly; without them a 3-digit CVC could not be told
// from a timestamp.
function echoText(region, value) {
  return `ECHO ${region.anchor} {{${value}}}`;
}

// The window whose FX.values collects every field of the instance: the main tab's top, also
// for a popup, which reports to and reads from its opener (same origin).
function collector() {
  return window.opener ? window.opener.top : window.top;
}

function sourceValue(region) {
  if (region.value !== undefined) return region.value;
  const values = collector() === window ? FX.values : collector().__fx.values();
  return region.sources.map((id) => values.get(id) ?? "").join("");
}

function slotOf(text, region) {
  if (text === null || text === undefined) return null;
  const start = text.indexOf(`ECHO ${region.anchor} {{`);
  if (start < 0) return null;
  const from = start + region.anchor.length + 8;
  const end = text.indexOf("}}", from);
  return end < 0 ? null : text.slice(from, end);
}

function report(id, value) {
  if (collector() === window) {
    FX.values.set(id, value);
    refreshEchoes();
  } else {
    collector().postMessage({ fx: "value", id, value }, "*");
  }
}

function mirror(input, region) {
  // What a React-controlled input does: the value attribute follows every edit.
  const update = () => {
    input.setAttribute("value", input.value);
    report(region.id, input.value);
  };
  input.addEventListener("input", update);
  input.addEventListener("change", update);
}

function renderField(region, parent) {
  const input = el("input");
  for (const [k, v] of region.attrs) input.setAttribute(k, v);
  input.id = region.id;
  const label = el("label", {}, `${region.words} ${region.anchor} `);
  label.append(input);
  FX.fields.set(region.id, input);
  mirror(input, region);
  if (region.navigates) {
    // Enter submits: the server answers with a document that echoes the value, so the
    // field is gone and only the echo remains. POST, so the value is never in a URL.
    const form = el("form", { method: "post", action: `/fx/${FX.slice.instance}/echo` });
    form.append(el("input", { type: "hidden", name: "region", value: region.id }));
    input.name = "value";
    form.append(label);
    parent.append(form);
  } else {
    parent.append(el("div", { class: "f" }));
    parent.lastChild.append(label);
  }
}

function renderFields(regions, parent) {
  const splits = new Map();
  for (const region of regions) {
    if (region.kind === "otp_split" && region.exposure === "field") {
      if (!splits.has(region.plant)) {
        const form = el("form", { class: "split" });
        form.append(el("p", {}, "Enter the verification code"));
        parent.append(form);
        splits.set(region.plant, form);
      }
      renderField(region, splits.get(region.plant));
    } else {
      renderField(region, parent);
    }
  }
}

function plant(region) {
  const input = FX.fields.get(region.id);
  input.value = region.value;
  input.setAttribute("value", region.value);
  // Design risk R3: a toggle before Neo's first touch, so no seal ever saw a password.
  if (region.kind === "gap_early_toggle") input.type = "text";
  report(region.id, region.value);
}

function render() {
  const slice = FX.slice;
  document.body.className = slice.layout;
  document.body.append(el("h2", {}, `${slice.instance} ${slice.location}`));
  const fieldRegion = (r) => r.exposure === "field" || r.exposure === "pasted";
  const here = (loc) => (r) => r.location === loc && fieldRegion(r);
  renderFields(regionsWhere(here(slice.location)), document.body);
  for (const [loc, mode] of [
    ["shadow_open", "open"],
    ["shadow_closed", "closed"],
  ]) {
    const regions = regionsWhere(here(loc));
    if (!regions.length) continue;
    const host = el("div", { id: `host-${mode}` });
    document.body.append(host);
    const root = host.attachShadow({ mode });
    if (mode === "closed") FX.closed = root;
    renderFields(regions, root);
  }
  const echoes = el("div", { id: "echoes" });
  document.body.append(echoes);
  for (const region of regionsWhere((r) => r.exposure === "echo" || r.exposure === "server_echo")) {
    echoes.append(el("p", { class: "echo", id: region.id }));
  }
  for (const frame of slice.frames) if (!frame.late) addFrame(frame);
  for (const region of regionsWhere((r) => r.value !== undefined && r.exposure === "field")) {
    if (!region.late) plant(region);
  }
  for (const region of regionsWhere((r) => r.exposure === "field" || r.exposure === "pasted")) {
    if (FX.fields.get(region.id).value === "") report(region.id, "");
  }
  if (slice.layout === "pixels" || slice.layout === "owner") ACTS.repaint();
  if (slice.layout === "owner" && slice.location === "main") {
    // The owner opens the popup with a key on this button: on Neo's display only the
    // owner's own input brings a new tab to the front, where the next keys go.
    const button = el("button", { id: "fx-open-popup", type: "button" }, "Open");
    button.addEventListener("click", ACTS.open_popup);
    document.body.prepend(button);
  }
  if (slice.location === "popup") {
    const titled = regionsWhere((r) => r.exposure === "popup_title");
    if (titled.length) document.title = titled.map((r) => echoText(r, sourceValue(r))).join(" ");
  }
  refreshEchoes();
}

function addFrame(frame) {
  document.body.append(el("iframe", { id: frame.id, src: frame.src }));
}

function refreshEchoes() {
  for (const region of regionsWhere((r) => r.exposure === "echo" || r.exposure === "server_echo")) {
    const p = document.getElementById(region.id);
    const text = echoText(region, sourceValue(region));
    if (p.textContent !== text) {
      p.textContent = text;
      p.setAttribute("data-echo", text);
    }
  }
}

// What one document holds now: a digest per region it renders, null where the region does
// not exist (yet). The top adds the regions only it can see: title, URL, dialog, download.
function ownState() {
  const out = {};
  for (const region of FX.slice.regions) {
    let text = null;
    if (region.exposure === "field" || region.exposure === "pasted") {
      text = FX.fields.has(region.id) ? FX.fields.get(region.id).value : null;
    } else if (region.exposure === "echo" || region.exposure === "server_echo") {
      const p = document.getElementById(region.id);
      text = p ? slotOf(p.textContent, region) : null;
    } else if (region.exposure === "title" || region.exposure === "popup_title") {
      text = slotOf(document.title, region);
    } else if (region.exposure === "url") {
      text = slotOf(decodeURIComponent(location.search), region);
    } else {
      text = slotOf(FX.facts.get(region.id) ?? null, region);
    }
    out[region.id] = text === null ? null : [fnv1a(text), text.length];
  }
  return out;
}

function descendants(win) {
  const out = [];
  for (let i = 0; i < win.frames.length; i++) {
    out.push(win.frames[i], ...descendants(win.frames[i]));
  }
  return out;
}

function ask(message, targets, timeoutMs) {
  // Top only: post `message` with a fresh nonce to `targets`; collect one reply from each,
  // or whatever arrived when the time is up.
  const nonce = Math.random().toString(36).slice(2);
  const replies = [];
  return new Promise((resolve) => {
    const done = () => {
      FX.replies.delete(nonce);
      resolve(replies);
    };
    FX.replies.set(nonce, (reply) => {
      replies.push(reply);
      if (replies.length >= targets.length) done();
    });
    for (const target of targets) target.postMessage({ ...message, nonce }, "*");
    if (!targets.length) done();
    setTimeout(done, timeoutMs);
  });
}

async function state() {
  const frames = descendants(window);
  const replies = await ask({ fx: "state" }, frames, 2000);
  const regions = ownState();
  const documents = [FX.slice.location];
  for (const reply of replies) {
    Object.assign(regions, reply.regions);
    documents.push(reply.location);
  }
  return { at: Date.now(), regions, documents, frames: frames.length };
}

async function act(name) {
  if (ACTS[name]) ACTS[name]();
  if (BROADCAST.has(name)) await ask({ fx: "act", name }, descendants(window), 2000);
  return true;
}

// The owner's click: focus each iframe on the path from the top, then the field itself.
async function focus(id, path) {
  const [reply] = await ask({ fx: "focus", id, path }, [window], 3000);
  return reply ?? { focused: false, why: "no answer" };
}

function focusStep(data) {
  if (data.path.length) {
    const frame = document.getElementById(data.path[0]);
    frame.focus();
    frame.contentWindow.postMessage({ ...data, path: data.path.slice(1) }, "*");
    return;
  }
  const input = FX.fields.get(data.id) ?? document.getElementById(data.id);
  if (!input) return;
  input.focus();
  const focused = input.getRootNode().activeElement === input;
  window.top.postMessage(
    { fx: "reply", nonce: data.nonce, focused, hasFocus: document.hasFocus() },
    "*",
  );
}

const BROADCAST = new Set(["show_password", "late_plant"]);
const ACTS = {
  show_password: () => {
    for (const r of regionsWhere((r) => r.kind === "show_password" && FX.fields.has(r.id))) {
      FX.fields.get(r.id).type = "text";
    }
  },
  late_plant: () => {
    // What autofill does: set the value, then fire input and change.
    for (const r of regionsWhere((r) => r.late && r.value !== undefined && FX.fields.has(r.id))) {
      const input = FX.fields.get(r.id);
      input.value = r.value;
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.dispatchEvent(new Event("change", { bubbles: true }));
    }
  },
  echo_title: () => {
    document.title = regionsWhere((r) => r.exposure === "title")
      .map((r) => echoText(r, sourceValue(r)))
      .join(" ");
  },
  echo_url: () => {
    const text = regionsWhere((r) => r.exposure === "url")
      .map((r) => echoText(r, sourceValue(r)))
      .join(" ");
    history.pushState(null, "", `${location.pathname}?e=${encodeURIComponent(text)}`);
  },
  alert: () => {
    const regions = regionsWhere((r) => r.exposure === "dialog");
    const text = regions.map((r) => echoText(r, sourceValue(r))).join(" ");
    for (const r of regions) FX.facts.set(r.id, text);
    setTimeout(() => alert(text), 0); // returns first: an open alert blocks this page
  },
  download: () => {
    const regions = regionsWhere((r) => r.exposure === "download");
    const name = `${regions.map((r) => echoText(r, sourceValue(r))).join(" ")}.txt`;
    for (const r of regions) FX.facts.set(r.id, name);
    el("a", { download: name, href: `/fx/${FX.slice.instance}/dl` }).click();
  },
  popup: () => {
    window.open(`/fx/${FX.slice.instance}/popup`);
  },
  open_popup: () => {
    window.open(`/fx/${FX.slice.instance}/popup`);
  },
  add_late_frame: () => {
    for (const frame of FX.slice.frames) if (frame.late) addFrame(frame);
  },
  repaint: () => {
    // Live streams send frames only for a page that repaints, and a stream with no frames
    // cannot show that it stopped.
    if (document.getElementById("tick")) return;
    const tick = el("div", { id: "tick" }, "0");
    document.body.prepend(tick);
    let n = 0;
    setInterval(() => {
      tick.textContent = String(++n);
    }, 200);
  },
};

window.addEventListener("message", (event) => {
  const data = event.data;
  if (!data || typeof data.fx !== "string") return;
  if (data.fx === "value" && collector() === window) {
    FX.values.set(data.id, data.value);
    refreshEchoes();
  } else if (data.fx === "state") {
    event.source.postMessage(
      { fx: "reply", nonce: data.nonce, location: FX.slice.location, regions: ownState() },
      "*",
    );
  } else if (data.fx === "act") {
    if (ACTS[data.name]) ACTS[data.name]();
    event.source.postMessage({ fx: "reply", nonce: data.nonce }, "*");
  } else if (data.fx === "focus") {
    focusStep(data);
  } else if (data.fx === "reply" && FX.replies.has(data.nonce)) {
    FX.replies.get(data.nonce)(data);
  }
});

function boot() {
  const inline = document.getElementById("fx-slice");
  if (inline) {
    FX.slice = JSON.parse(inline.textContent);
  } else {
    // Synchronous on purpose: the document's load event then waits for the plant, so an
    // agent's goto (waitUntil: load) resolves only once every frame holds its values.
    const request = new XMLHttpRequest();
    request.open("GET", `${location.pathname}.json`, false);
    request.send();
    FX.slice = JSON.parse(request.responseText);
  }
  render();
}

window.__fx = {
  booted: new Promise((resolve) => resolve(boot())),
  state,
  act,
  focus,
  values: () => FX.values,
};
