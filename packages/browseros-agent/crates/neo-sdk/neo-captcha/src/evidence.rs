//! The page evidence the verify check reads, and the text form the judge sees.
//!
//! One read-only DOM probe (`CaptchaPage::scan_dom`, DOM world, closed shadow roots included)
//! fills a [`PageBrief`]: what covers the page, the widget's face, the submit control, the page's
//! live alerts and status lines, and a little visible text. Tier 1 reads its facts
//! ([`Measured`]); tier 2 sends [`PageBrief::to_evidence`] to the judge as tagged lines. The brief
//! never holds input values, URL paths or queries, so neither the judge nor a log line can
//! receive them.
//!
//! ```text
//! page ─scan_dom(top, probe_script)─▶ Probe (raw JSON: words, layer, widgets, submit)
//!      ─scan_dom(frame, text)───────▶ frame texts (≤ 3 cross-site frames over the page)
//!                  │
//!                  ▼ PageBrief::from_probe: host only, no e-mail / long digit runs, capped
//!      PageBrief { words…, measured: Measured }
//!           ├─ measured ─▶ verify::signals (tier 1)
//!           └─ to_evidence ─▶ "[SITE] …\n[ON_TOP] …" ─▶ judge (tier 2)
//! ```
//!
//! The script only gathers. Every rule about what may leave the page (no e-mail, no digit runs,
//! host only) runs here in Rust, where a test pins it, and runs again when the text is rendered.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tokio::time::Instant;

use crate::{
    detect::DOM_QUERIES,
    page::CaptchaPage,
    types::{CaptchaKind, PageError},
};

/// Line and size caps on the judge's text. Irrelevant state lowers the judge's answer quality,
/// and input tokens are what a call costs.
pub(crate) const MAX_EVIDENCE_LINES: usize = 40;
pub(crate) const MAX_EVIDENCE_CHARS: usize = 2_000;
/// Below this much evidence the judge answers confidently on nothing, so it is not asked.
pub(crate) const MIN_EVIDENCE_CHARS: usize = 60;
/// How much of the page's visible text the brief keeps.
const TEXT_CHARS: usize = 600;
/// Share of the view a layer without a vendor frame must cover to count as covering the page;
/// smaller layers (a cookie strip, a chat button) leave the page usable.
const COVER_SHARE: f64 = 0.3;
/// Cross-site frames the brief reads text from, and how large an unrelated one must be.
const MAX_FRAME_TEXTS: usize = 3;
const LARGE_FRAME_SHARE: f64 = 0.25;
/// The frame walk behind those texts; a slow walk costs the texts, never the brief.
const FRAME_WALK: Duration = Duration::from_secs(1);

/// One page line the probe read. `fresh` marks a line that was not there before the flow ran
/// (`PageBrief::new_since`), which is how a late "Verification failed" alert is told from an
/// old one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Line {
    pub text: String,
    pub fresh: bool,
}

/// Text read inside one visible cross-site frame, tagged with its host only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FrameText {
    pub host: String,
    pub text: String,
}

/// How a CAPTCHA vendor's frame takes part in the page. Read from the frame's address, so it
/// says what the frame is for, not whether it passed: a checkbox frame looks the same ticked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FramePart {
    /// A widget that may show a pass: Turnstile, reCAPTCHA's anchor, hCaptcha's checkbox.
    Checkbox,
    /// A challenge only a pass removes: reCAPTCHA's image puzzle, hCaptcha's challenge, a
    /// DataDome, HUMAN, Arkose, GeeTest or AWS WAF challenge.
    Puzzle,
    /// A vendor's automatic check still running (DataDome's device check).
    Checking,
}

/// A vendor frame inside what covers the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OnTop {
    pub kind: CaptchaKind,
    pub part: FramePart,
}

/// What the probe measured for tier 1, beside the words. Never rendered into evidence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Measured {
    /// The probe read the whole document: no closed shadow root left out, not still loading.
    /// `false` by default, so a brief nobody measured never reads as a clear page.
    pub complete: bool,
    /// The vendor frame inside the layer that takes a click on the submit control (or, with no
    /// usable submit control, the layer over the view's center and four quarter points).
    pub on_top: Option<OnTop>,
    /// A vendor's puzzle frame open anywhere in the view (reCAPTCHA's bframe, hCaptcha's
    /// challenge, DataDome's captcha), on top or not.
    pub puzzle: Option<CaptchaKind>,
    /// A layer without a vendor frame covers the submit control or a large part of the view.
    pub covered: bool,
    pub submit_disabled: bool,
}

/// What one probe saw. Every field is already safe to send: hosts not URLs, labels not values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PageBrief {
    pub site: Option<String>,
    pub title: Option<String>,
    /// What covers the view right now, in words ("dialog covering the whole view: …").
    pub on_top: Option<String>,
    /// The widget as measured ("Cloudflare Turnstile widget is visible inside the dialog").
    pub widget: Option<String>,
    /// The submit control as measured ("the Search button is covered by the dialog").
    pub submit: Option<String>,
    pub alerts: Vec<Line>,
    pub statuses: Vec<Line>,
    pub frames: Vec<FrameText>,
    pub http_status: Option<u16>,
    pub headings: Vec<String>,
    pub buttons: Vec<String>,
    pub text: Option<String>,
    pub measured: Measured,
}

