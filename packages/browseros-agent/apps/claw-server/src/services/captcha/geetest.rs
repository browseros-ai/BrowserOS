use std::{collections::BTreeMap, future::Future};

use serde_json::{Value, json};
use tokio::time::{Duration, Instant, sleep_until};

use super::{
    SolveCtx,
    page::CaptchaPage,
    providers::CaptchaBackend,
    types::{
        CaptchaEventName, CaptchaKind, Challenge, FlowResult, GeetestV3Params, GeetestV4Params,
        PageError, ProviderError, ProviderErrorKind, SolveAnswer, SolveMethod, SolveReason,
        SolveStatus, SolveStep, SolveTask,
    },
};

pub const SUPPORTED: bool = true;
pub(super) const CAPSOLVER_ESTIMATE_USD: f64 = 0.0012;

const WIDGET: &str = r#"
const roots = [document];
for (const root of roots) for (const e of root.querySelectorAll('*')) if (e.shadowRoot) roots.push(e.shadowRoot);
const all = selector => roots.flatMap(root => [...root.querySelectorAll(selector)]);
const visible = e => !!e && e.isConnected && e.getBoundingClientRect().width > 0
    && e.getBoundingClientRect().height > 0 && getComputedStyle(e).visibility !== 'hidden';
const containers = all('.geetest_holder, .geetest_captcha').filter(visible);
const widgets = containers.filter(e => !containers.some(parent => parent !== e && parent.contains(e)));
const widget = widgets.length === 1 ? widgets[0] : null;
const resources = performance.getEntriesByType('resource');
const urls = resources.map(e => { try { return new URL(e.name); } catch { return null; } }).filter(Boolean);
const gee = u => /(^|\.)(geetest\.com|geevisit\.com)$/.test(u.hostname);
const v4 = typeof window.initGeetest4 === 'function' || urls.some(u => gee(u) && (/\/gt4\.js$/.test(u.pathname) || u.hostname === 'gcaptcha4.geetest.com'));
const version = v4 ? 4 : 3;
const load = [...urls].reverse().find(u => gee(u) && (v4
    ? u.pathname === '/load' && u.searchParams.has('captcha_id')
    : /\/(gettype|get|ajax)\.php$/.test(u.pathname) && u.searchParams.has('gt')));
const identity = load?.href ?? null;
const failed = all('[class*="geetest_error"], [data-error-code], [aria-invalid="true"]').some(visible);
const ready = !!widget && (v4 ? widget.classList.contains('geetest_nextReady')
    : widget.classList.contains('geetest_ready') || widget.classList.contains('geetest_detect'));
const state = {version, href: location.href, identity, live: !!widget && !!load && !failed,
    loading: version === 3 && !!widget && !failed && !ready,
    gt: load?.searchParams.get('gt'), challenge: load?.searchParams.get('challenge'),
    captchaId: load?.searchParams.get('captcha_id'), riskType: load?.searchParams.get('risk_type'),
    apiServer: load && load.hostname !== 'api.geetest.com' ? load.hostname : null};
"#;

const READ: &str = r#"
/* geetest:read */
if (args.fresh && state.live && version === 3) {
    const first = resources.findIndex(e => { try { const u = new URL(e.name);
        return gee(u) && /\/(gettype|get)\.php$/.test(u.pathname) && u.searchParams.get('gt') === state.gt;
    } catch { return false; } });
    const candidates = resources.slice(0, Math.max(first, 0)).filter(e =>
        ['fetch', 'xmlhttprequest'].includes(e.initiatorType) && /geetest|gee-test|captcha|register|challenge/i.test(e.name))
        .reverse().slice(0, 3);
    state.live = false;
    for (const entry of candidates) {
        try {
            const response = await fetch(entry.name, {credentials: 'include', cache: 'no-store', signal: AbortSignal.timeout(500)});
            if (!response.ok) continue;
            const raw = await response.json(), data = raw?.data?.challenge ? raw.data : raw;
            if (data?.gt !== state.gt || !/^[0-9a-f]{32}$/i.test(data.challenge ?? '')) continue;
            if (data.success === 0 || data.success === false) break;
            if (data.challenge === state.challenge) continue;
            state.challenge = data.challenge;
            state.live = true;
            break;
        } catch {}
    }
}
return state;
"#;

