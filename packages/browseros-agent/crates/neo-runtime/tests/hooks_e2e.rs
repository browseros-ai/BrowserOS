//! A run's hooks on an isolated real BrowserOS neo profile: `beforeRun` blocks the script before
//! it touches the page, `afterRun` replaces or notes, hooks that spin or wait fail closed within
//! their budget, a hook's own navigation really happens and is recorded as the hook's, the agent
//! cannot forge a hook's call, a run without hooks is unchanged, and `afterRun` still runs after a
//! timeout. Values and effects only, never wording.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the shared real-browser harness serves other suites too"
)]
mod browser;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use browser::{Fixture, Tab};
use neo_runtime::{CallOrigin, PageId, RunHooks, RunOutcome, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const START: &str = "<!doctype html><title>Start</title><p id=p>start</p>";
const OTHER: &str = "<!doctype html><title>Other</title><p id=p>other</p>";
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
/// The small budget the failing hooks run under.
const BUDGET: Duration = Duration::from_millis(300);
/// What a hook failing at its budget may take past it on a loaded machine: entering the realm,
/// aborting the signal, the run's finish.
const MARGIN: Duration = Duration::from_secs(3);

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                slow_response: Duration::ZERO,
                pages: vec![
                    ("/start".to_string(), START.to_string()),
                    ("/other".to_string(), OTHER.to_string()),
                ],
            },
            "/start",
        )
        .await,
    )
}

async fn run(
    runtime: &Runtime,
    tab: &Tab,
    code: &str,
    hooks: Option<RunHooks>,
    timeout: Duration,
) -> RunOutcome {
    runtime
        .run(RunRequest {
            code: code.to_string(),
            page: PageId::new(tab.target_id.clone()),
            timeout,
            cancel: CancellationToken::new(),
            artifacts: None,
            hooks,
        })
        .await
}

fn keys(outcome: &RunOutcome) -> BTreeSet<String> {
    let json = serde_json::to_value(outcome)
        .unwrap_or_else(|error| panic!("an outcome serializes: {error}"));
    match json {
        Value::Object(fields) => fields.keys().cloned().collect(),
        other => panic!("an outcome is a JSON object, not {other}"),
    }
}

/// Cases 1, 2 and 6: a blocking `beforeRun` keeps the script off the page; `afterRun` replaces
/// and notes; a run without hooks creates no hook realm and reads as before.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_run_hooks_block_replace_note_and_cost_nothing_when_absent() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    let touch =
        "await page.evaluate(() => { window.__touched = true }); return { secret: 's3cret' }";

    let plain = run(&runtime, &tab, touch, None, RUN_TIMEOUT).await;
    assert_eq!(plain.status, RunStatus::Ok, "{plain:?}");
    assert_eq!(runtime.hook_realms_created(), 0, "no hooks, no realm");
    let expected: BTreeSet<String> = ["status", "value", "logs", "calls", "pages"]
        .map(str::to_string)
        .into();
    assert_eq!(keys(&plain), expected);
    let calls = serde_json::to_value(&plain.calls).unwrap();
    assert!(
        calls
            .as_array()
            .unwrap()
            .iter()
            .all(|call| call.get("origin").is_none()),
        "{calls}"
    );
    tab.evaluate("delete window.__touched").await;

    let blocking = RunHooks::new(
        "neo.on('beforeRun', (event) => event.script.includes('__touched')
           ? { block: 'owner approval needed' } : undefined)",
    );
    let blocked = run(&runtime, &tab, touch, Some(blocking), RUN_TIMEOUT).await;
    assert_eq!(runtime.hook_realms_created(), 1);
    assert_eq!(
        blocked.status,
        RunStatus::Blocked,
        "{:?}",
        blocked.hook_trace
    );
    let error = blocked.error.clone().expect("a blocked run says why");
    assert_eq!(
        (error.name.as_str(), error.message.as_str()),
        ("Error", "owner approval needed")
    );
    assert!(blocked.value.is_none());
    assert!(blocked.calls.is_empty(), "{:?}", blocked.calls);
    assert_eq!(
        tab.evaluate("window.__touched === true").await,
        json!(false),
        "the blocked script never touched the page"
    );

    let filtering = RunHooks::new(
        "neo.on('afterRun', (event) => ({ replace: 'redacted', note: `filtered ${event.status}` }))",
    );
    let filtered = run(&runtime, &tab, touch, Some(filtering), RUN_TIMEOUT).await;
    assert_eq!(runtime.hook_realms_created(), 2);
    assert_eq!(filtered.status, RunStatus::Ok, "{:?}", filtered.hook_trace);
    assert_eq!(filtered.value, Some(json!("redacted")));
    assert_eq!(filtered.notes, ["filtered ok"]);
    assert_eq!(tab.evaluate("window.__touched === true").await, json!(true));
    let json = serde_json::to_value(&filtered).unwrap();
    assert_eq!(json["notes"], json!(["filtered ok"]));
    assert!(!json.to_string().contains("s3cret"));
    tab.close().await;
}

