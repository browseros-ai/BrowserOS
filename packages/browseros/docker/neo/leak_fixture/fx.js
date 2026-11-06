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
  closedHost: null, // its host, where document.activeElement stops
  facts: new Map(), // region id -> the dialog text or download name this page produced
  replies: new Map(), // top only: nonce -> callback for stateReply / focused
  rebased: 0, // copy layout: how many baseline tokens this page has made
  uploads: new Map(), // file input region id -> the text the page read from its file
  digests: new Map(), // deny-list or control input id -> promise of [{size, sha256} | {error}]
};

// The copy layout's two plain fields in MAIN (leak_plan.COPY_BASE, COPY_DROP): the baseline
// token the agent copies first, and the box it pastes into. No plant's value is ever set in
// either; a chord that launders one is what puts it in the paste box.
const COPY_BOXES = [
  ["fx-base", "base"],
  ["fx-drop", "drop"],
];

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
  if (region.digests) return watchDigests(input, region.id);
  if (input.type === "file") {
    // An upload: the page reads the chosen file and holds its text as the field's value.
    input.addEventListener("change", () => {
      const file = input.files[0];
      if (!file) return;
      file.text().then((text) => {
        FX.uploads.set(region.id, text);
        report(region.id, text);
      });
    });
    return;
  }
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
  // The locked layout: an action that waits for an editable field waits in vain.
  if (FX.slice.layout === "locked") input.readOnly = true;
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
    if (mode === "closed") [FX.closed, FX.closedHost] = [root, host];
    renderFields(regions, root);
  }
  if (slice.layout === "copy" && slice.location === "main") {
    for (const [id, word] of COPY_BOXES) {
      const label = el("label", {}, `Field ${word} `);
      label.append(el("input", { type: "text", id }));
      document.body.prepend(label);
    }
  }
  const echoes = el("div", { id: "echoes" });
  document.body.append(echoes);
  for (const region of regionsWhere((r) => r.exposure === "echo" || r.exposure === "server_echo")) {
    echoes.append(el("p", { class: "echo", id: region.id }));
  }
  // A document the agent wrote with setContent brings its own frames (content_html).
  if (!slice.written) for (const frame of slice.frames) if (!frame.late) addFrame(frame);
  for (const region of regionsWhere((r) => r.value !== undefined && r.exposure === "field")) {
    if (!region.late) plant(region);
  }
  for (const region of regionsWhere((r) => r.exposure === "field" || r.exposure === "pasted")) {
    if (FX.fields.get(region.id).value === "") report(region.id, "");
  }
  if (slice.layout === "upload" && slice.location === "main") renderDigestControl();
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

// A deny-list input (leak_plan.deny_uploads) and the read-back's control input
// (leak_plan.DIGEST_CONTROL): the page reads every file it is given, whole, and keeps a
// sha256 of each for the oracle's read_back, which compares it with the file inside the
// container. Nothing read here reaches FX.values, an echo or a proof, so neither the bytes
// of Neo's own files nor a digest of them can leave the container.
function watchDigests(input, id) {
  input.addEventListener("change", () => {
    FX.digests.set(id, Promise.all([...input.files].map(digestOf)));
  });
}

function digestOf(file) {
  return file.arrayBuffer().then(
    (bytes) => ({ size: bytes.byteLength, sha256: sha256(bytes) }),
    (error) => ({ error: (error && error.name) || String(error) }),
  );
}

function renderDigestControl() {
  const input = el("input", { type: "file", id: "fx-digest-control" });
  watchDigests(input, input.id);
  document.body.append(input);
}

// SHA-256 (FIPS 180-4) of an ArrayBuffer, as hex. crypto.subtle needs a secure context,
// which http://a.test is not.
// prettier-ignore
const SHA256_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

function ror(x, n) {
  return (x >>> n) | (x << (32 - n));
}

