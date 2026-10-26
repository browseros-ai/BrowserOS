use std::{
    future::Future,
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tokio::time::{Duration, Instant, sleep_until};

use super::{
    SolveCtx,
    page::CaptchaPage,
    providers::CaptchaBackend,
    types::{
        AwsWafParams, CaptchaEventName, CaptchaKind, Challenge, FlowResult, PageError,
        ProviderAnswer, ProviderError, ProviderErrorKind, SolveAnswer, SolveMethod, SolveReason,
        SolveStatus, SolveStep, SolveTask,
    },
};

pub const SUPPORTED: bool = true;
pub(super) const CAPSOLVER_ESTIMATE_USD: f64 = 0.002;
const VERIFY: Duration = Duration::from_secs(6);

const DOCUMENT: &str = r#"
const all = (selector, root = document) => {
    const found = [...root.querySelectorAll(selector)];
    for (const e of root.querySelectorAll('*')) if (e.shadowRoot) found.push(...all(selector, e.shadowRoot));
    return found;
};
const visible = e => !!e && e.isConnected && e.getBoundingClientRect().width > 0
    && e.getBoundingClientRect().height > 0 && e.checkVisibility({checkVisibilityCSS:true});
const awsUrl = (s, file) => {
    try { const u = new URL(s); return u.protocol === 'https:' && u.hostname.endsWith('.awswaf.com')
        && !u.username && !u.password && (!u.port || u.port === '443') && u.pathname.endsWith('/' + file); }
    catch { return false; }
};
const cookieValues = () => document.cookie.split(/;\s*/).filter(s => s.startsWith('aws-waf-token=')).map(s => s.slice(14));
const cookieSafe = s => typeof s === 'string' && /^[\x21\x23-\x2b\x2d-\x3a\x3c-\x5b\x5d-\x7e]+$/.test(s);
const documentStamp = () => JSON.stringify([location.href, performance.timeOrigin]);
function readState() {
    const resources = performance.getEntriesByType('resource').map(e => e.name).reverse();
    const urls = [...resources, ...all('script[src]').map(e => e.src)];
    const find = file => urls.find(s => awsUrl(s, file)) || null;
    const widgets = all('awswaf-captcha').filter(visible);
    const widget = widgets.length === 1 ? widgets[0] : null;
    const config = widget?.config;
    const interstitial = !!window.gokuProps && visible(document.querySelector('#captcha-container'));
    const problem = resources.find(s => awsUrl(s, 'problem') && (!config?.apiKey
        || new URL(s).searchParams.get('api_key') === config.apiKey)) || null;
    const p = window.gokuProps;
    const params = {website_url:location.href, key:p?.key ?? null, iv:p?.iv ?? null, context:p?.context ?? null,
        challenge_script:find('challenge.js'), captcha_script:find('captcha.js'), jsapi_script:find('jsapi.js'),
        api_key:config?.apiKey ?? (problem ? new URL(problem).searchParams.get('api_key') : null),
        existing_token:cookieValues()[0] || null, problem_url:problem};
    const puzzle = widget && all('canvas, .amzn-grid-focus, .amzn-captcha-audio-play-btn', widget.shadowRoot || widget).some(visible);
    return {params, interstitial, live:widgets.length <= 1 && (interstitial || !!puzzle),
        document:documentStamp(), stamp:JSON.stringify([documentStamp(), interstitial, params])};
}
"#;

const READ: &str = "/* aws-waf:read */ return readState();";
const APPLY: &str = r#"
/* aws-waf:apply */
const fresh = () => { const s = readState(); return s.live && s.stamp === args.state.stamp; };
if (!fresh()) return {written:false};
let token = args.answer.cookie;
if (!token) {
    const challenge = args.state.params.challenge_script;
    if (!awsUrl(challenge, 'challenge.js')) return {written:false};
    const endpoint = new URL('voucher', challenge);
    const region = args.state.params.problem_url && new URL(args.state.params.problem_url).searchParams.get('region');
    if (region) endpoint.searchParams.set('region', region);
    try {
        const response = await fetch(endpoint.href, {method:'POST', redirect:'error', credentials:'omit',
            body:JSON.stringify({captcha_voucher:args.answer.captcha_voucher, existing_token:args.answer.existing_token})});
        if (!response.ok) return {written:false};
        token = (await response.json()).token;
    } catch { return {written:false}; }
}
if (!cookieSafe(token) || !fresh()) return {written:false};
const host = location.hostname;
const domains = (Array.isArray(window.awsWafCookieDomainList) ? window.awsWafCookieDomainList : [])
    .filter(d => typeof d === 'string').map(d => d.replace(/^\./, '').toLowerCase())
    .filter(d => /^[a-z0-9.-]+$/.test(d) && (host === d || host.endsWith('.' + d)));
const domain = domains.sort((a,b) => a.length - b.length)[0] || host;
const attributes = '; path=/; secure; samesite=Lax';
document.cookie = 'aws-waf-token=; max-age=0' + attributes;
for (const d of new Set([host, domain])) document.cookie = 'aws-waf-token=; max-age=0; domain=' + d + attributes;
document.cookie = 'aws-waf-token=' + token + '; domain=' + domain + attributes;
const tokens = cookieValues();
if (tokens.length !== 1 || tokens[0] !== token) return {written:false};
let callbackRan = false, callbackFailed = false;
const widget = all('awswaf-captcha').find(visible);
const wasVisible = visible(widget);
try {
    if (!args.state.interstitial && typeof widget?.config?.onSuccess === 'function') {
        await widget.config.onSuccess(token); callbackRan = true;
    }
} catch { callbackFailed = true; }
if (args.state.interstitial) setTimeout(() => location.reload(), 50);
return {written:true, callbackRan, callbackFailed, wasVisible, interstitial:args.state.interstitial,
    document:args.state.document, token};
"#;

const TRANSITION: &str = r#"
/* aws-waf:verify */
return {ready:document.readyState === 'complete' && /^https?:$/.test(location.protocol),
    status:performance.getEntriesByType('navigation')[0]?.responseStatus ?? null,
    navigated:documentStamp() !== args.document,
    transition:!args.interstitial && args.wasVisible && !all('awswaf-captcha').some(visible),
    captchaFree:!window.gokuProps && !window._cf_chl_opt && !all('#captcha-container, awswaf-captcha, .g-recaptcha, .h-captcha, .cf-turnstile, [class*="geetest_"], [id*="funcaptcha"], [name="g-recaptcha-response"], [name="cf-turnstile-response"], [name="h-captcha-response"], iframe[src*="recaptcha"], iframe[src*="hcaptcha"], iframe[src*="challenges.cloudflare.com"]').length};
"#;
const RETAINED: &str = "/* aws-waf:retained */ const tokens = cookieValues(); return tokens.length === 1 && tokens[0] === args.token;";

fn script(body: &str, args: &Value) -> String {
    format!("(async () => {{ const args = {args}; {DOCUMENT} {body} }})()")
}

type Terminal = (SolveStatus, SolveReason, String);

pub async fn solve<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
) -> FlowResult {
    let mut result = super::terminal(SolveStatus::Failed, SolveReason::BrowserError, "");
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
                "AWS WAF stopped; take a fresh snapshot before continuing.".into(),
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

async fn eval<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
    body: &str,
    args: &Value,
) -> Result<Value, PageError> {
    let expression = script(body, args);
    match &challenge.host_frame {
        Some(frame) => bounded(ctx, ctx.page.eval_in_frame_main(frame, &expression)).await,
        None => bounded(ctx, ctx.page.eval_main(&expression)).await,
    }
}

