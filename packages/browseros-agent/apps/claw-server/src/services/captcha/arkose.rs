use std::future::Future;

use serde_json::{Value, json};
use tokio::time::{Duration, Instant, sleep_until};

use super::{
    SolveCtx,
    page::CaptchaPage,
    providers::{CaptchaBackend, optional},
    types::{
        ArkoseParams, CaptchaEventName, CaptchaKind, Challenge, FlowResult, PageError,
        ProviderError, ProviderErrorKind, SolveAnswer, SolveMethod, SolveReason, SolveStatus,
        SolveStep, SolveTask,
    },
};

pub const SUPPORTED: bool = true;
pub(super) const CAPSOLVER_ESTIMATE_USD: f64 = 0.0;
const VERIFY: Duration = Duration::from_secs(6);
const IDENTITY: [&str; 6] = [
    "url",
    "publicKey",
    "blob",
    "instance",
    "session",
    "frameSrc",
];

const WIDGET: &str = r#"
const all = selector => {
    const roots = [document], result = [];
    for (let i = 0; i < roots.length; i++) {
        result.push(...roots[i].querySelectorAll(selector));
        for (const el of roots[i].querySelectorAll('*')) if (el.shadowRoot) roots.push(el.shadowRoot);
    }
    return result;
};
const vendor = src => {
    try { return /(^|\.)(arkoselabs\.(com|cn)|funcaptcha\.com)$/.test(new URL(src, location.href).hostname); }
    catch { return false; }
};
const visible = el => !!el?.isConnected && el.getBoundingClientRect().width > 0
    && el.getBoundingClientRect().height > 0 && el.checkVisibility({checkVisibilityCSS:true});
const apis = Object.keys(window).filter(k => /^arkoseLabsClientApi/.test(k))
    .map(k => window[k]).filter(api => api?.state);
const wanted = args.publicKey || args.sitekey;
const api = apis.find(api => api.state.publicKey === wanted) || (apis.length === 1 ? apis[0] : null);
const st = api?.state, config = st?.config || {};
const scripts = all('script[src]').filter(s => vendor(s.src));
const script = scripts.find(s => wanted && s.src.includes(wanted)) || (scripts.length === 1 ? scripts[0] : null);
const widgets = all('[data-pkey]');
const widget = widgets.find(el => el.getAttribute('data-pkey') === wanted) || (widgets.length === 1 ? widgets[0] : null);
const frames = all('iframe[src]').filter(el => vendor(el.src));
const frame = st?.iframe?.tagName === 'IFRAME' ? st.iframe
    : frames.find(el => wanted && el.src.includes(wanted)) || (frames.length === 1 ? frames[0] : null);
