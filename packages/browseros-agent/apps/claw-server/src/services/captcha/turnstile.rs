use std::future::Future;

use serde_json::{Value, json};
use tokio::time::{Duration, Instant, sleep_until};

use super::{
    SolveCtx,
    config::ForcedMethod,
    page::CaptchaPage,
    providers::CaptchaBackend,
    types::{
        Attempts, CaptchaEventName, CaptchaKind, Challenge, FlowResult, PageError, ProviderError,
        ProviderErrorKind, Rect, SolveAnswer, SolveMethod, SolveReason, SolveStatus, SolveStep,
        SolveTask, TurnstileParams,
    },
};

const POLL: Duration = Duration::from_millis(250);
const VERIFY: Duration = Duration::from_secs(6);

// All scripts run in MAIN; only apply mutates the page, after the document check.
const WIDGET: &str = r#"
const widgets = [...document.querySelectorAll('.cf-turnstile')];
const widget = widgets.find(e => e.getAttribute('data-sitekey') === args.sitekey)
    || (widgets.length === 1 ? widgets[0] : null);
const visible = e => !!e && e.isConnected && e.getBoundingClientRect().width > 0
    && e.getBoundingClientRect().height > 0 && getComputedStyle(e).visibility !== 'hidden';
"#;

const READ: &str = r#"
/* turnstile:read */
let token = [...document.querySelectorAll('input[name="cf-turnstile-response"]')].some(e => !!e.value.trim());
try { token ||= !!window.turnstile?.getResponse(); } catch {}
const sitekey = widget?.getAttribute('data-sitekey') ?? args.sitekey;
return { token, sitekey,
    action: widget?.getAttribute('data-action') ?? args.action,
    cdata: widget?.getAttribute('data-cdata') ?? args.cdata,
    callback: widget?.getAttribute('data-callback') ?? args.callback,
    failed: !!widget?.querySelector('[data-error-code], [aria-invalid="true"]') };
"#;

const APPLY: &str = r#"
/* turnstile:apply */
let selector = null;
if (widget) {
    const path = [];
    for (let e = widget; e; e = e.parentElement) {
        path.unshift(e.tagName.toLowerCase() + ':nth-child(' + ([...e.parentNode.children].indexOf(e) + 1) + ')');
    }
    selector = path.join(' > ');
}
const wasVisible = visible(widget);
const allInputs = [...document.querySelectorAll('input[name="cf-turnstile-response"]')];
const inputs = widget ? [...widget.querySelectorAll('input[name="cf-turnstile-response"]')]
    : (allInputs.length === 1 ? allInputs : []);
for (const input of inputs) {
    input.value = args.token;
    input.dispatchEvent(new Event('input', {bubbles: true}));
    input.dispatchEvent(new Event('change', {bubbles: true}));
}
let callbackRan = false;
let callbackDeclared = !!(widget?.getAttribute('data-callback') ?? args.callback);
try {
    let owner = window, fn;
    const name = widget?.getAttribute('data-callback') ?? args.callback;
    if (name) {
        const parts = name.split('.');
        fn = parts.pop();
        for (const part of parts) owner = owner?.[part];
        fn = owner?.[fn];
    }
    // Some render integrations expose their widget records; closure-only callbacks are inaccessible.
    const seen = new Set();
    function find(value, depth = 0) {
        if (!value || typeof value !== 'object' || depth > 4 || seen.has(value) || seen.size >= 100) return null;
        seen.add(value);
        const entries = value instanceof Map ? [...value.values()] : Object.values(Object.getOwnPropertyDescriptors(value)).map(d => d.value);
        if (value.sitekey === args.sitekey && typeof value.callback === 'function') return value;
        for (const child of entries.slice(0, 100)) { const match = find(child, depth + 1); if (match) return match; }
        return null;
    }
    if (typeof fn !== 'function') { owner = find(window.turnstile); fn = owner?.callback; }
    callbackDeclared ||= typeof fn === 'function';
    if (typeof fn === 'function') { await fn.call(owner, args.token); callbackRan = true; }
} catch { callbackDeclared = true; }
return { written: inputs.length > 0, callbackRan, callbackDeclared, selector, wasVisible };
"#;

const TRANSITION: &str = r#"
/* turnstile:verify */
return { transition: args.wasVisible === true && !!args.selector && !visible(document.querySelector(args.selector)),
    ready: document.readyState === 'complete',
    captchaFree: !document.querySelector('.cf-turnstile, .g-recaptcha, .h-captcha, [name="cf-turnstile-response"], [name="g-recaptcha-response"], [name="h-captcha-response"], iframe[src*="challenges.cloudflare.com"], iframe[src*="recaptcha"], iframe[src*="hcaptcha"]')
        && !window._cf_chl_opt };
"#;

const TOKEN_PRESENT: &str = r#"
/* turnstile:token-present */
const allInputs = [...document.querySelectorAll('input[name="cf-turnstile-response"]')];
const inputs = widget ? [...widget.querySelectorAll('input[name="cf-turnstile-response"]')]
    : (allInputs.length === 1 ? allInputs : []);
return inputs.length > 0 && inputs.every(input => input.value === args.token);
"#;

fn script(body: &str, args: &Value) -> String {
    format!("(async () => {{ const args = {args}; {WIDGET} {body} }})()")
}

pub async fn solve<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
) -> FlowResult {
    let mut result = FlowResult {
        status: SolveStatus::Failed,
        method: SolveMethod::None,
        reason: SolveReason::BrowserError,
        attempts: Attempts::default(),
        paid_tasks: 0,
        cost_usd: 0.0,
        callback_ran: false,
        message: String::new(),
    };
    let (status, reason, message) = match run(ctx, challenge, &mut result).await {
        Ok(terminal) => terminal,
        Err(error) => {
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
                "Turnstile stopped; take a fresh snapshot before continuing.".into(),
            )
        }
    };
    result.status = status;
    result.reason = reason;
    result.message = message;
    result
}

type Terminal = (SolveStatus, SolveReason, String);

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

async fn checked<T, P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    future: impl Future<Output = Result<T, PageError>>,
) -> Result<T, PageError> {
    guard(ctx).await?;
    let value = bounded(ctx, future).await?;
    guard(ctx).await?;
    Ok(value)
}

async fn pause<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    until: Instant,
) -> Result<(), PageError> {
    bounded(ctx, async {
        sleep_until(until.min(Instant::now() + POLL)).await;
        Ok(())
    })
    .await
}