impl PageBrief {
    /// The judge's state: one tagged line per fact, facts first, capped. The tags are the only
    /// trusted part; the judge's instructions say so. A line holding an e-mail address or a long
    /// digit run is dropped here too, so a brief built by hand cannot leak one either.
    pub(crate) fn to_evidence(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        let mut push = |tag: &str, value: &str| {
            let value = scrub_links(value.trim());
            if !value.is_empty() && !private(&value) {
                lines.push(format!("[{tag}] {value}"));
            }
        };
        for (tag, value) in [
            ("SITE", &self.site),
            ("TITLE", &self.title),
            ("ON_TOP", &self.on_top),
            ("WIDGET", &self.widget),
            ("SUBMIT", &self.submit),
        ] {
            if let Some(value) = value {
                push(tag, value);
            }
        }
        for line in &self.alerts {
            push(if line.fresh { "NEW ALERT" } else { "ALERT" }, &line.text);
        }
        for line in &self.statuses {
            push(if line.fresh { "NEW STATUS" } else { "STATUS" }, &line.text);
        }
        for frame in &self.frames {
            push(&format!("FRAME {}", frame.host), &frame.text);
        }
        if let Some(status) = self.http_status {
            push("HTTP", &status.to_string());
        }
        for heading in self.headings.iter().take(8) {
            push("HEADING", heading);
        }
        for button in self.buttons.iter().take(10) {
            push("BUTTON", button);
        }
        if let Some(text) = &self.text {
            push("TEXT", text);
        }
        let mut evidence = String::new();
        for line in lines.into_iter().take(MAX_EVIDENCE_LINES) {
            let room = MAX_EVIDENCE_CHARS.saturating_sub(evidence.chars().count());
            if room <= 1 {
                break;
            }
            if !evidence.is_empty() {
                evidence.push('\n');
            }
            evidence.extend(line.chars().take(room - 1));
        }
        evidence
    }

    /// This brief with every alert and status line absent from `before` marked fresh.
    #[must_use]
    pub(crate) fn new_since(mut self, before: &Self) -> Self {
        for line in &mut self.alerts {
            line.fresh = !before.alerts.iter().any(|old| old.text == line.text);
        }
        for line in &mut self.statuses {
            line.fresh = !before.statuses.iter().any(|old| old.text == line.text);
        }
        self
    }

    /// Adds a measured fact about the widget (its own frame's face, read by the verify check)
    /// to the `[WIDGET]` line.
    pub(crate) fn note_widget(&mut self, fact: &str) {
        self.widget = Some(match self.widget.take() {
            Some(widget) => format!("{widget}; {fact}"),
            None => fact.to_owned(),
        });
    }

    /// The brief from the probe's raw answer (what [`probe_script`] returns). `Err` only for an
    /// answer of the wrong shape; the caller treats that as a probe that did not run.
    pub(crate) fn from_probe(value: Value) -> Result<Self, PageError> {
        serde_json::from_value::<Probe>(value)
            .map(Probe::into_brief)
            .map_err(|_| PageError::Browser("invalid verify probe".into()))
    }
}

/// Reads the page into a brief: one DOM call on the top document, plus a text read in at most
/// [`MAX_FRAME_TEXTS`] cross-site frames that sit over the page or fill a quarter of it.
/// Read-only: it never focuses, clicks or writes. Vendor frames are not read here; the verify
/// check reads their faces with the vendors' own readers.
pub(crate) async fn brief<P: CaptchaPage>(page: &P) -> Result<PageBrief, PageError> {
    let value = page.scan_dom(None, &probe_script()).await?;
    let probe: Probe = serde_json::from_value(value)
        .map_err(|_| PageError::Browser("invalid verify probe".into()))?;
    let wanted: Vec<String> = probe
        .frames
        .iter()
        .filter(|frame| !frame.vendor && (frame.in_layer || frame.share >= LARGE_FRAME_SHARE))
        .map(|frame| frame.host.clone())
        .take(MAX_FRAME_TEXTS)
        .collect();
    let mut brief = probe.into_brief();
    if !wanted.is_empty() {
        brief.frames = frame_texts(page, &wanted).await?;
    }
    Ok(brief)
}

