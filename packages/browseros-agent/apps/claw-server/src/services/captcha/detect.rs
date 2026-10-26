use std::{future::Future, time::Duration};

use serde::Deserialize;
use serde_json::Value;
use tokio::time::{Instant, timeout_at};
use url::Url;

use super::{
    page::CaptchaPage,
    types::{CaptchaKind, Challenge, ChallengeState, Detection, FrameRef, PageError, ScanOutcome},
};

const DOM_SCAN: &str = r#"
const widgets = [], roots = [], frames = [];
let complete = true;
const queryRoots = root => {
    const list = [root];
    for (const r of list) for (const e of r.querySelectorAll('*')) if (e.shadowRoot) list.push(e.shadowRoot);
    return list;
};
const scopes = queryRoots(scope);
const all = (s, root = scope) => (root === scope ? scopes : queryRoots(root)).flatMap(r => [...r.querySelectorAll(s)]);
const closest = (e, selector) => {
    for (let p = e; p; p = p.parentElement || p.getRootNode().host) if (p.matches(selector)) return p;
    return null;
};
const visible = (e, onScreen = false) => {
    if (!e || !e.isConnected) return false;
    for (let p = e; p; p = p.parentElement || p.getRootNode().host) {
        const s = getComputedStyle(p);
        if (s.display === 'none' || s.visibility === 'hidden' || s.visibility === 'collapse' || s.opacity === '0') return false;
    }
    const r = e.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && (!onScreen || (r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth));
};
const addresses = [...new Set([
    ...all('script[src]').map(e => e.src),
    ...(scope === document ? performance.getEntriesByType('resource').map(e => e.name) : [])
])].filter(raw => {
    try { return /(^|\.)(google\.com|recaptcha\.net|hcaptcha\.com|challenges\.cloudflare\.com|geetest\.com|geevisit\.com|awswaf\.com|arkoselabs\.com|arkoselabs\.cn|funcaptcha\.com)$/.test(new URL(raw).hostname); }
    catch { return false; }
});
const add = (e, kind, state) => {
    const field = ({turnstile:'input[name="cf-turnstile-response"]', hcaptcha:'textarea[name="h-captcha-response"]',
        geetest:'[name="geetest_validate"], [name="geetest_seccode"], [name="captcha_output"]',
        aws_waf:'[name="aws-waf-token"], [name="captcha_voucher"]'})[kind] || 'textarea[name="g-recaptcha-response"]';
    const response = kind === 'arkose' ? null : e && (e.matches(field) ? e : all(field, e)[0]);
    const a = name => e?.getAttribute('data-' + name) || null;
    const w = {kind, inferred: !e, state: response?.value.trim() ? 'passed' : state, invisible: a('size') === 'invisible', enterprise: false,
        sitekey: a('sitekey') || a('pkey') || a('captcha-id') || a('gt') || a('api-key') || e?.getAttribute('apiKey') || null,
        recaptchaS: a('s'), action: a('action'), cdata: a('cdata'), callback: a('callback'),
        frameUrl: e && all('iframe', e)[0]?.src || null};
    widgets.push(w); roots.push(e); return w;
};
for (const e of all('.g-recaptcha[data-sitekey], .cf-turnstile, .h-captcha')) {
    const kind = e.matches('.cf-turnstile') ? 'turnstile' : e.matches('.h-captcha') ? 'hcaptcha' : 'recaptcha_v2';
    if (visible(e) || (kind === 'recaptcha_v2' && e.dataset.size === 'invisible'))
        add(e, kind, kind === 'recaptcha_v2' ? (e.dataset.size === 'invisible' ? 'widget_only' : 'unknown') : 'active');
}
for (const [selector, kind] of [['textarea[name="g-recaptcha-response"]', 'recaptcha_v2'], ['input[name="cf-turnstile-response"]', 'turnstile']]) {
    for (const e of all(selector)) {
        if (kind === 'recaptcha_v2' && e.parentElement?.querySelector('textarea[name="h-captcha-response"]')) continue;
        if (!closest(e, '.g-recaptcha, .cf-turnstile, .h-captcha')) add(e, kind, 'unknown');
    }
}
const awsCaptcha = typeof window.gokuProps !== 'undefined' || addresses.some(raw => /(^|\.)captcha\.awswaf\.com$/.test(new URL(raw).hostname));
for (const [selector, kind] of [['[class*="geetest_"]', 'geetest'],
    ['awswaf-captcha, [id*="awswaf"], [class*="awswaf"]' + (awsCaptcha ? ', #captcha-container' : ''), 'aws_waf'],
    ['[id*="funcaptcha"], [class*="funcaptcha"], [data-pkey]', 'arkose']]) {
    const e = all(selector).find(e => visible(e));
    if (e) add(e, kind, 'active');
}
for (const e of all('iframe')) frames.push({url: e.src, visible: visible(e, /\/bframe(?:\?|$)/.test(e.src))});
const anchor = all('#recaptcha-anchor')[0];
const checked = anchor ? anchor.getAttribute('aria-checked') === 'true' : null;
const expired = !!anchor && anchor.classList.contains('recaptcha-checkbox-expired');
"#;

const MAIN_SCAN: &str = r#"
const enterprise = !!window.grecaptcha?.enterprise;
const renderUrl = all('script[src]').map(e => { try { return new URL(e.src); } catch { return null; } })
    .find(u => u && /\/recaptcha\/(api|enterprise)\.js$/.test(u.pathname) && u.searchParams.get('render') && u.searchParams.get('render') !== 'explicit');
const renderKey = renderUrl?.searchParams.get('render') || null;
const clients = window.___grecaptcha_cfg?.clients || {};
const configured = new Set();
const configs = [];
for (const client of Object.values(clients)) {
    const meta = {}, nodes = [], seen = new WeakSet(), queue = [[client, 0]];
    let count = 0;
    while (queue.length && count++ < 256) {
        const [value, depth] = queue.shift();
        if (!value || typeof value !== 'object' || seen.has(value)) continue;
        seen.add(value);
        if (value instanceof Element) { nodes.push(value); continue; }
        if (depth > 6) { complete = false; continue; }
        for (const [key, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(value))) {
            if (!('value' in descriptor)) continue;
            const v = descriptor.value;
            if (['sitekey', 'action', 's', 'size', 'callback', 'promise-callback'].includes(key)) {
                const dest = key === 's' ? 'recaptchaS' : key === 'promise-callback' ? 'callback' : key;
                if (typeof v === 'string') meta[dest] = v;
                if (dest === 'callback' && typeof v === 'function') meta.callback = '<inline>';
            }
            if (v && typeof v === 'object') queue.push([v, depth + 1]);
        }
    }
    if (queue.length) complete = false;
    const v3 = !!renderKey && (!meta.sitekey || meta.sitekey === renderKey);
    configs.push({meta, nodes, v3});
}
configs.sort((a, b) => Number(!a.nodes.length) - Number(!b.nodes.length));
for (const {meta, nodes, v3} of configs) {
    const node = nodes.find(n => visible(n));
    if (!node && nodes.length && meta.size !== 'invisible' && !v3) continue;
    let index = widgets.findIndex((w, i) => !configured.has(i) && w.kind.startsWith('recaptcha') &&
        nodes.some(n => roots[i] && (n === roots[i] || roots[i].contains(n) || n.contains(roots[i]))));
    if (index < 0 && !nodes.length) index = widgets.findIndex((w, i) => !configured.has(i) &&
        w.kind.startsWith('recaptcha') && meta.sitekey && w.sitekey === meta.sitekey);
    if (index < 0) {
        add(node || nodes[0] || null, v3 ? 'recaptcha_v3' : 'recaptcha_v2',
            (v3 || meta.size === 'invisible') ? 'widget_only' : 'unknown'); index = widgets.length - 1;
    }
    configured.add(index);
    const w = widgets[index];
    w.frameUrl ||= node && all('iframe', node)[0]?.src || null;
    for (const key of ['sitekey', 'action', 'recaptchaS', 'callback']) w[key] ||= meta[key] || null;
    w.invisible ||= meta.size === 'invisible';
    if (w.invisible && w.state === 'unknown') w.state = 'widget_only';
    w.enterprise = enterprise || !!renderUrl?.pathname.includes('enterprise');
    if (v3) { w.kind = 'recaptcha_v3'; w.state = 'widget_only'; }
}
if (!widgets.some(w => w.kind.startsWith('recaptcha')) && (renderKey || all('.grecaptcha-badge').some(e => visible(e)))) {
    const w = add(null, 'recaptcha_v3', 'widget_only'); w.sitekey = renderKey; w.enterprise = enterprise;
}
for (const w of widgets.filter(w => w.kind.startsWith('recaptcha'))) w.enterprise ||= enterprise;
if (typeof window.turnstile !== 'undefined' && !widgets.some(w => w.kind === 'turnstile')) add(null, 'turnstile', 'widget_only');
for (const [present, kind] of [[typeof window.initGeetest === 'function', 'geetest'], [typeof window.AwsWafCaptcha !== 'undefined', 'aws_waf'], [typeof window.ArkoseEnforcement !== 'undefined', 'arkose']])
    if (present && !widgets.some(w => w.kind === kind)) add(null, kind, 'widget_only');