const APPLY: &str = r#"
/* geetest:apply */
if (!state.live || state.identity !== args.identity || state.href !== args.href) return {written: false};
const answer = args.answer;
let written = false;
if (version === 3) {
    const form = widget?.closest('form') || all('input[name="geetest_validate"]')[0]?.form
        || (all('form').length === 1 ? all('form')[0] : null);
    if (form) {
        for (const key of ['challenge', 'validate', 'seccode']) {
            const name = 'geetest_' + key;
            let input = form.querySelector('input[name="' + name + '"]');
            if (!input) { input = document.createElement('input'); input.type = 'hidden'; input.name = name; form.append(input); }
            input.value = answer[key];
            input.dispatchEvent(new Event('input', {bubbles: true}));
            input.dispatchEvent(new Event('change', {bubbles: true}));
        }
        written = ['challenge', 'validate', 'seccode'].every(key => form.querySelector('input[name="geetest_' + key + '"]')?.value === answer[key]);
    }
} else {
    const prototype = window.Geetest4?.prototype;
    if (prototype && typeof prototype.getValidate === 'function') {
        prototype.getValidate = function() { return {...answer}; };
        written = Object.keys(answer).every(key => prototype.getValidate()[key] === answer[key]);
    }
}
let callbackRan = false;
if (written && args.callback) {
    try {
        const parts = args.callback.split('.'), key = parts.pop();
        let owner = window;
        for (const part of parts) owner = owner?.[part];
        if (typeof owner?.[key] === 'function') { await owner[key](answer); callbackRan = true; }
    } catch {}
}
return {written, callbackDeclared: !!args.callback, callbackRan};
"#;

const VERIFY: &str = r#"
/* geetest:verify */
let retained = false;
if (args.version === 3) {
    retained = ['challenge', 'validate', 'seccode'].every(key => all('input[name="geetest_' + key + '"]')
        .some(input => input.value === args.answer[key]));
} else {
    try { const value = window.Geetest4?.prototype.getValidate();
        retained = Object.keys(args.answer).every(key => value?.[key] === args.answer[key]); } catch {}
}
const captchaFree = !all('[class*="geetest_"], .g-recaptcha, .h-captcha, .cf-turnstile, awswaf-captcha, iframe[src*="captcha"], iframe[src*="geetest"]').some(visible);
return {retained, accepted: !widget && captchaFree, captchaFree, ready: document.readyState === 'complete'};
"#;

fn script(body: &str, args: &Value) -> String {
    format!("(async () => {{ const args = {args}; {WIDGET} {body} }})()")
}

pub async fn solve<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
) -> FlowResult {
    let mut result = super::terminal(
        SolveStatus::Failed,
        SolveReason::BrowserError,
        "GeeTest stopped.",
    );
    let (status, reason, message) = match run(ctx, challenge, &mut result).await {
        Ok(terminal) => terminal,
        Err(error) => (
            match error {
                PageError::Closed | PageError::Navigated | PageError::Cancelled => {
                    SolveStatus::Cancelled
                }
                _ => SolveStatus::Failed,
            },
            match error {
                PageError::Closed => SolveReason::TabClosed,
                PageError::Navigated => SolveReason::Navigated,
                PageError::Cancelled => SolveReason::CallerCancelled,
                PageError::Timeout => SolveReason::DeadlineExceeded,
                PageError::Browser(_) => SolveReason::BrowserError,
            },
            "GeeTest stopped; take a fresh snapshot.".into(),
        ),
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
    tokio::select! { biased;
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

async fn eval<P: CaptchaPage, B: CaptchaBackend>(
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
    fresh: bool,
) -> Result<Value, PageError> {
    guard(ctx).await?;
    let value = eval(ctx, challenge, READ, &json!({"fresh":fresh})).await?;
    guard(ctx).await?;
    Ok(value)
}

async fn ready<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
) -> Result<Value, PageError> {
    let until = Instant::now() + Duration::from_secs(8);
    let mut state = read(ctx, challenge, false).await?;
    while flag(&state, "loading") && Instant::now() < until {
        bounded(ctx, async {
            sleep_until(until.min(Instant::now() + Duration::from_millis(250))).await;
            Ok(())
        })
        .await?;
        match tokio::time::timeout_at(until, read(ctx, challenge, false)).await {
            Ok(value) => state = value?,
            Err(_) if Instant::now() >= ctx.deadline => return Err(PageError::Timeout),
            Err(_) => break,
        }
    }
    if flag(&state, "loading") {
        state["live"] = json!(false);
    }
    Ok(state)
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}
fn hex_id(value: &str) -> bool {
    value.len() == 32
        && value.bytes().all(|b| b.is_ascii_hexdigit())
        && value.bytes().any(|b| b != b'0')
}

fn params(state: &Value, user_agent: Option<String>) -> Option<SolveTask> {
    if !flag(state, "live") {
        return None;
    }
    let website_url = text(state, "href")?.to_owned();
    let api_server = text(state, "apiServer").map(str::to_owned);
    let task = if state["version"] == 4 {
        SolveTask::GeetestV4(GeetestV4Params {
            website_url,
            captcha_id: text(state, "captchaId")?.into(),
            risk_type: text(state, "riskType").map(str::to_owned),
            api_server,
            user_agent,
        })
    } else if state["version"] == 3 {
        SolveTask::GeetestV3(GeetestV3Params {
            website_url,
            gt: text(state, "gt")?.into(),
            challenge: text(state, "challenge")?.into(),
            api_server,
            user_agent,
        })
    } else {
        return None;
    };
    capsolver_task(&task).ok()?;
    Some(task)
}

fn unchanged(before: &Value, after: &Value) -> bool {
    ["version", "href", "identity", "gt", "captchaId"]
        .iter()
        .all(|key| before.get(key) == after.get(key))
}

fn fresh_task<'a, P: CaptchaPage, B: CaptchaBackend>(
    ctx: &'a SolveCtx<'_, P, B>,
    challenge: &'a Challenge,
    state: &'a Value,
    attempts: &'a std::sync::atomic::AtomicU32,
    error: &'a std::sync::Mutex<Option<PageError>>,
) -> futures_util::future::BoxFuture<'a, Option<SolveTask>> {
    Box::pin(async move {
        match read(ctx, challenge, true).await {
            Ok(fresh) if unchanged(state, &fresh) => {
                let task = params(&fresh, ctx.user_agent.clone())?;
                let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                ctx.lease
                    .set_progress(Some(CaptchaKind::Geetest), SolveStep::Token);
                ctx.events.emit(
                    CaptchaEventName::Solving,
                    json!({"type":"geetest","method":"token","attempt":attempt}),
                );
                Some(task)
            }
            Ok(_) => None,
            Err(value) => {
                *error.lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
                None
            }
        }
    })
}