async fn frame_texts<P: CaptchaPage>(
    page: &P,
    hosts: &[String],
) -> Result<Vec<FrameText>, PageError> {
    let walk = match page.scan_frames(Instant::now() + FRAME_WALK).await {
        Ok(walk) => walk,
        Err(error @ (PageError::Closed | PageError::Cancelled)) => return Err(error),
        Err(_) => return Ok(Vec::new()),
    };
    let mut texts = Vec::new();
    for host in hosts {
        // The main frame is first in a walk; only child frames are cross-site candidates.
        let Some(frame) = walk.frames.iter().skip(1).find(|frame| {
            url::Url::parse(&frame.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .as_deref()
                == Some(host.as_str())
        }) else {
            continue;
        };
        match page.scan_dom(Some(frame), &frame_text_script()).await {
            Ok(value) => {
                let chunks: Vec<String> = serde_json::from_value(value).unwrap_or_default();
                if let Some(text) = safe_chunks(&chunks, 300) {
                    texts.push(FrameText {
                        host: host.clone(),
                        text,
                    });
                }
            }
            Err(error @ (PageError::Closed | PageError::Cancelled)) => return Err(error),
            Err(_) => {}
        }
    }
    Ok(texts)
}

/// Visible text as chunks (one per text node), walked through open and closed shadow roots and
/// slots. Form fields, editable regions, frames and scripts are skipped, so no value a visitor
/// typed is ever read. `visits` bounds the work on a huge page.
const TEXT_JS: &str = r#"
const SKIP = 'script, style, noscript, template, input, textarea, select, option, datalist, iframe, object, embed, svg, canvas, [contenteditable]:not([contenteditable="false"])';
const seen = e => typeof e.checkVisibility !== 'function' || e.checkVisibility({checkVisibilityCSS: true, checkOpacity: true});
let visits = 12000;
const chunks = (root, limit) => {
    const out = [];
    let size = 0, own = 3000;
    const walk = node => {
        if (size >= limit || visits <= 0 || own <= 0) return;
        if (node.nodeType === 3) {
            const t = node.nodeValue.replace(/\s+/g, ' ').trim();
            if (t) { out.push(t); size += t.length + 1; }
            return;
        }
        if (node.nodeType === 1) {
            visits--; own--;
            if (node.matches(SKIP)) return;
            if (node.localName === 'slot') { for (const c of node.assignedNodes({flatten: true})) walk(c); return; }
            if (!seen(node) && getComputedStyle(node).display !== 'contents') return;
            const shadow = node.shadowRoot || closed.get(node);
            for (const c of (shadow || node).childNodes) walk(c);
        } else if (node.nodeType === 9 || node.nodeType === 11) {
            for (const c of node.childNodes) walk(c);
        }
    };
    if (root) walk(root);
    return out;
};
"#;

/// The verify probe's body, run after `DOM_QUERIES` and `TEXT_JS` inside
/// `function(closedRoots, truncated)`. It answers "what covers the page, and does that hold a
/// CAPTCHA vendor's frame?" with hit tests, and gathers the page's words. Read-only: no writes,
/// no focus, no globals. Its answer is raw; `PageBrief::from_probe` decides what may be kept.
///
/// The cover test: with a submit control in view (the form that holds a CAPTCHA field first),
/// whatever takes a click at its center other than the control is on top; a reachable control
/// inside a dialog or fixed panel over other content means that panel is on top. With no such
/// control, the view's center and four quarter points are hit-tested, and the outermost
/// positioned or dialog ancestor that paints over other content is on top. A layer that is the
/// page's own base (a fixed app shell) covers nothing beneath it, so it never counts.
pub(crate) const PROBE_JS: &str = r#"
const W = innerWidth, H = innerHeight, VIEW = Math.max(1, W * H);
const up = e => e.parentElement || e.getRootNode().host || null;
const lineage = e => { const list = []; for (let p = e; p; p = up(p)) list.push(p); return list; };
const share = e => {
    const r = e.getBoundingClientRect();
    return Math.max(0, Math.min(r.right, W) - Math.max(r.left, 0)) * Math.max(0, Math.min(r.bottom, H) - Math.max(r.top, 0)) / VIEW;
};
const shown = e => !!e && e.isConnected && share(e) > 0 && seen(e);
const inside = (root, selector) => (root.matches(selector) ? [root] : []).concat(all(selector, root));
const vendorOf = raw => {
    let u;
    try { u = new URL(raw, location.href); } catch { return null; }
    const h = u.hostname, on = n => h === n || h.endsWith('.' + n);
    if ((on('google.com') || on('recaptcha.net')) && /^\/recaptcha\/(api2|enterprise)\/(anchor|bframe)$/.test(u.pathname))
        return u.pathname.endsWith('/bframe') ? {vendor: 'recaptcha', part: 'puzzle'}
            : u.searchParams.get('size') === 'invisible' ? null : {vendor: 'recaptcha', part: 'checkbox'};
    if (h === 'challenges.cloudflare.com') return {vendor: 'turnstile', part: 'checkbox'};
    if (on('hcaptcha.com')) {
        const hash = new URLSearchParams(u.hash.slice(1));
        return hash.get('frame') === 'challenge' ? {vendor: 'hcaptcha', part: 'puzzle'}
            : hash.get('size') === 'invisible' ? null : {vendor: 'hcaptcha', part: 'checkbox'};
    }
    if (on('captcha-delivery.com')) return {vendor: 'datadome', part: u.pathname.startsWith('/interstitial') ? 'checking' : 'puzzle'};
    if (on('px-cdn.net') || on('px-cloud.net') || on('perimeterx.net')) return {vendor: 'perimeterx', part: 'puzzle'};
    if (on('arkoselabs.com') || on('arkoselabs.cn') || on('funcaptcha.com')) return {vendor: 'arkose', part: 'puzzle'};
    if (on('geetest.com') || on('geevisit.com')) return {vendor: 'geetest', part: 'puzzle'};
    if (on('awswaf.com')) return {vendor: 'aws_waf', part: 'puzzle'};
    return null;
};
const MARKS = [['#px-captcha, iframe#px-captcha-modal', 'perimeterx'], ['[class*="geetest_"]', 'geetest'], ['awswaf-captcha', 'aws_waf']];
const RANK = {puzzle: 3, checkbox: 2, checking: 1};
const vendorIn = layer => {
    let best = null;
    const keep = v => { if (v && (!best || RANK[v.part] > RANK[best.part])) best = v; };
    for (const f of inside(layer, 'iframe')) if (shown(f)) keep(vendorOf(f.src));
    for (const [selector, vendor] of MARKS) if (inside(layer, selector).some(shown)) keep({vendor, part: 'puzzle'});
    return best;
};
const dialogish = e => e.matches('dialog[open], [role="dialog"], [role="alertdialog"], [aria-modal="true"]');
const position = e => getComputedStyle(e).position;
const covers = (layer, stack) => {
    const mine = new Set(lineage(layer));
    return stack.some(e => !mine.has(e) && !lineage(e).includes(layer));
};
const deepAt = (x, y) => {
    let e = document.elementFromPoint(x, y);
    for (let i = 0; e && i < 16; i++) {
        const root = e.shadowRoot || closed.get(e);
        const inner = root && root.elementFromPoint(x, y);
        if (!inner || inner === e) break;
        e = inner;
    }
    return e;
};
const base = e => e === document.body || e === document.documentElement;
const SUBMIT = 'button:not([type]), button[type="submit"], input[type="submit"], input[type="image"]';
const FIELDS = 'textarea[name="g-recaptcha-response"], input[name="cf-turnstile-response"], textarea[name="h-captcha-response"], .g-recaptcha, .cf-turnstile, .h-captcha';
const firstSubmit = form => inside(form, SUBMIT).find(shown)
    || (form.id ? [...document.querySelectorAll(SUBMIT)].find(b => b.getAttribute('form') === form.id && shown(b)) : null)
    || null;
const submit = (() => {
    const forms = [...new Set(all(FIELDS).map(e => lineage(e).find(p => p.localName === 'form')).filter(Boolean)), ...all('form')];
    for (const form of forms) { const b = firstSubmit(form); if (b) return b; }
    return null;
})();
let layer = null, overSubmit = false;
if (submit) {
    const r = submit.getBoundingClientRect(), x = r.left + r.width / 2, y = r.top + r.height / 2;
    const top = deepAt(x, y);
    if (top && !lineage(top).includes(submit)) {
        const below = new Set(lineage(submit));
        for (const p of lineage(top)) { if (below.has(p)) break; layer = p; }
        overSubmit = !!layer;
    } else {
        const stack = document.elementsFromPoint(x, y);
        layer = lineage(submit).slice(1).find(p => !base(p) && (dialogish(p) || position(p) === 'fixed')
            && share(p) >= 0.3 && covers(p, stack)) || null;
    }
} else {
    let fallback = null;
    for (const [fx, fy] of [[0.5, 0.5], [0.25, 0.25], [0.75, 0.25], [0.25, 0.75], [0.75, 0.75]]) {
        const x = W * fx, y = H * fy, top = deepAt(x, y);
        if (!top) continue;
        const stack = document.elementsFromPoint(x, y);
        let found = null;
        for (const p of lineage(top)) {
            if (base(p)) break;
            if ((dialogish(p) || ['fixed', 'absolute'].includes(position(p))) && covers(p, stack)) found = p;
        }
        if (found && vendorIn(found)) { layer = found; break; }
        if (found && (!fallback || share(found) > share(fallback))) fallback = found;
    }
    layer = layer || fallback;
    if (!layer) {
        const t = deepAt(W / 2, H / 2);
        if (t && t.localName === 'iframe' && share(t) >= 0.25 && vendorOf(t.src)) layer = t;
    }
}
const onTop = layer ? vendorIn(layer) : null;
const within = e => !!layer && lineage(e).includes(layer);
const puzzle = all('iframe').map(f => shown(f) && share(f) >= 0.01 ? vendorOf(f.src) : null)
    .find(v => v && v.part === 'puzzle' && ['recaptcha', 'hcaptcha', 'datadome'].includes(v.vendor)) || null;
const widgets = [];
for (const f of all('iframe')) {
    if (widgets.length >= 3) break;
    const v = vendorOf(f.src);
    const visible = shown(f);
    if (v && (visible || v.vendor === 'turnstile')) widgets.push({vendor: v.vendor, part: v.part, shown: visible, inLayer: within(f)});
}
const label = (e, limit) => (chunks(e, limit).join(' ') || e.getAttribute('aria-label') || e.getAttribute('title')
    || (e.matches('input[type="submit"], input[type="button"], input[type="image"]') ? e.getAttribute('value') || e.getAttribute('alt') : '')
    || '').replace(/\s+/g, ' ').trim().slice(0, limit);
const lines = (selector, max, short) => {
    const out = [];
    for (const e of all(selector)) {
        if (out.length >= max) break;
        if (!seen(e)) continue;
        const parts = chunks(e, 400), text = parts.join(' ');
        if (!text || (short && text.length > 200) || out.some(o => o.join(' ') === text)) continue;
        out.push(parts);
    }
    return out;
};
const FOOTER = 'footer, [role="contentinfo"]';
const headings = all('h1, h2, h3, h4, h5, h6, [role="heading"]')
    .filter(e => seen(e) && !lineage(e).some(p => p.matches && p.matches(FOOTER)))
    .map(e => label(e, 120)).filter(Boolean).slice(0, 8);
const buttons = all('button, [role="button"], input[type="submit"], input[type="button"]')
    .filter(shown).map(e => label(e, 60)).filter(Boolean).slice(0, 10);
const frames = [];
for (const f of all('iframe')) {
    if (frames.length >= 6) break;
    if (!shown(f)) continue;
    let host = '';
    try { host = new URL(f.src).hostname; } catch { continue; }
    if (!host || host === location.hostname) continue;
    frames.push({host, vendor: !!vendorOf(f.src), inLayer: within(f), share: share(f)});
}
const nav = performance.getEntriesByType('navigation')[0];
return {
    complete: !truncated && document.readyState !== 'loading',
    site: location.hostname,
    title: document.title,
    httpStatus: nav && nav.responseStatus > 0 ? nav.responseStatus : null,
    layer: layer && {dialog: dialogish(layer) || inside(layer, 'dialog[open], [role="dialog"], [role="alertdialog"], [aria-modal="true"]').some(seen),
        share: share(layer), text: chunks(layer, 600), vendor: onTop && onTop.vendor, part: onTop && onTop.part, overSubmit},
    puzzle: puzzle && puzzle.vendor,
    widgets,
    submit: submit && {label: label(submit, 60), disabled: !!submit.disabled || submit.getAttribute('aria-disabled') === 'true',
        covered: overSubmit, inLayer: !overSubmit && within(submit)},
    alerts: lines('[role="alert"], [role="alertdialog"], [aria-live="assertive"]', 6, false)
        .concat(lines('[class*="alert" i], [class*="error" i]', 4, true)).slice(0, 6),
    statuses: lines('[role="status"], [aria-live="polite"], output', 4, false),
    frames,
    headings,
    buttons,
    text: chunks(document.querySelector('main, [role="main"]') || document.body, 1500),
};
"#;

/// The DOM function `CaptchaPage::scan_dom` runs on the top document.
pub(crate) fn probe_script() -> String {
    format!(
        "function(closedRoots, truncated) {{ /* captcha:verify-probe */ {DOM_QUERIES} {TEXT_JS} {PROBE_JS} }}"
    )
}

/// The DOM function that reads one cross-site frame's visible text as chunks.
fn frame_text_script() -> String {
    format!(
        "function(closedRoots, truncated) {{ /* captcha:verify-frame */ {DOM_QUERIES} {TEXT_JS} return chunks(document.body, 600); }}"
    )
}

/// The probe's raw answer. Unknown or missing fields read as absent, so an older probe or an
/// odd page cannot fail the parse, only thin the brief.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Probe {
    complete: bool,
    site: Option<String>,
    title: Option<String>,
    http_status: Option<u16>,
    layer: Option<LayerSeen>,
    puzzle: Option<Vendor>,
    widgets: Vec<WidgetSeen>,
    submit: Option<SubmitSeen>,
    alerts: Vec<Vec<String>>,
    statuses: Vec<Vec<String>>,
    frames: Vec<FrameSeen>,
    headings: Vec<String>,
    buttons: Vec<String>,
    text: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LayerSeen {
    dialog: bool,
    share: f64,
    text: Vec<String>,
    vendor: Option<Vendor>,
    part: Option<FramePart>,
    over_submit: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WidgetSeen {
    vendor: Vendor,
    part: FramePart,
    #[serde(default)]
    shown: bool,
    #[serde(default)]
    in_layer: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SubmitSeen {
    label: String,
    disabled: bool,
    covered: bool,
    in_layer: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FrameSeen {
    host: String,
    vendor: bool,
    in_layer: bool,
    share: f64,
}

/// A CAPTCHA vendor as the probe names it from a frame's address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Vendor {
    Recaptcha,
    Turnstile,
    Hcaptcha,
    Datadome,
    Perimeterx,
    Arkose,
    Geetest,
    AwsWaf,
}

impl Vendor {
    /// The family the solver knows it as. DataDome has none: it is a human check Neo cannot
    /// solve, which tier 1 reports as an unsupported wall.
    fn kind(self) -> CaptchaKind {
        match self {
            Self::Recaptcha => CaptchaKind::RecaptchaV2,
            Self::Turnstile => CaptchaKind::Turnstile,
            Self::Hcaptcha => CaptchaKind::Hcaptcha,
            Self::Datadome => CaptchaKind::Unknown,
            Self::Perimeterx => CaptchaKind::PerimeterX,
            Self::Arkose => CaptchaKind::Arkose,
            Self::Geetest => CaptchaKind::Geetest,
            Self::AwsWaf => CaptchaKind::AwsWaf,
        }
    }

    /// The name a page visitor would know, for the judge's text.
    fn name(self) -> &'static str {
        match self {
            Self::Recaptcha => "Google reCAPTCHA",
            Self::Turnstile => "Cloudflare Turnstile",
            Self::Hcaptcha => "hCaptcha",
            Self::Datadome => "DataDome",
            Self::Perimeterx => "HUMAN (PerimeterX)",
            Self::Arkose => "Arkose Labs",
            Self::Geetest => "GeeTest",
            Self::AwsWaf => "AWS WAF",
        }
    }
}

impl Probe {
    fn into_brief(self) -> PageBrief {
        let where_layer = match &self.layer {
            Some(layer) if layer.dialog => "the dialog",
            _ => "the overlay",
        };
        let measured = Measured {
            complete: self.complete,
            on_top: self.layer.as_ref().and_then(|layer| {
                Some(OnTop {
                    kind: layer.vendor?.kind(),
                    part: layer.part.unwrap_or(FramePart::Checkbox),
                })
            }),
            puzzle: self.puzzle.map(Vendor::kind),
            covered: self.layer.as_ref().is_some_and(|layer| {
                layer.vendor.is_none() && (layer.over_submit || layer.share >= COVER_SHARE)
            }),
            submit_disabled: self.submit.as_ref().is_some_and(|submit| submit.disabled),
        };
        let on_top = self.layer.as_ref().map(|layer| {
            let what = if layer.dialog { "dialog" } else { "overlay" };
            let extent = if layer.share >= 0.9 {
                "the whole view"
            } else if layer.share >= 0.5 {
                "most of the view"
            } else if layer.over_submit {
                "part of the view, over the submit control"
            } else {
                "part of the view"
            };
            match safe_chunks(&layer.text, 300) {
                Some(text) => format!("{what} covering {extent}: {text}"),
                None => format!("{what} covering {extent}"),
            }
        });
        let widget = (!self.widgets.is_empty()).then(|| {
            self.widgets
                .iter()
                .take(2)
                .map(|widget| {
                    let noun = match widget.part {
                        FramePart::Checkbox => "widget",
                        FramePart::Puzzle => "puzzle",
                        FramePart::Checking => "automatic check",
                    };
                    let place = if !widget.shown {
                        "is on the page but hidden".to_owned()
                    } else if widget.in_layer {
                        format!("is visible inside {where_layer}")
                    } else {
                        "is visible on the page".to_owned()
                    };
                    format!("{} {noun} {place}", widget.vendor.name())
                })
                .collect::<Vec<_>>()
                .join("; ")
        });
        let submit = self.submit.as_ref().map(|submit| {
            let name = safe_text(&submit.label, 60).map_or_else(
                || "the form's submit button".to_owned(),
                |label| format!("the {label} button"),
            );
            if submit.covered {
                format!("{name} is covered by {where_layer}")
            } else if submit.disabled {
                format!("{name} is disabled")
            } else if submit.in_layer {
                format!("{name} is inside {where_layer}")
            } else {
                format!("{name} is visible and enabled")
            }
        });
        let lines = |groups: &[Vec<String>]| -> Vec<Line> {
            let mut out: Vec<Line> = Vec::new();
            for group in groups {
                if let Some(text) = safe_chunks(group, 300)
                    && !out.iter().any(|line| line.text == text)
                {
                    out.push(Line { text, fresh: false });
                }
            }
            out
        };
        PageBrief {
            site: self
                .site
                .map(|site| site.trim().to_ascii_lowercase())
                .filter(|site| is_host(site)),
            title: self
                .title
                .as_deref()
                .and_then(|title| safe_text(title, 200)),
            on_top,
            widget,
            submit,
            alerts: lines(&self.alerts),
            statuses: lines(&self.statuses),
            frames: Vec::new(),
            http_status: self.http_status.filter(|status| *status >= 400),
            headings: self
                .headings
                .iter()
                .filter_map(|heading| safe_text(heading, 120))
                .take(8)
                .collect(),
            buttons: self
                .buttons
                .iter()
                .filter_map(|button| safe_text(button, 60))
                .take(10)
                .collect(),
            text: safe_chunks(&self.text, TEXT_CHARS),
            measured,
        }
    }
}

/// A host name as `location.hostname` gives it: letters, digits, dots, hyphens (and an IPv6
/// literal's brackets and colons). Anything else is not a host and is left out.
fn is_host(site: &str) -> bool {
    !site.is_empty()
        && site.len() <= 253
        && site
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'))
}

/// One text, whitespace collapsed, links reduced to their host, cut to `max` characters, and
/// left out entirely when it holds an e-mail address or a long digit run. Links go first, so a
/// number in a link's path costs nothing once the path is gone.
fn safe_text(text: &str, max: usize) -> Option<String> {
    let text = scrub_links(&text.split_whitespace().collect::<Vec<_>>().join(" "));
    if text.is_empty() || private(&text) {
        return None;
    }
    Some(text.chars().take(max).collect())
}

/// Chunks of page text joined into one, each sentence holding an e-mail address or a long digit
/// run dropped on its own, so one phone number costs one sentence and not the whole dialog.
fn safe_chunks(chunks: &[String], max: usize) -> Option<String> {
    let mut kept: Vec<String> = Vec::new();
    let mut size = 0;
    for chunk in chunks {
        for sentence in sentences(chunk) {
            if size >= max {
                break;
            }
            if let Some(text) = safe_text(sentence, max) {
                size += text.chars().count() + 1;
                kept.push(text);
            }
        }
    }
    let joined: String = kept.join(" ").chars().take(max).collect();
    (!joined.is_empty()).then_some(joined)
}

/// Splits after `.`, `!` or `?` followed by a space, keeping the mark with its sentence.
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(byte, b'.' | b'!' | b'?') && bytes.get(index + 1) == Some(&b' ') {
            out.push(&text[start..=index]);
            start = index + 2;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// An e-mail address or a run of seven or more digits (spaces, dashes, dots, brackets and a
/// plus may separate them, as in phone, card and claim numbers). Never sent to the judge.
fn private(text: &str) -> bool {
    has_email(text) || has_digit_run(text)
}

fn has_email(text: &str) -> bool {
    text.match_indices('@').any(|(at, _)| {
        let local = text[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || "._%+-".contains(c));
        let domain: String = text[at + 1..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '.' | '-'))
            .collect();
        let domain = domain.trim_end_matches('.');
        local
            && domain.contains('.')
            && domain
                .rsplit('.')
                .next()
                .is_some_and(|tld| tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic))
    })
}

fn has_digit_run(text: &str) -> bool {
    let mut digits = 0;
    let mut separators = 0;
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits += 1;
            separators = 0;
            if digits >= 7 {
                return true;
            }
        } else if matches!(c, ' ' | '-' | '.' | '(' | ')' | '+') && separators < 2 {
            separators += 1;
        } else {
            digits = 0;
            separators = 0;
        }
    }
    false
}