for (const w of widgets.filter(w => w.kind === 'aws_waf')) w.sitekey ||= window.gokuProps?.key || window.apiKey || null;
if (/^Just a moment/i.test(document.title) || all('#challenge-form').length || window._cf_chl_opt || location.pathname.startsWith('/cdn-cgi/challenge-platform/'))
    add(null, 'cloudflare_challenge', 'active');
"#;

fn script(main: bool) -> String {
    format!(
        "(() => {{ /* captcha:{} */ const scope = document; {DOM_SCAN} {} return {{widgets, frames, addresses, checked, expired, complete, top: window === window.top, url: location.href}}; }})()",
        if main { "main" } else { "frame" },
        if main { MAIN_SCAN } else { "" },
    )
}

fn closed_script() -> String {
    format!(
        "function() {{ const scope = this; {DOM_SCAN} return {{widgets, frames, addresses, checked, expired, complete, top: window === window.top, url: location.href}}; }}"
    )
}

#[derive(Deserialize)]
struct Probe {
    widgets: Vec<Widget>,
    frames: Vec<Visibility>,
    checked: Option<bool>,
    expired: bool,
    complete: bool,
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default)]
    top: bool,
    url: Option<String>,
}

#[derive(Deserialize)]
struct Widget {
    #[serde(flatten)]
    challenge: Challenge,
    #[serde(rename = "frameUrl")]
    frame_url: Option<String>,
    #[serde(default)]
    inferred: bool,
}

impl Widget {
    fn new(kind: CaptchaKind) -> Self {
        Self {
            challenge: Challenge {
                kind,
                state: ChallengeState::Unknown,
                frame: None,
                challenge_frame: None,
                host_frame: None,
                sitekey: None,
                invisible: false,
                enterprise: false,
                recaptcha_s: None,
                action: None,
                cdata: None,
                callback: None,
            },
            frame_url: None,
            inferred: true,
        }
    }

    fn merge(&mut self, mut other: Self) {
        let state = if self.challenge.challenge_frame.is_some()
            || other.challenge.challenge_frame.is_some()
        {
            ChallengeState::Active
        } else {
            [self.challenge.state, other.challenge.state]
                .into_iter()
                .max_by_key(|state| match state {
                    ChallengeState::Passed => 3,
                    ChallengeState::Active => 2,
                    ChallengeState::Unknown => 1,
                    ChallengeState::WidgetOnly => 0,
                })
                .unwrap_or(ChallengeState::WidgetOnly)
        };
        if self.inferred && !other.inferred {
            std::mem::swap(self, &mut other);
        }
        let w = &mut self.challenge;
        let extra = other.challenge;
        w.state = state;
        w.frame = w.frame.take().or(extra.frame);
        w.challenge_frame = w.challenge_frame.take().or(extra.challenge_frame);
        w.sitekey = w.sitekey.take().or(extra.sitekey);
        w.recaptcha_s = w.recaptcha_s.take().or(extra.recaptcha_s);
        w.action = w.action.take().or(extra.action);
        w.cdata = w.cdata.take().or(extra.cdata);
        w.callback = w.callback.take().or(extra.callback);
        w.enterprise |= extra.enterprise;
        w.invisible |= extra.invisible;
        self.frame_url = self.frame_url.take().or(other.frame_url);
    }
}

#[derive(Deserialize)]
struct Visibility {
    url: String,
    visible: bool,
    #[serde(skip)]
    host_frame: Option<FrameRef>,
}

async fn bounded<T>(
    deadline: Instant,
    future: impl Future<Output = Result<T, PageError>>,
) -> Result<T, PageError> {
    timeout_at(deadline, future)
        .await
        .map_err(|_| PageError::Timeout)?
}

fn partial<T>(result: Result<T, PageError>, complete: &mut bool) -> Result<Option<T>, PageError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error @ (PageError::Closed | PageError::Cancelled)) => Err(error),
        Err(_) => {
            *complete = false;
            Ok(None)
        }
    }
}

fn decode(value: Value) -> Result<Probe, PageError> {
    serde_json::from_value(value).map_err(|_| PageError::Browser("invalid CAPTCHA scan".into()))
}

pub async fn detect(page: &impl CaptchaPage, budget: Duration) -> Result<Detection, PageError> {
    let deadline = Instant::now() + budget;
    let mut document = bounded(deadline, page.document()).await?;
    let mut retried = false;
    loop {
        let (mut widgets, complete) = scan(page, deadline).await?;
        let mut result = Detection {
            outcome: ScanOutcome::Unknown,
            challenges: vec![],
            complete,
            document: document.clone(),
        };
        match bounded(deadline, page.document()).await {
            Ok(after) if after != document => {
                document = after;
                if !retried && Instant::now() < deadline {
                    retried = true;
                    continue;
                }
                result.document = document;
                result.complete = false;
                return Ok(result);
            }
            other => {
                partial(other, &mut result.complete)?;
            }
        }
        result.complete &= Instant::now() < deadline;
        result.challenges = widgets.drain(..).map(|w| w.challenge).collect();
        result.outcome = if !result.challenges.is_empty() {
            ScanOutcome::Detected
        } else if result.complete {
            ScanOutcome::None
        } else {
            ScanOutcome::Unknown
        };
        return Ok(result);
    }
}