fn fresh_source<'a, P: CaptchaPage, B: CaptchaBackend>(
    ctx: &'a SolveCtx<'_, P, B>,
    challenge: &'a Challenge,
    state: &'a Value,
    attempts: &'a std::sync::atomic::AtomicU32,
    error: &'a std::sync::Mutex<Option<PageError>>,
) -> Box<dyn FnMut() -> futures_util::future::BoxFuture<'a, Option<SolveTask>> + Send + 'a> {
    Box::new(move || fresh_task(ctx, challenge, state, attempts, error))
}

fn purchase<'a, P: CaptchaPage, B: CaptchaBackend>(
    ctx: &'a SolveCtx<'_, P, B>,
    challenge: &'a Challenge,
    state: &'a Value,
    attempts: &'a std::sync::atomic::AtomicU32,
    error: &'a std::sync::Mutex<Option<PageError>>,
) -> futures_util::future::BoxFuture<'a, Result<super::types::ProviderAnswer, ProviderError>> {
    let source = fresh_source(ctx, challenge, state, attempts, error);
    Box::pin(ctx.backend.solve_fresh(source, ctx.deadline, ctx.cancel))
}

async fn run<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
    result: &mut FlowResult,
) -> Result<Terminal, PageError> {
    let state = ready(ctx, challenge).await?;
    if params(&state, ctx.user_agent.clone()).is_none() {
        return Ok((
            SolveStatus::NeedsHuman,
            SolveReason::ChallengeNotPassed,
            "GeeTest has no usable live parameters; ask for human help.".into(),
        ));
    }
    if !ctx.backend.is_configured() {
        return Ok((
            SolveStatus::NeedsHuman,
            SolveReason::NoProvider,
            "No GeeTest provider is configured; ask for human help.".into(),
        ));
    }
    let fresh_error = std::sync::Mutex::new(None);
    let attempts = std::sync::atomic::AtomicU32::new(0);
    let answer = bounded(ctx, async {
        Ok(purchase(ctx, challenge, &state, &attempts, &fresh_error).await)
    })
    .await;
    result.attempts.token = attempts.load(std::sync::atomic::Ordering::Relaxed);
    if result.attempts.token > 0 {
        result.method = SolveMethod::Token;
        result.paid_tasks = 1;
    }
    let answer = answer?;
    if let Some(error) = fresh_error.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(error);
    }
    // Re-read without registering again: the vendor consumed the fresh v3 challenge.
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            result.paid_tasks = u32::from(error.charged);
            if error.kind == ProviderErrorKind::BadRequest
                && error.provider.is_none()
                && !error.charged
            {
                return Ok((
                    SolveStatus::NeedsHuman,
                    SolveReason::ChallengeNotPassed,
                    "Fresh GeeTest parameters are unavailable; ask for human help.".into(),
                ));
            }
            return Ok(provider_failure(error));
        }
    };
    result.paid_tasks = 1;
    result.cost_usd = answer
        .cost_usd
        .filter(|c| c.is_finite() && *c >= 0.0)
        .unwrap_or(0.0);
    guard(ctx).await?;
    let task = params(&state, ctx.user_agent.clone())
        .ok_or_else(|| PageError::Browser("GeeTest parameters changed".into()))?;
    let SolveAnswer::Fields(fields) = answer.answer else {
        return Ok((
            SolveStatus::NeedsHuman,
            SolveReason::ProviderError,
            "Provider returned an unusable GeeTest answer.".into(),
        ));
    };
    let Some(SolveAnswer::Fields(fields)) = capsolver_answer(&task, &json!(fields)) else {
        return Ok((
            SolveStatus::NeedsHuman,
            SolveReason::ProviderError,
            "Provider returned incomplete GeeTest fields.".into(),
        ));
    };
    let fresh = read(ctx, challenge, false).await?;
    if !unchanged(&state, &fresh) || !flag(&fresh, "live") {
        return Ok(unconfirmed());
    }
    let mut args = state;
    args["answer"] = json!(fields);
    args["callback"] = json!(challenge.callback);
    ctx.lease
        .set_progress(Some(CaptchaKind::Geetest), SolveStep::Verifying);
    guard(ctx).await?;
    let applied = eval(ctx, challenge, APPLY, &args).await?;
    result.callback_ran = flag(&applied, "callbackRan");
    if !flag(&applied, "written") || (flag(&applied, "callbackDeclared") && !result.callback_ran) {
        return Ok(unconfirmed());
    }
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let document = bounded(ctx, ctx.page.document()).await?;
        let evidence = eval(ctx, challenge, VERIFY, &args).await?;
        if bounded(ctx, ctx.page.document()).await? != document {
            return Err(PageError::Navigated);
        }
        if document != ctx.document {
            if challenge.host_frame.is_some()
                || !flag(&evidence, "ready")
                || !flag(&evidence, "captchaFree")
            {
                return Err(PageError::Navigated);
            }
        } else if !flag(&evidence, "accepted") {
            if !flag(&evidence, "retained") {
                return Ok(unconfirmed());
            }
            if Instant::now() >= until {
                return Ok((
                    SolveStatus::TokenApplied,
                    SolveReason::AwaitingSubmit,
                    super::token_applied_message(result.callback_ran),
                ));
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
            "The page accepted the GeeTest answer; continue.".into(),
        ));
    }
}

