use std::{future::Future, time::Duration};

use serde::Deserialize;
use serde_json::Value;
use tokio::time::{Instant, timeout_at};
use url::Url;

use super::{
    page::CaptchaPage,
    types::{CaptchaKind, Challenge, ChallengeState, Detection, PageError, ScanOutcome},
};

const DOM_SCAN: &str = r#"
const widgets = [], roots = [], frames = [];
let complete = true;
const all = (s, root = document) => [...root.querySelectorAll(s)];
const visible = (e, onScreen = false) => {
    if (!e || !e.isConnected) return false;
    for (let p = e; p; p = p.parentElement) {
        const s = getComputedStyle(p);
        if (s.display === 'none' || s.visibility === 'hidden' || s.visibility === 'collapse' || s.opacity === '0') return false;
    }
    const r = e.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && (!onScreen || (r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth));
};
const add = (e, kind, state) => {
    const field = kind === 'turnstile' ? 'input[name="cf-turnstile-response"]' : 'textarea[name="g-recaptcha-response"]';
    const response = e && (e.matches(field) ? e : e.querySelector(field));
    const a = name => e?.getAttribute('data-' + name) || null;
    const w = {kind, state: response?.value.trim() ? 'passed' : state, invisible: a('size') === 'invisible', enterprise: false,
        sitekey: a('sitekey'), recaptchaS: a('s'), action: a('action'), cdata: a('cdata'), callback: a('callback'),
        frameUrl: e?.querySelector('iframe')?.src || null};
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
        if (!e.closest('.g-recaptcha, .cf-turnstile, .h-captcha')) add(e, kind, 'unknown');
    }
}
for (const [selector, kind] of [['[class*="geetest_"]', 'geetest'], ['[id*="awswaf"], [class*="awswaf"]', 'aws_waf'], ['[id*="funcaptcha"], [class*="funcaptcha"]', 'arkose']]) {
    const e = all(selector).find(e => visible(e));
    if (e) add(e, kind, 'unknown');
}
for (const e of all('iframe')) frames.push({url: e.src, visible: visible(e, /\/bframe(?:\?|$)/.test(e.src))});
const anchor = document.querySelector('#recaptcha-anchor');
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
    w.frameUrl ||= node?.querySelector('iframe')?.src || null;
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
    if (present && !widgets.some(w => w.kind === kind)) add(null, kind, 'unknown');
if (/^Just a moment/i.test(document.title) || document.querySelector('#challenge-form') || window._cf_chl_opt || location.pathname.startsWith('/cdn-cgi/challenge-platform/'))
    add(null, 'cloudflare_challenge', 'active');
"#;

fn script(main: bool) -> String {
    format!(
        "(() => {{ /* captcha:{} */ {DOM_SCAN} {} return {{widgets, frames, checked, expired, complete, top: window === window.top, url: location.href}}; }})()",
        if main { "main" } else { "frame" },
        if main { MAIN_SCAN } else { "" },
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
    top: bool,
    url: Option<String>,
}

#[derive(Deserialize)]
struct Widget {
    #[serde(flatten)]
    challenge: Challenge,
    #[serde(rename = "frameUrl")]
    frame_url: Option<String>,
}

impl Widget {
    fn new(kind: CaptchaKind) -> Self {
        Self {
            challenge: Challenge {
                kind,
                state: ChallengeState::Unknown,
                frame: None,
                challenge_frame: None,
                sitekey: None,
                invisible: false,
                enterprise: false,
                recaptcha_s: None,
                action: None,
                cdata: None,
                callback: None,
            },
            frame_url: None,
        }
    }
}

#[derive(Deserialize)]
struct Visibility {
    url: String,
    visible: bool,
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
    if let Some(probe) = partial(
        bounded(deadline, page.eval_main(&script(true)))
            .await
            .and_then(decode),
        &mut complete,
    )? {
        complete &= probe.complete;
        widgets = probe.widgets;
        visibility = probe.frames;
    }
    let frames =
        partial(bounded(deadline, page.frames()).await, &mut complete)?.unwrap_or_default();
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
        let probe = partial(
            bounded(deadline, page.eval_in_frame(frame, &script(false)))
                .await
                .and_then(decode),
            &mut complete,
        )?;
        if let Some(probe) = &probe {
            complete &= probe.complete;
        }
        probes.push(probe);
    }
    for probe in probes.iter_mut().flatten().filter(|p| !p.top) {
        visibility.append(&mut probe.frames);
        widgets.append(&mut probe.widgets);
    }
    let full_page = widgets
        .iter()
        .any(|w| w.challenge.kind == CaptchaKind::CloudflareChallenge);
    if full_page {
        widgets.retain(|w| w.challenge.kind != CaptchaKind::Turnstile);
    }
    for (frame, probe) in frames.iter().zip(&probes) {
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
        let sitekey = param(&url, "k")
            .or_else(|| param(&url, "sitekey"))
            .or_else(|| {
                url.path_segments()?
                    .find(|s| {
                        s.starts_with("0x") || (s.starts_with(['1', '2', '3']) && s.len() == 24)
                    })
                    .map(str::to_owned)
            });
        let matches_kind = |w: &Widget| {
            w.challenge.kind == kind
                || (kind == CaptchaKind::RecaptchaV2
                    && w.challenge.kind == CaptchaKind::RecaptchaV3)
        };
        let index = widgets
            .iter()
            .position(|w| {
                matches_kind(w)
                    && w.challenge.frame.is_none()
                    && w.frame_url.as_deref() == Some(frame_url)
            })
            .or_else(|| {
                widgets.iter().position(|w| {
                    matches_kind(w)
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
        } else if kind == CaptchaKind::Turnstile && w.state != ChallengeState::Passed {
            w.state = ChallengeState::Active;
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
    Ok((widgets, complete))
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
    } else if domain("geetest.com") {
        CaptchaKind::Geetest
    } else if domain("awswaf.com") {
        CaptchaKind::AwsWaf
    } else if domain("arkoselabs.com") || domain("funcaptcha.com") {
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

        async fn eval_in_frame(
            &self,
            _frame: &FrameRef,
            expression: &str,
        ) -> Result<Value, PageError> {
            assert!(expression.contains("captcha:frame"));
            self.scripts
                .lock()
                .map_err(|e| PageError::Browser(e.to_string()))?
                .push(expression.into());
            self.frame_result.clone()
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
}
