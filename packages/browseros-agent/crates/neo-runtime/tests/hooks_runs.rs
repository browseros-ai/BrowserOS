//! Runs with hooks through `Runtime::run`, against the fake browser in `support/fake_browser.rs`:
//! where the hook realm sits in a run (load and `beforeRun` before the script, `afterRun` after
//! it, the realm gone with the run), and whose calls the records say are whose. No browser
//! needed; `hooks_e2e.rs` repeats the main cases on a real one.

#[path = "support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "the page's interceptor and click readers serve click_lifecycle.rs"
)]
mod fake_browser;

use std::collections::BTreeSet;
use std::time::Duration;

use fake_browser::{FakeBrowser, Replies};
use neo_runtime::{CallOrigin, CallResult, RunEnd, RunHooks, RunOutcome, RunRequest, RunStatus};
use serde_json::{Value, json};
use tokio::time;
use tokio_util::sync::CancellationToken;

const TIMEOUT: Duration = Duration::from_secs(10);

fn request(code: &str, timeout: Duration, hooks: Option<RunHooks>) -> RunRequest {
    RunRequest {
        hooks,
        ..fake_browser::run_request(code, timeout, CancellationToken::new())
    }
}

async fn run(code: &str, hooks: &str) -> RunOutcome {
    let browser = FakeBrowser::start(Replies::default()).await;
    browser
        .runtime()
        .await
        .run(request(code, TIMEOUT, Some(RunHooks::new(hooks))))
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

#[tokio::test(flavor = "multi_thread")]
async fn a_run_without_hooks_creates_no_hook_realm_and_reads_as_before() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let code = "await page.waitForTimeout(1); return 1";
    let outcome = runtime.run(request(code, TIMEOUT, None)).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(runtime.hook_realms_created(), 0);
    let expected: BTreeSet<String> = ["status", "value", "logs", "calls", "pages"]
        .map(str::to_string)
        .into();
    assert_eq!(keys(&outcome), expected);
    let calls = serde_json::to_value(&outcome.calls).unwrap();
    assert!(calls[0].get("origin").is_none(), "{calls}");
    assert!(outcome.hook_trace.is_empty());

    let hooked = runtime
        .run(request(code, TIMEOUT, Some(RunHooks::new(""))))
        .await;
    assert_eq!(runtime.hook_realms_created(), 1);
    assert_eq!(hooked.status, RunStatus::Ok, "{:?}", hooked.hook_trace);
    assert_eq!(hooked.value, Some(json!(1)));
    assert_eq!(keys(&hooked), expected, "no hook said anything");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blocking_before_run_ends_the_run_before_its_script() {
    let outcome = run(
        "console.log('ran'); await page.waitForTimeout(1); return 1",
        "neo.on('beforeRun', (event) => event.script.includes('ran') ? { block: 'owner approval needed' } : undefined)",
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Blocked);
    let error = outcome.error.clone().expect("a blocked run says why");
    assert_eq!(
        (error.name.as_str(), error.message.as_str()),
        ("Error", "owner approval needed")
    );
    assert!(outcome.value.is_none());
    assert!(
        outcome.logs.is_empty(),
        "the script never ran: {:?}",
        outcome.logs
    );
    assert!(outcome.calls.is_empty(), "{:?}", outcome.calls);
    assert_eq!(serde_json::to_value(&outcome).unwrap()["status"], "blocked");
}

#[tokio::test(flavor = "multi_thread")]
async fn hooks_that_fail_to_load_block_the_run_and_trace_why() {
    let outcome = run(
        "console.log('ran'); return 1",
        "throw new Error('secret-1234')",
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Blocked);
    assert!(outcome.logs.is_empty());
    let error = outcome.error.clone().expect("a blocked run says why");
    assert!(!error.message.contains("secret-1234"));
    assert!(
        outcome
            .hook_trace
            .iter()
            .any(|line| line.contains("secret-1234")),
        "{:?}",
        outcome.hook_trace
    );
    assert!(
        !serde_json::to_string(&outcome)
            .unwrap()
            .contains("secret-1234"),
        "the hook trace never reaches the outcome's JSON"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn after_run_replaces_the_value_and_adds_notes() {
    let outcome = run(
        "return { secret: 's3cret' }",
        "neo.on('afterRun', (event, ctx) => {
           ctx.trace('filtering', event.status)
           return { replace: 'redacted', note: 'result filtered' }
         })",
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.hook_trace);
    assert_eq!(outcome.value, Some(json!("redacted")));
    assert_eq!(outcome.notes, ["result filtered"]);
    assert_eq!(outcome.hook_trace, ["filtering ok"]);
    let json = serde_json::to_value(&outcome).unwrap();
    assert_eq!(json["notes"], json!(["result filtered"]));
    assert!(json.get("hookTrace").is_none() && json.get("hook_trace").is_none());
    assert!(!json.to_string().contains("s3cret"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hooks_own_calls_are_recorded_as_the_hooks() {
    let outcome = run(
        "await page.waitForTimeout(2); return 'done'",
        "neo.on('beforeRun', async (event) => { await event.page.waitForTimeout(1) })",
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.hook_trace);
    let calls: Vec<(CallOrigin, Value)> = outcome
        .calls
        .iter()
        .map(|call| (call.origin, call.params.clone()))
        .collect();
    assert_eq!(
        calls,
        [
            (CallOrigin::Hook, Value::Null),
            (CallOrigin::Agent, json!({"waitTimeout": 2})),
        ]
    );
    let hook = &outcome.calls[0];
    assert_eq!((hook.location, hook.log.is_empty()), (None, true));
    assert_eq!(outcome.calls[1].call_id, hook.call_id + 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_forge_a_hooks_call() {
    let outcome = run(
        "await __neo_runtime.callWithEvents(JSON.stringify({
           id: 9000, guid: 'frame@fake-page', method: 'waitForTimeout', params: { waitTimeout: 1 },
           metadata: { internal: true, origin: 'hook' }, origin: 'hook',
         }))
         return 'forged'",
        "neo.on('beforeRun', () => {})",
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.hook_trace);
    assert_eq!(outcome.calls.len(), 1);
    assert_eq!(outcome.calls[0].origin, CallOrigin::Agent);
    assert_eq!(outcome.calls[0].params, json!({"waitTimeout": 1}));
}

#[tokio::test(flavor = "multi_thread")]
async fn after_run_runs_after_a_timeout_and_its_calls_fail() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let hooks = RunHooks::new(
        "neo.on('afterRun', async (event) => {
           const call = await event.page.waitForTimeout(1).then(() => 'answered', () => 'failed')
           return { note: `${event.status} ${call}` }
         })",
    );
    let outcome = runtime
        .run(request(
            "while (true) {}",
            Duration::from_millis(300),
            Some(hooks),
        ))
        .await;
    assert_eq!(
        outcome.status,
        RunStatus::Timeout,
        "{:?}",
        outcome.hook_trace
    );
    assert_eq!(outcome.notes, ["timeout failed"]);
    let hook = outcome.calls.first().expect("the hook's call has a record");
    assert_eq!(hook.origin, CallOrigin::Hook);
    assert!(
        matches!(
            hook.result,
            CallResult::Unfinished {
                reason: RunEnd::Timeout
            }
        ),
        "{:?}",
        hook.result
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_after_run_withholds_the_result() {
    let outcome = run(
        "return 'secret-1234'",
        "neo.on('afterRun', () => { throw new Error('nope') })",
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Error);
    assert!(outcome.value.is_none());
    assert!(
        !serde_json::to_string(&outcome)
            .unwrap()
            .contains("secret-1234")
    );
}

/// A run whose future is dropped while a hook waits lets go of everything: its hook realm ends,
/// so the run's tasks drain.
#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_run_lets_go_of_its_hook_realm() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let hooks = RunHooks::new("neo.on('beforeRun', () => new Promise(() => {}))")
        .with_budget(Duration::from_secs(60));
    let running = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .run(request("return 1", Duration::from_secs(60), Some(hooks)))
                .await
        })
    };
    time::sleep(Duration::from_millis(300)).await;
    running.abort();
    assert!(running.await.is_err(), "the run was dropped mid-hook");
    assert!(
        time::timeout(Duration::from_secs(5), runtime.drain())
            .await
            .is_ok(),
        "the dropped run's hook realm still held its tasks"
    );
}