fn unconfirmed() -> Terminal {
    (
        SolveStatus::Failed,
        SolveReason::AcceptanceUnconfirmed,
        "GeeTest acceptance is unconfirmed; take a fresh snapshot.".into(),
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
            "GeeTest provider unavailable: {}; ask for human help.",
            error.message
        ),
    )
}

pub(super) fn capsolver_task(task: &SolveTask) -> Result<Value, String> {
    payload(task, false)
}
pub(super) fn twocaptcha_task(task: &SolveTask) -> Result<Value, String> {
    payload(task, true)
}

fn payload(task: &SolveTask, twocaptcha: bool) -> Result<Value, String> {
    let kind = if twocaptcha {
        "GeeTestTaskProxyless"
    } else {
        "GeeTestTaskProxyLess"
    };
    let (mut payload, website_url) = match task {
        SolveTask::GeetestV3(p) if hex_id(&p.gt) && hex_id(&p.challenge) => {
            let mut value =
                json!({"type":kind,"websiteURL":p.website_url,"gt":p.gt,"challenge":p.challenge});
            super::providers::optional(&mut value, "geetestApiServerSubdomain", &p.api_server);
            if twocaptcha {
                super::providers::optional(&mut value, "userAgent", &p.user_agent);
            }
            (value, &p.website_url)
        }
        SolveTask::GeetestV4(p) if hex_id(&p.captcha_id) => {
            let mut value = if twocaptcha {
                json!({"type":kind,"websiteURL":p.website_url,"version":4,"initParameters":{"captcha_id":p.captcha_id}})
            } else {
                json!({"type":kind,"websiteURL":p.website_url,"captchaId":p.captcha_id})
            };
            super::providers::optional(&mut value, "riskType", &p.risk_type);
            (value, &p.website_url)
        }
        _ => return Err("invalid GeeTest parameters".into()),
    };
    let url = url::Url::parse(website_url).map_err(|_| "invalid GeeTest website URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("invalid GeeTest website URL".into());
    }
    // Keep the API's task-type capitalization, which differs between vendors.
    payload["type"] = json!(kind);
    Ok(payload)
}

pub(super) fn capsolver_answer(task: &SolveTask, solution: &Value) -> Option<SolveAnswer> {
    let required: &[&str] = match task {
        SolveTask::GeetestV3(_) => &["challenge", "validate", "seccode"],
        SolveTask::GeetestV4(p) if text(solution, "captcha_id") == Some(p.captcha_id.as_str()) => {
            &[
                "captcha_id",
                "lot_number",
                "pass_token",
                "gen_time",
                "captcha_output",
            ]
        }
        _ => return None,
    };
    let mut fields = BTreeMap::new();
    for key in required {
        let value = if *key == "gen_time" && solution.get(key)?.is_u64() {
            solution.get(key)?.as_u64()?.to_string()
        } else {
            text(solution, key)?.into()
        };
        fields.insert((*key).into(), value);
    }
    Some(SolveAnswer::Fields(fields))
}

pub(super) fn twocaptcha_answer(task: &SolveTask, solution: &Value) -> Option<SolveAnswer> {
    capsolver_answer(task, solution)
}