async fn scan(
    page: &impl CaptchaPage,
    deadline: Instant,
) -> Result<(Vec<Widget>, bool), PageError> {
    let mut complete = true;
    let mut widgets = Vec::new();
    let mut visibility = Vec::new();
    let mut addresses = Vec::new();
    let mut top_url = None;
    if let Some(probe) = partial(
        bounded(deadline, page.eval_main(&script(true)))
            .await
            .and_then(decode),
        &mut complete,
    )? {
        complete &= probe.complete;
        widgets = probe.widgets;
        visibility = probe.frames;
        addresses.extend(probe.addresses.into_iter().map(|url| (url, None)));
        top_url = probe.url;
    }
    let frames =
        partial(bounded(deadline, page.frames()).await, &mut complete)?.unwrap_or_default();
    let parents = partial(
        bounded(deadline, page.frame_parent_ids()).await,
        &mut complete,
    )?
    .unwrap_or_default();
    let mut probes = Vec::with_capacity(frames.len());
    for widget in &mut widgets {
        let w = &mut widget.challenge;
        if w.kind == CaptchaKind::RecaptchaV2
            && !w.invisible
            && w.state == ChallengeState::WidgetOnly
        {
            w.state = ChallengeState::Unknown;
        }
    }
    for frame in &frames {
        if Instant::now() >= deadline {
            complete = false;
            probes.push(None);
            continue;
        }
        let mut probe = partial(
            bounded(deadline, page.eval_in_frame(frame, &script(false)))
                .await
                .and_then(decode),
            &mut complete,
        )?;
        if let Some(probe) = &probe {
            complete &= probe.complete;
        }
        let host = if probe.as_ref().is_some_and(|p| p.top)
            || (frames.first() == Some(frame) && top_url.as_deref() == Some(&frame.url))
        {
            None
        } else {
            Some(frame.clone())
        };
        match bounded(
            deadline,
            page.scan_closed_shadow_roots(frame, &closed_script()),
        )
        .await
        {
            Ok(values) => {
                for value in values {
                    match decode(value) {
                        Ok(mut closed) => {
                            complete &= closed.complete;
                            append_probe(
                                &mut closed,
                                host.clone(),
                                &mut widgets,
                                &mut visibility,
                                &mut addresses,
                            );
                        }
                        Err(_) => complete = false,
                    }
                }
            }
            Err(_) => complete = false,
        }
        if let Some(probe) = probe.as_mut().filter(|p| !p.top) {
            append_probe(probe, host, &mut widgets, &mut visibility, &mut addresses);
        }
        probes.push(probe);
    }
    for (raw, host) in addresses {
        let Some((kind, url)) = frame_kind(&raw) else {
            continue;
        };
        let key = sitekey(kind, &url);
        if let Some(widget) = widgets.iter_mut().find(|w| {
            w.challenge.host_frame == host
                && (w.challenge.kind == kind
                    || (kind == CaptchaKind::RecaptchaV2
                        && w.challenge.kind == CaptchaKind::RecaptchaV3))
        }) {
            widget.challenge.sitekey = widget.challenge.sitekey.take().or(key);
        } else {
            let mut widget = Widget::new(kind);
            widget.challenge.state = ChallengeState::WidgetOnly;
            widget.challenge.host_frame = host;
            widget.challenge.sitekey = key;
            widgets.push(widget);
        }
    }
    let full_page = widgets
        .iter()
        .any(|w| w.challenge.kind == CaptchaKind::CloudflareChallenge);
    if full_page {
        widgets.retain(|w| w.challenge.kind != CaptchaKind::Turnstile);
    }
    for (frame, probe) in frames.iter().zip(&probes) {
        if probe.as_ref().is_some_and(|p| p.top)
            || (frames.first() == Some(frame) && top_url.as_deref() == Some(&frame.url))
        {
            continue;
        }
        // Page.getFrameTree omits URL fragments used to identify hCaptcha widgets.
        let frame_url = probe
            .as_ref()
            .and_then(|p| p.url.as_deref())
            .unwrap_or(&frame.url);
        let Some((kind, url)) = frame_kind(frame_url) else {
            continue;
        };
        if url.path().ends_with("/bframe") || (full_page && kind == CaptchaKind::Turnstile) {
            continue;
        }
        let invisible = param(&url, "size").as_deref() == Some("invisible");
        if !invisible
            && (frame
                .rect
                .is_some_and(|r| r.width <= 0.0 || r.height <= 0.0)
                || visibility.iter().any(|v| v.url == frame_url && !v.visible))
        {
            continue;
        }
        let sitekey = sitekey(kind, &url);
        let owner = visibility.iter().find(|v| v.url == frame_url);
        let host = owner.and_then(|v| v.host_frame.clone());
        let matches_kind = |w: &Widget| {
            w.challenge.kind == kind
                || (kind == CaptchaKind::RecaptchaV2
                    && w.challenge.kind == CaptchaKind::RecaptchaV3)
        };
        let index = widgets
            .iter()
            .position(|w| {
                matches_kind(w)
                    && w.challenge.host_frame == host
                    && w.challenge.frame.is_none()
                    && w.frame_url.as_deref() == Some(frame_url)
            })
            .or_else(|| {
                widgets.iter().position(|w| {
                    matches_kind(w)
                        && w.challenge.host_frame == host
                        && w.challenge.frame.is_none()
                        && w.frame_url.is_none()
                        && (w.challenge.sitekey.is_none() || w.challenge.sitekey == sitekey)
                })
            });
        let index = index.unwrap_or_else(|| {
            widgets.push(Widget::new(kind));
            widgets.len() - 1
        });
        let w = &mut widgets[index].challenge;
        w.frame = Some(frame.clone());
        w.host_frame = host;
        w.invisible |= invisible;
        w.enterprise |= url.path().contains("/enterprise/");
        w.sitekey = w.sitekey.take().or(sitekey);
        w.action = w
            .action
            .take()
            .or_else(|| param(&url, "sa"))
            .or_else(|| param(&url, "action"));
        w.recaptcha_s = w.recaptcha_s.take().or_else(|| param(&url, "s"));
        w.cdata = w.cdata.take().or_else(|| param(&url, "cData"));
        if w.kind == CaptchaKind::RecaptchaV3 {
            w.state = ChallengeState::WidgetOnly;
        } else if kind == CaptchaKind::RecaptchaV2 {
            if let Some(probe) = probe {
                w.state = if probe.expired {
                    ChallengeState::Active
                } else if probe.checked == Some(true) || w.state == ChallengeState::Passed {
                    ChallengeState::Passed
                } else if w.invisible {
                    ChallengeState::WidgetOnly
                } else if probe.checked == Some(false) {
                    ChallengeState::Active
                } else {
                    ChallengeState::Unknown
                };
            }
        } else if kind == CaptchaKind::Hcaptcha && w.state == ChallengeState::WidgetOnly {
            w.state = ChallengeState::Unknown;
        } else if kind == CaptchaKind::Turnstile && w.state != ChallengeState::Passed {
            w.state = ChallengeState::Active;
        } else if matches!(
            kind,
            CaptchaKind::Geetest | CaptchaKind::AwsWaf | CaptchaKind::Arkose
        ) && w.state != ChallengeState::Passed
        {
            w.state = if owner.is_some_and(|v| v.visible)
                || frame.rect.is_some_and(|r| r.width > 0.0 && r.height > 0.0)
            {
                ChallengeState::Active
            } else {
                ChallengeState::WidgetOnly
            };
        }
    }
    for frame in &frames {
        let Some((CaptchaKind::RecaptchaV2, url)) = frame_kind(&frame.url) else {
            continue;
        };
        if !url.path().ends_with("/bframe") {
            continue;
        }
        let visible = visibility
            .iter()
            .find(|v| v.url == frame.url)
            .map(|v| v.visible);
        if visible.is_none() && frame.rect.is_some_and(|r| r.width > 0.0 && r.height > 0.0) {
            complete = false;
        }
        if visible != Some(true)
            || frame.rect.is_some_and(|r| {
                r.width <= 0.0 || r.height <= 0.0 || r.x + r.width <= 0.0 || r.y + r.height <= 0.0
            })
        {
            continue;
        }
        let sitekey = param(&url, "k");
        if !widgets.iter().any(|w| {
            w.challenge.kind == CaptchaKind::RecaptchaV2
                && (sitekey.is_none() || w.challenge.sitekey == sitekey)
        }) {
            let mut widget = Widget::new(CaptchaKind::RecaptchaV2);
            widget.challenge.sitekey = sitekey.clone();
            widget.challenge.enterprise = url.path().contains("/enterprise/");
            widgets.push(widget);
        }
        let mut candidates: Vec<_> = widgets
            .iter_mut()
            .filter(|w| {
                w.challenge.kind == CaptchaKind::RecaptchaV2
                    && (sitekey.is_none() || w.challenge.sitekey == sitekey)
            })
            .collect();
        if let [widget] = candidates.as_mut_slice() {
            widget.challenge.challenge_frame = Some(frame.clone());
            widget.challenge.state = ChallengeState::Active;
        } else {
            complete = false;
        }
    }
    let top = frames
        .iter()
        .zip(&probes)
        .find(|(frame, probe)| {
            probe.as_ref().is_some_and(|p| p.top)
                || (frames.first() == Some(frame) && top_url.as_deref() == Some(&frame.url))
        })
        .map(|(frame, _)| frame.frame_id.as_str());
    let widgets = fold_vendor_widgets(widgets, &frames, &parents, top, &visibility, &mut complete);
    Ok((widgets, complete))
}

fn vendor_owner(
    source: &FrameRef,
    frames: &[FrameRef],
    parents: &std::collections::HashMap<String, String>,
    top: Option<&str>,
    complete: &mut bool,
) -> (Option<FrameRef>, Option<FrameRef>) {
    let mut current = Some(source);
    let mut outer = None;
    let mut seen = std::collections::HashSet::new();
    while let Some(frame) = current {
        if Some(frame.frame_id.as_str()) == top {
            return (None, outer);
        }
        if !seen.insert(frame.frame_id.as_str()) {
            *complete = false;
            break;
        }
        if frame_kind(&frame.url).is_none() {
            return (Some(frame.clone()), outer);
        }
        outer = Some(frame.clone());
        current = parents
            .get(&frame.frame_id)
            .and_then(|id| frames.iter().find(|f| &f.frame_id == id));
    }
    (None, outer)
}

fn fold_vendor_widgets(
    widgets: Vec<Widget>,
    frames: &[FrameRef],
    parents: &std::collections::HashMap<String, String>,
    top: Option<&str>,
    visibility: &[Visibility],
    complete: &mut bool,
) -> Vec<Widget> {
    let mut folded: Vec<Widget> = Vec::new();
    let mut inferred = Vec::new();
    for mut widget in widgets {
        let w = &mut widget.challenge;
        if let Some(source) = w
            .frame
            .as_ref()
            .or(w.challenge_frame.as_ref())
            .or(w.host_frame.as_ref())
        {
            let (host, outer) = vendor_owner(source, frames, parents, top, complete);
            w.host_frame = host;
            if let Some(outer) = outer {
                if let Some((kind, _)) = frame_kind(&outer.url)
                    && !(kind == CaptchaKind::RecaptchaV2 && w.kind == CaptchaKind::RecaptchaV3)
                {
                    w.kind = kind;
                }
                if w.state != ChallengeState::Passed
                    && w.challenge_frame.is_none()
                    && (outer
                        .rect
                        .is_some_and(|r| r.width <= 0.0 || r.height <= 0.0)
                        || visibility.iter().any(|v| v.url == outer.url && !v.visible))
                {
                    w.state = ChallengeState::WidgetOnly;
                }
                w.frame = Some(outer);
            }
        }
        if widget.inferred && w.frame.is_none() {
            inferred.push(widget);
            continue;
        }
        let existing = folded.iter_mut().find(|other| {
            let a = &other.challenge;
            a.kind == w.kind
                && a.host_frame == w.host_frame
                && w.frame.as_ref().is_some_and(|f| {
                    a.frame.as_ref().is_some_and(|o| o.frame_id == f.frame_id)
                        || a.challenge_frame
                            .as_ref()
                            .is_some_and(|o| o.frame_id == f.frame_id)
                })
        });
        if let Some(existing) = existing {
            existing.merge(widget);
        } else {
            folded.push(widget);
        }
    }
    for widget in inferred {
        let w = &widget.challenge;
        let candidates: Vec<_> = folded
            .iter_mut()
            .filter(|other| {
                let a = &other.challenge;
                a.kind == w.kind
                    && a.host_frame == w.host_frame
                    && (a.sitekey.is_none() || w.sitekey.is_none() || a.sitekey == w.sitekey)
            })
            .collect();
        if candidates.is_empty() {
            folded.push(widget);
        } else {
            for candidate in candidates {
                candidate.merge(Widget {
                    challenge: widget.challenge.clone(),
                    frame_url: widget.frame_url.clone(),
                    inferred: true,
                });
            }
        }
    }
    folded
}