function sha256(buffer) {
  const bytes = new Uint8Array(buffer);
  // The message, a 1 bit, zeros, then its length in bits as 64 bits: whole 64-byte blocks.
  const padded = new Uint8Array(((bytes.length + 72) >> 6) << 6);
  padded.set(bytes);
  padded[bytes.length] = 0x80;
  const view = new DataView(padded.buffer);
  view.setUint32(padded.length - 8, Math.floor(bytes.length / 0x20000000));
  view.setUint32(padded.length - 4, (bytes.length << 3) >>> 0);
  // prettier-ignore
  const h = new Uint32Array([
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ]);
  const w = new Uint32Array(64);
  for (let at = 0; at < padded.length; at += 64) {
    for (let i = 0; i < 16; i++) w[i] = view.getUint32(at + i * 4);
    for (let i = 16; i < 64; i++) {
      const s0 = ror(w[i - 15], 7) ^ ror(w[i - 15], 18) ^ (w[i - 15] >>> 3);
      const s1 = ror(w[i - 2], 17) ^ ror(w[i - 2], 19) ^ (w[i - 2] >>> 10);
      w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }
    let [a, b, c, d, e, f, g, k] = h;
    for (let i = 0; i < 64; i++) {
      const t1 = k + (ror(e, 6) ^ ror(e, 11) ^ ror(e, 25)) + ((e & f) ^ (~e & g)) + SHA256_K[i] + w[i];
      const t2 = (ror(a, 2) ^ ror(a, 13) ^ ror(a, 22)) + ((a & b) ^ (a & c) ^ (b & c));
      [k, g, f, e, d, c, b, a] = [g, f, e, (d + t1) | 0, c, b, a, (t1 + t2) | 0];
    }
    h[0] += a;
    h[1] += b;
    h[2] += c;
    h[3] += d;
    h[4] += e;
    h[5] += f;
    h[6] += g;
    h[7] += k;
  }
  return Array.from(h, (x) => x.toString(16).padStart(8, "0")).join("");
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
      const input = FX.fields.get(region.id);
      text = !input ? null : input.type === "file" ? (FX.uploads.get(region.id) ?? "") : input.value;
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
  for (const [id] of COPY_BOXES) {
    const box = document.getElementById(id);
    if (box) out[id] = [fnv1a(box.value), box.value.length];
  }
  return out;
}

// The element keyboard input reaches in this document, by id, or null when the document
// does not have the focus. It follows the focus into open shadow roots and into the closed
// root this page made; an iframe's own document answers for what is inside it.
function focusedId() {
  if (!document.hasFocus()) return null;
  let node = document.activeElement;
  for (;;) {
    const root = node && (node.shadowRoot || (node === FX.closedHost ? FX.closed : null));
    if (!root || !root.activeElement) break;
    node = root.activeElement;
  }
  return node && node.id ? node.id : null;
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
  const focus = [focusedId()];
  for (const reply of replies) {
    Object.assign(regions, reply.regions);
    documents.push(reply.location);
    focus.push(reply.focus);
  }
  return {
    at: Date.now(),
    regions,
    documents,
    frames: frames.length,
    focus: focus.filter((id) => id),
  };
}

async function act(name) {
  if (ACTS[name]) ACTS[name]();
  if (BROADCAST.has(name)) await ask({ fx: "act", name }, descendants(window), 2000);
  return true;
}

// The owner's click: focus each iframe on the path from the top, then the field itself, with
// its text selected when `select` is set (as a click and select-all would leave it).
async function focus(id, path, select = false) {
  const [reply] = await ask({ fx: "focus", id, path, select }, [window], 3000);
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
  if (data.select && input.select) input.select();
  const focused = input.getRootNode().activeElement === input;
  window.top.postMessage(
    { fx: "reply", nonce: data.nonce, focused, hasFocus: document.hasFocus() },
    "*",
  );
}

const BROADCAST = new Set(["show_password", "late_plant", "rebase"]);
const ACTS = {
  rebase: () => {
    // Before each copy probe. Every field gets its planted value back, as at load (no input
    // events), so a cut that went through, or keys a chord typed, leave nothing for the next
    // probe. In MAIN: a fresh baseline token, selected with the focus on it, and an empty
    // paste box.
    for (const r of regionsWhere((r) => r.value !== undefined && FX.fields.has(r.id))) {
      plant(r);
    }
    const base = document.getElementById("fx-base");
    if (!base) return;
    FX.rebased += 1;
    base.value = `${Math.random().toString(36).slice(2, 10)}q${FX.rebased}`;
    document.getElementById("fx-drop").value = "";
    base.focus();
    base.select();
  },
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
    const reply = {
      fx: "reply",
      nonce: data.nonce,
      location: FX.slice.location,
      regions: ownState(),
      focus: focusedId(),
    };
    event.source.postMessage(reply, "*");
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
  // What a deny-list or control input read, once every file is read; null before a change.
  digests: (id) => FX.digests.get(id) ?? null,
};