pub(super) fn secrets(task: &SolveTask) -> Vec<&str> {
    match task {
        SolveTask::GeetestV3(p) => vec![
            &p.website_url,
            &p.gt,
            &p.challenge,
            p.api_server.as_deref().unwrap_or(""),
            p.user_agent.as_deref().unwrap_or(""),
        ],
        SolveTask::GeetestV4(p) => vec![
            &p.website_url,
            &p.captcha_id,
            p.risk_type.as_deref().unwrap_or(""),
            p.api_server.as_deref().unwrap_or(""),
            p.user_agent.as_deref().unwrap_or(""),
        ],
        _ => vec![],
    }
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
            CaptchaKind, ChallengeState, DocumentId, FrameRef, GeetestV3Params, GeetestV4Params,
            PageError, ProviderAnswer, ProviderError, ProviderErrorKind, ProviderName, Rect,
            SolveMethod,
        },
    };

    const ID: &str = "81388ea1fc187e0c335c0a8907ff2625";
    const FRESH: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn tests_that_geetest_is_supported() {
        assert!(CaptchaKind::Geetest.is_supported());
    }

    fn task(version: u8) -> SolveTask {
        if version == 3 {
            SolveTask::GeetestV3(GeetestV3Params {
                website_url: "https://fixture.test/widget".into(),
                gt: ID.into(),
                challenge: FRESH.into(),
                api_server: None,
                user_agent: None,
            })
        } else {
            SolveTask::GeetestV4(GeetestV4Params {
                website_url: "https://fixture.test/widget".into(),
                captcha_id: ID.into(),
                risk_type: None,
                api_server: None,
                user_agent: None,
            })
        }
    }

    fn solution(version: u8) -> Value {
        if version == 3 {
            json!({"challenge": format!("{FRESH}ab"), "validate": "vendor-validate",
                "seccode": "vendor-validate|jordan", "userAgent": "ignored"})
        } else {
            json!({"captcha_id": ID, "lot_number": "vendor-lot", "pass_token": "vendor-pass",
                "gen_time": "1693924478", "captcha_output": "vendor-output", "risk_type": "slide",
                "userAgent": "ignored"})
        }
    }

    #[test]
    fn tests_that_both_vendors_receive_exact_versioned_payloads() -> anyhow::Result<()> {
        for (version, capsolver, twocaptcha) in [
            (
                3,
                json!({"type":"GeeTestTaskProxyLess", "websiteURL":"https://fixture.test/widget", "gt":ID, "challenge":FRESH}),
                json!({"type":"GeeTestTaskProxyless", "websiteURL":"https://fixture.test/widget", "gt":ID, "challenge":FRESH}),
            ),
            (
                4,
                json!({"type":"GeeTestTaskProxyLess", "websiteURL":"https://fixture.test/widget", "captchaId":ID}),
                json!({"type":"GeeTestTaskProxyless", "websiteURL":"https://fixture.test/widget", "version":4, "initParameters":{"captcha_id":ID}}),
            ),
        ] {
            assert_eq!(
                capsolver_task(&task(version)).map_err(anyhow::Error::msg)?,
                capsolver
            );
            assert_eq!(
                twocaptcha_task(&task(version)).map_err(anyhow::Error::msg)?,
                twocaptcha
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_optional_vendor_fields_are_preserved_or_omitted() -> anyhow::Result<()> {
        let mut v3 = task(3);
        if let SolveTask::GeetestV3(params) = &mut v3 {
            params.api_server = Some("api-na.geetest.com".into());
            params.user_agent = Some("browser-agent".into());
        }
        let cap = capsolver_task(&v3).map_err(anyhow::Error::msg)?;
        let two = twocaptcha_task(&v3).map_err(anyhow::Error::msg)?;
        assert_eq!(cap["geetestApiServerSubdomain"], "api-na.geetest.com");
        assert!(cap.get("userAgent").is_none());
        assert_eq!(two["userAgent"], "browser-agent");
        if let SolveTask::GeetestV3(params) = &mut v3 {
            params.api_server = Some("  ".into());
            params.user_agent = Some(String::new());
        }
        assert_eq!(capsolver_task(&v3), capsolver_task(&task(3)));
        assert_eq!(twocaptcha_task(&v3), twocaptcha_task(&task(3)));
        let mut v4 = task(4);
        if let SolveTask::GeetestV4(params) = &mut v4 {
            params.risk_type = Some("slide".into());
        }
        assert_eq!(
            capsolver_task(&v4).map_err(anyhow::Error::msg)?["riskType"],
            "slide"
        );
        Ok(())
    }

    #[test]
    fn tests_that_vendor_answers_keep_only_geetest_fields_and_returned_challenge()
    -> anyhow::Result<()> {
        for version in [3, 4] {
            for normalize in [capsolver_answer, twocaptcha_answer] {
                let answer = normalize(&task(version), &solution(version));
                let Some(SolveAnswer::Fields(fields)) = answer else {
                    anyhow::bail!("missing normalized fields");
                };
                assert_eq!(fields.len(), if version == 3 { 3 } else { 5 });
                assert!(!fields.contains_key("userAgent"));
                assert!(!fields.contains_key("risk_type"));
                if version == 3 {
                    assert_eq!(fields.get("challenge"), Some(&format!("{FRESH}ab")));
                } else {
                    assert_eq!(
                        fields.get("gen_time").map(String::as_str),
                        Some("1693924478")
                    );
                }
            }
        }
        let mut value = solution(4);
        value["gen_time"] = json!(1693924478);
        assert!(capsolver_answer(&task(4), &value).is_some());
        Ok(())
    }

    #[test]
    fn tests_that_incomplete_and_mismatched_vendor_answers_are_rejected() {
        for version in [3, 4] {
            for normalize in [capsolver_answer, twocaptcha_answer] {
                for field in if version == 3 {
                    vec!["challenge", "validate", "seccode"]
                } else {
                    vec![
                        "captcha_id",
                        "lot_number",
                        "pass_token",
                        "gen_time",
                        "captcha_output",
                    ]
                } {
                    for invalid in [Value::Null, json!("  "), json!({})] {
                        let mut value = solution(version);
                        value[field] = invalid;
                        assert!(normalize(&task(version), &value).is_none());
                    }
                }
            }
        }
        let mut value = solution(4);
        value["captcha_id"] = json!(FRESH);
        assert!(capsolver_answer(&task(4), &value).is_none());
    }

    #[test]
    fn tests_that_invalid_task_parameters_never_reach_a_vendor() {
        for version in [3, 4] {
            let mut invalid = task(version);
            match &mut invalid {
                SolveTask::GeetestV3(p) => p.challenge.clear(),
                SolveTask::GeetestV4(p) => p.captcha_id = "0".repeat(32),
                _ => unreachable!(),
            }
            assert!(capsolver_task(&invalid).is_err());
            assert!(twocaptcha_task(&invalid).is_err());
            assert!(!secrets(&task(version)).is_empty());
        }
    }

    struct State {
        read: Value,
        applied: Option<String>,
        retained: bool,
        accepted: bool,
        callback_declared: bool,
        callback_ran: bool,
        written: bool,
        navigated: bool,
        frame_calls: usize,
        top_calls: usize,
        delay: Duration,
        ready_at: Option<usize>,
        reads: usize,
    }

    struct FakePage(Arc<Mutex<State>>);
    impl FakePage {
        fn new(version: u8) -> Self {
            Self(Arc::new(Mutex::new(State {
                read: json!({"version":version, "href":"https://fixture.test/widget", "identity":"loaded-widget",
                    "gt":ID, "challenge":FRESH, "captchaId":ID, "live":true}),
                applied: None,
                retained: true,
                accepted: false,
                callback_declared: false,
                callback_ran: false,
                written: true,
                navigated: false,
                frame_calls: 0,
                top_calls: 0,
                delay: Duration::ZERO,
                ready_at: None,
                reads: 0,
            })))
        }
        fn state(&self) -> MutexGuard<'_, State> {
            self.0.lock().unwrap_or_else(|e| e.into_inner())
        }
        async fn eval(&self, expression: &str) -> Result<Value, PageError> {
            let delay = self.state().delay;
            tokio::time::sleep(delay).await;
            let mut s = self.state();
            if expression.contains("geetest:apply") {
                s.applied = Some(expression.into());
                Ok(
                    json!({"written":s.written, "callbackDeclared":s.callback_declared, "callbackRan":s.callback_ran}),
                )
            } else if expression.contains("geetest:verify") {
                Ok(
                    json!({"retained":s.retained, "accepted":s.accepted, "ready":true, "captchaFree":s.accepted}),
                )
            } else {
                s.reads += 1;
                if s.ready_at.is_some_and(|at| s.reads >= at) {
                    s.read["live"] = json!(true);
                    s.read["loading"] = json!(false);
                }
                Ok(s.read.clone())
            }
        }
    }
    fn document() -> DocumentId {
        DocumentId {
            url: "https://fixture.test/top".into(),
            loader_id: "original".into(),
        }
    }
    impl CaptchaPage for FakePage {
        async fn eval_main(&self, expression: &str) -> Result<Value, PageError> {
            self.state().top_calls += 1;
            self.eval(expression).await
        }
        async fn eval_in_frame_main(
            &self,
            frame: &FrameRef,
            expression: &str,
        ) -> Result<Value, PageError> {
            assert_eq!(frame.frame_id, "host");
            self.state().frame_calls += 1;
            self.eval(expression).await
        }
        async fn document(&self) -> Result<DocumentId, PageError> {
            let mut doc = document();
            if self.state().navigated {
                doc.loader_id = "next".into();
            }
            Ok(doc)
        }
        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            Ok(vec![])
        }
        async fn eval_in_frame(&self, _: &FrameRef, _: &str) -> Result<Value, PageError> {
            panic!("must use main world")
        }
        async fn query_rect_in_frame(
            &self,
            _: &FrameRef,
            _: &str,
        ) -> Result<Option<Rect>, PageError> {
            Ok(None)
        }
        async fn click_at(&self, _: f64, _: f64) -> Result<(), PageError> {
            panic!("unexpected click")
        }
        async fn screenshot_clip(&self, _: Rect) -> Result<String, PageError> {
            panic!("unexpected screenshot")
        }
    }

    struct FakeBackend {
        answer: Result<ProviderAnswer, ProviderError>,
        calls: Mutex<Vec<SolveTask>>,
        change: Option<(&'static str, Arc<Mutex<State>>)>,
    }
    impl FakeBackend {
        fn new(version: u8) -> Self {
            let fields = solution(version)
                .as_object()
                .into_iter()
                .flat_map(|v| v.iter())
                .filter(|(k, _)| k.as_str() != "userAgent" && k.as_str() != "risk_type")
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.into())))
                .collect();
            Self {
                answer: Ok(ProviderAnswer {
                    answer: SolveAnswer::Fields(fields),
                    provider: ProviderName::Capsolver,
                    task_id: Some("task".into()),
                    cost_usd: Some(0.0012),
                }),
                calls: Mutex::default(),
                change: None,
            }
        }
    }
    impl CaptchaBackend for FakeBackend {
        fn is_configured(&self) -> bool {
            !matches!(&self.answer,Err(e) if e.kind==ProviderErrorKind::NotConfigured)
        }
        async fn solve(
            &self,
            task: &SolveTask,
            _: Instant,
            _: &CancellationToken,
        ) -> Result<ProviderAnswer, ProviderError> {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(task.clone());
            if let Some((field, state)) = &self.change {
                let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
                if *field == "navigation" {
                    state.navigated = true;
                } else {
                    state.read[*field] = json!("changed");
                }
            }
            self.answer.clone()
        }
    }
    async fn run(
        page: &FakePage,
        backend: &FakeBackend,
        frame: bool,
        cancel: &CancellationToken,
    ) -> anyhow::Result<FlowResult> {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "cap_test")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        let events = EventLog::new(CaptchaEventBus::new(), "cap_test".into(), 42);
        let challenge = Challenge {
            kind: CaptchaKind::Geetest,
            state: ChallengeState::Active,
            frame: None,
            challenge_frame: None,
            host_frame: frame.then(|| FrameRef {
                frame_id: "host".into(),
                url: "https://fixture.test/widget".into(),
                rect: None,
            }),
            sitekey: None,
            invisible: false,
            enterprise: false,
            recaptcha_s: None,
            action: None,
            cdata: None,
            callback: None,
        };
        Ok(solve(
            &SolveCtx {
                page,
                backend,
                events: &events,
                lease: &lease,
                cancel,
                deadline: Instant::now() + Duration::from_secs(20),
                page_url: document().url,
                document: document(),
                user_agent: None,
                forced_method: None,
            },
            &challenge,
        )
        .await)
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_acceptance_and_retained_answers_have_honest_statuses() -> anyhow::Result<()>
    {
        for version in [3, 4] {
            for accepted in [true, false] {
                let page = FakePage::new(version);
                page.state().accepted = accepted;
                let backend = FakeBackend::new(version);
                let result = run(&page, &backend, false, &CancellationToken::new()).await?;
                assert_eq!(
                    (result.status, result.reason),
                    if accepted {
                        (SolveStatus::Solved, SolveReason::TokenAccepted)
                    } else {
                        (SolveStatus::TokenApplied, SolveReason::AwaitingSubmit)
                    }
                );
                assert_eq!(result.method, SolveMethod::Token);
                assert_eq!(result.paid_tasks, 1);
                assert_eq!(result.cost_usd, 0.0012);
                assert_eq!(result.attempts.token, 1);
                let calls = backend.calls.lock().unwrap_or_else(|e| e.into_inner());
                assert_eq!(calls.as_slice(), [task(version)]);
                let state = page.state();
                assert!(
                    state
                        .applied
                        .as_deref()
                        .is_some_and(|s| s.contains(if version == 3 {
                            "vendor-validate"
                        } else {
                            "vendor-lot"
                        }))
                );
                assert!(!result.message.contains("vendor-"));
            }
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_v3_waits_for_sdk_readiness_before_registering_or_paying()
    -> anyhow::Result<()> {
        let page = FakePage::new(3);
        {
            let mut state = page.state();
            state.read["live"] = json!(false);
            state.read["loading"] = json!(true);
            state.ready_at = Some(3);
        }
        let backend = FakeBackend::new(3);
        let result = run(&page, &backend, false, &CancellationToken::new()).await?;
        assert_eq!(result.status, SolveStatus::TokenApplied);
        assert_eq!(result.paid_tasks, 1);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_sdk_readiness_wait_is_bounded_without_payment() -> anyhow::Result<()> {
        let page = FakePage::new(3);
        page.state().read["live"] = json!(false);
        page.state().read["loading"] = json!(true);
        let backend = FakeBackend::new(3);
        let start = Instant::now();
        let result = run(&page, &backend, false, &CancellationToken::new()).await?;
        assert!(start.elapsed() >= Duration::from_secs(8));
        assert!(start.elapsed() <= Duration::from_secs(9));
        assert_eq!(result.reason, SolveReason::ChallengeNotPassed);
        assert_eq!(result.paid_tasks, 0);
        assert!(
            backend
                .calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_broken_widgets_and_missing_providers_never_pay() -> anyhow::Result<()> {
        for version in [3, 4] {
            for field in ["live", "challenge", "captchaId", "gt"] {
                if (version == 3 && field == "captchaId")
                    || (version == 4 && ["challenge", "gt"].contains(&field))
                {
                    continue;
                }
                let page = FakePage::new(version);
                page.state().read[field] = if field == "live" {
                    json!(false)
                } else {
                    json!("00000000000000000000000000000000")
                };
                let backend = FakeBackend::new(version);
                let result = run(&page, &backend, false, &CancellationToken::new()).await?;
                assert_eq!(
                    (result.status, result.reason),
                    (SolveStatus::NeedsHuman, SolveReason::ChallengeNotPassed)
                );
                assert_eq!(result.paid_tasks, 0);
                assert!(
                    backend
                        .calls
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_empty()
                );
            }
        }
        let mut backend = FakeBackend::new(3);
        backend.answer = Err(ProviderError {
            provider: None,
            kind: ProviderErrorKind::NotConfigured,
            message: "none".into(),
            charged: false,
        });
        let result = run(
            &FakePage::new(3),
            &backend,
            false,
            &CancellationToken::new(),
        )
        .await?;
        assert_eq!(result.reason, SolveReason::NoProvider);
        assert_eq!(result.paid_tasks, 0);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_frames_use_the_host_document_in_the_main_world() -> anyhow::Result<()> {
        for version in [3, 4] {
            let page = FakePage::new(version);
            let backend = FakeBackend::new(version);
            let result = run(&page, &backend, true, &CancellationToken::new()).await?;
            assert_eq!(result.status, SolveStatus::TokenApplied);
            assert!(page.state().frame_calls >= 4);
            assert_eq!(page.state().top_calls, 0);
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_stale_or_navigated_documents_reject_answers() -> anyhow::Result<()> {
        for field in ["identity", "href", "navigation"] {
            let page = FakePage::new(3);
            let mut backend = FakeBackend::new(3);
            backend.change = Some((field, Arc::clone(&page.0)));
            let result = run(&page, &backend, false, &CancellationToken::new()).await?;
            assert_eq!(
                result.reason,
                if field == "navigation" {
                    SolveReason::Navigated
                } else {
                    SolveReason::AcceptanceUnconfirmed
                }
            );
            assert_eq!(result.paid_tasks, 1);
            assert!(page.state().applied.is_none());
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_navigation_cancel_and_deadline_prevent_payment() -> anyhow::Result<()> {
        for mode in ["navigation", "cancel", "deadline"] {
            let page = FakePage::new(4);
            let backend = FakeBackend::new(4);
            let cancel = CancellationToken::new();
            match mode {
                "navigation" => page.state().navigated = true,
                "cancel" => cancel.cancel(),
                _ => page.state().delay = Duration::from_secs(60),
            }
            let result = run(&page, &backend, false, &cancel).await?;
            assert_eq!(
                result.reason,
                match mode {
                    "navigation" => SolveReason::Navigated,
                    "cancel" => SolveReason::CallerCancelled,
                    _ => SolveReason::DeadlineExceeded,
                }
            );
            assert_eq!(result.paid_tasks, 0);
            assert!(page.state().applied.is_none());
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_provider_errors_preserve_charge_and_reason() -> anyhow::Result<()> {
        for (kind, reason) in [
            (ProviderErrorKind::Auth, SolveReason::ProviderUnavailable),
            (
                ProviderErrorKind::SpendLimit,
                SolveReason::SpendLimitReached,
            ),
            (ProviderErrorKind::Unsolvable, SolveReason::ProviderError),
            (ProviderErrorKind::Cancelled, SolveReason::CallerCancelled),
        ] {
            for charged in [true, false] {
                let page = FakePage::new(3);
                let mut backend = FakeBackend::new(3);
                backend.answer = Err(ProviderError {
                    provider: Some(ProviderName::Capsolver),
                    kind,
                    message: "vendor error".into(),
                    charged,
                });
                let result = run(&page, &backend, false, &CancellationToken::new()).await?;
                assert_eq!(result.reason, reason);
                assert_eq!(result.paid_tasks, u32::from(charged));
                assert!(page.state().applied.is_none());
            }
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_failed_application_or_unrun_callbacks_cannot_claim_success()
    -> anyhow::Result<()> {
        for mode in ["write", "retained", "callback", "answer"] {
            let page = FakePage::new(4);
            let mut backend = FakeBackend::new(4);
            match mode {
                "write" => page.state().written = false,
                "retained" => page.state().retained = false,
                "callback" => page.state().callback_declared = true,
                _ => {
                    if let Ok(answer) = &mut backend.answer {
                        answer.answer = SolveAnswer::Token("wrong".into());
                    }
                }
            }
            let result = run(&page, &backend, false, &CancellationToken::new()).await?;
            assert_eq!(
                result.reason,
                if mode == "answer" {
                    SolveReason::ProviderError
                } else {
                    SolveReason::AcceptanceUnconfirmed
                }
            );
            assert_eq!(result.paid_tasks, 1);
        }
        Ok(())
    }
}