async fn read<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
) -> Result<Value, PageError> {
    checked(ctx, ctx.page.eval_main(&script(READ, &json!(challenge)))).await
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}

fn passed(result: &mut FlowResult) -> Terminal {
    let (method, reason) = if result.attempts.checkbox > 0 {
        (SolveMethod::Checkbox, SolveReason::CheckboxPassed)
    } else {
        (SolveMethod::AutoPass, SolveReason::AutoPassed)
    };
    result.method = method;
    (
        SolveStatus::Solved,
        reason,
        "Turnstile returned a passing response; continue on the page.".into(),
    )
}

fn attempt<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    result: &mut FlowResult,
    method: SolveMethod,
    step: SolveStep,
    number: u32,
) {
    result.method = method;
    ctx.lease.set_progress(Some(CaptchaKind::Turnstile), step);
    ctx.events.emit(
        CaptchaEventName::Solving,
        json!({"type": "turnstile", "method": method, "attempt": number}),
    );
}

async fn wait_token<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
    duration: Duration,
) -> Result<Value, PageError> {
    let until = Instant::now() + duration;
    let mut state = json!({"token": false});
    loop {
        match tokio::time::timeout_at(until, read(ctx, challenge)).await {
            Ok(value) => state = value?,
            Err(_) if Instant::now() >= ctx.deadline => return Err(PageError::Timeout),
            Err(_) => return Ok(state),
        }
        if flag(&state, "token") || Instant::now() >= until {
            return Ok(state);
        }
        pause(ctx, until).await?;
        if Instant::now() >= until {
            return Ok(state);
        }
    }
}

async fn checkbox<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
) -> Result<Option<Rect>, PageError> {
    let frames = checked(ctx, ctx.page.frames()).await?;
    let mut frames: Vec<_> = frames
        .into_iter()
        .filter(|frame| {
            url::Url::parse(&frame.url)
                .ok()
                .is_some_and(|url| url.host_str() == Some("challenges.cloudflare.com"))
        })
        .collect();
    if let Some(hint) = &challenge.frame {
        if frames.iter().any(|frame| frame.frame_id == hint.frame_id) {
            frames.retain(|frame| frame.frame_id == hint.frame_id);
        } else if frames.len() != 1 {
            return Ok(None);
        }
    }
    for frame in frames {
        if let Some(rect) = checked(
            ctx,
            ctx.page.query_rect_in_frame(&frame, "input[type=checkbox]"),
        )
        .await?
            && [rect.x, rect.y, rect.width, rect.height]
                .iter()
                .all(|v| v.is_finite())
            && rect.width > 0.0
            && rect.height > 0.0
        {
            return Ok(Some(rect));
        }
    }
    Ok(None)
}

async fn run<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
    result: &mut FlowResult,
) -> Result<Terminal, PageError> {
    if challenge.kind != CaptchaKind::Turnstile {
        return Ok((
            SolveStatus::Unsupported,
            SolveReason::UnsupportedType,
            "Only ordinary Turnstile widgets are supported by this flow.".into(),
        ));
    }
    guard(ctx).await?;
    if ctx.forced_method == Some(ForcedMethod::Token) {
        return token(ctx, challenge, result).await;
    }
    attempt(ctx, result, SolveMethod::AutoPass, SolveStep::AutoPass, 1);
    let state = wait_token(ctx, challenge, Duration::from_secs(8)).await?;
    if flag(&state, "token") {
        return Ok(passed(result));
    }

    for _ in 0..2 {
        let until = Instant::now() + VERIFY;
        let rect = loop {
            let state = match tokio::time::timeout_at(until, read(ctx, challenge)).await {
                Ok(value) => value?,
                Err(_) if Instant::now() >= ctx.deadline => return Err(PageError::Timeout),
                Err(_) => break None,
            };
            if flag(&state, "token") {
                return Ok(passed(result));
            }
            let rect = match tokio::time::timeout_at(until, checkbox(ctx, challenge)).await {
                Ok(value) => value?,
                Err(_) if Instant::now() >= ctx.deadline => return Err(PageError::Timeout),
                Err(_) => break None,
            };
            if let Some(rect) = rect {
                break Some(rect);
            }
            if Instant::now() >= until {
                break None;
            }
            pause(ctx, until).await?;
            if Instant::now() >= until {
                break None;
            }
        };
        let Some(rect) = rect else {
            break;
        };
        let (x, y) = rect.center();
        let x = x + rand::random_range(-1.0..=1.0) * (rect.width * 0.1).min(1.5);
        let y = y + rand::random_range(-1.0..=1.0) * (rect.height * 0.1).min(1.5);
        if x < 0.0 || y < 0.0 {
            break;
        }
        result.attempts.checkbox += 1;
        attempt(
            ctx,
            result,
            SolveMethod::Checkbox,
            SolveStep::Checkbox,
            result.attempts.checkbox,
        );
        checked(ctx, ctx.page.click_at(x, y)).await?;
        if flag(&wait_token(ctx, challenge, VERIFY).await?, "token") {
            return Ok(passed(result));
        }
    }

    token(ctx, challenge, result).await
}