async fn read<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
) -> Result<Value, PageError> {
    guard(ctx).await?;
    let state = eval(ctx, challenge, READ, &Value::Null).await?;
    guard(ctx).await?;
    Ok(state)
}

fn flag(v: &Value, key: &str) -> bool {
    v[key].as_bool() == Some(true)
}
fn text(v: &Value, key: &str) -> Option<String> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}

fn state_params(state: &Value) -> Option<AwsWafParams> {
    if !flag(state, "live") {
        return None;
    }
    let p = &state["params"];
    let params = AwsWafParams {
        website_url: text(p, "website_url")?,
        key: text(p, "key"),
        iv: text(p, "iv"),
        context: text(p, "context"),
        challenge_script: text(p, "challenge_script"),
        captcha_script: text(p, "captcha_script"),
        jsapi_script: text(p, "jsapi_script"),
        api_key: text(p, "api_key"),
        existing_token: text(p, "existing_token"),
        problem_url: text(p, "problem_url"),
    };
    validate(&params).ok()?;
    if !flag(state, "interstitial") && params.key.is_none() && present(&params.api_key).is_none() {
        return None;
    }
    Some(params)
}

fn fresh_task<'a, P: CaptchaPage, B: CaptchaBackend>(
    ctx: &'a SolveCtx<'_, P, B>,
    challenge: &'a Challenge,
    latest: &'a Mutex<Option<Value>>,
    error: &'a Mutex<Option<PageError>>,
    attempts: &'a AtomicU32,
) -> futures_util::future::BoxFuture<'a, Option<SolveTask>> {
    Box::pin(async move {
        match read(ctx, challenge).await {
            Ok(state) => {
                let params = state_params(&state)?;
                *latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(state);
                let number = attempts.fetch_add(1, Ordering::Relaxed) + 1;
                ctx.lease
                    .set_progress(Some(CaptchaKind::AwsWaf), SolveStep::Token);
                ctx.events.emit(
                    CaptchaEventName::Solving,
                    json!({"type":"aws_waf","method":"token","attempt":number}),
                );
                Some(SolveTask::AwsWaf(params))
            }
            Err(e) => {
                *error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                None
            }
        }
    })
}

fn fresh_source<'a, P: CaptchaPage, B: CaptchaBackend>(
    ctx: &'a SolveCtx<'_, P, B>,
    challenge: &'a Challenge,
    latest: &'a Mutex<Option<Value>>,
    error: &'a Mutex<Option<PageError>>,
    attempts: &'a AtomicU32,
) -> Box<dyn FnMut() -> futures_util::future::BoxFuture<'a, Option<SolveTask>> + Send + 'a> {
    Box::new(move || fresh_task(ctx, challenge, latest, error, attempts))
}

fn purchase<'a, P: CaptchaPage, B: CaptchaBackend>(
    ctx: &'a SolveCtx<'_, P, B>,
    challenge: &'a Challenge,
    latest: &'a Mutex<Option<Value>>,
    error: &'a Mutex<Option<PageError>>,
    attempts: &'a AtomicU32,
) -> futures_util::future::BoxFuture<'a, Result<ProviderAnswer, ProviderError>> {
    Box::pin(ctx.backend.solve_fresh(
        fresh_source(ctx, challenge, latest, error, attempts),
        ctx.deadline,
        ctx.cancel,
    ))
}