const inputs = all('input[name="fc-token"], input[name="verification-token"], input#fc-token, input#verification-token');
const legacy = inputs.find(el => wanted && el.value.includes(wanted)) || (inputs.length === 1 ? inputs[0] : null);
const publicKey = st?.publicKey || widget?.getAttribute('data-pkey')
    || script?.src.match(/\/v2\/([\da-f-]{36})\/api\.js/i)?.[1]
    || frame?.src.match(/(?:[?#&](?:pk|pkey|public_key)=?|#)([\da-f-]{36})/i)?.[1]
    || legacy?.value.match(/(?:^|\|)pk=([^|]+)/)?.[1] || null;
let gate = widget;
try { gate ||= config.selector ? all(config.selector)[0] : null; } catch {}
gate ||= frame;
const snapshot = () => ({url:location.href, publicKey, blob:config.data?.blob || null,
    apiHost:script ? new URL(script.src).host : (frame ? new URL(frame.src).host : null),
    instance:st?.id || null, session:st?.token || legacy?.value || null, frameSrc:frame?.src || null,
    loaded:!!frame?.isConnected && !!(st ? st.token && st.enforcementSetup : legacy?.value),
    failed:!!(st?.error || st?.failed), completed:st?.completed === true});
"#;
const READ: &str = "/* arkose:read */ return snapshot();";
const APPLY: &str = r#"
/* arkose:apply */
const current = snapshot();
if (!current.loaded || current.failed || ['url','publicKey','blob','instance','session','frameSrc'].some(k => current[k] !== args[k]))
    return {written:false};
// Keep the actual element across verification, including widgets in shadow roots.
const mark = 'data-browseros-arkose-' + crypto.randomUUID();
const wasVisible = visible(gate);
if (gate) gate.setAttribute(mark, '');
const beforeFrames = frames.filter(visible).length;
let written = false, callbackRan = false, callbackDeclared = !!api;
try {
    if (api && typeof api.onComplete === 'function') {
        await api.onComplete({id:st.id, token:args.token});
        written = st.completed === true && st.token === args.token;
        callbackRan = written;
    } else if (api && typeof st?.events?.onCompleted === 'function') {
        st.token = args.token;
        st.completed = true;
        await st.events.onCompleted({token:args.token});
        written = true;
        callbackRan = true;
    } else {
        const name = widget?.getAttribute('data-callback');
        callbackDeclared = !!name;
        if (name) {
            const path = name.split('.'); let owner = window;
            for (const part of path.slice(0,-1)) owner = owner?.[part];
            const fn = owner?.[path.at(-1)];
            if (typeof fn === 'function') { await fn.call(owner, args.token); callbackRan = true; }
        }
    }
    for (const input of inputs) {
        if (input !== legacy && (widgets.length > 1 || apis.length > 1)) continue;
        input.value = args.token;
        input.dispatchEvent(new Event('input', {bubbles:true}));
        input.dispatchEvent(new Event('change', {bubbles:true}));
        written = true;
    }
} catch { callbackDeclared = true; callbackRan = false; }
return {written, callbackRan, callbackDeclared, wasVisible, mark, beforeFrames};
"#;
const CHECK: &str = r#"
/* arkose:verify */
const marked = args.mark ? all('[' + args.mark + ']')[0] : null;
const remaining = all('[data-pkey], [id*="funcaptcha"], iframe[src]')
    .filter(el => !el.matches('iframe') || vendor(el.src)).some(visible);
return {transition:args.wasVisible === true && !visible(marked) && !remaining,
    retained:(st?.completed === true && st?.token === args.token)
        || inputs.some(input => input.value === args.token),
    ready:document.readyState === 'complete',
    captchaFree:!remaining && !all('.g-recaptcha, .h-captcha, .cf-turnstile, awswaf-captcha, [class*="geetest_"]').some(visible)
        && !all('iframe[src]').some(el => /recaptcha|hcaptcha|challenges\.cloudflare\.com/.test(el.src))};
"#;

fn script(body: &str, args: &Value) -> String {
    format!("(async () => {{ const args = {args}; {WIDGET} {body} }})()")
}
fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}
fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}
fn valid_key(key: &str) -> bool {
    key.len() == 36 && uuid::Uuid::try_parse(key).is_ok_and(|key| !key.is_nil())
}
fn valid_url(url: &str) -> bool {
    url::Url::parse(url)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

type Terminal = (SolveStatus, SolveReason, String);

pub async fn solve<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
) -> FlowResult {
    let mut result = super::terminal(
        SolveStatus::Failed,
        SolveReason::BrowserError,
        "Arkose could not be verified.",
    );
    let terminal = run(ctx, challenge, &mut result)
        .await
        .unwrap_or_else(|error| {
            let (status, reason) = match error {
                PageError::Closed => (SolveStatus::Cancelled, SolveReason::TabClosed),
                PageError::Navigated => (SolveStatus::Cancelled, SolveReason::Navigated),
                PageError::Cancelled => (SolveStatus::Cancelled, SolveReason::CallerCancelled),
                PageError::Timeout => (SolveStatus::Failed, SolveReason::DeadlineExceeded),
                PageError::Browser(_) => (SolveStatus::Failed, SolveReason::BrowserError),
            };
            (
                status,
                reason,
                "Arkose stopped; take a fresh snapshot before continuing.".into(),
            )
        });
    (result.status, result.reason, result.message) = terminal;
    result
}

async fn bounded<T, P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    future: impl Future<Output = Result<T, PageError>>,
) -> Result<T, PageError> {
    tokio::select! {
        biased;
        _ = ctx.cancel.cancelled() => Err(PageError::Cancelled),
        _ = sleep_until(ctx.deadline) => Err(PageError::Timeout),
        value = future => value,
    }
}
async fn guard<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
) -> Result<(), PageError> {
    if bounded(ctx, ctx.page.document()).await? != ctx.document {
        return Err(PageError::Navigated);
    }
    Ok(())
}
async fn evaluate<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
    body: &str,
    args: &Value,
) -> Result<Value, PageError> {
    let expression = script(body, args);
    bounded(ctx, async {
        match &challenge.host_frame {
            Some(frame) => ctx.page.eval_in_frame_main(frame, &expression).await,
            None => ctx.page.eval_main(&expression).await,
        }
    })
    .await
}
async fn read<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
) -> Result<Value, PageError> {
    guard(ctx).await?;
    let value = evaluate(ctx, challenge, READ, &json!({"sitekey":challenge.sitekey})).await?;
    guard(ctx).await?;
    Ok(value)
}
fn unconfirmed() -> Terminal {
    (
        SolveStatus::Failed,
        SolveReason::AcceptanceUnconfirmed,
        "Arkose acceptance was not confirmed; take a fresh snapshot.".into(),
    )
}