fn append_probe(
    probe: &mut Probe,
    host: Option<FrameRef>,
    widgets: &mut Vec<Widget>,
    visibility: &mut Vec<Visibility>,
    addresses: &mut Vec<(String, Option<FrameRef>)>,
) {
    for mut widget in probe.widgets.drain(..) {
        widget.challenge.host_frame = host.clone();
        if let Some(existing) = widgets.iter_mut().find(|w| {
            w.challenge.kind == widget.challenge.kind
                && w.challenge.host_frame == host
                && (w.inferred
                    || widget.inferred
                    || (w.frame_url.is_some() && w.frame_url == widget.frame_url))
        }) {
            existing.merge(widget);
        } else {
            widgets.push(widget);
        }
    }
    for frame in &mut probe.frames {
        frame.host_frame = host.clone();
    }
    visibility.append(&mut probe.frames);
    addresses.extend(probe.addresses.drain(..).map(|url| (url, host.clone())));
}

fn sitekey(kind: CaptchaKind, url: &Url) -> Option<String> {
    param(url, "k")
        .or_else(|| param(url, "sitekey"))
        .or_else(|| match kind {
            CaptchaKind::Geetest => param(url, "captcha_id").or_else(|| param(url, "gt")),
            CaptchaKind::AwsWaf => param(url, "apiKey"),
            CaptchaKind::Arkose => url
                .path()
                .strip_prefix("/v2/")?
                .strip_suffix("/api.js")
                .filter(|s| !s.is_empty() && !s.contains('/'))
                .map(str::to_owned),
            _ => None,
        })
        .or_else(|| {
            url.path_segments()?
                .find(|s| s.starts_with("0x") || (s.starts_with(['1', '2', '3']) && s.len() == 24))
                .map(str::to_owned)
        })
}