async fn run<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
    result: &mut FlowResult,
) -> Result<Terminal, PageError> {
    if state_params(&read(ctx, challenge).await?).is_none() {
        return Ok(unloaded());
    }
    if !ctx.backend.is_configured() {
        return Ok((
            SolveStatus::NeedsHuman,
            SolveReason::NoProvider,
            "No AWS WAF provider is configured; ask for human help.".into(),
        ));
    }
    let latest = Mutex::new(None);
    let read_error = Mutex::new(None);
    let attempts = AtomicU32::new(0);
    let answer = bounded(ctx, async {
        Ok(purchase(ctx, challenge, &latest, &read_error, &attempts).await)
    })
    .await;
    result.attempts.token = attempts.load(Ordering::Relaxed);
    if result.attempts.token > 0 {
        result.method = SolveMethod::Token;
        result.paid_tasks = 1;
    }
    let answer = answer?;
    if let Err(error) = &answer {
        result.paid_tasks = u32::from(error.charged);
    }
    if let Some(error) = read_error.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(error);
    }
    let Some(state) = latest.into_inner().unwrap_or_else(|e| e.into_inner()) else {
        return Ok(unloaded());
    };
    let answer = match answer {
        Ok(a) => a,
        Err(e) => return Ok(provider_failure(e)),
    };
    result.cost_usd = answer
        .cost_usd
        .filter(|c| c.is_finite() && *c >= 0.0)
        .unwrap_or(0.0);
    let fresh = read(ctx, challenge).await?;
    if fresh["stamp"] != state["stamp"] || state_params(&fresh) != state_params(&state) {
        return Ok(unconfirmed());
    }
    let SolveAnswer::Fields(fields) = answer.answer else {
        return Ok(bad_answer());
    };
    let solution = json!(fields);
    let Some(params) = state_params(&fresh) else {
        return Ok(unconfirmed());
    };
    let task = SolveTask::AwsWaf(params);
    if capsolver_answer(&task, &solution).is_none() && twocaptcha_answer(&task, &solution).is_none()
    {
        return Ok(bad_answer());
    }
    ctx.lease
        .set_progress(Some(CaptchaKind::AwsWaf), SolveStep::Verifying);
    let until = (Instant::now() + VERIFY).min(ctx.deadline);
    let applied = match tokio::time::timeout_at(
        until,
        eval(
            ctx,
            challenge,
            APPLY,
            &json!({"state":fresh,"answer":solution}),
        ),
    )
    .await
    {
        Ok(v) => v?,
        Err(_) => return Ok(unconfirmed()),
    };
    result.callback_ran = flag(&applied, "callbackRan");
    if !flag(&applied, "written") || flag(&applied, "callbackFailed") {
        return Ok(unconfirmed());
    }
    loop {
        let document = bounded(ctx, ctx.page.document()).await?;
        let evidence = if document != ctx.document {
            bounded(ctx, ctx.page.eval_main(&script(TRANSITION, &applied))).await?
        } else {
            eval(ctx, challenge, TRANSITION, &applied).await?
        };
        if bounded(ctx, ctx.page.document()).await? != document {
            return Err(PageError::Navigated);
        }
        let navigated = flag(&evidence, "navigated") || document != ctx.document;
        let accepted = if navigated {
            flag(&evidence, "ready")
                && flag(&evidence, "captchaFree")
                && evidence["status"]
                    .as_u64()
                    .is_some_and(|status| (200..300).contains(&status) && status != 202)
        } else {
            flag(&evidence, "transition")
        };
        if accepted {
            return Ok((
                SolveStatus::Solved,
                SolveReason::TokenAccepted,
                "The page accepted the AWS WAF token; continue.".into(),
            ));
        }
        if Instant::now() >= until {
            if flag(&applied, "interstitial")
                || flag(&evidence, "navigated")
                || document != ctx.document
            {
                return Ok(unconfirmed());
            }
            if eval(ctx, challenge, RETAINED, &applied).await?.as_bool() != Some(true) {
                return Ok(unconfirmed());
            }
            return Ok((
                SolveStatus::TokenApplied,
                SolveReason::AwaitingSubmit,
                super::token_applied_message(result.callback_ran).replace(
                    "no callback was declared",
                    "the site's success callback was unavailable",
                ),
            ));
        }
        bounded(ctx, async {
            sleep_until(until.min(Instant::now() + Duration::from_millis(250))).await;
            Ok(())
        })
        .await?;
    }
}

fn unloaded() -> Terminal {
    (SolveStatus::NeedsHuman,SolveReason::ChallengeNotPassed,
    "The AWS WAF widget has no valid live challenge; ask for human help. No paid task was submitted.".into())
}
fn unconfirmed() -> Terminal {
    (
        SolveStatus::Failed,
        SolveReason::AcceptanceUnconfirmed,
        "AWS WAF acceptance was not confirmed; take a fresh snapshot and retry the action.".into(),
    )
}
fn bad_answer() -> Terminal {
    (
        SolveStatus::NeedsHuman,
        SolveReason::ProviderError,
        "The provider returned an unusable AWS WAF answer; ask for human help.".into(),
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
        "AWS WAF provider unavailable; ask for human help.".into(),
    )
}

fn present(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|v| !v.trim().is_empty())
}
fn encoded(v: &str, min: usize) -> bool {
    STANDARD.decode(v).is_ok_and(|v| v.len() >= min)
}
fn api_key(key: &str) -> bool {
    let body = if let Some((prefix, version)) = key.rsplit_once('_') {
        let Some((body, kind)) = prefix.rsplit_once('_') else {
            return false;
        };
        if [kind, version]
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
        {
            return false;
        }
        body
    } else {
        key
    };
    encoded(body, 32)
}
fn aws_url(s: &str, file: &str) -> bool {
    url::Url::parse(s).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some_and(|h| h.ends_with(".awswaf.com"))
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none()
            && u.path().ends_with(&format!("/{file}"))
    })
}

fn validate(p: &AwsWafParams) -> Result<(), String> {
    let site = url::Url::parse(&p.website_url)
        .ok()
        .is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
    let goku = [present(&p.key), present(&p.iv), present(&p.context)];
    let valid_goku = goku.iter().all(Option::is_none)
        || matches!(goku,[Some(k),Some(i),Some(c)] if encoded(k,32) && encoded(i,12) && encoded(c,32));
    let source = present(&p.challenge_script).is_some()
        || present(&p.jsapi_script).is_some()
        || present(&p.problem_url).is_some();
    let scripts = [
        (&p.challenge_script, "challenge.js"),
        (&p.captcha_script, "captcha.js"),
        (&p.jsapi_script, "jsapi.js"),
        (&p.problem_url, "problem"),
    ]
    .iter()
    .all(|(s, file)| present(s).is_none_or(|s| aws_url(s, file)));
    let valid_api = present(&p.api_key).is_none_or(api_key);
    if !site || !valid_goku || !source || !scripts || !valid_api {
        return Err("missing or invalid AWS WAF parameters".into());
    }
    Ok(())
}