async fn token<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
    result: &mut FlowResult,
) -> Result<Terminal, PageError> {
    let configured = ctx.backend.is_configured();
    let state = read(ctx, challenge).await?;
    if flag(&state, "token") {
        return Ok(passed(result));
    }
    if !configured {
        return Ok((
            SolveStatus::NeedsHuman,
            if flag(&state, "failed") {
                SolveReason::ChallengeNotPassed
            } else {
                SolveReason::NoProvider
            },
            "Turnstile is still blocked and no provider is configured; ask for human help.".into(),
        ));
    }
    let Some(key) = state
        .get("sitekey")
        .and_then(Value::as_str)
        .filter(|key| !key.trim().is_empty())
    else {
        return Ok((
            SolveStatus::NeedsHuman,
            SolveReason::ChallengeNotPassed,
            "No Turnstile site key was found; ask for human help.".into(),
        ));
    };
    let params = TurnstileParams {
        website_url: ctx.page_url.clone(),
        website_key: key.into(),
        action: state
            .get("action")
            .and_then(Value::as_str)
            .map(str::to_owned),
        cdata: state
            .get("cdata")
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    result.attempts.token += 1;
    attempt(
        ctx,
        result,
        SolveMethod::Token,
        SolveStep::Token,
        result.attempts.token,
    );
    result.paid_tasks = 1;
    let answer = bounded(ctx, async {
        Ok(ctx
            .backend
            .solve(&SolveTask::TurnstileToken(params), ctx.deadline, ctx.cancel)
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
        return Ok((
            SolveStatus::NeedsHuman,
            SolveReason::ProviderError,
            "Provider returned an unusable Turnstile answer; ask for human help.".into(),
        ));
    };
    if token.trim().is_empty() {
        return Ok((
            SolveStatus::NeedsHuman,
            SolveReason::ProviderError,
            "Provider returned an empty Turnstile token; ask for human help.".into(),
        ));
    }
    let fresh = read(ctx, challenge).await?;
    if flag(&fresh, "token") {
        return Ok(passed(result));
    }
    if fresh.get("sitekey") != state.get("sitekey") {
        return Ok((
            SolveStatus::Failed,
            SolveReason::AcceptanceUnconfirmed,
            "Turnstile changed during the provider request; take a fresh snapshot.".into(),
        ));
    }
    let mut args = fresh;
    args["token"] = json!(token);
    guard(ctx).await?;
    ctx.lease
        .set_progress(Some(CaptchaKind::Turnstile), SolveStep::Verifying);
    let until = Instant::now() + VERIFY;
    let applied = match tokio::time::timeout_at(
        until,
        bounded(ctx, ctx.page.eval_main(&script(APPLY, &args))),
    )
    .await
    {
        Ok(value) => value?,
        Err(_) if Instant::now() >= ctx.deadline => return Err(PageError::Timeout),
        Err(_) => return Ok(unconfirmed()),
    };
    result.callback_ran = flag(&applied, "callbackRan");
    if !flag(&applied, "written") || (flag(&applied, "callbackDeclared") && !result.callback_ran) {
        return Ok(unconfirmed());
    }
    loop {
        let document = bounded(ctx, ctx.page.document()).await?;
        let evidence = bounded(ctx, ctx.page.eval_main(&script(TRANSITION, &applied))).await?;
        if bounded(ctx, ctx.page.document()).await? != document {
            return Err(PageError::Navigated);
        }
        if document != ctx.document {
            if !flag(&evidence, "ready") || !flag(&evidence, "captchaFree") {
                return Err(PageError::Navigated);
            }
        } else if !flag(&evidence, "transition") {
            if Instant::now() >= until {
                let retained =
                    checked(ctx, ctx.page.eval_main(&script(TOKEN_PRESENT, &args))).await?;
                if retained.as_bool() != Some(true) {
                    return Ok((
                        SolveStatus::Failed,
                        SolveReason::AcceptanceUnconfirmed,
                        "The applied token is no longer in the form; take a fresh snapshot and retry.".into(),
                    ));
                }
                return Ok((
                    SolveStatus::TokenApplied,
                    SolveReason::AwaitingSubmit,
                    super::token_applied_message(result.callback_ran),
                ));
            }
            pause(ctx, until).await?;
            continue;
        }
        return Ok((
            SolveStatus::Solved,
            SolveReason::TokenAccepted,
            "The page accepted the Turnstile token; continue.".into(),
        ));
    }
}

fn unconfirmed() -> Terminal {
    (
        SolveStatus::Failed,
        SolveReason::AcceptanceUnconfirmed,
        "The Turnstile token is in the form; submit it and check the page.".into(),
    )
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
            "Turnstile provider unavailable: {}; ask for human help.",
            error.message
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, MutexGuard};

    use serde_json::{Value, json};
    use tokio::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::services::captcha::{
        events::{CaptchaEventBus, EventLog},
        reservation::TabReservations,
        types::{
            CaptchaKind, ChallengeState, DocumentId, FrameRef, PageError, ProviderAnswer,
            ProviderError, ProviderErrorKind, ProviderName, Rect, SolveAnswer, SolveTask,
        },
    };

    struct State {
        polls: usize,
        token_at: usize,
        pass_on_click: bool,
        rect: Option<Rect>,
        secondary_rect: Option<Rect>,
        selected_frame: bool,
        clicks: Vec<(f64, f64)>,
        navigated: bool,
        error: Option<PageError>,
        applied: Option<String>,
        written: bool,
        token_retained: bool,
        callback_declared: bool,
        callback: bool,
        transition: bool,
        failed: bool,
        delay: Duration,
        delay_once: bool,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                polls: 0,
                token_at: usize::MAX,
                pass_on_click: false,
                rect: None,
                secondary_rect: None,
                selected_frame: false,
                clicks: Vec::new(),
                navigated: false,
                error: None,
                applied: None,
                written: true,
                token_retained: true,
                callback_declared: true,
                callback: false,
                transition: false,
                failed: false,
                delay: Duration::ZERO,
                delay_once: false,
            }
        }
    }

    #[derive(Default)]
    struct FakePage(Arc<Mutex<State>>);

    impl FakePage {
        fn state(&self) -> MutexGuard<'_, State> {
            self.0.lock().unwrap_or_else(|error| error.into_inner())
        }
    }

    impl CaptchaPage for FakePage {
        async fn eval_main(&self, expression: &str) -> Result<Value, PageError> {
            let delay = {
                let mut state = self.state();
                let delay = state.delay;
                if state.delay_once {
                    state.delay = Duration::ZERO;
                }
                delay
            };
            tokio::time::sleep(delay).await;
            let mut state = self.state();
            if let Some(error) = &state.error {
                return Err(error.clone());
            }
            if expression.contains("turnstile:apply") {
                state.applied = Some(expression.into());
                return Ok(
                    json!({"written": state.written, "callbackRan": state.callback,
                    "callbackDeclared": state.callback_declared,
                    "transition": state.transition}),
                );
            }
            if expression.contains("turnstile:token-present") {
                return Ok(json!(state.token_retained));
            }
            if expression.contains("turnstile:verify") {
                return Ok(json!({"transition": state.transition}));
            }
            state.polls += 1;
            Ok(json!({
                "token": state.polls >= state.token_at
                    || (state.pass_on_click && !state.clicks.is_empty()),
                "sitekey": "fresh-key", "action": "fresh-action", "cdata": "fresh-data",
                "callback": if state.callback_declared { Some("onAccepted") } else { None }, "failed": state.failed,
            }))
        }

        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            let mut frames = vec![FrameRef {
                frame_id: "fresh-frame".into(),
                url: "https://challenges.cloudflare.com/turnstile/widget".into(),
                rect: Some(Rect {
                    x: 50.0,
                    y: 60.0,
                    width: 300.0,
                    height: 65.0,
                }),
            }];
            if self.state().secondary_rect.is_some() {
                frames.push(FrameRef {
                    frame_id: "other-frame".into(),
                    url: "https://challenges.cloudflare.com/turnstile/other-widget".into(),
                    rect: None,
                });
            }
            Ok(frames)
        }

        async fn eval_in_frame(&self, _: &FrameRef, _: &str) -> Result<Value, PageError> {
            Err(PageError::Browser("unexpected frame evaluation".into()))
        }

        async fn query_rect_in_frame(
            &self,
            frame: &FrameRef,
            selector: &str,
        ) -> Result<Option<Rect>, PageError> {
            assert_eq!(selector, "input[type=checkbox]");
            Ok(if frame.frame_id == "fresh-frame" {
                self.state().rect
            } else {
                self.state().secondary_rect
            })
        }

        async fn click_at(&self, x: f64, y: f64) -> Result<(), PageError> {
            self.state().clicks.push((x, y));
            Ok(())
        }

        async fn screenshot_clip(&self, _: Rect) -> Result<String, PageError> {
            Err(PageError::Browser("unexpected screenshot".into()))
        }

        async fn document(&self) -> Result<DocumentId, PageError> {
            let mut document = document();
            if self.state().navigated {
                document.loader_id = "next-document".into();
            }
            Ok(document)
        }
    }

    struct FakeBackend {
        answer: Result<ProviderAnswer, ProviderError>,
        tasks: Mutex<Vec<SolveTask>>,
        on_solve: Option<Arc<Mutex<State>>>,
        on_configured: Option<Arc<Mutex<State>>>,
        configured: Option<bool>,
    }

    impl FakeBackend {
        fn error(kind: ProviderErrorKind) -> Self {
            Self {
                answer: Err(ProviderError {
                    provider: Some(ProviderName::Capsolver),
                    kind,
                    message: "ERROR_KEY_DENIED_ACCESS; ERROR_ZERO_BALANCE".into(),
                    charged: false,
                }),
                tasks: Mutex::default(),
                on_solve: None,
                on_configured: None,
                configured: None,
            }
        }

        fn token() -> Self {
            Self {
                answer: Ok(ProviderAnswer {
                    answer: SolveAnswer::Token("test-token'\\\"".into()),
                    provider: ProviderName::Capsolver,
                    task_id: Some("task".into()),
                    cost_usd: Some(0.001),
                }),
                tasks: Mutex::default(),
                on_solve: None,
                on_configured: None,
                configured: None,
            }
        }

        fn tasks(&self) -> MutexGuard<'_, Vec<SolveTask>> {
            self.tasks.lock().unwrap_or_else(|error| error.into_inner())
        }
    }

    impl CaptchaBackend for FakeBackend {
        fn is_configured(&self) -> bool {
            if let Some(state) = &self.on_configured {
                state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .token_at = 0;
            }
            self.configured.unwrap_or(!matches!(&self.answer, Err(error) if error.kind == ProviderErrorKind::NotConfigured))
        }

        async fn solve(
            &self,
            task: &SolveTask,
            _: Instant,
            _: &CancellationToken,
        ) -> Result<ProviderAnswer, ProviderError> {
            self.tasks().push(task.clone());
            if let Some(state) = &self.on_solve {
                state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .navigated = true;
            }
            self.answer.clone()
        }
    }

    fn document() -> DocumentId {
        DocumentId {
            url: "https://fixture.test/form".into(),
            loader_id: "document".into(),
        }
    }

    fn challenge() -> Challenge {
        Challenge {
            kind: CaptchaKind::Turnstile,
            state: ChallengeState::Active,
            frame: None,
            challenge_frame: None,
            sitekey: Some("stale-key".into()),
            invisible: false,
            enterprise: false,
            recaptcha_s: None,
            action: None,
            cdata: None,
            callback: None,
        }
    }

    async fn run(
        page: &FakePage,
        backend: &FakeBackend,
        cancel: &CancellationToken,
        budget: Duration,
    ) -> anyhow::Result<FlowResult> {
        run_with_method(page, backend, cancel, budget, None)
            .await
            .map(|(result, _)| result)
    }

    async fn run_with_method(
        page: &FakePage,
        backend: &FakeBackend,
        cancel: &CancellationToken,
        budget: Duration,
        forced_method: Option<super::super::config::ForcedMethod>,
    ) -> anyhow::Result<(FlowResult, Vec<super::super::types::CaptchaEvent>)> {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "cap_test")
            .map_err(|_| anyhow::anyhow!("unexpected busy page"))?;
        let events = EventLog::new(CaptchaEventBus::new(), "cap_test".into(), 42);
        let mut challenge = challenge();
        if page.state().selected_frame {
            challenge.frame = Some(FrameRef {
                frame_id: "fresh-frame".into(),
                url: "https://challenges.cloudflare.com/turnstile/widget".into(),
                rect: None,
            });
        }
        let result = solve(
            &SolveCtx {
                page,
                backend,
                events: &events,
                lease: &lease,
                cancel,
                deadline: Instant::now() + budget,
                page_url: document().url,
                document: document(),
                user_agent: None,
                forced_method,
            },
            &challenge,
        )
        .await;
        Ok((result, events.events()))
    }

    async fn finish(page: &FakePage, backend: &FakeBackend) -> anyhow::Result<FlowResult> {
        run(
            page,
            backend,
            &CancellationToken::new(),
            Duration::from_secs(120),
        )
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_applied_tokens_await_submit_without_a_visible_transition()
    -> anyhow::Result<()> {
        for callback_declared in [false, true] {
            let page = FakePage::default();
            {
                let mut state = page.state();
                state.callback_declared = callback_declared;
                state.callback = callback_declared;
            }
            let result = finish(&page, &FakeBackend::token()).await?;
            assert_eq!(serde_json::to_value(result.status)?, "token_applied");
            assert_eq!(serde_json::to_value(result.reason)?, "awaiting_submit");
            assert_eq!(result.method, SolveMethod::Token);
            assert_eq!(result.paid_tasks, 1);
            assert_eq!(result.callback_ran, callback_declared);
            assert!(result.message.contains(if callback_declared {
                "callback ran"
            } else {
                "no callback was declared"
            }));
            assert!(result.message.to_lowercase().contains("submit"));
            assert!(result.message.contains("solve_captcha"));
            assert!(!result.message.contains("test-token"));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_cleared_tokens_remain_unconfirmed() -> anyhow::Result<()> {
        let page = FakePage::default();
        {
            let mut state = page.state();
            state.callback = true;
            state.token_retained = false;
        }
        let result = finish(&page, &FakeBackend::token()).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
        );
        assert!(!result.message.contains("Submit"));
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_tokens_without_response_fields_remain_unconfirmed() -> anyhow::Result<()> {
        let page = FakePage::default();
        {
            let mut state = page.state();
            state.written = false;
            state.callback = true;
            state.transition = true;
        }
        let result = finish(&page, &FakeBackend::token()).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_forced_tokens_skip_turnstile_waits_and_clicks() -> anyhow::Result<()> {
        for token_at in [3, usize::MAX] {
            let page = FakePage::default();
            {
                let mut state = page.state();
                state.token_at = token_at;
                state.pass_on_click = true;
                state.rect = Some(Rect {
                    x: 20.,
                    y: 20.,
                    width: 24.,
                    height: 24.,
                });
                state.callback = true;
                state.transition = true;
            }
            let backend = FakeBackend::token();
            let started = Instant::now();
            let (result, events) = run_with_method(
                &page,
                &backend,
                &CancellationToken::new(),
                Duration::from_secs(120),
                Some(super::super::config::ForcedMethod::Token),
            )
            .await?;
            assert_eq!(
                (result.status, result.method, result.reason),
                (
                    SolveStatus::Solved,
                    SolveMethod::Token,
                    SolveReason::TokenAccepted
                )
            );
            assert_eq!(
                result.attempts,
                Attempts {
                    token: 1,
                    ..Default::default()
                }
            );
            assert_eq!(result.paid_tasks, 1);
            assert!(matches!(
                backend.tasks().as_slice(),
                [SolveTask::TurnstileToken(_)]
            ));
            assert!(page.state().clicks.is_empty());
            assert!(page.state().applied.is_some());
            assert_eq!(started.elapsed(), Duration::ZERO);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].data["method"], "token");
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_forced_turnstile_tokens_still_require_acceptance() -> anyhow::Result<()> {
        let page = FakePage::default();
        let (result, _) = run_with_method(
            &page,
            &FakeBackend::token(),
            &CancellationToken::new(),
            Duration::from_secs(120),
            Some(super::super::config::ForcedMethod::Token),
        )
        .await?;
        assert_eq!(
            (result.status, result.method, result.reason),
            (
                SolveStatus::Failed,
                SolveMethod::Token,
                SolveReason::AcceptanceUnconfirmed
            )
        );
        assert!(page.state().clicks.is_empty());
        assert!(page.state().applied.is_some());
        Ok(())
    }

    async fn script_context() -> browseros_mcp::framework::ToolCtx {
        use browseros_mcp::framework::{BrowserToolOptions, ToolCtx};
        struct NoBrowser;
        impl browseros_core::CdpConnection for NoBrowser {
            fn send<'a>(
                &'a self,
                _: &'a str,
                _: Value,
                _: Option<&'a browseros_core::SessionId>,
            ) -> futures_util::future::BoxFuture<'a, Result<Value, browseros_cdp::CdpError>>
            {
                Box::pin(async { Err(browseros_cdp::CdpError::NotConnected) })
            }
            fn send_raw_json<'a>(
                &'a self,
                _: &'a str,
                _: &'a str,
                _: Option<&'a browseros_core::SessionId>,
            ) -> futures_util::future::BoxFuture<'a, Result<String, browseros_cdp::CdpError>>
            {
                Box::pin(async { Err(browseros_cdp::CdpError::NotConnected) })
            }
            fn events(&self) -> tokio::sync::broadcast::Receiver<browseros_cdp::CdpEvent> {
                tokio::sync::broadcast::channel(1).1
            }
            fn is_connected(&self) -> bool {
                true
            }
            fn connection_epoch(&self) -> u64 {
                1
            }
        }
        let browser = browseros_core::Browser::new(Arc::new(NoBrowser));
        ToolCtx::new(BrowserToolOptions {
            browser: browser.view().await,
            target: None,
            defaults: Default::default(),
            cancel: CancellationToken::new(),
            output_files: browseros_mcp::output_file::create_browser_output_file_access(),
            inner_call_hook: None,
            preloaded_helpers: vec![],
        })
    }

    #[tokio::test]
    async fn tests_that_apply_distinguishes_absent_missing_throwing_and_render_callbacks()
    -> anyhow::Result<()> {
        use browseros_mcp::framework::execute_tool;
        let ctx = script_context().await;
        for (mode, declared, ran, written) in [
            ("absent", false, false, true),
            ("missing", true, false, true),
            ("throws", true, false, true),
            ("rejects", true, false, true),
            ("named", true, true, true),
            ("render", true, true, true),
            ("explicit", false, false, true),
            ("gone", false, false, false),
        ] {
            let code = format!(
                r#"
                const mode = {};
                class Event {{}}
                const input = {{value:'',dispatchEvent(){{}}}};
                const widget = {{
                    tagName:'DIV',parentElement:null,parentNode:{{children:[]}},isConnected:true,
                    getBoundingClientRect:()=>({{width:300,height:65}}),
                    getAttribute:()=>['missing','throws','rejects','named'].includes(mode)?'accepted':null,
                    querySelectorAll:()=>[input]
                }};
                widget.parentNode.children=[widget];
                const document = {{querySelectorAll:s=>s==='.cf-turnstile'?(['gone','explicit'].includes(mode)?[]:[widget]):(mode==='gone'?[]:[input]),querySelector:()=>null}};
                const getComputedStyle=()=>({{visibility:'visible'}});
                const window = {{turnstile:mode==='render'?{{record:{{sitekey:'test-key',callback(){{}}}}}}:{{}}}};
                if(mode==='named')window.accepted=()=>{{}};
                if(mode==='throws')window.accepted=()=>{{throw new Error('callback failed')}};
                if(mode==='rejects')window.accepted=async()=>{{throw new Error('callback failed')}};
                return await {};
            "#,
                json!(mode),
                script(APPLY, &json!({"sitekey":"test-key","token":"test-token"}))
            );
            let result = execute_tool(
                &browseros_mcp::tools::run::definition(),
                json!({"code":code}),
                &ctx,
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
            assert!(!result.is_error, "{mode}: {:?}", result.structured_content);
            let output = result
                .structured_content
                .ok_or_else(|| anyhow::anyhow!("missing script result"))?;
            assert_eq!(output["ok"], true, "{mode}: {output}");
            assert_eq!(output["value"]["callbackDeclared"], declared, "{mode}");
            assert_eq!(output["value"]["callbackRan"], ran, "{mode}");
            assert_eq!(output["value"]["written"], written, "{mode}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_retained_tokens_must_match_the_applied_value() -> anyhow::Result<()> {
        use browseros_mcp::framework::execute_tool;
        let ctx = script_context().await;
        for (values, retained) in [
            (vec!["test-token"], true),
            (vec![""], false),
            (vec!["replaced-token"], false),
            (vec![], false),
        ] {
            let code = format!(
                "const inputs = {}.map(value=>({{value}})); const document = {{querySelectorAll:s=>s==='.cf-turnstile'?[]:inputs}}; return await {};",
                json!(values),
                script(
                    TOKEN_PRESENT,
                    &json!({"sitekey":"test-key","token":"test-token"})
                ),
            );
            let result = execute_tool(
                &browseros_mcp::tools::run::definition(),
                json!({"code":code}),
                &ctx,
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
            assert!(!result.is_error);
            let output = result
                .structured_content
                .ok_or_else(|| anyhow::anyhow!("missing script result"))?;
            assert_eq!(output["ok"], true);
            assert_eq!(output["value"], retained);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_widget_errors_require_error_attributes() -> anyhow::Result<()> {
        use browseros_mcp::framework::execute_tool;
        let ctx = script_context().await;
        for key in ["2x00000000000000000000AB", "ordinary-public-key"] {
            for error in [None, Some("data-error-code"), Some("aria-invalid")] {
                let code = format!(
                    "const key={}; const error={}; const widget={{getAttribute:name=>name==='data-sitekey'?key:null,querySelector:selector=>error&&selector.includes('['+error)?{{}}:null}}; const document={{querySelectorAll:selector=>selector==='.cf-turnstile'?[widget]:[]}}; const window={{}}; return await {};",
                    json!(key),
                    json!(error),
                    script(READ, &json!({"sitekey":key})),
                );
                let result = execute_tool(
                    &browseros_mcp::tools::run::definition(),
                    json!({"code":code}),
                    &ctx,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{error}"))?;
                assert!(!result.is_error, "{:?}", result.structured_content);
                let output = result
                    .structured_content
                    .ok_or_else(|| anyhow::anyhow!("missing script result"))?;
                assert_eq!(output["ok"], true);
                assert_eq!(
                    output["value"]["failed"],
                    error.is_some(),
                    "{key}: {error:?}"
                );
            }
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_auto_pass_avoids_clicks_and_payment() -> anyhow::Result<()> {
        let page = FakePage::default();
        page.state().token_at = 3;
        let backend = FakeBackend::token();
        let result = finish(&page, &backend).await?;
        assert_eq!(
            (result.status, result.method, result.reason),
            (
                SolveStatus::Solved,
                SolveMethod::AutoPass,
                SolveReason::AutoPassed
            )
        );
        assert!(page.state().clicks.is_empty());
        assert!(backend.tasks().is_empty());
        assert_eq!(result.paid_tasks, 0);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_click_uses_the_measured_viewport_checkbox() -> anyhow::Result<()> {
        let page = FakePage::default();
        {
            let mut state = page.state();
            state.rect = Some(Rect {
                x: 130.0,
                y: 220.0,
                width: 24.0,
                height: 24.0,
            });
            state.pass_on_click = true;
        }
        let backend = FakeBackend::token();
        let result = finish(&page, &backend).await?;
        assert_eq!(
            (result.status, result.method, result.reason),
            (
                SolveStatus::Solved,
                SolveMethod::Checkbox,
                SolveReason::CheckboxPassed
            )
        );
        let state = page.state();
        assert_eq!(state.clicks.len(), 1);
        let (x, y) = state.clicks[0];
        assert!((x - 142.0).abs() <= 2.0 && (y - 232.0).abs() <= 2.0);
        assert_eq!(result.attempts.checkbox, 1);
        assert!(backend.tasks().is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_loading_widget_does_not_click_another_widget() -> anyhow::Result<()> {
        let page = FakePage::default();
        {
            let mut state = page.state();
            state.selected_frame = true;
            state.secondary_rect = Some(Rect {
                x: 500.0,
                y: 500.0,
                width: 24.0,
                height: 24.0,
            });
            state.pass_on_click = true;
        }
        let result = finish(&page, &FakeBackend::error(ProviderErrorKind::NotConfigured)).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::NeedsHuman, SolveReason::NoProvider)
        );
        assert!(page.state().clicks.is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_token_status_distinguishes_application_from_acceptance()
    -> anyhow::Result<()> {
        for (callback, transition, status, reason) in [
            (true, true, SolveStatus::Solved, SolveReason::TokenAccepted),
            (
                true,
                false,
                SolveStatus::TokenApplied,
                SolveReason::AwaitingSubmit,
            ),
            (
                false,
                true,
                SolveStatus::Failed,
                SolveReason::AcceptanceUnconfirmed,
            ),
            (
                false,
                false,
                SolveStatus::Failed,
                SolveReason::AcceptanceUnconfirmed,
            ),
        ] {
            let page = FakePage::default();
            {
                let mut state = page.state();
                state.callback = callback;
                state.transition = transition;
            }
            let backend = FakeBackend::token();
            let result = finish(&page, &backend).await?;
            assert_eq!(result.status, status);
            assert_eq!(result.reason, reason);
            assert_eq!(result.method, SolveMethod::Token);
            assert_eq!(result.paid_tasks, 1);
            assert_eq!(result.cost_usd, 0.001);
            assert_eq!(result.attempts.token, 1);
            let tasks = backend.tasks();
            assert!(matches!(&tasks[..], [SolveTask::TurnstileToken(params)]
                if params.website_url == document().url && params.website_key == "fresh-key"
                    && params.action.as_deref() == Some("fresh-action") && params.cdata.as_deref() == Some("fresh-data")));
            assert!(page.state().applied.is_some());
            assert!(!result.message.contains("test-token"));
            if status != SolveStatus::Solved {
                assert!(result.message.to_lowercase().contains("submit"));
            }
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_provider_failures_have_the_contract_reasons() -> anyhow::Result<()> {
        for (kind, reason) in [
            (ProviderErrorKind::NotConfigured, SolveReason::NoProvider),
            (ProviderErrorKind::Auth, SolveReason::ProviderUnavailable),
            (
                ProviderErrorKind::NoBalance,
                SolveReason::ProviderUnavailable,
            ),
            (
                ProviderErrorKind::Unsupported,
                SolveReason::ProviderUnavailable,
            ),
            (
                ProviderErrorKind::SpendLimit,
                SolveReason::SpendLimitReached,
            ),
            (ProviderErrorKind::Unsolvable, SolveReason::ProviderError),
            (ProviderErrorKind::Timeout, SolveReason::ProviderError),
            (ProviderErrorKind::BadRequest, SolveReason::ProviderError),
            (ProviderErrorKind::RateLimited, SolveReason::ProviderError),
            (ProviderErrorKind::Transport, SolveReason::ProviderError),
            (ProviderErrorKind::Uncertain, SolveReason::ProviderError),
        ] {
            let result = finish(&FakePage::default(), &FakeBackend::error(kind)).await?;
            assert_eq!(result.status, SolveStatus::NeedsHuman);
            assert_eq!(result.reason, reason);
            if kind == ProviderErrorKind::Auth {
                assert!(result.message.contains("ERROR_KEY_DENIED_ACCESS"));
            }
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_visibly_failed_widget_needs_human() -> anyhow::Result<()> {
        let page = FakePage::default();
        page.state().failed = true;
        let result = finish(&page, &FakeBackend::error(ProviderErrorKind::NotConfigured)).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::NeedsHuman, SolveReason::ChallengeNotPassed)
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_backend_not_configured_error_returns_no_provider() -> anyhow::Result<()> {
        let page = FakePage::default();
        page.state().failed = true;
        let mut backend = FakeBackend::error(ProviderErrorKind::NotConfigured);
        backend.configured = Some(true);
        let result = finish(&page, &backend).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::NeedsHuman, SolveReason::NoProvider)
        );
        assert_eq!(result.paid_tasks, 0);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_navigation_and_cancel_prevent_payment() -> anyhow::Result<()> {
        let page = FakePage::default();
        page.state().navigated = true;
        let backend = FakeBackend::token();
        let result = finish(&page, &backend).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Cancelled, SolveReason::Navigated)
        );
        page.state().navigated = false;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = run(&page, &backend, &cancel, Duration::from_secs(120)).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Cancelled, SolveReason::CallerCancelled)
        );
        assert!(backend.tasks().is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_the_deadline_bounds_polling() -> anyhow::Result<()> {
        let page = FakePage::default();
        let backend = FakeBackend::token();
        let start = Instant::now();
        let result = run(
            &page,
            &backend,
            &CancellationToken::new(),
            Duration::from_millis(700),
        )
        .await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Failed, SolveReason::DeadlineExceeded)
        );
        assert!(start.elapsed() <= Duration::from_millis(700));
        assert!(backend.tasks().is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_stalled_auto_pass_read_obeys_the_eight_second_budget()
    -> anyhow::Result<()> {
        let page = FakePage::default();
        page.state().delay = Duration::from_secs(60);
        page.state().delay_once = true;
        let backend = FakeBackend::error(ProviderErrorKind::NotConfigured);
        let start = Instant::now();
        let result = finish(&page, &backend).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::NeedsHuman, SolveReason::NoProvider)
        );
        assert!(start.elapsed() <= Duration::from_secs(14));
        assert!(backend.tasks().is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_nonzero_page_latency_continues_after_each_free_phase() -> anyhow::Result<()>
    {
        for (interactive, pass_on_click) in [(false, false), (true, false), (true, true)] {
            let page = FakePage::default();
            {
                let mut state = page.state();
                state.delay = Duration::from_millis(130);
                state.pass_on_click = pass_on_click;
                if interactive {
                    state.rect = Some(Rect {
                        x: 100.0,
                        y: 100.0,
                        width: 24.0,
                        height: 24.0,
                    });
                }
            }
            let result =
                finish(&page, &FakeBackend::error(ProviderErrorKind::NotConfigured)).await?;
            assert_eq!(
                (result.status, result.reason),
                if pass_on_click {
                    (SolveStatus::Solved, SolveReason::CheckboxPassed)
                } else {
                    (SolveStatus::NeedsHuman, SolveReason::NoProvider)
                }
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inflight_browser_operations_obey_cancel_and_deadline() -> anyhow::Result<()>
    {
        let page = FakePage::default();
        page.state().delay = Duration::from_secs(60);
        let backend = FakeBackend::token();
        let cancel = CancellationToken::new();
        let (result, ()) = tokio::join!(
            run(&page, &backend, &cancel, Duration::from_secs(10)),
            async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                cancel.cancel();
            }
        );
        assert_eq!(result?.reason, SolveReason::CallerCancelled);
        let result = run(
            &page,
            &backend,
            &CancellationToken::new(),
            Duration::from_millis(500),
        )
        .await?;
        assert_eq!(result.reason, SolveReason::DeadlineExceeded);
        assert!(backend.tasks().is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_clicks_are_bounded_and_page_failures_are_reported() -> anyhow::Result<()> {
        let page = FakePage::default();
        page.state().rect = Some(Rect {
            x: 1.0,
            y: 2.0,
            width: 24.0,
            height: 24.0,
        });
        let result = finish(&page, &FakeBackend::error(ProviderErrorKind::NotConfigured)).await?;
        assert_eq!(result.status, SolveStatus::NeedsHuman);
        assert!((1..=2).contains(&result.attempts.checkbox));
        assert_eq!(page.state().clicks.len(), result.attempts.checkbox as usize);
        for (error, status, reason) in [
            (
                PageError::Closed,
                SolveStatus::Cancelled,
                SolveReason::TabClosed,
            ),
            (
                PageError::Browser("gone".into()),
                SolveStatus::Failed,
                SolveReason::BrowserError,
            ),
        ] {
            page.state().error = Some(error);
            let result = finish(&page, &FakeBackend::token()).await?;
            assert_eq!((result.status, result.reason), (status, reason));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_late_auto_pass_never_creates_a_paid_task() -> anyhow::Result<()> {
        let page = FakePage::default();
        let backend = FakeBackend::token();
        let start = Instant::now();
        let (result, ()) = tokio::join!(finish(&page, &backend), async {
            tokio::time::sleep(Duration::from_secs(9)).await;
            page.state().token_at = 0;
        });
        let result = result?;
        assert_eq!(result.status, SolveStatus::Solved);
        assert!(backend.tasks().is_empty());
        assert!(start.elapsed() >= Duration::from_secs(9));
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_token_is_rechecked_immediately_before_payment() -> anyhow::Result<()> {
        let page = FakePage::default();
        let mut backend = FakeBackend::token();
        backend.on_configured = Some(Arc::clone(&page.0));
        let result = finish(&page, &backend).await?;
        assert_eq!(result.status, SolveStatus::Solved);
        assert!(backend.tasks().is_empty());
        assert_eq!(result.paid_tasks, 0);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_provider_answers_cannot_be_applied_after_navigation() -> anyhow::Result<()>
    {
        let page = FakePage::default();
        let mut backend = FakeBackend::token();
        backend.on_solve = Some(Arc::clone(&page.0));
        let result = finish(&page, &backend).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Cancelled, SolveReason::Navigated)
        );
        assert_eq!(result.paid_tasks, 1);
        assert!(page.state().applied.is_none());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_wrong_or_empty_provider_answers_are_rejected() -> anyhow::Result<()> {
        for answer in [
            SolveAnswer::Token(String::new()),
            SolveAnswer::Tiles(vec![0]),
        ] {
            let page = FakePage::default();
            let mut backend = FakeBackend::token();
            if let Ok(result) = &mut backend.answer {
                result.answer = answer;
            }
            let result = finish(&page, &backend).await?;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::NeedsHuman, SolveReason::ProviderError)
            );
            assert_eq!(result.paid_tasks, 1);
            assert!(page.state().applied.is_none());
        }
        Ok(())
    }

    struct FixturePage {
        client: reqwest::Client,
        endpoint: String,
        session: String,
    }

    impl FixturePage {
        async fn call<T: serde::de::DeserializeOwned>(
            &self,
            op: &str,
            data: Value,
        ) -> Result<T, PageError> {
            let response = self
                .client
                .post(&self.endpoint)
                .json(&json!({"op": op, "session": self.session, "data": data}))
                .send()
                .await
                .map_err(|_| PageError::Browser("fixture bridge unavailable".into()))?;
            if !response.status().is_success() {
                return Err(PageError::Browser("fixture bridge operation failed".into()));
            }
            response
                .json()
                .await
                .map_err(|_| PageError::Browser("invalid fixture bridge response".into()))
        }
    }

    impl CaptchaPage for FixturePage {
        async fn eval_main(&self, expression: &str) -> Result<Value, PageError> {
            self.call("eval", json!({"expression": expression})).await
        }
        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            self.call("frames", json!({})).await
        }
        async fn eval_in_frame(
            &self,
            frame: &FrameRef,
            expression: &str,
        ) -> Result<Value, PageError> {
            self.call(
                "frame_eval",
                json!({"frame": frame, "expression": expression}),
            )
            .await
        }
        async fn query_rect_in_frame(
            &self,
            frame: &FrameRef,
            selector: &str,
        ) -> Result<Option<Rect>, PageError> {
            self.call("rect", json!({"frame": frame, "selector": selector}))
                .await
        }
        async fn click_at(&self, x: f64, y: f64) -> Result<(), PageError> {
            self.call("click", json!({"x": x, "y": y})).await
        }
        async fn screenshot_clip(&self, _: Rect) -> Result<String, PageError> {
            Err(PageError::Browser("unexpected screenshot".into()))
        }
        async fn document(&self) -> Result<DocumentId, PageError> {
            self.call("document", json!({})).await
        }
    }

    #[tokio::test]
    #[ignore = "requires an owned dev browser and local CAPTCHA_TURNSTILE_LIVE_BRIDGE"]
    async fn tests_that_live_cloudflare_test_keys_follow_the_free_paths() -> anyhow::Result<()> {
        let endpoint = std::env::var("CAPTCHA_TURNSTILE_LIVE_BRIDGE")?;
        let client = reqwest::Client::new();
        for (route, method, reason) in [
            (
                "turnstile-auto",
                SolveMethod::AutoPass,
                SolveReason::AutoPassed,
            ),
            (
                "turnstile-force",
                SolveMethod::Checkbox,
                SolveReason::CheckboxPassed,
            ),
            (
                "turnstile-block",
                SolveMethod::AutoPass,
                SolveReason::NoProvider,
            ),
        ] {
            let mut page = FixturePage {
                client: client.clone(),
                endpoint: endpoint.clone(),
                session: String::new(),
            };
            page.session = page.call("new", json!({"route": route})).await?;
            let document = page.document().await?;
            let backend = FakeBackend::error(ProviderErrorKind::NotConfigured);
            let reservations = Arc::<TabReservations>::default();
            let lease = reservations
                .try_reserve(42, "cap_live")
                .map_err(|_| anyhow::anyhow!("unexpected busy page"))?;
            let events = EventLog::new(CaptchaEventBus::new(), "cap_live".into(), 42);
            let mut challenge = challenge();
            challenge.sitekey = None;
            let result = solve(
                &SolveCtx {
                    page: &page,
                    backend: &backend,
                    events: &events,
                    lease: &lease,
                    cancel: &CancellationToken::new(),
                    deadline: Instant::now() + Duration::from_secs(90),
                    page_url: document.url.clone(),
                    document,
                    user_agent: None,
                    forced_method: None,
                },
                &challenge,
            )
            .await;
            page.call::<Value>(
                "record",
                json!({"route": route, "status": result.status,
                "method": result.method, "reason": result.reason, "attempts": result.attempts,
                "paidTasks": result.paid_tasks}),
            )
            .await?;
            assert_eq!(result.reason, reason, "{route}: {}", result.message);
            if route != "turnstile-block" {
                assert_eq!(result.method, method);
            }
            assert_eq!(result.paid_tasks, 0);
            assert!(backend.tasks().is_empty());
        }
        Ok(())
    }
}