/// Every `http(s)://…` word reduced to its host: page text can quote links with paths and
/// queries, and only hosts may leave the page.
fn scrub_links(text: &str) -> String {
    if !text.contains("://") {
        return text.to_owned();
    }
    text.split(' ')
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            if lower.starts_with("http://") || lower.starts_with("https://") {
                url::Url::parse(word)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_else(|| "[link]".to_owned())
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ny_failure_dialog() -> PageBrief {
        PageBrief {
            site: Some("ouf.osc.ny.gov".into()),
            title: Some("Search for Unclaimed Property".into()),
            on_top: Some(
                "dialog covering the whole view: Please check the box below to continue. Close"
                    .into(),
            ),
            widget: Some(
                "Cloudflare Turnstile frame is visible inside the dialog; its state could not be \
                 read; an older response token is present"
                    .into(),
            ),
            submit: Some("the Search button is hidden behind the dialog".into()),
            alerts: vec![Line {
                text: "There was an issue verifying your browser. Please try again.".into(),
                fresh: false,
            }],
            ..PageBrief::default()
        }
    }

    /// The probe's answer on NY's failure dialog (Nov 19, `03-ny-unclaimed` 2:15), as the
    /// script would return it.
    fn ny_probe() -> Value {
        json!({
            "complete": true,
            "site": "ouf.osc.ny.gov",
            "title": "",
            "httpStatus": 200,
            "layer": {"dialog": true, "share": 1.0, "overSubmit": true, "vendor": "turnstile", "part": "checkbox",
                "text": ["Please check the box below to continue.", "Close"]},
            "puzzle": null,
            "widgets": [{"vendor": "turnstile", "part": "checkbox", "shown": true, "inLayer": true}],
            "submit": {"label": "Search", "disabled": false, "covered": true, "inLayer": false},
            "alerts": [["There was an issue verifying your browser. Please try again. If this problem persists, refresh the page and submit the form again."]],
            "statuses": [],
            "frames": [{"host": "challenges.cloudflare.com", "vendor": true, "inLayer": true, "share": 0.02}],
            "headings": ["Claim Unclaimed Property", "Search for Unclaimed Property", "SEARCH", "RESULTS"],
            "buttons": ["Close", "Search"],
            "text": ["Claim Unclaimed Property", "Search for Unclaimed Property",
                "Your search returned 1000 unclaimed properties."]
        })
    }

    #[test]
    fn tests_that_evidence_puts_measured_facts_first_with_their_tags() {
        let evidence = ny_failure_dialog().to_evidence();
        let tags: Vec<&str> = evidence
            .lines()
            .filter_map(|line| line.split(']').next())
            .collect();
        assert_eq!(
            tags,
            ["[SITE", "[TITLE", "[ON_TOP", "[WIDGET", "[SUBMIT", "[ALERT"],
            "the judge trusts only the tag, and facts lead so they survive the cap"
        );
        assert!(evidence.chars().count() >= MIN_EVIDENCE_CHARS);
    }

    #[test]
    fn tests_that_evidence_is_capped_in_lines_and_characters() {
        let brief = PageBrief {
            buttons: (0..100).map(|n| format!("Button {n}")).collect(),
            headings: (0..100).map(|n| format!("Heading {n}")).collect(),
            frames: (0..100)
                .map(|n| FrameText {
                    host: format!("frame{n}.example"),
                    text: "x".repeat(300),
                })
                .collect(),
            ..ny_failure_dialog()
        };
        let evidence = brief.to_evidence();
        assert!(evidence.lines().count() <= MAX_EVIDENCE_LINES);
        assert!(evidence.chars().count() <= MAX_EVIDENCE_CHARS);
    }

    #[test]
    fn tests_that_only_lines_new_since_the_solve_began_are_fresh() {
        let before = ny_failure_dialog();
        let mut after = ny_failure_dialog();
        after.alerts.push(Line {
            text: "Verification failed".into(),
            fresh: false,
        });
        let after = after.new_since(&before);
        assert_eq!(
            after
                .alerts
                .iter()
                .map(|line| line.fresh)
                .collect::<Vec<_>>(),
            [false, true]
        );
        assert!(
            after
                .to_evidence()
                .contains("[NEW ALERT] Verification failed")
        );
    }

    #[test]
    fn tests_that_an_empty_brief_is_too_thin_to_ask_about() {
        assert!(PageBrief::default().to_evidence().chars().count() < MIN_EVIDENCE_CHARS);
        assert!(
            !PageBrief::default().measured.complete,
            "a brief nobody measured must never read as a whole page"
        );
    }

    #[test]
    fn tests_that_the_ny_probe_renders_claudefs_shape() -> Result<(), PageError> {
        let brief = PageBrief::from_probe(ny_probe())?;
        assert_eq!(
            brief.measured,
            Measured {
                complete: true,
                on_top: Some(OnTop {
                    kind: CaptchaKind::Turnstile,
                    part: FramePart::Checkbox
                }),
                puzzle: None,
                covered: false,
                submit_disabled: false,
            },
            "a vendor frame on top is a vendor fact, not a plain cover"
        );
        assert_eq!(
            brief.to_evidence(),
            "[SITE] ouf.osc.ny.gov\n\
             [ON_TOP] dialog covering the whole view: Please check the box below to continue. Close\n\
             [WIDGET] Cloudflare Turnstile widget is visible inside the dialog\n\
             [SUBMIT] the Search button is covered by the dialog\n\
             [ALERT] There was an issue verifying your browser. Please try again. If this problem persists, refresh the page and submit the form again.\n\
             [HEADING] Claim Unclaimed Property\n\
             [HEADING] Search for Unclaimed Property\n\
             [HEADING] SEARCH\n\
             [HEADING] RESULTS\n\
             [BUTTON] Close\n\
             [BUTTON] Search\n\
             [TEXT] Claim Unclaimed Property Search for Unclaimed Property Your search returned 1000 unclaimed properties."
        );
        Ok(())
    }

    #[test]
    fn tests_that_private_lines_never_reach_the_judge() -> Result<(), PageError> {
        let brief = PageBrief::from_probe(json!({
            "complete": true,
            "site": "www.example.gov",
            "title": "Results for jane.doe@example.com",
            "alerts": [["Verification failed. Call us at (555) 123-4567 for help."]],
            "headings": ["Claim 123-45-6789", "Your results"],
            "buttons": ["Search"],
            "text": ["Write to help@example.gov.", "Your search returned 1000 properties.",
                "Card 4111 1111 1111 1111 on file.", "See https://www.example.gov/claims/view?id=42&ssn=1 for more."]
        }))?;
        let evidence = brief.to_evidence();
        for leak in [
            "jane.doe", "@", "555", "4567", "6789", "4111", "/claims", "ssn", "id=42",
        ] {
            assert!(!evidence.contains(leak), "{leak} leaked into:\n{evidence}");
        }
        assert!(
            evidence.contains("[ALERT] Verification failed."),
            "one private sentence costs only itself:\n{evidence}"
        );
        assert!(evidence.contains("Your search returned 1000 properties."));
        assert!(
            evidence.contains("See www.example.gov for more."),
            "{evidence}"
        );
        assert!(evidence.contains("[HEADING] Your results"));
        assert!(!evidence.contains("[TITLE]"), "the title held an e-mail");
        let by_hand = PageBrief {
            text: Some("Reach me at a.b@c.org".into()),
            ..PageBrief::default()
        };
        assert_eq!(
            by_hand.to_evidence(),
            "",
            "rendering re-checks hand-built briefs"
        );
        Ok(())
    }

    #[test]
    fn tests_that_ordinary_numbers_and_words_stay() {
        for text in [
            "Your search returned 1000 unclaimed properties.",
            "Copyright © Florida Department of Financial Services 2019",
            "Thu, Nov 19",
            "Page 1 of 25",
            "$1,234.56 due 11/19/2026",
        ] {
            assert!(!private(text), "{text} is not private");
        }
        for text in [
            "(555) 123-4567",
            "555-123-4567",
            "SSN 123-45-6789",
            "a@b.co",
            "x+tag@mail.example.org.",
        ] {
            assert!(private(text), "{text} is private");
        }
        assert!(!has_email("Price @ 5 each"));
        assert!(!has_email("follow @neo on social"));
    }

    #[test]
    fn tests_that_a_plain_cover_and_a_puzzle_are_measured_apart() -> Result<(), PageError> {
        let cookie = PageBrief::from_probe(json!({
            "complete": true,
            "layer": {"dialog": true, "share": 1.0, "text": ["Air France uses cookies", "Reject", "Accept"]},
        }))?;
        assert!(cookie.measured.covered && cookie.measured.on_top.is_none());
        let strip = PageBrief::from_probe(json!({
            "complete": true,
            "layer": {"dialog": false, "share": 0.12, "text": ["We use cookies"]},
        }))?;
        assert!(
            !strip.measured.covered,
            "a small strip leaves the page usable"
        );
        let bframe = PageBrief::from_probe(json!({
            "complete": true,
            "layer": {"dialog": false, "share": 0.23, "overSubmit": true, "vendor": "recaptcha", "part": "puzzle", "text": []},
            "puzzle": "recaptcha",
            "submit": {"label": "Search", "covered": true},
        }))?;
        assert_eq!(
            (bframe.measured.on_top, bframe.measured.puzzle),
            (
                Some(OnTop {
                    kind: CaptchaKind::RecaptchaV2,
                    part: FramePart::Puzzle
                }),
                Some(CaptchaKind::RecaptchaV2)
            )
        );
        let datadome = PageBrief::from_probe(json!({
            "complete": true,
            "layer": {"share": 1.0, "vendor": "datadome", "part": "puzzle", "text": []},
        }))?;
        assert_eq!(
            datadome.measured.on_top.map(|on_top| on_top.kind),
            Some(CaptchaKind::Unknown),
            "DataDome has no solver: it reads as an unknown, unsupported wall"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_misshapen_probe_is_an_error_not_an_empty_page() {
        assert!(PageBrief::from_probe(Value::Null).is_err());
        assert!(PageBrief::from_probe(json!("fake-agent")).is_err());
        assert!(
            PageBrief::from_probe(json!({"site": "x.test/path?q=1"}))
                .is_ok_and(|brief| brief.site.is_none()),
            "a site with a path is not a host"
        );
    }

    #[test]
    fn tests_that_the_probe_script_is_one_read_only_function() {
        let script = probe_script();
        assert!(script.starts_with("function(closedRoots, truncated) {"));
        assert!(script.contains("/* captcha:verify-probe */"));
        for write in [
            ".value =",
            ".click(",
            ".focus(",
            "setAttribute(",
            "window.",
            "localStorage",
            "dispatchEvent(",
        ] {
            assert!(!script.contains(write), "the probe must only read: {write}");
        }
        assert!(
            !script.contains(".value"),
            "the probe never reads a field's value; a submit control's label comes from its attribute"
        );
    }

    #[test]
    fn tests_that_widget_facts_join_the_widget_line() {
        let mut brief = PageBrief::default();
        brief.note_widget("its frame shows a failure message");
        assert_eq!(
            brief.widget.as_deref(),
            Some("its frame shows a failure message")
        );
        let mut brief = ny_failure_dialog();
        brief.widget = Some("Cloudflare Turnstile widget is visible inside the dialog".into());
        brief.note_widget("its frame shows a failure message");
        assert_eq!(
            brief.widget.as_deref(),
            Some(
                "Cloudflare Turnstile widget is visible inside the dialog; its frame shows a failure message"
            )
        );
    }
}