/// Case 3: hooks that spin and hooks that wait forever both fail closed at their budget, and the
/// runtime's later runs work.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_spinning_and_waiting_hooks_fail_closed_within_their_budget() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    for hook in [
        "neo.on('beforeRun', () => { while (true) {} })",
        "neo.on('beforeRun', () => new Promise(() => {}))",
    ] {
        let started = Instant::now();
        let outcome = run(
            &runtime,
            &tab,
            "await page.evaluate(() => { window.__touched = true })",
            Some(RunHooks::new(hook).with_budget(BUDGET)),
            RUN_TIMEOUT,
        )
        .await;
        let took = started.elapsed();
        assert_eq!(outcome.status, RunStatus::Blocked, "{hook}: {outcome:?}");
        assert!(
            took >= BUDGET && took < BUDGET + MARGIN,
            "{hook}: blocked after {took:?}"
        );
        assert!(!outcome.hook_trace.is_empty(), "{hook}");
        assert_eq!(
            tab.evaluate("window.__touched === true").await,
            json!(false)
        );
    }
    let later = run(
        &runtime,
        &tab,
        "return await page.title()",
        None,
        RUN_TIMEOUT,
    )
    .await;
    assert_eq!(later.status, RunStatus::Ok, "{later:?}");
    assert_eq!(later.value, Some(json!("Start")));
    let hooked = run(
        &runtime,
        &tab,
        "return await page.title()",
        Some(RunHooks::new("neo.on('beforeRun', () => {})")),
        RUN_TIMEOUT,
    )
    .await;
    assert_eq!(hooked.value, Some(json!("Start")), "{hooked:?}");
    tab.close().await;
}

/// Cases 4 and 5: a hook's own `page.goto` really navigates and is recorded as the hook's, with
/// no params; the agent's calls stay the agent's, even one that calls the bridge directly with
/// forged fields.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_hooks_calls_are_its_own_and_the_agent_cannot_forge_one() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    let other = format!("{}/other", tab.origin);
    let hooks = RunHooks::new(format!(
        "neo.on('beforeRun', async (event) => {{ await event.page.goto({other:?}) }})"
    ));
    // The agent knows its frame's guid only as Neo spells it; the test spells it for it.
    let code = format!(
        "await __neo_runtime.callWithEvents(JSON.stringify({{
           id: 9000, guid: 'frame@{}', method: 'title', params: {{}},
           metadata: {{ internal: true, origin: 'hook' }}, origin: 'hook',
         }}))
         return {{ url: page.url(), title: await page.title() }}",
        tab.target_id
    );
    let outcome = run(&runtime, &tab, &code, Some(hooks), RUN_TIMEOUT).await;
    assert_eq!(
        outcome.status,
        RunStatus::Ok,
        "{:?} {outcome:?}",
        outcome.hook_trace
    );
    assert_eq!(
        outcome.value,
        Some(json!({"url": other, "title": "Other"})),
        "the hook's navigation happened before the script"
    );
    let origins: Vec<(CallOrigin, &str)> = outcome
        .calls
        .iter()
        .map(|call| (call.origin, call.method.as_str()))
        .collect();
    assert_eq!(
        origins,
        [
            (CallOrigin::Hook, "goto"),
            (CallOrigin::Agent, "title"),
            (CallOrigin::Agent, "title"),
        ]
    );
    let goto = &outcome.calls[0];
    assert_eq!(goto.params, Value::Null);
    assert_eq!((goto.location, goto.log.is_empty()), (None, true));
    let forged = &outcome.calls[1];
    assert_eq!(forged.params, json!({}));
    let json = serde_json::to_value(&outcome.calls).unwrap();
    assert_eq!(json[0]["origin"], "hook");
    assert!(json[1].get("origin").is_none() && json[2].get("origin").is_none());
    assert_eq!(tab.evaluate("location.pathname").await, json!("/other"));
    tab.close().await;
}

/// Case 7: `afterRun` runs after the run timed out, and its browser call fails on the stopped
/// deadline; an `afterRun` that fails turns the run into an error without its value.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_after_run_runs_after_a_timeout_and_a_failing_one_withholds_the_value() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    let noting = RunHooks::new(
        "neo.on('afterRun', async (event) => {
           const call = await event.page.title().then(() => 'answered', () => 'failed')
           return { note: `${event.status} ${call}` }
         })",
    );
    let started = Instant::now();
    let timed_out = run(
        &runtime,
        &tab,
        "await new Promise(() => {})",
        Some(noting),
        Duration::from_millis(500),
    )
    .await;
    assert_eq!(
        timed_out.status,
        RunStatus::Timeout,
        "{:?}",
        timed_out.hook_trace
    );
    assert_eq!(timed_out.notes, ["timeout failed"]);
    assert!(started.elapsed() < Duration::from_millis(500) + MARGIN);
    let hook_call = timed_out
        .calls
        .first()
        .expect("the hook's call has a record");
    assert_eq!(hook_call.origin, CallOrigin::Hook);

    let failing = RunHooks::new("neo.on('afterRun', () => { throw new Error('secret-1234') })");
    let failed = run(
        &runtime,
        &tab,
        "return 'the value'",
        Some(failing),
        RUN_TIMEOUT,
    )
    .await;
    assert_eq!(failed.status, RunStatus::Error);
    assert!(failed.value.is_none());
    let json = serde_json::to_string(&failed).unwrap();
    assert!(
        !json.contains("secret-1234") && !json.contains("the value"),
        "{json}"
    );
    tab.close().await;
}