pub(super) fn capsolver_task(task: &SolveTask) -> Result<Value, String> {
    let SolveTask::AwsWaf(p) = task else {
        return Err("not an AWS WAF task".into());
    };
    validate(p)?;
    let mut out = json!({"type":"AntiAwsWafTaskProxyLess","websiteURL":p.website_url});
    for (name, value) in [
        ("awsKey", &p.key),
        ("awsIv", &p.iv),
        ("awsContext", &p.context),
        ("awsChallengeJS", &p.challenge_script),
        ("awsApiJs", &p.jsapi_script),
        ("awsProblemUrl", &p.problem_url),
        ("awsApiKey", &p.api_key),
        ("awsExistingToken", &p.existing_token),
    ] {
        super::providers::optional(&mut out, name, value);
    }
    Ok(out)
}

pub(super) fn twocaptcha_task(task: &SolveTask) -> Result<Value, String> {
    let SolveTask::AwsWaf(p) = task else {
        return Err("not an AWS WAF task".into());
    };
    validate(p)?;
    let mut out = json!({"type":"AmazonTaskProxyless","websiteURL":p.website_url});
    if let (Some(k), Some(i), Some(c)) = (present(&p.key), present(&p.iv), present(&p.context)) {
        out["websiteKey"] = json!(k);
        out["iv"] = json!(i);
        out["context"] = json!(c);
        super::providers::optional(&mut out, "challengeScript", &p.challenge_script);
        super::providers::optional(&mut out, "captchaScript", &p.captcha_script);
    } else if let (Some(k), Some(s)) = (present(&p.api_key), present(&p.jsapi_script)) {
        out["websiteKey"] = json!(k);
        out["jsapiScript"] = json!(s);
    } else {
        return Err("missing AWS WAF key or JS API parameters".into());
    }
    Ok(out)
}

