//! Milestones M1 and M2 on a real browser: the built binary, launched the way Chromium launches
//! it, against a headless BrowserOS, driven over MCP by an rmcp client, and read back over the
//! host socket the way the host reads it, including a host's lock on a tab that is then closed from
//! outside Neo.
//!
//! ```text
//!  BrowserOS --headless --remote-debugging-port=<cdp>  ◀── CDP ──  browseros-claw-sdk
//!                                                                   ▲ POST /mcp: run
//!  this test ── launch JSON {ports: {server, cdp}} ─────────────────┤
//!            ── GET /v1/runs/{id} on CLAW_HOST_SOCKET, bearer token ┘
//!            ── after SIGTERM: $BROWSERCLAW_DIR/claw-sdk/trace.db
//! ```
//!
//! It needs BrowserOS, not stock Chrome: core lists and opens tabs with BrowserOS-only CDP
//! (`Browser.getTabs`, `Browser.createTab`). It runs only when `CLAW_SDK_E2E_BROWSER` names the
//! browser binary, e.g. `/Applications/BrowserOS.app/Contents/MacOS/BrowserOS`, and passes
//! without doing anything otherwise, so CI without a browser stays green.
//!
//! No test here has a real CAPTCHA provider key, and none calls a real provider. One Neo runs
//! without a key, as a deployment without one does. The clear-page test gets a placeholder key
//! file and a closed local provider port, ending at detection. The CAPTCHA page tests point
//! `BROWSEROS_CAPSOLVER_API_URL` at the local loopback stub in `support/captcha_pages.rs`, which
//! either refuses requests or mints sequential `FRESH-n` tokens. The stale-reCAPTCHA test is the
//! only one that needs network access, to load Google's public test checkbox widget.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;
#[path = "support/captcha_pages.rs"]
mod captcha_pages;

use std::error::Error;
use std::time::Duration;
use std::{env, fs};

use browseros_rig::{
    CaptchaKey, Client, HOST_TOKEN, PATIENCE, Rig, TestResult, browser_or_skip, get, outcome, run,
};
use captcha_pages::{CaptchaPages, ProviderMode};
use neo_trace::{Delivery, Effect, Outcome, RunId, RunStatus, TraceStore};
use serde_json::{Value, json};
use tokio::time;

/// The whole drive. A step that hangs fails the test here, so the guards still kill both
/// processes: a killed test process runs no `Drop`.
const TEST_BUDGET: Duration = Duration::from_secs(240);
/// A page whose `<title>` the tab strip shows once it loads.
const PAGE: &str = "data:text/html,<title>Neo e2e</title><p>claw-sdk end to end</p>";
const TITLE: &str = "Neo e2e";
/// The page a host locks, then the owner closes.
const LOCKED_PAGE: &str = "data:text/html,<title>Neo e2e lock</title><p>the owner's now</p>";
const LOCKED_TITLE: &str = "Neo e2e lock";