async fn run<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
    result: &mut FlowResult,
) -> Result<Terminal, PageError> {
    if challenge.kind != CaptchaKind::Arkose {
        return Ok((
            SolveStatus::Unsupported,
            SolveReason::UnsupportedType,
            "This flow handles only Arkose CAPTCHA.".into(),
        ));
    }
    let state = read(ctx, challenge).await?;
    if flag(&state, "completed") {
        return Ok((
            SolveStatus::AlreadyClear,
            SolveReason::AlreadyPassed,
            "Arkose already reports completion.".into(),
        ));
    }
    let params = ArkoseParams {
        website_url: string(&state, "url").unwrap_or_default(),
        public_key: string(&state, "publicKey").unwrap_or_default(),
        api_subdomain: string(&state, "apiHost"),
        data_blob: string(&state, "blob"),
        user_agent: ctx.user_agent.clone(),
    };
    if !flag(&state, "loaded")
        || flag(&state, "failed")
        || !valid_key(&params.public_key)
        || !valid_url(&params.website_url)
    {
        return Ok((
            SolveStatus::NeedsHuman,
            SolveReason::ChallengeNotPassed,
            "Arkose has no usable, loaded challenge; ask for human help.".into(),
        ));
    }
    if !ctx.backend.is_configured() {
        return Ok((
            SolveStatus::NeedsHuman,
            SolveReason::NoProvider,
            "No CAPTCHA provider is configured; ask for human help.".into(),
        ));
    }
    guard(ctx).await?;
    result.method = SolveMethod::Token;
    result.attempts.token += 1;
    ctx.lease
        .set_progress(Some(CaptchaKind::Arkose), SolveStep::Token);
    ctx.events.emit(
        CaptchaEventName::Solving,
        json!({"type":"arkose", "method":"token", "attempt":result.attempts.token}),
    );
    result.paid_tasks = 1;
    let answer = bounded(ctx, async {
        Ok(ctx
            .backend
            .solve(&SolveTask::Arkose(params), ctx.deadline, ctx.cancel)
            .await)
    })
    .await?;
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            result.paid_tasks = u32::from(error.charged);
            return Ok(provider_failure(error));
        }
    };
    result.cost_usd = answer
        .cost_usd
        .filter(|cost| cost.is_finite() && *cost >= 0.0)
        .unwrap_or(0.0);
    guard(ctx).await?;
    let SolveAnswer::Token(token) = answer.answer else {
        return Ok(unconfirmed());
    };
    if token.trim().is_empty() {
        return Ok(unconfirmed());
    }
    let fresh = read(ctx, challenge).await?;
    if IDENTITY
        .iter()
        .any(|key| fresh.get(*key) != state.get(*key))
        || !flag(&fresh, "loaded")
        || flag(&fresh, "failed")
    {
        return Ok(unconfirmed());
    }
    let mut args = fresh;
    args["token"] = json!(token);
    ctx.lease
        .set_progress(Some(CaptchaKind::Arkose), SolveStep::Verifying);
    let until = (Instant::now() + VERIFY).min(ctx.deadline);
    guard(ctx).await?;
    let applied = match tokio::time::timeout_at(until, evaluate(ctx, challenge, APPLY, &args)).await
    {
        Ok(value) => value?,
        Err(_) => return Ok(unconfirmed()),
    };
    result.callback_ran = flag(&applied, "callbackRan");
    if !flag(&applied, "written") || (flag(&applied, "callbackDeclared") && !result.callback_ran) {
        return Ok(unconfirmed());
    }
    for (key, value) in applied.as_object().into_iter().flatten() {
        args[key] = value.clone();
    }
    loop {
        let document = bounded(ctx, ctx.page.document()).await?;
        // Following top-level navigation, inspect the new top document, not a stale child frame.
        let top = Challenge {
            host_frame: None,
            ..challenge.clone()
        };
        let target = if document != ctx.document {
            &top
        } else {
            challenge
        };
        let evidence = evaluate(ctx, target, CHECK, &args).await?;
        if bounded(ctx, ctx.page.document()).await? != document {
            return Err(PageError::Navigated);
        }
        if document != ctx.document {
            if !flag(&evidence, "ready") || !flag(&evidence, "captchaFree") {
                return Err(PageError::Navigated);
            }
        } else if !flag(&evidence, "transition") {
            if Instant::now() >= until {
                return Ok(if flag(&evidence, "retained") {
                    (
                        SolveStatus::TokenApplied,
                        SolveReason::AwaitingSubmit,
                        super::token_applied_message(result.callback_ran),
                    )
                } else {
                    unconfirmed()
                });
            }
            bounded(ctx, async {
                sleep_until(until.min(Instant::now() + Duration::from_millis(250))).await;
                Ok(())
            })
            .await?;
            continue;
        }
        return Ok((
            SolveStatus::Solved,
            SolveReason::TokenAccepted,
            "The page accepted the Arkose token; continue.".into(),
        ));
    }
}
fn provider_failure(error: ProviderError) -> Terminal {
    let (status, reason) = match error.kind {
        ProviderErrorKind::NotConfigured => (SolveStatus::NeedsHuman, SolveReason::NoProvider),
        ProviderErrorKind::Auth | ProviderErrorKind::NoBalance | ProviderErrorKind::Unsupported => {
            (SolveStatus::NeedsHuman, SolveReason::ProviderUnavailable)
        }
        ProviderErrorKind::SpendLimit => (SolveStatus::NeedsHuman, SolveReason::SpendLimitReached),
        ProviderErrorKind::Cancelled => (SolveStatus::Cancelled, SolveReason::CallerCancelled),
        _ => (SolveStatus::NeedsHuman, SolveReason::ProviderError),
    };
    (
        status,
        reason,
        format!(
            "Arkose provider unavailable: {}; ask for human help.",
            error.message
        ),
    )
}