fn cookie_safe(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| matches!(b,0x21|0x23..=0x2b|0x2d..=0x3a|0x3c..=0x5b|0x5d..=0x7e))
}
pub(super) fn capsolver_answer(task: &SolveTask, solution: &Value) -> Option<SolveAnswer> {
    if !matches!(task, SolveTask::AwsWaf(_)) {
        return None;
    }
    let cookie = text(solution, "cookie").filter(|s| cookie_safe(s))?;
    Some(SolveAnswer::Fields([("cookie".into(), cookie)].into()))
}
pub(super) fn twocaptcha_answer(task: &SolveTask, solution: &Value) -> Option<SolveAnswer> {
    if !matches!(task, SolveTask::AwsWaf(_)) {
        return None;
    }
    let voucher = text(solution, "captcha_voucher")?;
    let token = text(solution, "existing_token")?;
    Some(SolveAnswer::Fields(
        [
            ("captcha_voucher".into(), voucher),
            ("existing_token".into(), token),
        ]
        .into(),
    ))
}
pub(super) fn secrets(task: &SolveTask) -> Vec<&str> {
    let SolveTask::AwsWaf(p) = task else {
        return vec![];
    };
    std::iter::once(p.website_url.as_str())
        .chain(
            [
                &p.key,
                &p.iv,
                &p.context,
                &p.challenge_script,
                &p.captcha_script,
                &p.jsapi_script,
                &p.api_key,
                &p.existing_token,
                &p.problem_url,
            ]
            .into_iter()
            .filter_map(present),
        )
        .collect()
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
            AwsWafParams, CaptchaKind, ChallengeState, DocumentId, FrameRef, PageError,
            ProviderAnswer, ProviderError, ProviderErrorKind, ProviderName, Rect, SolveMethod,
        },
    };

    fn params() -> AwsWafParams {
        AwsWafParams {
            website_url: "https://protected.test/gate".into(),
            key: Some("AQID".repeat(62)),
            iv: Some("CgAAFDIlckAAAAid".into()),
            context: Some("YWJj".repeat(100)),
            challenge_script: Some("https://example.token.awswaf.com/a/b/challenge.js".into()),
            captcha_script: Some("https://example.captcha.awswaf.com/a/b/captcha.js".into()),
            jsapi_script: None,
            api_key: None,
            existing_token: None,
            problem_url: None,
        }
    }

    fn fields(values: &[(&str, &str)]) -> SolveAnswer {
        SolveAnswer::Fields(
            values
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        )
    }

    #[test]
    fn tests_that_interstitial_payloads_use_each_vendors_exact_fields() -> anyhow::Result<()> {
        let p = params();
        let task = SolveTask::AwsWaf(p.clone());
        assert_eq!(
            capsolver_task(&task).map_err(anyhow::Error::msg)?,
            json!({
                "type":"AntiAwsWafTaskProxyLess", "websiteURL":p.website_url,
                "awsKey":p.key, "awsIv":p.iv, "awsContext":p.context,
                "awsChallengeJS":p.challenge_script
            })
        );
        assert_eq!(
            twocaptcha_task(&task).map_err(anyhow::Error::msg)?,
            json!({
                "type":"AmazonTaskProxyless", "websiteURL":p.website_url,
                "websiteKey":p.key, "iv":p.iv, "context":p.context,
                "challengeScript":p.challenge_script, "captchaScript":p.captcha_script
            })
        );
        Ok(())
    }

    #[test]
    fn tests_that_jsapi_payloads_omit_blank_fields() -> anyhow::Result<()> {
        let mut p = params();
        p.key = None;
        p.iv = None;
        p.context = None;
        p.captcha_script = Some(" ".into());
        p.jsapi_script = Some("https://example.captcha-sdk.awswaf.com/a/jsapi.js".into());
        p.api_key = Some("YWJj".repeat(40));
        p.existing_token = Some("old:token==".into());
        p.problem_url = Some("https://example.captcha.awswaf.com/a/problem?api_key=key".into());
        let task = SolveTask::AwsWaf(p.clone());
        assert_eq!(
            capsolver_task(&task).map_err(anyhow::Error::msg)?,
            json!({
                "type":"AntiAwsWafTaskProxyLess", "websiteURL":p.website_url,
                "awsChallengeJS":p.challenge_script,"awsApiJs":p.jsapi_script,
                "awsApiKey":p.api_key,"awsProblemUrl":p.problem_url,"awsExistingToken":p.existing_token
            })
        );
        assert_eq!(
            twocaptcha_task(&task).map_err(anyhow::Error::msg)?,
            json!({
                "type":"AmazonTaskProxyless", "websiteURL":p.website_url,
                "websiteKey":p.api_key,"jsapiScript":p.jsapi_script
            })
        );
        Ok(())
    }

    #[test]
    fn tests_that_versioned_jsapi_keys_reach_both_vendors_unchanged() -> anyhow::Result<()> {
        for suffix in ["", "_0_1", "_12_34"] {
            let key = format!("{}{suffix}", STANDARD.encode([b'k'; 64]));
            let mut p = params();
            p.key = None;
            p.iv = None;
            p.context = None;
            p.api_key = Some(key.clone());
            p.jsapi_script = Some("https://example.captcha-sdk.awswaf.com/a/jsapi.js".into());
            let task = SolveTask::AwsWaf(p);
            let cap = capsolver_task(&task).map_err(anyhow::Error::msg)?;
            let two = twocaptcha_task(&task).map_err(anyhow::Error::msg)?;
            assert_eq!(cap["awsApiKey"].as_str(), Some(key.as_str()));
            assert_eq!(two["websiteKey"].as_str(), Some(key.as_str()));
        }
        Ok(())
    }

    #[test]
    fn tests_that_answers_require_a_cookie_or_the_complete_voucher_pair() {
        let task = SolveTask::AwsWaf(params());
        assert_eq!(
            capsolver_answer(&task, &json!({"cookie":"issued:token=="})),
            Some(fields(&[("cookie", "issued:token==")]))
        );
        assert_eq!(
            twocaptcha_answer(
                &task,
                &json!({"captcha_voucher":"voucher", "existing_token":"paired:token=="})
            ),
            Some(fields(&[
                ("captcha_voucher", "voucher"),
                ("existing_token", "paired:token==")
            ]))
        );
        for value in [
            json!({}),
            json!({"cookie":" "}),
            json!({"cookie":7}),
            json!({"cookie":"token; domain=elsewhere.test"}),
            json!({"captcha_voucher":"voucher"}),
            json!({"captcha_voucher":"voucher", "existing_token":" "}),
        ] {
            assert_eq!(capsolver_answer(&task, &value), None);
            assert_eq!(twocaptcha_answer(&task, &value), None);
        }
    }

    #[test]
    fn tests_that_invalid_params_are_rejected_and_secrets_cover_all_inputs() {
        let mut p = params();
        p.website_url = "file:///secret".into();
        assert!(capsolver_task(&SolveTask::AwsWaf(p.clone())).is_err());
        assert!(twocaptcha_task(&SolveTask::AwsWaf(p)).is_err());
        let mut p = params();
        p.key = Some("fake-key".into());
        assert!(capsolver_task(&SolveTask::AwsWaf(p.clone())).is_err());
        assert!(twocaptcha_task(&SolveTask::AwsWaf(p)).is_err());
        let p = params();
        let task = SolveTask::AwsWaf(p.clone());
        let hidden = secrets(&task);
        for secret in [
            Some(&p.website_url),
            p.key.as_ref(),
            p.iv.as_ref(),
            p.context.as_ref(),
            p.challenge_script.as_ref(),
            p.captcha_script.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            assert!(hidden.contains(&secret.as_str()));
        }
    }

    #[tokio::test]
    async fn tests_that_real_provider_clients_fall_back_with_fresh_aws_fields() -> anyhow::Result<()>
    {
        use crate::services::captcha::{
            config::{
                CAPSOLVER_KEY_FILE_ENV, CAPSOLVER_URL_ENV, CaptchaConfig, TWOCAPTCHA_KEY_FILE_ENV,
                TWOCAPTCHA_URL_ENV,
            },
            providers::ProviderChain,
        };
        use axum::{Json, Router, extract::State as HttpState, http::Uri, routing::post};
        use std::{
            io::Write,
            sync::atomic::{AtomicUsize, Ordering},
        };

        type Requests = Arc<Mutex<Vec<(String, Value)>>>;
        async fn handle(
            HttpState(requests): HttpState<Requests>,
            uri: Uri,
            Json(body): Json<Value>,
        ) -> Json<Value> {
            requests
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((uri.path().into(), body));
            Json(match uri.path() {
                "/cap/createTask" => json!({"errorId":1,"errorCode":"ERROR_ZERO_BALANCE"}),
                "/two/createTask" => json!({"errorId":0,"taskId":42}),
                _ => json!({"errorId":0,"status":"ready","solution":{
                    "captcha_voucher":"voucher","existing_token":"solver:token=="},"cost":"0.00145"}),
            })
        }
        let requests = Arc::new(Mutex::new(vec![]));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let app = Router::new()
            .route("/{*method}", post(handle))
            .with_state(requests.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let mut key = tempfile::NamedTempFile::new()?;
        key.write_all(b"aws-unit-test-key")?;
        let mut config = CaptchaConfig::from_lookup(|name| match name {
            CAPSOLVER_KEY_FILE_ENV | TWOCAPTCHA_KEY_FILE_ENV => {
                Some(key.path().to_string_lossy().into_owned())
            }
            CAPSOLVER_URL_ENV => Some(format!("http://{address}/cap")),
            TWOCAPTCHA_URL_ENV => Some(format!("http://{address}/two")),
            _ => None,
        });
        config.paid_task_limit = 1;
        let chain = ProviderChain::new(&config);
        let reads = AtomicUsize::new(0);
        let answer = chain
            .solve_fresh(
                || async {
                    let mut p = params();
                    if reads.fetch_add(1, Ordering::SeqCst) > 0 {
                        p.key = Some("ZWZn".repeat(62));
                    }
                    Some(SolveTask::AwsWaf(p))
                },
                Instant::now() + Duration::from_secs(30),
                &CancellationToken::new(),
            )
            .await;
        server.abort();
        let answer = answer?;
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        assert_eq!(answer.provider, ProviderName::TwoCaptcha);
        assert_eq!(
            answer.answer,
            fields(&[
                ("captcha_voucher", "voucher"),
                ("existing_token", "solver:token==")
            ])
        );
        assert_eq!(answer.cost_usd, Some(0.00145));
        assert_eq!(chain.paid_tasks(), 1);
        let captured = requests.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(captured.len(), 3);
        assert_eq!(captured[0].1["task"]["type"], "AntiAwsWafTaskProxyLess");
        assert_eq!(captured[0].1["task"]["awsKey"], json!(params().key));
        assert_eq!(captured[1].1["task"]["type"], "AmazonTaskProxyless");
        assert_eq!(captured[1].1["task"]["websiteKey"], "ZWZn".repeat(62));
        Ok(())
    }

    fn snapshot() -> Value {
        let p = params();
        json!({"stamp":"document-and-challenge-1", "interstitial":true, "live":true,
            "params":{"website_url":p.website_url,"key":p.key,"iv":p.iv,"context":p.context,
                "challenge_script":p.challenge_script,"captcha_script":p.captcha_script,
                "jsapi_script":null,"api_key":null,"existing_token":null,"problem_url":null}})
    }

    struct State {
        snapshot: Value,
        next: Option<Value>,
        calls: Vec<(bool, String)>,
        applied: Value,
        evidence: Value,
        retained: bool,
        navigate_on_solve: bool,
        navigated: bool,
        framed: bool,
        apply_delay: Duration,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                snapshot: snapshot(),
                next: None,
                calls: vec![],
                applied: json!({"written":true,"callbackRan":false,"callbackFailed":false,"interstitial":true}),
                evidence: json!({"ready":true,"captchaFree":true,"navigated":true,"transition":false,"status":200}),
                retained: true,
                navigate_on_solve: false,
                navigated: false,
                framed: false,
                apply_delay: Duration::ZERO,
            }
        }
    }

    #[derive(Default, Clone)]
    struct FakePage(Arc<Mutex<State>>);

    impl FakePage {
        fn state(&self) -> MutexGuard<'_, State> {
            self.0.lock().unwrap_or_else(|e| e.into_inner())
        }
        async fn evaluate(&self, expression: &str, framed: bool) -> Result<Value, PageError> {
            if expression.contains("aws-waf:apply") {
                let delay = self.state().apply_delay;
                tokio::time::sleep(delay).await;
            }
            let mut state = self.state();
            assert_eq!(framed, state.framed, "wrong document");
            state.calls.push((framed, expression.to_owned()));
            if expression.contains("aws-waf:read") {
                return Ok(state.snapshot.clone());
            }
            if expression.contains("aws-waf:apply") {
                return Ok(state.applied.clone());
            }
            if expression.contains("aws-waf:verify") {
                return Ok(state.evidence.clone());
            }
            if expression.contains("aws-waf:retained") {
                return Ok(json!(state.retained));
            }
            Err(PageError::Browser("unexpected script".into()))
        }
    }

    impl CaptchaPage for FakePage {
        async fn eval_main(&self, s: &str) -> Result<Value, PageError> {
            self.evaluate(s, false).await
        }
        async fn eval_in_frame_main(&self, f: &FrameRef, s: &str) -> Result<Value, PageError> {
            assert_eq!(f.frame_id, "host-frame");
            self.evaluate(s, true).await
        }
        async fn eval_in_frame(&self, _: &FrameRef, _: &str) -> Result<Value, PageError> {
            unreachable!()
        }
        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            unreachable!()
        }
        async fn click_at(&self, _: f64, _: f64) -> Result<(), PageError> {
            unreachable!()
        }
        async fn screenshot_clip(&self, _: Rect) -> Result<String, PageError> {
            unreachable!()
        }
        async fn query_rect_in_frame(
            &self,
            _: &FrameRef,
            _: &str,
        ) -> Result<Option<Rect>, PageError> {
            unreachable!()
        }
        async fn document(&self) -> Result<DocumentId, PageError> {
            let mut d = document();
            if self.state().navigated {
                d.loader_id = "next-document".into();
            }
            Ok(d)
        }
    }

    struct FakeBackend {
        page: FakePage,
        tasks: Mutex<Vec<SolveTask>>,
        answer: Result<ProviderAnswer, ProviderError>,
        configured: bool,
        refresh_before: Option<Value>,
        retry: Option<Value>,
    }

    impl FakeBackend {
        fn new(page: &FakePage) -> Self {
            Self {
                page: page.clone(),
                tasks: Mutex::default(),
                configured: true,
                refresh_before: None,
                retry: None,
                answer: Ok(ProviderAnswer {
                    answer: fields(&[("cookie", "issued:token==")]),
                    provider: ProviderName::Capsolver,
                    task_id: Some("task".into()),
                    cost_usd: Some(0.00145),
                }),
            }
        }
        fn tasks(&self) -> MutexGuard<'_, Vec<SolveTask>> {
            self.tasks.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    impl CaptchaBackend for FakeBackend {
        fn is_configured(&self) -> bool {
            self.configured
        }
        async fn solve_fresh<'a, F, Fut>(
            &'a self,
            mut fresh: F,
            deadline: Instant,
            cancel: &'a CancellationToken,
        ) -> Result<ProviderAnswer, ProviderError>
        where
            F: FnMut() -> Fut + Send + 'a,
            Fut: Future<Output = Option<SolveTask>> + Send + 'a,
        {
            if let Some(state) = &self.refresh_before {
                self.page.state().snapshot = state.clone();
            }
            let unavailable = |charged| ProviderError {
                provider: None,
                kind: ProviderErrorKind::BadRequest,
                message: "fresh CAPTCHA parameters unavailable".into(),
                charged,
            };
            let mut task = fresh().await.ok_or_else(|| unavailable(false))?;
            if let Some(state) = &self.retry {
                self.tasks().push(task);
                self.page.state().snapshot = state.clone();
                task = fresh().await.ok_or_else(|| unavailable(true))?;
            }
            self.solve(&task, deadline, cancel).await
        }
        async fn solve(
            &self,
            task: &SolveTask,
            _: Instant,
            _: &CancellationToken,
        ) -> Result<ProviderAnswer, ProviderError> {
            self.tasks().push(task.clone());
            let mut state = self.page.state();
            if let Some(next) = state.next.take() {
                state.snapshot = next;
            }
            state.navigated = state.navigate_on_solve;
            self.answer.clone()
        }
    }

    fn document() -> DocumentId {
        DocumentId {
            url: "https://top.test/form".into(),
            loader_id: "original".into(),
        }
    }

    async fn run(
        page: &FakePage,
        backend: &FakeBackend,
        cancel: &CancellationToken,
    ) -> anyhow::Result<FlowResult> {
        run_budget(page, backend, cancel, Duration::from_secs(120)).await
    }

    async fn run_budget(
        page: &FakePage,
        backend: &FakeBackend,
        cancel: &CancellationToken,
        budget: Duration,
    ) -> anyhow::Result<FlowResult> {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "aws-test")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        let events = EventLog::new(CaptchaEventBus::new(), "aws-test".into(), 42);
        let challenge = Challenge {
            kind: CaptchaKind::AwsWaf,
            state: ChallengeState::Active,
            host_frame: page.state().framed.then(|| FrameRef {
                frame_id: "host-frame".into(),
                url: params().website_url,
                rect: None,
            }),
            frame: None,
            challenge_frame: None,
            sitekey: None,
            invisible: false,
            enterprise: false,
            recaptcha_s: None,
            action: None,
            cdata: None,
            callback: None,
        };
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
                forced_method: None,
            },
            &challenge,
        )
        .await;
        if result.attempts.token > 0 {
            assert!(
                events
                    .events()
                    .iter()
                    .any(|e| e.data["type"] == "aws_waf" && e.data["method"] == "token")
            );
        }
        Ok(result)
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_clear_navigation_accepts_a_cookie_in_the_host_document()
    -> anyhow::Result<()> {
        for framed in [false, true] {
            let page = FakePage::default();
            page.state().framed = framed;
            let backend = FakeBackend::new(&page);
            let result = run(&page, &backend, &CancellationToken::new()).await?;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::Solved, SolveReason::TokenAccepted)
            );
            assert_eq!(result.method, SolveMethod::Token);
            assert_eq!(result.paid_tasks, 1);
            assert_eq!(result.cost_usd, 0.00145);
            assert_eq!(backend.tasks().first(), Some(&SolveTask::AwsWaf(params())));
            assert!(
                page.state()
                    .calls
                    .iter()
                    .any(|(_, s)| s.contains("aws-waf:apply"))
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_jsapi_cookies_await_submission_without_visible_acceptance()
    -> anyhow::Result<()> {
        for callback in [false, true] {
            let page = FakePage::default();
            page.state().snapshot["interstitial"] = json!(false);
            for field in ["key", "iv", "context"] {
                page.state().snapshot["params"][field] = Value::Null;
            }
            page.state().snapshot["params"]["api_key"] =
                json!(format!("{}_0_1", STANDARD.encode([b'k'; 64])));
            page.state().snapshot["params"]["jsapi_script"] =
                json!("https://example.captcha-sdk.awswaf.com/a/jsapi.js");
            page.state().applied =
                json!({"written":true,"callbackRan":callback,"interstitial":false});
            page.state().evidence =
                json!({"ready":true,"captchaFree":false,"navigated":false,"transition":false});
            let result = run(&page, &FakeBackend::new(&page), &CancellationToken::new()).await?;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::TokenApplied, SolveReason::AwaitingSubmit)
            );
            assert_eq!(result.callback_ran, callback);
            assert!(result.message.contains("action"));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_unloaded_or_invalid_widgets_never_buy_a_task() -> anyhow::Result<()> {
        let mut jsapi = snapshot();
        jsapi["interstitial"] = json!(false);
        for name in ["key", "iv", "context"] {
            jsapi["params"][name] = Value::Null;
        }
        jsapi["params"]["jsapi_script"] =
            json!("https://example.captcha-sdk.awswaf.com/a/jsapi.js");
        let mut fake_key = jsapi.clone();
        fake_key["params"]["api_key"] = json!("FAKE_API_KEY_DO_NOT_USE");
        for state in [
            json!({"live":false}),
            json!({"live":true,"params":{}}),
            jsapi,
            fake_key,
        ] {
            let page = FakePage::default();
            page.state().snapshot = state;
            let backend = FakeBackend::new(&page);
            let result = run(&page, &backend, &CancellationToken::new()).await?;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::NeedsHuman, SolveReason::ChallengeNotPassed)
            );
            assert_eq!(result.paid_tasks, 0);
            assert!(backend.tasks().is_empty());
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_no_provider_hands_off_without_payment() -> anyhow::Result<()> {
        let page = FakePage::default();
        let mut backend = FakeBackend::new(&page);
        backend.configured = false;
        let result = run(&page, &backend, &CancellationToken::new()).await?;
        assert_eq!(result.reason, SolveReason::NoProvider);
        assert_eq!(result.paid_tasks, 0);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_stale_challenges_and_navigation_never_apply_answers() -> anyhow::Result<()>
    {
        for navigation in [false, true] {
            let page = FakePage::default();
            if navigation {
                page.state().navigate_on_solve = true;
            } else {
                let mut changed = snapshot();
                changed["stamp"] = json!("new-challenge");
                page.state().next = Some(changed);
            }
            let result = run(&page, &FakeBackend::new(&page), &CancellationToken::new()).await?;
            assert_eq!(
                result.reason,
                if navigation {
                    SolveReason::Navigated
                } else {
                    SolveReason::AcceptanceUnconfirmed
                }
            );
            assert!(
                !page
                    .state()
                    .calls
                    .iter()
                    .any(|(_, s)| s.contains("aws-waf:apply"))
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_cancellation_prevents_payment_and_apply() -> anyhow::Result<()> {
        let page = FakePage::default();
        let backend = FakeBackend::new(&page);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = run(&page, &backend, &cancel).await?;
        assert_eq!(result.reason, SolveReason::CallerCancelled);
        assert_eq!(result.paid_tasks, 0);
        assert!(page.state().calls.is_empty());
        assert!(backend.tasks().is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_deadlines_before_payment_and_during_apply_keep_their_reason()
    -> anyhow::Result<()> {
        for seconds in [0, 1] {
            let page = FakePage::default();
            page.state().apply_delay = Duration::from_secs(7);
            let backend = FakeBackend::new(&page);
            let result = run_budget(
                &page,
                &backend,
                &CancellationToken::new(),
                Duration::from_secs(seconds),
            )
            .await?;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::Failed, SolveReason::DeadlineExceeded)
            );
            assert_eq!(result.paid_tasks, u32::from(seconds > 0));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_navigation_before_payment_does_not_call_the_backend() -> anyhow::Result<()>
    {
        let page = FakePage::default();
        page.state().navigated = true;
        let backend = FakeBackend::new(&page);
        let result = run(&page, &backend, &CancellationToken::new()).await?;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Cancelled, SolveReason::Navigated)
        );
        assert_eq!(result.paid_tasks, 0);
        assert!(backend.tasks().is_empty());
        assert!(page.state().calls.is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_each_vendor_uses_fresh_params_and_only_the_last_snapshot_is_applied()
    -> anyhow::Result<()> {
        let page = FakePage::default();
        let mut backend = FakeBackend::new(&page);
        let mut second = snapshot();
        second["params"]["key"] = json!("ZWZn".repeat(62));
        second["stamp"] = json!("new-key-for-backup");
        backend.retry = Some(second.clone());
        let result = run(&page, &backend, &CancellationToken::new()).await?;
        assert_eq!(result.status, SolveStatus::Solved);
        let tasks = backend.tasks();
        assert_eq!(tasks.len(), 2);
        let [SolveTask::AwsWaf(first), SolveTask::AwsWaf(last)] = tasks.as_slice() else {
            anyhow::bail!("wrong tasks");
        };
        assert_eq!(first.key, params().key);
        assert_eq!(last.key.as_deref(), second["params"]["key"].as_str());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_disappearing_challenge_prevents_the_next_vendor_attempt()
    -> anyhow::Result<()> {
        for fallback in [false, true] {
            let page = FakePage::default();
            let mut backend = FakeBackend::new(&page);
            if fallback {
                backend.retry = Some(json!({"live":false}));
            } else {
                backend.refresh_before = Some(json!({"live":false}));
            }
            let result = run(&page, &backend, &CancellationToken::new()).await?;
            assert_eq!(result.status, SolveStatus::NeedsHuman);
            assert_eq!(result.paid_tasks, u32::from(fallback));
            assert_eq!(backend.tasks().len(), usize::from(fallback));
            assert!(
                !page
                    .state()
                    .calls
                    .iter()
                    .any(|(_, s)| s.contains("aws-waf:apply"))
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_apply_failures_and_lost_cookies_are_not_accepted() -> anyhow::Result<()> {
        for failure in ["write", "callback", "retention"] {
            let page = FakePage::default();
            page.state().snapshot["interstitial"] = json!(false);
            page.state().applied = json!({"written":failure!="write","callbackFailed":failure=="callback","interstitial":false});
            page.state().evidence =
                json!({"ready":true,"captchaFree":false,"navigated":false,"transition":false});
            page.state().retained = false;
            let result = run(&page, &FakeBackend::new(&page), &CancellationToken::new()).await?;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_unaccepted_cookies_do_not_report_solved() -> anyhow::Result<()> {
        for evidence in [
            json!({"ready":true,"captchaFree":false,"navigated":true}),
            json!({"ready":false,"captchaFree":true,"navigated":true}),
            json!({"ready":true,"captchaFree":true,"navigated":false}),
            json!({"ready":true,"captchaFree":true,"navigated":true,"status":405}),
            json!({"ready":true,"captchaFree":true,"navigated":true,"status":null}),
            json!({"ready":true,"captchaFree":true,"navigated":true,"status":405,"transition":true}),
            json!({"ready":true,"captchaFree":true,"navigated":true,"status":403}),
            json!({"ready":true,"captchaFree":true,"navigated":true,"status":500}),
            json!({"ready":true,"captchaFree":true,"navigated":true,"status":202}),
        ] {
            let page = FakePage::default();
            page.state().evidence = evidence;
            let result = run(&page, &FakeBackend::new(&page), &CancellationToken::new()).await?;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_provider_errors_preserve_charges_and_handoff_reasons() -> anyhow::Result<()>
    {
        for (kind, reason) in [
            (ProviderErrorKind::Auth, SolveReason::ProviderUnavailable),
            (
                ProviderErrorKind::SpendLimit,
                SolveReason::SpendLimitReached,
            ),
            (ProviderErrorKind::Transport, SolveReason::ProviderError),
        ] {
            for charged in [false, true] {
                let page = FakePage::default();
                let mut backend = FakeBackend::new(&page);
                backend.answer = Err(ProviderError {
                    provider: Some(ProviderName::Capsolver),
                    kind,
                    message: "redacted".into(),
                    charged,
                });
                let result = run(&page, &backend, &CancellationToken::new()).await?;
                assert_eq!(result.reason, reason);
                assert_eq!(result.paid_tasks, u32::from(charged));
                assert!(
                    !page
                        .state()
                        .calls
                        .iter()
                        .any(|(_, s)| s.contains("aws-waf:apply"))
                );
            }
        }
        Ok(())
    }
}