fn param(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn frame_kind(raw: &str) -> Option<(CaptchaKind, Url)> {
    let url = Url::parse(raw).ok()?;
    let host = url.host_str()?;
    let domain = |name: &str| {
        host == name
            || host
                .strip_suffix(name)
                .is_some_and(|prefix| prefix.ends_with('.'))
    };
    let kind = if (domain("google.com") || domain("recaptcha.net"))
        && [
            "/recaptcha/api2/anchor",
            "/recaptcha/enterprise/anchor",
            "/recaptcha/api2/bframe",
            "/recaptcha/enterprise/bframe",
        ]
        .contains(&url.path())
    {
        CaptchaKind::RecaptchaV2
    } else if host == "challenges.cloudflare.com" {
        CaptchaKind::Turnstile
    } else if domain("hcaptcha.com") {
        CaptchaKind::Hcaptcha
    } else if domain("geetest.com") || domain("geevisit.com") {
        CaptchaKind::Geetest
    } else if domain("awswaf.com") {
        CaptchaKind::AwsWaf
    } else if domain("arkoselabs.com") || domain("arkoselabs.cn") || domain("funcaptcha.com") {
        CaptchaKind::Arkose
    } else {
        return None;
    };
    Some((kind, url))
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::services::captcha::types::{
        CaptchaKind, ChallengeState, DocumentId, FrameRef, Rect, ScanOutcome,
    };

    struct FakePage {
        main: Mutex<VecDeque<Value>>,
        frames: Vec<FrameRef>,
        frame_result: Result<Value, PageError>,
        documents: Mutex<VecDeque<DocumentId>>,
        frame_delay: Duration,
        scripts: Mutex<Vec<String>>,
        closed: Result<Vec<Value>, PageError>,
        parent_ids: std::collections::HashMap<String, String>,
        frame_results: std::collections::HashMap<String, Value>,
    }

    fn document(loader: &str) -> DocumentId {
        DocumentId {
            url: "https://fixture.test/".into(),
            loader_id: loader.into(),
        }
    }

    fn probe(widgets: Value) -> Value {
        json!({"widgets": widgets, "frames": [], "checked": null, "expired": false, "complete": true})
    }

    fn widget(kind: &str, state: &str) -> Value {
        json!({"kind": kind, "state": state, "invisible": false, "enterprise": false})
    }

    fn frame(id: &str, url: &str) -> FrameRef {
        FrameRef {
            frame_id: id.into(),
            url: url.into(),
            rect: Some(Rect {
                x: 20.0,
                y: 30.0,
                width: 300.0,
                height: 78.0,
            }),
        }
    }

    impl FakePage {
        fn new(widgets: Value) -> Self {
            Self {
                main: Mutex::new(VecDeque::from([probe(widgets)])),
                frames: vec![],
                frame_result: Ok(probe(json!([]))),
                documents: Mutex::new(VecDeque::from([document("a")])),
                frame_delay: Duration::ZERO,
                scripts: Mutex::new(vec![]),
                closed: Ok(vec![]),
                parent_ids: Default::default(),
                frame_results: Default::default(),
            }
        }

        fn recaptcha(checked: bool) -> Self {
            let mut page = Self::new(json!([widget("recaptcha_v2", "unknown")]));
            page.frames.push(frame(
                "anchor",
                "https://www.google.com/recaptcha/api2/anchor?k=key&sa=login",
            ));
            page.frame_result = Ok(
                json!({"widgets":[], "frames":[], "checked":checked, "expired":false, "complete":true}),
            );
            page
        }
    }

    impl CaptchaPage for FakePage {
        async fn eval_main(&self, expression: &str) -> Result<Value, PageError> {
            assert!(expression.contains("captcha:main"));
            self.scripts
                .lock()
                .map_err(|e| PageError::Browser(e.to_string()))?
                .push(expression.into());
            let mut values = self
                .main
                .lock()
                .map_err(|e| PageError::Browser(e.to_string()))?;
            Ok(if values.len() > 1 {
                values.pop_front()
            } else {
                values.front().cloned()
            }
            .unwrap_or(Value::Null))
        }

        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            tokio::time::sleep(self.frame_delay).await;
            Ok(self.frames.clone())
        }

        async fn frame_parent_ids(
            &self,
        ) -> Result<std::collections::HashMap<String, String>, PageError> {
            Ok(self.parent_ids.clone())
        }

        async fn eval_in_frame(
            &self,
            frame: &FrameRef,
            expression: &str,
        ) -> Result<Value, PageError> {
            assert!(expression.contains("captcha:frame"));
            self.scripts
                .lock()
                .map_err(|e| PageError::Browser(e.to_string()))?
                .push(expression.into());
            self.frame_results
                .get(&frame.frame_id)
                .cloned()
                .map_or_else(|| self.frame_result.clone(), Ok)
        }

        async fn document(&self) -> Result<DocumentId, PageError> {
            let mut values = self
                .documents
                .lock()
                .map_err(|e| PageError::Browser(e.to_string()))?;
            if values.len() > 1 {
                values.pop_front()
            } else {
                values.front().cloned()
            }
            .ok_or(PageError::Closed)
        }

        async fn scan_closed_shadow_roots(
            &self,
            _frame: &FrameRef,
            _function: &str,
        ) -> Result<Vec<Value>, PageError> {
            self.closed.clone()
        }

        async fn click_at(&self, _x: f64, _y: f64) -> Result<(), PageError> {
            panic!("detection clicked")
        }
        async fn screenshot_clip(&self, _rect: Rect) -> Result<String, PageError> {
            panic!("detection captured an image")
        }
        async fn query_rect_in_frame(
            &self,
            _frame: &FrameRef,
            _selector: &str,
        ) -> Result<Option<Rect>, PageError> {
            panic!("detection used non-scan operations")
        }
    }

    async fn scan(page: &FakePage) -> Result<Detection, PageError> {
        detect(page, Duration::from_secs(2)).await
    }

    #[tokio::test]
    async fn tests_that_closed_shadow_widgets_keep_their_host_frame() -> Result<(), PageError> {
        for kind in ["geetest", "aws_waf", "arkose"] {
            let mut page = FakePage::new(json!([]));
            let host = frame("host", "https://fixture.test/child");
            page.frames.push(host.clone());
            page.closed = Ok(vec![probe(json!([widget(kind, "active")]))]);
            let result = scan(&page).await?;
            assert!(result.complete);
            assert_eq!(result.challenges.len(), 1);
            assert_eq!(result.challenges[0].kind.as_str(), kind);
            assert_eq!(result.challenges[0].state, ChallengeState::Active);
            assert_eq!(result.challenges[0].host_frame, Some(host));
            assert!(result.challenges[0].frame.is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_closed_widgets_replace_global_only_findings() -> Result<(), PageError> {
        for state in ["active", "passed"] {
            let mut global = widget("aws_waf", "widget_only");
            global["inferred"] = json!(true);
            global["sitekey"] = json!("global-key");
            let mut page = FakePage::new(json!([global]));
            page.frames.push(frame("top", "https://fixture.test/"));
            let mut top = probe(json!([]));
            top["top"] = json!(true);
            page.frame_result = Ok(top);
            page.closed = Ok(vec![probe(json!([widget("aws_waf", state)]))]);
            let result = scan(&page).await?;
            assert_eq!(result.challenges.len(), 1);
            assert_eq!(
                serde_json::to_value(result.challenges[0].state)
                    .map_err(|e| PageError::Browser(e.to_string()))?,
                json!(state)
            );
            assert_eq!(result.challenges[0].sitekey.as_deref(), Some("global-key"));
            assert!(result.challenges[0].host_frame.is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_visible_child_widgets_keep_their_host_frame() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([]));
        let host = frame("host", "https://fixture.test/child");
        page.frames.push(host.clone());
        page.frame_result = Ok(probe(json!([widget("geetest", "active")])));
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].state, ChallengeState::Active);
        assert_eq!(result.challenges[0].host_frame, Some(host));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_failed_closed_root_scans_are_incomplete() -> Result<(), PageError> {
        for error in [
            PageError::Timeout,
            PageError::Closed,
            PageError::Browser("miss".into()),
        ] {
            let mut page = FakePage::new(json!([widget("geetest", "active")]));
            page.frames
                .push(frame("host", "https://fixture.test/child"));
            page.closed = Err(error);
            let result = scan(&page).await?;
            assert!(!result.complete);
            assert_eq!(result.challenges[0].state, ChallengeState::Active);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_loaded_vendor_addresses_are_not_blocking() -> anyhow::Result<()> {
        use crate::services::captcha::{
            CaptchaService, LiveDriver,
            config::CaptchaConfig,
            events::CaptchaEventBus,
            providers::ProviderChain,
            types::{SolveReason, SolveStatus},
        };
        use browseros_core::PageId;
        use tokio_util::sync::CancellationToken;

        for (address, kind, key) in [
            (
                "https://static.geetest.com/v4/gt4.js?captcha_id=gt-key",
                CaptchaKind::Geetest,
                Some("gt-key"),
            ),
            (
                "https://static.geevisit.com/gt.js?gt=v3-key",
                CaptchaKind::Geetest,
                Some("v3-key"),
            ),
            (
                "https://a.token.awswaf.com/challenge.js",
                CaptchaKind::AwsWaf,
                None,
            ),
            (
                "https://client-api.arkoselabs.com/v2/pk-key/api.js",
                CaptchaKind::Arkose,
                Some("pk-key"),
            ),
            (
                "https://client-api.arkoselabs.cn/v2/cn-key/api.js",
                CaptchaKind::Arkose,
                Some("cn-key"),
            ),
            (
                "https://funcaptcha.com/fc/api.js",
                CaptchaKind::Arkose,
                None,
            ),
        ] {
            let page = FakePage::new(json!([]));
            let mut loaded = probe(json!([]));
            loaded["addresses"] = json!([address]);
            *page.main.lock().map_err(|e| anyhow::anyhow!("{e}"))? = VecDeque::from([loaded]);
            let result = scan(&page).await?;
            assert_eq!(result.challenges.len(), 1, "{address}");
            assert_eq!(result.challenges[0].kind, kind);
            assert_eq!(result.challenges[0].state, ChallengeState::WidgetOnly);
            assert_eq!(result.challenges[0].sitekey.as_deref(), key);
            let service = CaptchaService {
                providers: ProviderChain::new(&CaptchaConfig::default()),
                reservations: Default::default(),
                events: CaptchaEventBus::new(),
                forced_method: None,
            };
            let outcome = service
                .solve_with(
                    &page,
                    PageId(1),
                    CancellationToken::new(),
                    false,
                    Instant::now() + Duration::from_secs(2),
                    &LiveDriver,
                )
                .await
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            assert_eq!(outcome.status, SolveStatus::AlreadyClear);
            assert_eq!(outcome.reason, SolveReason::NotBlocking);
            assert_eq!(outcome.paid_tasks, 0);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_loaded_addresses_enrich_widgets_without_duplicates() -> Result<(), PageError>
    {
        let mut page = FakePage::new(json!([]));
        let host = frame("host", "https://fixture.test/child");
        page.frames.push(host.clone());
        let mut loaded = probe(json!([widget("arkose", "active")]));
        loaded["addresses"] = json!(["https://client-api.arkoselabs.com/v2/public-key/api.js"]);
        page.frame_result = Ok(loaded);
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].state, ChallengeState::Active);
        assert_eq!(result.challenges[0].sitekey.as_deref(), Some("public-key"));
        assert_eq!(result.challenges[0].host_frame, Some(host));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_loaded_anchor_addresses_do_not_duplicate_v3_widgets()
    -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(false);
        page.frames[0].url.push_str("&size=invisible");
        let mut badge = widget("recaptcha_v3", "widget_only");
        badge["sitekey"] = json!("key");
        let mut main = probe(json!([badge]));
        main["addresses"] = json!([page.frames[0].url]);
        page.main = Mutex::new(VecDeque::from([main]));
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].kind, CaptchaKind::RecaptchaV3);
        assert_eq!(result.challenges[0].state, ChallengeState::WidgetOnly);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_visible_hcaptcha_overrides_script_only_state() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([]));
        let mut main = probe(json!([]));
        main["addresses"] = json!(["https://js.hcaptcha.com/1/api.js"]);
        page.main = Mutex::new(VecDeque::from([main]));
        page.frames.push(frame(
            "h",
            "https://newassets.hcaptcha.com/captcha/v1/test/hcaptcha.html#sitekey=key",
        ));
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].kind, CaptchaKind::Hcaptcha);
        assert_eq!(result.challenges[0].state, ChallengeState::Unknown);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_nested_vendor_findings_fold_into_the_outer_widget() -> Result<(), PageError>
    {
        for (kind, url) in [
            (
                "arkose",
                "https://client-api.arkoselabs.com/v2/enforcement.html",
            ),
            ("geetest", "https://gcaptcha4.geetest.com/load"),
            ("aws_waf", "https://a.captcha.awswaf.com/widget"),
            (
                "hcaptcha",
                "https://newassets.hcaptcha.com/captcha/v1/hcaptcha.html",
            ),
            (
                "recaptcha_v2",
                "https://www.google.com/recaptcha/api2/anchor?k=key",
            ),
            (
                "turnstile",
                "https://challenges.cloudflare.com/widget?sitekey=key",
            ),
        ] {
            for child_host in [false, true] {
                let mut page = FakePage::new(json!([]));
                let outer = frame("outer", url);
                let inner = frame("inner", &format!("{url}#inner"));
                let host = frame("host", "https://fixture.test/child");
                if child_host {
                    page.frames.push(host.clone());
                    page.parent_ids.insert("outer".into(), "host".into());
                }
                page.frames.extend([outer.clone(), inner.clone()]);
                page.parent_ids.insert("inner".into(), "outer".into());
                for f in [&outer, &inner] {
                    let mut p = probe(json!([widget(kind, "active")]));
                    p["checked"] = json!(false);
                    p["addresses"] = json!([url]);
                    page.frame_results.insert(f.frame_id.clone(), p);
                }
                let result = scan(&page).await?;
                assert_eq!(
                    result.challenges.len(),
                    1,
                    "{kind}, child host {child_host}: {:?}",
                    result.challenges
                );
                let challenge = &result.challenges[0];
                assert_eq!(challenge.kind.as_str(), kind);
                assert_eq!(challenge.state, ChallengeState::Active);
                assert_eq!(
                    challenge.frame.as_ref().map(|f| &f.frame_id),
                    Some(&outer.frame_id)
                );
                assert_eq!(challenge.host_frame, child_host.then_some(host));
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_separate_vendor_widgets_are_not_folded_together() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([]));
        for index in 0..2 {
            let outer = frame(
                &format!("outer-{index}"),
                &format!("https://client-api.arkoselabs.com/widget/{index}"),
            );
            let inner = frame(
                &format!("inner-{index}"),
                &format!("https://client-api.arkoselabs.com/game/{index}"),
            );
            page.parent_ids
                .insert(inner.frame_id.clone(), outer.frame_id.clone());
            page.frame_results.insert(
                inner.frame_id.clone(),
                probe(json!([widget("arkose", "active")])),
            );
            page.frames.extend([outer, inner]);
        }
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 2);
        assert!(result.challenges.iter().all(|c| {
            c.host_frame.is_none()
                && c.frame
                    .as_ref()
                    .is_some_and(|f| f.frame_id.starts_with("outer-"))
        }));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_vendor_demo_top_document_is_not_an_iframe() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([]));
        let top = frame("top", "https://demo.arkoselabs.com/?key=test");
        let outer = frame(
            "outer",
            "https://client-api.arkoselabs.com/v2/enforcement.html",
        );
        let inner = frame("inner", "https://client-api.arkoselabs.com/game");
        let mut main = probe(json!([]));
        main["url"] = json!(top.url);
        main["addresses"] = json!(["https://client-api.arkoselabs.com/v2/key/api.js"]);
        page.main = Mutex::new(VecDeque::from([main.clone()]));
        main["top"] = json!(true);
        page.frame_results.insert("top".into(), main);
        for f in [&outer, &inner] {
            page.frame_results.insert(
                f.frame_id.clone(),
                probe(json!([widget("arkose", "active")])),
            );
        }
        page.parent_ids.insert("outer".into(), "top".into());
        page.parent_ids.insert("inner".into(), "outer".into());
        page.frames = vec![top, outer.clone(), inner];
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].frame, Some(outer));
        assert!(result.challenges[0].host_frame.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_complete_blank_scan_returns_none() -> Result<(), PageError> {
        let result = scan(&FakePage::new(json!([]))).await?;
        assert_eq!(result.outcome, ScanOutcome::None);
        assert!(result.complete);
        assert!(result.challenges.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_unchecked_anchor_is_active() -> Result<(), PageError> {
        let result = scan(&FakePage::recaptcha(false)).await?;
        assert_eq!(result.outcome, ScanOutcome::Detected);
        assert_eq!(result.challenges.len(), 1);
        let challenge = &result.challenges[0];
        assert_eq!(challenge.state, ChallengeState::Active);
        assert_eq!(challenge.sitekey.as_deref(), Some("key"));
        assert_eq!(challenge.action.as_deref(), Some("login"));
        assert_eq!(
            challenge.frame.as_ref().map(|f| f.frame_id.as_str()),
            Some("anchor")
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_open_bframe_is_active() -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(false);
        page.frames.push(frame(
            "tiles",
            "https://www.google.com/recaptcha/api2/bframe?k=key",
        ));
        let mut main = probe(json!([widget("recaptcha_v2", "widget_only")]));
        main["frames"] = json!([{"url": page.frames[1].url, "visible": true}]);
        page.main = Mutex::new(VecDeque::from([main]));
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].state, ChallengeState::Active);
        assert_eq!(
            result.challenges[0]
                .challenge_frame
                .as_ref()
                .map(|f| f.frame_id.as_str()),
            Some("tiles")
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_open_bframe_stays_active_with_a_hidden_badge() -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(false);
        page.frames[0].url.push_str("&size=invisible");
        let tiles = frame(
            "tiles",
            "https://www.google.com/recaptcha/api2/bframe?k=key",
        );
        let mut badge = widget("recaptcha_v2", "widget_only");
        badge["invisible"] = json!(true);
        let mut main = probe(json!([badge]));
        main["frames"] = json!([
            {"url": page.frames[0].url, "visible": false},
            {"url": tiles.url, "visible": true}
        ]);
        page.main = Mutex::new(VecDeque::from([main]));
        page.frames.push(tiles.clone());
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].state, ChallengeState::Active);
        assert_eq!(result.challenges[0].challenge_frame, Some(tiles));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_hidden_test_key_bframe_is_not_open() -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(false);
        let mut tiles = frame(
            "hidden",
            "https://www.google.com/recaptcha/api2/bframe?k=key",
        );
        tiles.rect = Some(Rect {
            x: 1.0,
            y: -9982.5,
            width: 0.0,
            height: 0.0,
        });
        page.frames.push(tiles);
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert!(result.challenges[0].challenge_frame.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_checked_and_token_widgets_are_passed() -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(true);
        page.main = Mutex::new(VecDeque::from([probe(json!([widget(
            "recaptcha_v2",
            "passed"
        )]))]));
        assert_eq!(
            scan(&page).await?.challenges[0].state,
            ChallengeState::Passed
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_expired_anchors_are_active() -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(true);
        page.frame_result =
            Ok(json!({"widgets":[], "frames":[], "checked":true, "expired":true, "complete":true}));
        assert_eq!(
            scan(&page).await?.challenges[0].state,
            ChallengeState::Active
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_untriggered_invisible_widget_is_not_blocking() -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(false);
        page.frames[0].url.push_str("&size=invisible");
        assert_eq!(
            scan(&page).await?.challenges[0].state,
            ChallengeState::WidgetOnly
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_v3_badge_is_widget_only() -> Result<(), PageError> {
        let result = scan(&FakePage::new(json!([widget(
            "recaptcha_v3",
            "widget_only"
        )])))
        .await?;
        assert_eq!(result.challenges[0].kind, CaptchaKind::RecaptchaV3);
        assert_eq!(result.challenges[0].state, ChallengeState::WidgetOnly);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_closed_shadow_turnstile_is_detected_from_frames() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([]));
        page.frames.push(frame("cf", "https://challenges.cloudflare.com/cdn-cgi/challenge-platform/h/g/turnstile/if/ov2/av0/rcv/id/1x00000000000000000000AA/light/normal"));
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].kind, CaptchaKind::Turnstile);
        assert_eq!(result.challenges[0].state, ChallengeState::Active);
        assert_eq!(
            result.challenges[0].sitekey.as_deref(),
            Some("1x00000000000000000000AA")
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_closed_shadow_turnstile_token_is_passed() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([widget("turnstile", "passed")]));
        page.frames.push(frame(
            "cf",
            "https://challenges.cloudflare.com/turnstile?sitekey=key",
        ));
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].state, ChallengeState::Passed);
        assert_eq!(
            result.challenges[0]
                .frame
                .as_ref()
                .map(|f| f.frame_id.as_str()),
            Some("cf")
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_cloudflare_full_page_challenges_are_unsupported() -> Result<(), PageError> {
        let result = scan(&FakePage::new(json!([widget(
            "cloudflare_challenge",
            "active"
        )])))
        .await?;
        assert_eq!(result.challenges[0].kind, CaptchaKind::CloudflareChallenge);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_hcaptcha_frames_are_recognized() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([]));
        page.frames.push(frame(
            "h",
            "https://newassets.hcaptcha.com/captcha/v1/abc/static/hcaptcha.html#sitekey=key",
        ));
        assert_eq!(scan(&page).await?.challenges[0].kind, CaptchaKind::Hcaptcha);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_fragment_stripped_frames_keep_widget_identity() -> Result<(), PageError> {
        let base = "https://newassets.hcaptcha.com/captcha/v1/abc/static/hcaptcha.html";
        let visible_url = format!("{base}#frame=checkbox&id=one&sitekey=key");
        let hidden_url = format!("{base}#frame=challenge&id=one&sitekey=key");
        let mut info = widget("hcaptcha", "active");
        info["sitekey"] = json!("key");
        info["frameUrl"] = json!(visible_url);
        let mut main = probe(json!([info]));
        main["frames"] = json!([
            {"url":visible_url,"visible":true},
            {"url":hidden_url,"visible":false}
        ]);
        let mut page = FakePage::new(json!([]));
        page.main = Mutex::new(VecDeque::from([main]));
        page.frames.push(frame("checkbox", base));
        let mut frame_probe = probe(json!([]));
        frame_probe["url"] = json!(visible_url);
        page.frame_result = Ok(frame_probe);
        let result = scan(&page).await?;
        assert!(result.complete);
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].state, ChallengeState::Active);
        assert_eq!(
            result.challenges[0]
                .frame
                .as_ref()
                .map(|f| f.frame_id.as_str()),
            Some("checkbox")
        );
        let mut hidden_probe = probe(json!([]));
        hidden_probe["url"] = json!(hidden_url);
        page.frame_result = Ok(hidden_probe);
        page.frames[0].frame_id = "hidden".into();
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert!(result.challenges[0].frame.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_other_unsupported_provider_frames_are_recognized() -> Result<(), PageError>
    {
        for (url, kind) in [
            (
                "https://client-api.arkoselabs.com/fc/widget",
                CaptchaKind::Arkose,
            ),
            ("https://captcha.awswaf.com/widget", CaptchaKind::AwsWaf),
            ("https://gcaptcha4.geetest.com/load", CaptchaKind::Geetest),
        ] {
            let mut page = FakePage::new(json!([]));
            page.frames.push(frame("unsupported", url));
            assert_eq!(scan(&page).await?.challenges[0].kind, kind);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_frame_errors_keep_partial_findings() -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(false);
        page.frame_result = Err(PageError::Browser("frame vanished".into()));
        let result = scan(&page).await?;
        assert!(!result.complete);
        assert_eq!(result.outcome, ScanOutcome::Detected);
        assert_eq!(result.challenges[0].state, ChallengeState::Unknown);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_unreadable_unrelated_frame_prevents_none() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([]));
        page.frames
            .push(frame("other", "https://fixture.test/child"));
        page.frame_result = Err(PageError::Browser("unreadable".into()));
        let result = scan(&page).await?;
        assert!(!result.complete);
        assert_eq!(result.outcome, ScanOutcome::Unknown);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_the_total_scan_budget_is_bounded() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([]));
        page.frame_delay = Duration::from_secs(10);
        let started = tokio::time::Instant::now();
        let result = detect(&page, Duration::from_secs(2)).await?;
        assert!(!result.complete);
        assert_eq!(result.outcome, ScanOutcome::Unknown);
        assert!(started.elapsed() <= Duration::from_secs(2));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_navigation_discards_findings_and_rescans() -> Result<(), PageError> {
        let page = FakePage::new(json!([]));
        *page
            .main
            .lock()
            .map_err(|e| PageError::Browser(e.to_string()))? = VecDeque::from([
            probe(json!([widget("hcaptcha", "active")])),
            probe(json!([])),
        ]);
        *page
            .documents
            .lock()
            .map_err(|e| PageError::Browser(e.to_string()))? =
            VecDeque::from([document("a"), document("b"), document("b")]);
        let result = scan(&page).await?;
        assert_eq!(result.outcome, ScanOutcome::None);
        assert!(result.complete);
        assert_eq!(result.document, document("b"));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_repeated_navigation_is_incomplete() -> Result<(), PageError> {
        let page = FakePage::new(json!([widget("hcaptcha", "active")]));
        *page
            .documents
            .lock()
            .map_err(|e| PageError::Browser(e.to_string()))? =
            VecDeque::from([document("a"), document("b"), document("c")]);
        let result = scan(&page).await?;
        assert!(!result.complete);
        assert_eq!(result.outcome, ScanOutcome::Unknown);
        assert!(result.challenges.is_empty());
        assert_eq!(result.document, document("c"));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_bad_probe_json_never_reports_none() -> Result<(), PageError> {
        let page = FakePage::new(json!([]));
        *page
            .main
            .lock()
            .map_err(|e| PageError::Browser(e.to_string()))? = VecDeque::from([Value::Null]);
        let result = scan(&page).await?;
        assert!(!result.complete);
        assert_eq!(result.outcome, ScanOutcome::Unknown);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_v3_anchor_is_not_a_second_v2_widget() -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(false);
        let mut badge = widget("recaptcha_v3", "widget_only");
        badge["sitekey"] = json!("key");
        page.main = Mutex::new(VecDeque::from([probe(json!([badge]))]));
        page.frames[0].url.push_str("&size=invisible");
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].kind, CaptchaKind::RecaptchaV3);
        assert_eq!(result.challenges[0].state, ChallengeState::WidgetOnly);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_separate_widgets_with_the_same_key_remain_separate() -> Result<(), PageError>
    {
        let mut page = FakePage::recaptcha(false);
        page.frames.push(frame(
            "second",
            "https://www.google.com/recaptcha/api2/anchor?k=key&cb=second",
        ));
        let mut first = widget("recaptcha_v2", "passed");
        first["sitekey"] = json!("key");
        first["frameUrl"] = json!(page.frames[0].url);
        let mut second = widget("recaptcha_v2", "unknown");
        second["sitekey"] = json!("key");
        second["frameUrl"] = json!(page.frames[1].url);
        page.main = Mutex::new(VecDeque::from([probe(json!([first, second]))]));
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 2);
        assert_eq!(result.challenges[0].state, ChallengeState::Passed);
        assert_eq!(result.challenges[1].state, ChallengeState::Active);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_full_page_cloudflare_is_not_an_ordinary_turnstile() -> Result<(), PageError>
    {
        let mut page = FakePage::new(json!([
            widget("cloudflare_challenge", "active"),
            widget("turnstile", "active")
        ]));
        page.frames.push(frame(
            "cf",
            "https://challenges.cloudflare.com/turnstile?sitekey=key",
        ));
        let result = scan(&page).await?;
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].kind, CaptchaKind::CloudflareChallenge);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_lookalike_hosts_are_not_provider_frames() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([]));
        page.frames.push(frame(
            "spoof",
            "https://evilgoogle.com/recaptcha/api2/anchor?k=key",
        ));
        assert_eq!(scan(&page).await?.outcome, ScanOutcome::None);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_enterprise_frame_metadata_is_retained() -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(false);
        page.frames[0].url =
            "https://www.recaptcha.net/recaptcha/enterprise/anchor?k=key&s=proof&sa=login".into();
        let mut info = widget("recaptcha_v2", "unknown");
        info["callback"] = json!("<inline>");
        page.main = Mutex::new(VecDeque::from([probe(json!([info]))]));
        let result = scan(&page).await?;
        let challenge = &result.challenges[0];
        assert!(challenge.enterprise);
        assert_eq!(challenge.sitekey.as_deref(), Some("key"));
        assert_eq!(challenge.recaptcha_s.as_deref(), Some("proof"));
        assert_eq!(challenge.callback.as_deref(), Some("<inline>"));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_exhausted_budget_never_reports_none() -> Result<(), PageError> {
        let result = detect(&FakePage::new(json!([])), Duration::ZERO).await?;
        assert!(!result.complete);
        assert_eq!(result.outcome, ScanOutcome::Unknown);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_visible_bframe_alone_is_a_detected_challenge() -> Result<(), PageError> {
        let mut page = FakePage::new(json!([]));
        page.frames.push(frame(
            "tiles",
            "https://www.google.com/recaptcha/api2/bframe?k=key",
        ));
        let mut main = probe(json!([]));
        main["frames"] = json!([{"url":page.frames[0].url,"visible":true}]);
        page.main = Mutex::new(VecDeque::from([main]));
        let result = scan(&page).await?;
        assert_eq!(result.outcome, ScanOutcome::Detected);
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].state, ChallengeState::Active);
        assert!(result.challenges[0].challenge_frame.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_unreadable_normal_config_widget_is_not_widget_only()
    -> Result<(), PageError> {
        let mut page = FakePage::recaptcha(false);
        page.main = Mutex::new(VecDeque::from([probe(json!([widget(
            "recaptcha_v2",
            "widget_only"
        )]))]));
        page.frame_result = Err(PageError::Browser("anchor unavailable".into()));
        let result = scan(&page).await?;
        assert!(!result.complete);
        assert_eq!(result.challenges[0].state, ChallengeState::Unknown);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CAPTCHA_LIVE_CDP_PORT, CAPTCHA_LIVE_FIXTURE_PORT and an isolated dev browser"]
    async fn tests_that_live_detection_tracks_test_widgets()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::sync::Arc;

        use browseros_cdp::{CdpClient, ConnectOptions};
        use browseros_core::{Browser, pages::NewPageOptions};

        use super::super::page::LivePage;

        let cdp_port: u16 = std::env::var("CAPTCHA_LIVE_CDP_PORT")?.parse()?;
        let fixture_port: u16 = std::env::var("CAPTCHA_LIVE_FIXTURE_PORT")?.parse()?;
        let output = std::path::PathBuf::from(std::env::var("CAPTCHA_LIVE_OUTPUT_DIR")?);
        std::fs::create_dir_all(&output)?;
        let mut results = Vec::new();
        assert!((9000..=9999).contains(&cdp_port));
        assert!(![9010, 9011, 9121, 9211, 9212, 9213, 9214, 9215].contains(&cdp_port));
        let connection = Arc::new(CdpClient::connect(ConnectOptions::new(cdp_port)).await?);
        let browser = Browser::new(connection.clone());
        for (route, expected) in [
            ("no-captcha.html", None),
            (
                "recaptcha-v2.html",
                Some((CaptchaKind::RecaptchaV2, ChallengeState::Active)),
            ),
            (
                "turnstile-interactive.html",
                Some((CaptchaKind::Turnstile, ChallengeState::Active)),
            ),
            (
                "turnstile-pass.html",
                Some((CaptchaKind::Turnstile, ChallengeState::Passed)),
            ),
            (
                "hcaptcha.html",
                Some((CaptchaKind::Hcaptcha, ChallengeState::Active)),
            ),
        ] {
            let url = format!("http://127.0.0.1:{fixture_port}/{route}");
            let id = browser.new_page(&url, NewPageOptions::default()).await?;
            let page = LivePage::new(browser.view().await, id.clone());
            let ready = format!(
                "location.href === {} && document.readyState === 'complete' && {}",
                serde_json::to_string(&url)?,
                if expected.is_some() {
                    "window.__captchaFixture?.rendered === true"
                } else {
                    "true"
                }
            );
            tokio::time::timeout(Duration::from_secs(45), async {
                while page.eval_main(&ready).await.ok() != Some(json!(true)) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            })
            .await?;
            let deadline = Instant::now() + Duration::from_secs(30);
            let before = loop {
                let result = detect(&page, Duration::from_secs(2)).await?;
                let matches = match expected {
                    None => result.outcome == ScanOutcome::None && result.challenges.is_empty(),
                    Some((kind, state)) => {
                        result.challenges.len() == 1
                            && result.challenges[0].kind == kind
                            && result.challenges[0].state == state
                            && result.challenges[0].frame.is_some()
                    }
                };
                if result.complete && matches {
                    break result;
                }
                assert!(Instant::now() < deadline, "{route}: {result:?}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            results.push(json!({"fixture":route,"before":before}));
            if route == "recaptcha-v2.html" {
                assert!(before.challenges[0].challenge_frame.is_none());
                let frame = before.challenges[0].frame.as_ref().ok_or("no anchor")?;
                let rect = page
                    .query_rect_in_frame(frame, "#recaptcha-anchor")
                    .await?
                    .ok_or("no checkbox")?;
                let (x, y) = rect.center();
                page.click_at(x, y).await?;
                let deadline = Instant::now() + Duration::from_secs(20);
                loop {
                    let after = detect(&page, Duration::from_secs(2)).await?;
                    if after.complete
                        && after.challenges.len() == 1
                        && after.challenges[0].state == ChallengeState::Passed
                    {
                        assert_eq!(before.document, after.document);
                        results.push(json!({"fixture":route,"after":after}));
                        break;
                    }
                    assert!(Instant::now() < deadline, "{route}: {after:?}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            browser.close_page(id).await?;
            std::fs::write(
                output.join("live-adapter-results.json"),
                serde_json::to_vec_pretty(&results)?,
            )?;
        }
        connection.disconnect().await;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires an owned dev browser and CAPTCHA_LIVE_CASES_FILE"]
    async fn tests_that_live_detection_matches_cases_within_budget() -> anyhow::Result<()> {
        use super::super::page::LivePage;
        use browseros_cdp::{CdpClient, ConnectOptions};
        use browseros_core::{Browser, pages::NewPageOptions};
        use std::sync::Arc;

        let port: u16 = std::env::var("CAPTCHA_LIVE_CDP_PORT")?.parse()?;
        assert!(![9010, 9011, 9121, 9211, 9212, 9213, 9214, 9215].contains(&port));
        let cases: Vec<Value> =
            serde_json::from_slice(&std::fs::read(std::env::var("CAPTCHA_LIVE_CASES_FILE")?)?)?;
        let connection = Arc::new(CdpClient::connect(ConnectOptions::new(port)).await?);
        let browser = Browser::new(connection.clone());
        let mut results = Vec::new();
        for case in cases {
            let url = case["url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing URL"))?;
            let id = browser.new_page(url, NewPageOptions::default()).await?;
            let page = LivePage::new(browser.view().await, id.clone());
            let ready = case["ready"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing readiness probe"))?;
            let deadline = Instant::now() + Duration::from_secs(30);
            while page.eval_main(ready).await.ok() != Some(json!(true)) {
                anyhow::ensure!(Instant::now() < deadline, "{} never ready", case["id"]);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            for _ in 0..case["samples"].as_u64().unwrap_or(1) {
                let start = Instant::now();
                let result = detect(&page, Duration::from_secs(2)).await?;
                let elapsed_ms = start.elapsed().as_millis();
                results.push(json!({"id":case["id"],"scanMs":elapsed_ms,"detection":result}));
                std::fs::write(
                    std::env::var("CAPTCHA_LIVE_RESULTS_FILE")?,
                    serde_json::to_vec_pretty(&results)?,
                )?;
                anyhow::ensure!(
                    result.complete && elapsed_ms < 2000,
                    "{} incomplete or over budget",
                    case["id"]
                );
                if let Some(kind) = case["kind"].as_str() {
                    let challenge = result
                        .challenges
                        .iter()
                        .find(|c| c.kind.as_str() == kind)
                        .ok_or_else(|| anyhow::anyhow!("{} missing {kind}", case["id"]))?;
                    assert_eq!(
                        serde_json::to_value(challenge.state)?,
                        case["state"],
                        "{}",
                        case["id"]
                    );
                    assert_eq!(
                        challenge.host_frame.is_some(),
                        case["child"] == true,
                        "{}",
                        case["id"]
                    );
                }
            }
            browser.close_page(id).await?;
        }
        connection.disconnect().await;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires an owned dev browser at CAPTCHA_LIVE_CDP_PORT"]
    async fn tests_that_live_arkose_session_tokens_remain_active() -> anyhow::Result<()> {
        use super::super::page::LivePage;
        use browseros_cdp::{CdpClient, ConnectOptions};
        use browseros_core::{Browser, pages::NewPageOptions};
        let port: u16 = std::env::var("CAPTCHA_LIVE_CDP_PORT")?.parse()?;
        assert!(![9010, 9011, 9121, 9211, 9212, 9213, 9214, 9215].contains(&port));
        let connection = std::sync::Arc::new(CdpClient::connect(ConnectOptions::new(port)).await?);
        let browser = Browser::new(connection.clone());
        let id = browser
            .new_page("about:blank", NewPageOptions::default())
            .await?;
        let page = LivePage::new(browser.view().await, id.clone());
        page.eval_main(r#"document.body.innerHTML = '<div data-pkey="test-key" style="width:300px;height:100px">FunCaptcha<input id="FunCaptcha-Token" name="fc-token" value="session-token|pk=test-key|surl=https://funcaptcha.com"></div>'; null"#).await?;
        let result = detect(&page, Duration::from_secs(2)).await?;
        assert!(result.complete);
        assert_eq!(result.challenges.len(), 1);
        assert_eq!(result.challenges[0].kind, CaptchaKind::Arkose);
        assert_eq!(result.challenges[0].state, ChallengeState::Active);
        browser.close_page(id).await?;
        connection.disconnect().await;
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_scan_snippets_do_not_assign_page_globals_or_ids() -> Result<(), PageError> {
        let page = FakePage::recaptcha(false);
        scan(&page).await?;
        for script in page
            .scripts
            .lock()
            .map_err(|e| PageError::Browser(e.to_string()))?
            .iter()
        {
            assert!(!script.contains(".id ="));
            assert!(!script.contains(".id="));
            for part in script.split("window.").skip(1) {
                let name = part
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or("");
                let tail = part[name.len()..].trim_start();
                assert!(!tail.starts_with('=') || tail.starts_with("=="));
            }
        }
        Ok(())
    }

    async fn live_dom_detection(html: &str, setup: &str) -> anyhow::Result<Detection> {
        use super::super::page::LivePage;
        use browseros_cdp::{CdpClient, ConnectOptions};
        use browseros_core::{Browser, pages::NewPageOptions};

        let port: u16 = std::env::var("CAPTCHA_LIVE_CDP_PORT")?.parse()?;
        assert!(![9010, 9011, 9121, 9211, 9212, 9213, 9214, 9215].contains(&port));
        let connection = std::sync::Arc::new(CdpClient::connect(ConnectOptions::new(port)).await?);
        let browser = Browser::new(connection.clone());
        let id = browser
            .new_page("about:blank", NewPageOptions::default())
            .await?;
        let page = LivePage::new(browser.view().await, id.clone());
        page.eval_main(&format!(
            "document.body.innerHTML = {}; {setup}; null",
            serde_json::to_string(html)?
        ))
        .await?;
        let result = detect(&page, Duration::from_secs(2)).await;
        browser.close_page(id).await?;
        connection.disconnect().await;
        Ok(result?)
    }

    #[tokio::test]
    #[ignore = "requires an owned dev browser at CAPTCHA_LIVE_CDP_PORT"]
    async fn tests_that_live_response_tokens_belong_to_the_selected_widget() -> anyhow::Result<()> {
        for (attribute, field, kind) in [
            (
                "class=\"geetest_widget\"",
                "captcha_output",
                CaptchaKind::Geetest,
            ),
            (
                "id=\"awswaf-widget\"",
                "captcha_voucher",
                CaptchaKind::AwsWaf,
            ),
        ] {
            for inside in [false, true] {
                let token = format!("<input name=\"{field}\" value=\"old-answer\">");
                let html = format!(
                    "<div {attribute} style=\"width:300px;height:100px\">Widget{}</div>{}",
                    if inside { &token } else { "" },
                    if inside { "" } else { &token }
                );
                let result = live_dom_detection(&html, "").await?;
                assert!(result.complete);
                assert_eq!(result.challenges.len(), 1);
                assert_eq!(result.challenges[0].kind, kind);
                assert_eq!(
                    result.challenges[0].state,
                    if inside {
                        ChallengeState::Passed
                    } else {
                        ChallengeState::Active
                    },
                    "{kind:?}, inside={inside}"
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires an owned dev browser at CAPTCHA_LIVE_CDP_PORT"]
    async fn tests_that_live_response_fields_in_shadows_do_not_duplicate_widgets()
    -> anyhow::Result<()> {
        for mode in ["open", "closed"] {
            let html = r#"<div class="g-recaptcha" data-sitekey="test" data-size="invisible" style="width:300px;height:100px"><x-response></x-response></div>"#;
            let setup = format!(
                "document.querySelector('x-response').attachShadow({{mode:'{mode}'}}).innerHTML = '<textarea name=\"g-recaptcha-response\"></textarea>'"
            );
            let result = live_dom_detection(html, &setup).await?;
            assert!(result.complete);
            assert_eq!(result.challenges.len(), 1, "{mode}");
            assert_eq!(result.challenges[0].kind, CaptchaKind::RecaptchaV2);
            assert_eq!(result.challenges[0].state, ChallengeState::WidgetOnly);
        }
        Ok(())
    }
}