pub(super) fn capsolver_task(_task: &SolveTask) -> Result<Value, String> {
    Err("CapSolver does not support FunCaptcha".into())
}
pub(super) fn capsolver_answer(_task: &SolveTask, _solution: &Value) -> Option<SolveAnswer> {
    None
}
// https://2captcha.com/api-docs/arkoselabs-funcaptcha
pub(super) fn twocaptcha_task(task: &SolveTask) -> Result<Value, String> {
    let SolveTask::Arkose(p) = task else {
        return Err("Expected an Arkose task".into());
    };
    if !valid_key(&p.public_key) || !valid_url(&p.website_url) {
        return Err("Invalid Arkose parameters".into());
    }
    let mut body = json!({"type":"FunCaptchaTaskProxyless", "websiteURL":p.website_url, "websitePublicKey":p.public_key});
    optional(&mut body, "funcaptchaApiJSSubdomain", &p.api_subdomain);
    optional(&mut body, "userAgent", &p.user_agent);
    if let Some(blob) = p.data_blob.as_ref().filter(|s| !s.trim().is_empty()) {
        body["data"] = json!(json!({"blob":blob}).to_string());
    }
    Ok(body)
}
pub(super) fn twocaptcha_answer(task: &SolveTask, solution: &Value) -> Option<SolveAnswer> {
    if !matches!(task, SolveTask::Arkose(_)) {
        return None;
    }
    string(solution, "token").map(SolveAnswer::Token)
}
pub(super) fn secrets(task: &SolveTask) -> Vec<&str> {
    let SolveTask::Arkose(p) = task else {
        return vec![];
    };
    let mut values = vec![p.website_url.as_str(), p.public_key.as_str()];
    values.extend(p.api_subdomain.as_deref());
    values.extend(p.data_blob.as_deref());
    values.extend(p.user_agent.as_deref());
    values
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, MutexGuard};

    use serde_json::json;
    use tokio::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::services::captcha::{
        events::{CaptchaEventBus, EventLog},
        reservation::TabReservations,
        types::{
            ArkoseParams, CaptchaKind, ChallengeState, DocumentId, FrameRef, PageError,
            ProviderAnswer, ProviderError, ProviderErrorKind, ProviderName, Rect, SolveMethod,
        },
    };

    const KEY: &str = "6220FF23-9856-3A6F-9FF1-A14F88123F55";

    struct State {
        read: Value,
        applied: bool,
        transition: bool,
        callback: bool,
        retained: bool,
        navigated: bool,
        frame_calls: usize,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                read: json!({"url":"https://fixture.test/widget", "publicKey":KEY,
                    "apiHost":"company-api.arkoselabs.com", "blob":"fresh-blob",
                    "instance":"widget-1", "session":"session-1", "frameSrc":"enforcement-1",
                    "loaded":true, "failed":false, "completed":false}),
                applied: false,
                transition: true,
                callback: true,
                retained: true,
                navigated: false,
                frame_calls: 0,
            }
        }
    }

    #[derive(Default)]
    struct FakePage(Mutex<State>);

    impl FakePage {
        fn state(&self) -> MutexGuard<'_, State> {
            self.0.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    impl CaptchaPage for FakePage {
        async fn eval_main(&self, expression: &str) -> Result<Value, PageError> {
            let mut state = self.state();
            if expression.contains("arkose:apply") {
                state.applied = true;
                Ok(json!({"written":true, "callbackRan":state.callback,
                    "callbackDeclared":true, "wasVisible":true, "selector":"#gate"}))
            } else if expression.contains("arkose:verify") {
                Ok(
                    json!({"transition":state.transition, "retained":state.retained,
                    "ready":true, "captchaFree":state.transition}),
                )
            } else {
                Ok(state.read.clone())
            }
        }
        async fn eval_in_frame_main(
            &self,
            frame: &FrameRef,
            expr: &str,
        ) -> Result<Value, PageError> {
            assert_eq!(frame.frame_id, "host-frame");
            self.state().frame_calls += 1;
            self.eval_main(expr).await
        }
        async fn eval_in_frame(&self, _: &FrameRef, _: &str) -> Result<Value, PageError> {
            Err(PageError::Browser("isolated world must not be used".into()))
        }
        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            Ok(vec![])
        }
        async fn click_at(&self, _: f64, _: f64) -> Result<(), PageError> {
            Ok(())
        }
        async fn screenshot_clip(&self, _: Rect) -> Result<String, PageError> {
            Ok(String::new())
        }
        async fn query_rect_in_frame(
            &self,
            _: &FrameRef,
            _: &str,
        ) -> Result<Option<Rect>, PageError> {
            Ok(None)
        }
        async fn document(&self) -> Result<DocumentId, PageError> {
            let mut doc = document();
            if self.state().navigated {
                doc.loader_id = "next".into();
            }
            Ok(doc)
        }
    }

    struct FakeBackend<'a> {
        page: &'a FakePage,
        tasks: Mutex<Vec<SolveTask>>,
        error: Option<ProviderError>,
        change: Option<(&'static str, Value)>,
        navigate: bool,
        configured: bool,
    }
    impl<'a> FakeBackend<'a> {
        fn new(page: &'a FakePage) -> Self {
            Self {
                page,
                tasks: Mutex::default(),
                error: None,
                change: None,
                navigate: false,
                configured: true,
            }
        }
        fn tasks(&self) -> MutexGuard<'_, Vec<SolveTask>> {
            self.tasks.lock().unwrap_or_else(|e| e.into_inner())
        }
    }
    impl CaptchaBackend for FakeBackend<'_> {
        fn is_configured(&self) -> bool {
            self.configured
        }
        async fn solve(
            &self,
            task: &SolveTask,
            _: Instant,
            _: &CancellationToken,
        ) -> Result<ProviderAnswer, ProviderError> {
            self.tasks().push(task.clone());
            if let Some((key, value)) = &self.change {
                self.page.state().read[key] = value.clone();
            }
            if self.navigate {
                self.page.state().navigated = true;
            }
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            Ok(ProviderAnswer {
                answer: SolveAnswer::Token("vendor-token'\"\\".into()),
                provider: ProviderName::TwoCaptcha,
                task_id: Some("task-1".into()),
                cost_usd: Some(0.002),
            })
        }
    }
    fn document() -> DocumentId {
        DocumentId {
            url: "https://fixture.test/form".into(),
            loader_id: "doc-1".into(),
        }
    }
    async fn run(
        page: &FakePage,
        backend: &FakeBackend<'_>,
        cancel: &CancellationToken,
        framed: bool,
    ) -> anyhow::Result<FlowResult> {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "arkose-test")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        let events = EventLog::new(CaptchaEventBus::new(), "arkose-test".into(), 42);
        let result = solve(
            &SolveCtx {
                page,
                backend,
                events: &events,
                lease: &lease,
                cancel,
                deadline: Instant::now() + Duration::from_secs(30),
                page_url: document().url,
                document: document(),
                user_agent: Some("BrowserOS test agent".into()),
                forced_method: None,
            },
            &Challenge {
                kind: CaptchaKind::Arkose,
                state: ChallengeState::Active,
                frame: None,
                challenge_frame: None,
                host_frame: framed.then(|| FrameRef {
                    frame_id: "host-frame".into(),
                    url: "https://fixture.test/widget".into(),
                    rect: None,
                }),
                sitekey: Some("stale-key".into()),
                invisible: false,
                enterprise: false,
                recaptcha_s: None,
                action: None,
                cdata: None,
                callback: None,
            },
        )
        .await;
        if result.attempts.token > 0 {
            assert!(
                events
                    .events()
                    .iter()
                    .any(|e| e.data == json!({"type":"arkose", "method":"token", "attempt":1}))
            );
        }
        Ok(result)
    }
    fn task() -> SolveTask {
        SolveTask::Arkose(ArkoseParams {
            website_url: "https://fixture.test/widget".into(),
            public_key: KEY.into(),
            api_subdomain: Some("company-api.arkoselabs.com".into()),
            data_blob: Some("fresh-blob".into()),
            user_agent: Some("BrowserOS test agent".into()),
        })
    }

    #[tokio::test]
    async fn tests_that_a_visible_acceptance_returns_solved_with_fresh_params() -> anyhow::Result<()>
    {
        let page = FakePage::default();
        let backend = FakeBackend::new(&page);
        let result = run(&page, &backend, &CancellationToken::new(), false).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Solved, SolveReason::TokenAccepted)
        );
        assert_eq!(
            (result.paid_tasks, result.cost_usd, result.method),
            (1, 0.002, SolveMethod::Token)
        );
        assert_eq!(*backend.tasks(), vec![task()]);
        assert!(page.state().applied);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_callback_completion_alone_is_token_applied() -> anyhow::Result<()> {
        let page = FakePage::default();
        page.state().transition = false;
        let result = run(
            &page,
            &FakeBackend::new(&page),
            &CancellationToken::new(),
            false,
        )
        .await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::TokenApplied, SolveReason::AwaitingSubmit)
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_broken_or_fake_widgets_never_pay() -> anyhow::Result<()> {
        for (key, value) in [
            ("loaded", json!(false)),
            ("failed", json!(true)),
            ("publicKey", json!("bad")),
            ("publicKey", json!("00000000-0000-0000-0000-000000000000")),
        ] {
            let page = FakePage::default();
            page.state().read[key] = value;
            let backend = FakeBackend::new(&page);
            let result = run(&page, &backend, &CancellationToken::new(), false).await?;
            assert_eq!(
                (result.status, result.reason, result.paid_tasks),
                (SolveStatus::NeedsHuman, SolveReason::ChallengeNotPassed, 0)
            );
            assert!(backend.tasks().is_empty());
            assert!(!page.state().applied);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_navigation_and_cancel_before_payment_do_not_apply() -> anyhow::Result<()> {
        for navigation in [true, false] {
            let page = FakePage::default();
            page.state().navigated = navigation;
            let cancel = CancellationToken::new();
            if !navigation {
                cancel.cancel();
            }
            let backend = FakeBackend::new(&page);
            let result = run(&page, &backend, &cancel, false).await?;
            assert_eq!(result.status, SolveStatus::Cancelled);
            assert_eq!(result.paid_tasks, 0);
            assert!(backend.tasks().is_empty());
            assert!(!page.state().applied);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_stale_sessions_blobs_frames_and_keys_are_not_applied() -> anyhow::Result<()>
    {
        for key in [
            "publicKey",
            "blob",
            "instance",
            "session",
            "frameSrc",
            "url",
        ] {
            let page = FakePage::default();
            let mut backend = FakeBackend::new(&page);
            backend.change = Some((key, json!("changed")));
            let result = run(&page, &backend, &CancellationToken::new(), false).await?;
            assert_eq!(
                (result.status, result.reason, result.paid_tasks),
                (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed, 1)
            );
            assert!(!page.state().applied);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_navigation_during_payment_keeps_the_charge_without_applying()
    -> anyhow::Result<()> {
        let page = FakePage::default();
        let mut backend = FakeBackend::new(&page);
        backend.navigate = true;
        let result = run(&page, &backend, &CancellationToken::new(), false).await?;
        assert_eq!(
            (result.status, result.reason, result.paid_tasks),
            (SolveStatus::Cancelled, SolveReason::Navigated, 1)
        );
        assert!(!page.state().applied);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_frame_widgets_read_and_apply_in_the_host_main_world() -> anyhow::Result<()>
    {
        let page = FakePage::default();
        let backend = FakeBackend::new(&page);
        let result = run(&page, &backend, &CancellationToken::new(), true).await?;
        assert_eq!(result.status, SolveStatus::Solved);
        assert!(page.state().frame_calls >= 4);
        assert!(page.state().applied);
        assert_eq!(*backend.tasks(), vec![task()]);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_provider_errors_preserve_reasons_and_charges() -> anyhow::Result<()> {
        for (kind, reason, charged) in [
            (
                ProviderErrorKind::NotConfigured,
                SolveReason::NoProvider,
                false,
            ),
            (
                ProviderErrorKind::Auth,
                SolveReason::ProviderUnavailable,
                false,
            ),
            (
                ProviderErrorKind::NoBalance,
                SolveReason::ProviderUnavailable,
                false,
            ),
            (
                ProviderErrorKind::SpendLimit,
                SolveReason::SpendLimitReached,
                false,
            ),
            (
                ProviderErrorKind::Unsolvable,
                SolveReason::ProviderError,
                true,
            ),
        ] {
            let page = FakePage::default();
            let mut backend = FakeBackend::new(&page);
            backend.error = Some(ProviderError {
                provider: Some(ProviderName::TwoCaptcha),
                kind,
                message: "ERROR_TEST".into(),
                charged,
            });
            let result = run(&page, &backend, &CancellationToken::new(), false).await?;
            assert_eq!(
                (result.status, result.reason, result.paid_tasks),
                (SolveStatus::NeedsHuman, reason, u32::from(charged))
            );
            assert!(!page.state().applied);
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_failed_callbacks_or_lost_tokens_do_not_claim_acceptance()
    -> anyhow::Result<()> {
        for callback in [true, false] {
            let page = FakePage::default();
            {
                let mut s = page.state();
                s.callback = callback;
                s.retained = false;
                s.transition = false;
            }
            let result = run(
                &page,
                &FakeBackend::new(&page),
                &CancellationToken::new(),
                false,
            )
            .await?;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_vendor_payload_has_exact_names_and_optional_fields() -> anyhow::Result<()> {
        assert_eq!(
            twocaptcha_task(&task()).map_err(anyhow::Error::msg)?,
            json!({"type":"FunCaptchaTaskProxyless", "websiteURL":"https://fixture.test/widget",
            "websitePublicKey":KEY, "funcaptchaApiJSSubdomain":"company-api.arkoselabs.com", "data":"{\"blob\":\"fresh-blob\"}",
            "userAgent":"BrowserOS test agent"})
        );
        let SolveTask::Arkose(mut p) = task() else {
            anyhow::bail!("wrong task")
        };
        p.api_subdomain = Some(" ".into());
        p.data_blob = Some(String::new());
        p.user_agent = None;
        let body = twocaptcha_task(&SolveTask::Arkose(p)).map_err(anyhow::Error::msg)?;
        assert_eq!(
            body,
            json!({"type":"FunCaptchaTaskProxyless", "websiteURL":"https://fixture.test/widget", "websitePublicKey":KEY})
        );
        assert_eq!(
            capsolver_task(&task()).err().as_deref(),
            Some("CapSolver does not support FunCaptcha")
        );
        assert_eq!(capsolver_answer(&task(), &json!({"token":"x"})), None);
        Ok(())
    }

    #[test]
    fn tests_that_malformed_vendor_answers_and_invalid_params_are_rejected() {
        assert_eq!(
            twocaptcha_answer(&task(), &json!({"token":"answer"})),
            Some(SolveAnswer::Token("answer".into()))
        );
        for solution in [json!({}), json!({"token":" "}), json!({"token":123})] {
            assert_eq!(twocaptcha_answer(&task(), &solution), None);
        }
        if let SolveTask::Arkose(mut p) = task() {
            for key in ["bad", "00000000-0000-0000-0000-000000000000"] {
                p.public_key = key.into();
                assert!(twocaptcha_task(&SolveTask::Arkose(p.clone())).is_err());
            }
            p.public_key = KEY.into();
            p.website_url = "file:///test".into();
            assert!(twocaptcha_task(&SolveTask::Arkose(p)).is_err());
        }
    }

    #[test]
    fn tests_that_sensitive_task_values_are_redacted() {
        assert_eq!(
            secrets(&task()),
            vec![
                "https://fixture.test/widget",
                KEY,
                "company-api.arkoselabs.com",
                "fresh-blob",
                "BrowserOS test agent"
            ]
        );
    }
}