#[tokio::test(flavor = "multi_thread")]
async fn the_playwright_api_drives_a_real_browseros_page() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let mut rig = Rig::start(&browser, CaptchaKey::Absent)?;
    let failure = match time::timeout(TEST_BUDGET, drive(&mut rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn drive(rig: &mut Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let code = format!(
        r"
        const p = await context.newPage();
        await p.goto({PAGE:?});
        const t = await p.title();
        await p.close();
        return t;"
    );
    let meta = json!({ "requestId": "e2e-1", "taskId": "e2e" });

    let first = run(&client, &code, &meta).await?;
    assert_eq!(
        first.structured_content,
        Some(json!({ "value": TITLE })),
        "the tab strip's title of the page BrowserOS loaded: {first:?}"
    );
    let first_meta = outcome(&first)?;
    assert_eq!(first_meta.get("status"), Some(&json!("ok")), "{first_meta}");
    let id: RunId = first_meta
        .get("id")
        .and_then(Value::as_str)
        .ok_or("every result names its run")?
        .parse()?;

    let targets = get(rig.cdp_port(), "/json/list").await?;
    assert!(
        !targets.contains(TITLE),
        "page.close closed the tab in the browser: {targets}"
    );

    let repeat = run(&client, &code, &meta).await?;
    assert_eq!(
        outcome(&repeat)?.get("id").and_then(Value::as_str),
        Some(id.to_string().as_str()),
        "a repeated requestId replays the first run instead of running again"
    );
    assert_eq!(
        repeat.structured_content, first.structured_content,
        "with its value"
    );
    solve_captcha_without_a_key(&client).await?;
    let (locked_run, locked_tab) = a_hosts_lock_ends_when_its_tab_closes(rig, &client).await?;
    client.cancel().await?;
    read_back_over_the_host_socket(rig, id).await?;

    assert_eq!(rig.stop_neo().await?, Some(0), "SIGTERM is a normal stop");
    let store = TraceStore::open(&rig.trace_path()).await?;
    let detail = store.run(id).await?.ok_or("the run is in the trace")?;
    assert_eq!(detail.summary.status, RunStatus::Ok);
    assert_eq!(
        (
            detail.summary.request_id.map(|request| request.to_string()),
            detail.summary.task_id.map(|task| task.to_string()),
        ),
        (Some("e2e-1".to_owned()), Some("e2e".to_owned())),
        "the host's ids are recorded with the run"
    );
    let ops: Vec<_> = detail
        .operations
        .iter()
        .filter(|op| !browseros_rig::is_worker_event_record(op))
        .map(|op| (op.name.as_str(), op.effect, op.outcome, op.delivery))
        .collect();
    assert_eq!(
        ops,
        [
            (
                "context.newPage",
                Effect::Write,
                Some(Outcome::Ok),
                Some(Delivery::Done)
            ),
            (
                "page.goto",
                Effect::Write,
                Some(Outcome::Ok),
                Some(Delivery::Done)
            ),
            (
                "page.title",
                Effect::Read,
                Some(Outcome::Ok),
                Some(Delivery::NotSent)
            ),
            (
                "page.close",
                Effect::Write,
                Some(Outcome::Ok),
                Some(Delivery::Done)
            ),
        ],
        "four script ops, once each (the replay ran nothing): writes done, reads never sent"
    );
    the_load_is_an_op_on_the_new_tab(&store, locked_run, locked_tab).await
}

/// The trace of the run that opened the locked tab: `context.newPage()` on no tab, then the load
/// as `page.goto` on the new tab, where the door's rules and the trace see the tab.
async fn the_load_is_an_op_on_the_new_tab(store: &TraceStore, run: RunId, tab: i64) -> TestResult {
    let opened = store
        .run(run)
        .await?
        .ok_or("the run that opened the locked tab is in the trace")?;
    let ops: Vec<(&str, Option<i64>, bool)> = opened
        .operations
        .iter()
        .filter(|op| !browseros_rig::is_worker_event_record(op))
        .map(|op| {
            (
                op.name.as_str(),
                op.tab.map(|tab| tab.0),
                op.parent.is_some(),
            )
        })
        .collect();
    assert_eq!(
        ops,
        [
            ("context.newPage", None, false),
            ("page.goto", Some(tab), false),
        ],
        "the load is the script's own op on the new tab, where the door's rules see it"
    );
    Ok(())
}

/// `neo.solveCaptcha` on a Neo with no provider key: the real prelude's `neo`, refused for the
/// host to fix, before anything reaches the page.
async fn solve_captcha_without_a_key(client: &Client) -> TestResult {
    let code = r"
        try {
          await neo.solveCaptcha(page);
          return 'solved without a key';
        } catch (e) {
          return { code: e.code, owner: e.owner, delivery: e.delivery };
        }";
    let result = run(client, code, &json!({ "taskId": "e2e-captcha-off" })).await?;
    assert_eq!(
        result.structured_content,
        Some(json!({ "value": {
            "code": "captcha_unavailable",
            "owner": "host",
            "delivery": "not_sent",
        } })),
        "{result:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn solve_captcha_detects_a_clear_page_on_a_real_browseros() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Placeholder)?;
    let failure = match time::timeout(TEST_BUDGET, solve_on_a_clear_page(&rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

/// One solve on a page with no CAPTCHA: the solver's detection runs through claw-sdk's door on
/// the real browser and reports `no_captcha` with `attempted: false` and no `verdict` key,
/// without reaching any provider.
async fn solve_on_a_clear_page(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let code = format!(
        r"
        const p = await context.newPage();
        await p.goto({PAGE:?});
        const solved = await neo.solveCaptcha(p);
        await p.close();
        return solved;"
    );
    let result = run(&client, &code, &json!({ "taskId": "e2e-captcha" })).await?;
    let solved = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the solve answered a value: {result:?}"))?;
    assert_wire_invariants(solved, "clear page");
    let field = |name: &str| solved.get(name).cloned().unwrap_or(Value::Null);
    assert_eq!(
        (
            field("attempted"),
            field("type"),
            field("method"),
            field("reason"),
            field("paidTasks"),
            field("provider"),
        ),
        (
            json!(false),
            Value::Null,
            json!("none"),
            json!("no_captcha"),
            json!(0),
            Value::Null,
        ),
        "nothing was detected or attempted, and no provider was asked: {solved}"
    );
    assert_eq!(
        field("message"),
        json!(
            "Neo found no CAPTCHA it knows on the page and attempted nothing; Neo cannot tell \
             whether the page is clear — inspect a fresh snapshot and screenshot before \
             continuing, and hand off to the user if a check still blocks it."
        ),
        "no_captcha message follows the factual contract: {solved}"
    );
    client.cancel().await?;
    Ok(())
}

/// Checks invariants shared by every `neo.solveCaptcha` wire value: no `verdict` key, a factual
/// message telling the caller to inspect a fresh snapshot and screenshot, and no claim to carry
/// on.
fn assert_wire_invariants(solved: &Value, context: &str) {
    assert!(
        solved.get("verdict").is_none(),
        "{context}: solveCaptcha must not emit a verdict key: {solved}"
    );
    let message = solved
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        message.contains("snapshot and screenshot"),
        "{context}: message must direct the caller to inspect snapshot and screenshot: {solved}"
    );
    assert!(
        !message.contains("carry on"),
        "{context}: message must not tell the caller to carry on: {solved}"
    );
}

/// Stable expectations for one poisoned detection-isolation page. Timings are never pinned.
struct PoisonedCase {
    route: &'static str,
    kind: Option<&'static str>,
    detected_state: Option<&'static str>,
    attempted: bool,
    reason: &'static str,
}

const POISONED_CASES: [PoisonedCase; 8] = [
    PoisonedCase {
        route: "/clear",
        kind: None,
        detected_state: None,
        attempted: false,
        reason: "no_captcha",
    },
    PoisonedCase {
        route: "/awkward",
        kind: None,
        detected_state: None,
        attempted: false,
        reason: "no_captcha",
    },
    PoisonedCase {
        route: "/aws-oopif",
        kind: Some("aws_waf"),
        detected_state: Some("active"),
        attempted: true,
        reason: "challenge_not_passed",
    },
    PoisonedCase {
        route: "/aws-same",
        kind: Some("aws_waf"),
        detected_state: Some("active"),
        attempted: true,
        reason: "challenge_not_passed",
    },
    PoisonedCase {
        route: "/turnstile",
        kind: Some("turnstile"),
        detected_state: Some("active"),
        attempted: true,
        reason: "provider_error",
    },
    PoisonedCase {
        route: "/hcaptcha",
        kind: Some("hcaptcha"),
        detected_state: Some("active"),
        attempted: false,
        reason: "unsupported_type",
    },
    // Its `type` stays `unknown`: claw-server's closed schema does not list PerimeterX.
    PoisonedCase {
        route: "/perimeterx",
        kind: Some("unknown"),
        detected_state: Some("active"),
        attempted: false,
        reason: "unsupported_type",
    },
    PoisonedCase {
        route: "/perimeterx-waiting",
        kind: None,
        detected_state: None,
        attempted: false,
        reason: "no_captcha",
    },
];

#[tokio::test(flavor = "multi_thread")]
async fn solve_captcha_reads_poisoned_pages_through_the_isolated_world() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let pages = CaptchaPages::start(ProviderMode::Reject).await?;
    let provider = pages.provider();
    let rig = Rig::start_with(
        &browser,
        CaptchaKey::Placeholder,
        &[
            ("BROWSEROS_CAPSOLVER_API_URL", &provider),
            (
                "CLAW_LOG",
                "info,neo_server::dispatch::operation_guard=debug",
            ),
        ],
    )?;
    let failure = match time::timeout(TEST_BUDGET, solve_on_captcha_pages(&rig, &pages)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

/// Each poisoned page through MCP `run` → `neo.solveCaptcha`, with the solver's `captcha.detected`
/// note read back from claw-sdk's debug log. `CAPTCHA_ENTRY_OUT` names a file for the observed
/// values.
async fn solve_on_captcha_pages(rig: &Rig, pages: &CaptchaPages) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let mut observed = Vec::new();
    let mut logged = rig.neo_log()?.len();
    for case in &POISONED_CASES {
        let url = pages.url(case.route);
        let code = format!(
            r"
            const p = await context.newPage();
            await p.goto({url:?});
            const solved = await neo.solveCaptcha(p);
            await p.close();
            return solved;"
        );
        let result = run(&client, &code, &json!({ "taskId": "e2e-captcha-pages" })).await?;
        let solved = result
            .structured_content
            .as_ref()
            .and_then(|content| content.get("value"))
            .cloned()
            .unwrap_or_else(|| json!({ "noValue": format!("{result:?}") }));
        // The solver's events are op notes, which reach only the debug log.
        let log = rig.neo_log()?;
        let notes: Vec<&str> = log
            .get(logged..)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                line.split_once("note=captcha.detected at ")
                    .map(|(_, note)| note)
            })
            .collect();
        logged = log.len();
        observed.push(json!({ "route": case.route, "solved": solved, "detected": notes }));
    }
    if let Some(path) = env::var_os("CAPTCHA_ENTRY_OUT") {
        fs::write(path, serde_json::to_vec_pretty(&observed)?)?;
    }
    for (case, seen) in POISONED_CASES.iter().zip(&observed) {
        let solved = &seen["solved"];
        assert_wire_invariants(solved, case.route);
        let expected_kind = case.kind.map_or(Value::Null, |kind| json!(kind));
        assert_eq!(
            (
                &solved["type"],
                &solved["attempted"],
                &solved["reason"],
                &solved["paidTasks"],
            ),
            (
                &expected_kind,
                &json!(case.attempted),
                &json!(case.reason),
                &json!(0),
            ),
            "{}: {seen}",
            case.route
        );
        assert_detected_state(case, &expected_kind, seen)?;
    }
    perimeterx_is_named(&observed)?;
    // Only `/turnstile` reaches the paid provider stub (1 `createTask` call, refused with
    // `errorId: 1`): detection reads `.cf-turnstile`'s sitekey in the isolated world, then
    // `turnstile::solve` runs in the poisoned main world where `querySelectorAll` returns `[]`,
    // treats `prior` as empty, waits out the free auto-pass and checkbox phases (~14 s), and
    // asks the provider with the detected sitekey. `/aws-oopif` and `/aws-same` are detected as
    // `active` in the isolated world, but `aws_waf::solve` re-reads `#captcha-container` in the
    // child frame's poisoned main world, sees no live challenge, and stops with
    // `challenge_not_passed` before calling the provider. `/hcaptcha` and `/perimeterx` return
    // `unsupported_type`, and the clear pages detect nothing.
    assert_eq!(
        pages.provider_calls(),
        1,
        "only /turnstile reaches the rejecting provider stub on the poisoned pages"
    );
    client.cancel().await?;
    Ok(())
}

/// Parses the JSON payload after the `<N> ms: ` timing prefix in each `captcha.detected` note and
/// pins its `state` and `type` without pinning any duration.
fn assert_detected_state(case: &PoisonedCase, expected_kind: &Value, seen: &Value) -> TestResult {
    let notes = seen
        .get("detected")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{}: detected notes must be an array: {seen}", case.route))?;
    match case.detected_state {
        None => {
            assert!(
                notes.is_empty(),
                "{}: expected no captcha.detected note: {seen}",
                case.route
            );
        }
        Some(expected_state) => {
            let mut matched = false;
            for note in notes.iter().filter_map(Value::as_str) {
                let (_, payload) = note.split_once(": ").ok_or_else(|| {
                    format!(
                        "{}: captcha.detected note missing payload separator: {note}",
                        case.route
                    )
                })?;
                let parsed: Value = serde_json::from_str(payload)?;
                if parsed.get("state") == Some(&json!(expected_state))
                    && parsed.get("type") == Some(expected_kind)
                {
                    matched = true;
                }
            }
            assert!(
                matched,
                "{}: expected captcha.detected note with type={expected_kind} and state={expected_state}: {seen}",
                case.route
            );
        }
    }
    Ok(())
}

/// Proves locally (no network) that `neo.solveCaptcha` mints a fresh Turnstile token even when the
/// page already holds a stale `cf-turnstile-response` value, and mints a second distinct token on
/// retry instead of treating its own prior token as already passed.
#[tokio::test(flavor = "multi_thread")]
async fn solve_captcha_solves_stale_turnstile_fresh_and_retries() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let pages = CaptchaPages::start(ProviderMode::Mint).await?;
    let provider = pages.provider();
    let rig = Rig::start_with(
        &browser,
        CaptchaKey::Placeholder,
        &[("BROWSEROS_CAPSOLVER_API_URL", &provider)],
    )?;
    let failure = match time::timeout(TEST_BUDGET, solve_stale_turnstile(&rig, &pages)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn solve_stale_turnstile(rig: &Rig, pages: &CaptchaPages) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let url = pages.url("/turnstile-stale");
    let code = format!(
        r#"
        const p = await context.newPage();
        await p.goto({url:?});
        const readState = () => p.evaluate(() => {{
            const input = document.querySelector('input[name="cf-turnstile-response"]');
            const value = input ? input.value : '';
            const callbacks = Array.isArray(window.__callbackTokens) ? window.__callbackTokens : [];
            const last = callbacks.length > 0 ? callbacks[callbacks.length - 1] : '';
            return {{
                inputLength: value.length,
                inputIsStale: value === 'STALE-TOKEN',
                inputFreshName: value.startsWith('FRESH-') ? value : null,
                callbackCount: callbacks.length,
                callbackFreshNames: callbacks.map((token) =>
                    typeof token === 'string' && token.startsWith('FRESH-') ? token : '<non-fresh>'
                ),
                callbackMatchesInput: callbacks.length > 0 && last === value,
            }};
        }});
        const first = await neo.solveCaptcha(p);
        const afterFirst = await readState();
        const second = await neo.solveCaptcha(p);
        const afterSecond = await readState();
        await p.close();
        return {{ first, afterFirst, second, afterSecond }};"#
    );
    let result = run(
        &client,
        &code,
        &json!({ "taskId": "e2e-turnstile-stale-retry" }),
    )
    .await?;
    let observed = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the Turnstile stale script returned a value: {result:?}"))?;
    if let Some(path) = env::var_os("CAPTCHA_STALE_TURNSTILE_OUT") {
        fs::write(path, serde_json::to_vec_pretty(observed)?)?;
    }
    assert_turnstile_token_solve(
        &observed["first"],
        &observed["afterFirst"],
        "FRESH-1",
        &["FRESH-1"],
        "turnstile first",
        observed,
    );
    assert_turnstile_token_solve(
        &observed["second"],
        &observed["afterSecond"],
        "FRESH-2",
        &["FRESH-1", "FRESH-2"],
        "turnstile second",
        observed,
    );
    assert_eq!(
        pages.minted_tasks(),
        2,
        "the minting provider stub must create exactly two tasks across both solves"
    );
    client.cancel().await?;
    Ok(())
}

/// Checks one Turnstile fresh-token solve result and the page DOM/callback state right after it.
fn assert_turnstile_token_solve(
    solved: &Value,
    after: &Value,
    expected_token: &str,
    expected_callbacks: &[&str],
    label: &str,
    observed: &Value,
) {
    assert_wire_invariants(solved, label);
    assert_eq!(
        (
            &solved["attempted"],
            &solved["type"],
            &solved["method"],
            &solved["paidTasks"],
            &solved["provider"],
        ),
        (
            &json!(true),
            &json!("turnstile"),
            &json!("token"),
            &json!(1),
            &json!("capsolver"),
        ),
        "{label} must mint a fresh paid Turnstile token: {observed}"
    );
    let reason = solved
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        matches!(reason, "awaiting_submit" | "token_accepted"),
        "{label} reason must report token application: {observed}"
    );
    assert!(
        !matches!(reason, "already_passed" | "auto_passed" | "checkbox_passed"),
        "{label} must never claim a free pass on a prior token: {observed}"
    );
    assert_eq!(
        (
            &after["inputIsStale"],
            &after["inputFreshName"],
            &after["inputLength"],
            &after["callbackCount"],
            &after["callbackFreshNames"],
            &after["callbackMatchesInput"],
        ),
        (
            &json!(false),
            &json!(expected_token),
            &json!(expected_token.len()),
            &json!(expected_callbacks.len()),
            &json!(expected_callbacks),
            &json!(true),
        ),
        "{label} must write {expected_token} into the input and pass it to the callback: {observed}"
    );
}

/// Proves that `neo.solveCaptcha` solves a stale-token reCAPTCHA v2 widget token-first when a
/// provider is configured (minting `FRESH-1` directly instead of clicking the checkbox), and on
/// retry mints a second fresh paid token (`FRESH-2`) instead of reporting `already_passed` or
/// `checkbox_passed`.
///
/// Needs network access to load Google's public reCAPTCHA v2 test widget
/// (`6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI`); kept as its own test so network failures isolate
/// from the local tests.
#[tokio::test(flavor = "multi_thread")]
async fn solve_captcha_solves_stale_recaptcha_token_first_and_retries() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let pages = CaptchaPages::start(ProviderMode::Mint).await?;
    let provider = pages.provider();
    let rig = Rig::start_with(
        &browser,
        CaptchaKey::Placeholder,
        &[("BROWSEROS_CAPSOLVER_API_URL", &provider)],
    )?;
    let failure = match time::timeout(TEST_BUDGET, solve_stale_recaptcha(&rig, &pages)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn solve_stale_recaptcha(rig: &Rig, pages: &CaptchaPages) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let url = pages.url("/recaptcha-stale");
    let code = format!(
        r#"
        const p = await context.newPage();
        await p.goto({url:?});
        await p.waitForFunction(() => {{
            const frame = document.querySelector('iframe[src*="/recaptcha/"][src*="/anchor"]');
            const field = document.querySelector('textarea[name="g-recaptcha-response"]');
            if (!frame || !field) return false;
            const rect = frame.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0;
        }}, null, {{ timeout: 30000 }});
        await p.frameLocator('iframe[src*="/recaptcha/"][src*="/anchor"]')
            .locator('#recaptcha-anchor')
            .waitFor({{ state: 'visible', timeout: 30000 }});
        await p.evaluate(() => {{
            const field = document.querySelector('textarea[name="g-recaptcha-response"]');
            field.value = 'STALE-TOKEN';
        }});
        const readState = () => p.evaluate(() => {{
            const field = document.querySelector('textarea[name="g-recaptcha-response"]');
            const value = field ? field.value : '';
            const callbacks = Array.isArray(window.__callbackTokens) ? window.__callbackTokens : [];
            const last = callbacks.length > 0 ? callbacks[callbacks.length - 1] : '';
            return {{
                tokenLength: value.length,
                isStale: value === 'STALE-TOKEN',
                freshName: value.startsWith('FRESH-') ? value : null,
                callbackCount: callbacks.length,
                lastCallbackFreshName:
                    typeof last === 'string' && last.startsWith('FRESH-') ? last : null,
                lastCallbackMatchesField: callbacks.length > 0 && last === value,
            }};
        }});
        const first = await neo.solveCaptcha(p);
        const afterFirst = await readState();
        const second = await neo.solveCaptcha(p);
        const afterSecond = await readState();
        await p.close();
        return {{ first, afterFirst, second, afterSecond }};"#
    );
    let result = run(
        &client,
        &code,
        &json!({ "taskId": "e2e-recaptcha-stale-retry" }),
    )
    .await?;
    let observed = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the reCAPTCHA stale script returned a value: {result:?}"))?;
    if let Some(path) = env::var_os("CAPTCHA_STALE_RECAPTCHA_OUT") {
        fs::write(path, serde_json::to_vec_pretty(observed)?)?;
    }
    assert_recaptcha_token_solve(
        &observed["first"],
        &observed["afterFirst"],
        "FRESH-1",
        1,
        "recaptcha first",
        observed,
    );
    assert_recaptcha_token_solve(
        &observed["second"],
        &observed["afterSecond"],
        "FRESH-2",
        2,
        "recaptcha second",
        observed,
    );
    assert_eq!(
        pages.minted_tasks(),
        2,
        "both token-first solves on the stale reCAPTCHA page must mint a fresh paid task"
    );
    client.cancel().await?;
    Ok(())
}

/// Checks one token-first reCAPTCHA solve call: mints a fresh paid `FRESH-n` token from the stub,
/// writes it into the textarea, invokes the callback, and never claims `already_passed` or
/// `checkbox_passed`.
fn assert_recaptcha_token_solve(
    solved: &Value,
    after: &Value,
    expected_token: &str,
    expected_callbacks: u64,
    label: &str,
    observed: &Value,
) {
    assert_wire_invariants(solved, label);
    assert_eq!(
        (
            &solved["attempted"],
            &solved["type"],
            &solved["method"],
            &solved["paidTasks"],
            &solved["provider"],
        ),
        (
            &json!(true),
            &json!("recaptcha_v2"),
            &json!("token"),
            &json!(1),
            &json!("capsolver"),
        ),
        "{label} must mint a fresh paid reCAPTCHA token directly: {observed}"
    );
    let reason = solved
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        matches!(reason, "awaiting_submit" | "token_accepted"),
        "{label} reason must report token application: {observed}"
    );
    assert!(
        !matches!(reason, "already_passed" | "checkbox_passed"),
        "{label} must never report already_passed or checkbox_passed: {observed}"
    );
    assert_eq!(
        (
            &after["isStale"],
            &after["freshName"],
            &after["tokenLength"],
            &after["callbackCount"],
            &after["lastCallbackFreshName"],
            &after["lastCallbackMatchesField"],
        ),
        (
            &json!(false),
            &json!(expected_token),
            &json!(expected_token.len()),
            &json!(expected_callbacks),
            &json!(expected_token),
            &json!(true),
        ),
        "{label} must write {expected_token} into the textarea and invoke the callback: {observed}"
    );
}

/// The `PerimeterX` refusal names the family its `type` cannot (claw-server's closed schema lists
/// no `perimeterx`), in the script's line and in the solver's `captcha.detected` note.
fn perimeterx_is_named(observed: &[Value]) -> TestResult {
    let seen = observed
        .iter()
        .find(|seen| seen.get("route") == Some(&json!("/perimeterx")))
        .ok_or("the PerimeterX page was run")?;
    let line = seen
        .pointer("/solved/message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        line.contains("PerimeterX's press-and-hold human check"),
        "{seen}"
    );
    let noted = seen
        .get("detected")
        .and_then(Value::as_array)
        .is_some_and(|notes| {
            notes
                .iter()
                .filter_map(Value::as_str)
                .any(|note| note.contains(r#""family":"perimeterx""#))
        });
    assert!(noted, "{seen}");
    Ok(())
}

/// A host locks a tab the agent opened with `context.newPage()` and loaded, the owner closes it from
/// outside Neo (Chromium's own `/json/close`), and the lock ends with the tab: the close reached
/// neo-http from BrowserOS's target events. Returns the run that opened the tab and
/// the tab, for the trace check after the stop.
async fn a_hosts_lock_ends_when_its_tab_closes(
    rig: &Rig,
    client: &Client,
) -> Result<(RunId, i64), Box<dyn Error>> {
    let code = format!(
        r"
        const p = await context.newPage();
        await p.goto({LOCKED_PAGE:?});
        return p;"
    );
    let opened = run(client, &code, &json!({ "taskId": "e2e-lock" })).await?;
    let page = opened
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the run returns its page: {opened:?}"))?;
    let tab = page
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("a page carries its tab: {page}"))?;
    let run_id: RunId = outcome(&opened)?
        .get("id")
        .and_then(Value::as_str)
        .ok_or("every result names its run")?
        .parse()?;

    let take = json!({ "tabId": tab, "ttlSeconds": 600 });
    let locked = rig
        .host_call("POST", "/v1/locks", Some(HOST_TOKEN), Some(&take))
        .await?;
    let (head, body) = locked.split_once("\r\n\r\n").ok_or("an HTTP response")?;
    assert!(
        head.starts_with("HTTP/1.1 200"),
        "the host locks the tab: {locked}"
    );
    let lock: Value = serde_json::from_str(body)?;
    let id = lock
        .get("id")
        .and_then(Value::as_str)
        .ok_or("a lock has an id")?;

    let listed = get(rig.cdp_port(), "/json/list").await?;
    let (_, targets) = listed.split_once("\r\n\r\n").ok_or("an HTTP response")?;
    let targets: Value = serde_json::from_str(targets)?;
    let target = targets
        .as_array()
        .and_then(|targets| {
            targets
                .iter()
                .find(|target| target.get("title").and_then(Value::as_str) == Some(LOCKED_TITLE))
        })
        .and_then(|target| target.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the locked page is a target: {listed}"))?;
    let closed = get(rig.cdp_port(), &format!("/json/close/{target}")).await?;
    assert!(
        closed.starts_with("HTTP/1.1 200"),
        "Chromium closes the tab: {closed}"
    );

    time::timeout(PATIENCE, async {
        loop {
            let read = rig
                .host_get(&format!("/v1/locks/{id}"), Some(HOST_TOKEN))
                .await?;
            let (_, body) = read.split_once("\r\n\r\n").ok_or("an HTTP response")?;
            let lock: Value = serde_json::from_str(body)?;
            if lock.get("releasedAt").is_some_and(Value::is_i64) {
                return Ok::<_, Box<dyn Error>>(());
            }
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_elapsed| format!("the lock did not end within {PATIENCE:?} of its tab closing"))??;
    Ok((run_id, tab))
}

/// The run as the host reads it over the host socket: refused without the bearer token, and with it
/// the run the trace recorded, including its four script ops and automatic monitoring reads.
async fn read_back_over_the_host_socket(rig: &Rig, id: RunId) -> TestResult {
    let refused = rig.host_get(&format!("/v1/runs/{id}"), None).await?;
    assert!(
        refused.starts_with("HTTP/1.1 401"),
        "the host socket wants the bearer token: {refused}"
    );
    // `wait` holds the answer until the run ended and every screenshot job it made was
    // answered, so `complete` says neo-http's camera took the writer's jobs here.
    let recorded = rig
        .host_get(&format!("/v1/runs/{id}?wait=30"), Some(HOST_TOKEN))
        .await?;
    let (head, body) = recorded.split_once("\r\n\r\n").ok_or("an HTTP response")?;
    assert!(head.starts_with("HTTP/1.1 200"), "{recorded}");
    let host_run: Value = serde_json::from_str(body)?;
    let field = |name: &str| host_run.get(name).cloned().unwrap_or(Value::Null);
    assert_eq!(
        (field("id"), field("status"), field("complete")),
        (json!(id.to_string()), json!("ok"), json!(true)),
        "the host reads the run as the trace recorded it: {host_run}"
    );
    let names: Vec<&str> = host_run
        .get("operations")
        .and_then(Value::as_array)
        .ok_or("a run lists its operations")?
        .iter()
        .filter(|op| !browseros_rig::is_worker_event_operation(op))
        .filter_map(|op| op.get("name").and_then(Value::as_str))
        .collect();
    assert_eq!(
        names,
        ["context.newPage", "page.goto", "page.title", "page.close"],
        "the host records script operations alongside worker monitoring: {host_run}"
    );
    Ok(())
}
