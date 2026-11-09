//! Action hooks on an isolated real BrowserOS neo profile, through `Runtime::run` with a real
//! hook file: blocking the `pageScript` group stops `evaluate`, `setContent` and `dispatchEvent`,
//! and a script's own bridge call too; replaced arguments take effect while the outcome keeps the
//! script's; `afterAction` replaces, notes and fails closed; a hook's own `page.evaluate` is not
//! hooked. The page's state is read over the test's own session. Values and effects only, never
//! wording.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the shared real-browser harness serves other suites too"
)]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{
    AfterHookResult, BeforeHookResult, CallOrigin, CallRecord, CallResult, PageId, RunHooks,
    RunOutcome, RunRequest, RunStatus, Runtime,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// A page whose button counts the clicks its listener sees.
const START: &str = "<!doctype html><title>Start</title><p id=p>start</p><input id=box>\
    <button id=b>Go</button><script>window.__clicks = 0;\
    document.getElementById('b').addEventListener('click', () => { window.__clicks += 1 })\
    </script>";
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                slow_response: Duration::ZERO,
                pages: vec![("/start".to_string(), START.to_string())],
            },
            "/start",
        )
        .await,
    )
}

async fn run(runtime: &Runtime, tab: &Tab, code: &str, hooks: &str) -> RunOutcome {
    let outcome = runtime
        .run(RunRequest {
            code: code.to_string(),
            page: PageId::new(tab.target_id.clone()),
            timeout: RUN_TIMEOUT,
            cancel: CancellationToken::new(),
            artifacts: None,
            hooks: Some(RunHooks::new(hooks)),
        })
        .await;
    assert_eq!(
        outcome.status,
        RunStatus::Ok,
        "{:?} {outcome:?}",
        outcome.hook_trace
    );
    outcome
}

/// The records of the calls named `method`, in order.
fn calls<'a>(outcome: &'a RunOutcome, method: &str) -> Vec<&'a CallRecord> {
    outcome
        .calls
        .iter()
        .filter(|call| call.method == method)
        .collect()
}

fn before(call: &CallRecord) -> Option<BeforeHookResult> {
    call.hook.and_then(|hook| hook.before)
}

fn after(call: &CallRecord) -> Option<AfterHookResult> {
    call.hook.and_then(|hook| hook.after)
}

/// Done when 1 and 2: blocking the `pageScript` group stops the script's `page.evaluate`,
/// `page.setContent` and `locator.dispatchEvent`, and an `evaluateExpression` the script sends
/// to its bridge itself, with or without a forged `metadata.internal`. The page shows none of it.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_blocking_page_script_stops_every_page_script_and_the_bridge_too() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    let hooks =
        "neo.on('beforeAction', { groups: ['pageScript'] }, () => ({ block: 'no page scripts' }))";
    let code = format!(
        "const tried = {{}}
         const attempts = {{
           evaluate: () => page.evaluate(() => {{ window.__touched = true }}),
           setContent: () => page.setContent('<p id=p>replaced</p>'),
           dispatchEvent: () => page.locator('#b').dispatchEvent('click'),
         }}
         for (const [name, attempt] of Object.entries(attempts)) {{
           try {{ await attempt(); tried[name] = 'ran' }}
           catch (error) {{ tried[name] = error.message.includes('no page scripts') ? 'blocked' : error.message }}
         }}
         for (const internal of [false, true]) {{
           const message = {{
             id: 9000 + Number(internal), guid: 'frame@{target}', method: 'evaluateExpression',
             params: {{ expression: '() => {{ window.__direct = true }}', isFunction: true,
                       arg: {{ value: {{ v: 'undefined' }}, handles: [] }} }},
             metadata: internal ? {{ internal: true }} : {{}},
           }}
           const {{ reply }} = await __neo_runtime.callWithEvents(JSON.stringify(message))
           tried[internal ? 'bridgeInternal' : 'bridge'] = JSON.parse(reply).error ? 'blocked' : 'ran'
         }}
         return tried",
        target = tab.target_id
    );
    let outcome = run(&runtime, &tab, &code, hooks).await;
    assert_eq!(
        outcome.value,
        Some(json!({
            "evaluate": "blocked",
            "setContent": "blocked",
            "dispatchEvent": "blocked",
            "bridge": "blocked",
            "bridgeInternal": "blocked",
        }))
    );
    let page = tab
        .evaluate(
            "({ touched: window.__touched ?? null, direct: window.__direct ?? null, \
              text: document.querySelector('#p').textContent, clicks: window.__clicks })",
        )
        .await;
    assert_eq!(
        page,
        json!({ "touched": null, "direct": null, "text": "start", "clicks": 0 }),
        "the page shows none of the blocked calls"
    );
    for method in ["evaluateExpression", "setContent", "dispatchEvent"] {
        let records = calls(&outcome, method);
        assert!(!records.is_empty(), "{method}");
        assert!(
            records
                .iter()
                .all(|call| before(call) == Some(BeforeHookResult::Blocked)),
            "{method}: {records:?}"
        );
    }
    assert_eq!(calls(&outcome, "evaluateExpression").len(), 3);
    tab.close().await;
}

/// Done when 3: a hook replaces `keyboard.type`'s text; the box receives the hook's text, while
/// the record keeps the script's params, says `argsReplaced`, keeps no call log, and the hook's
/// text appears nowhere in the outcome's JSON.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_replaced_arguments_take_effect_and_the_outcome_keeps_the_scripts() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    let hooks = "neo.on('beforeAction', { kinds: ['keyboard'] }, (event) =>
       event.method === 'keyboardType' ? { replaceArgs: { ...event.args, text: 's3cret-typed' } } : undefined)";
    let code = "await page.focus('#box'); await page.keyboard.type('public'); return 'typed'";
    let outcome = run(&runtime, &tab, code, hooks).await;
    assert_eq!(outcome.value, Some(json!("typed")));
    let typed = tab.evaluate("document.querySelector('#box').value").await;
    assert_eq!(typed, json!("s3cret-typed"), "the box got the hook's text");
    let records = calls(&outcome, "keyboardType");
    assert_eq!(records.len(), 1);
    let record = records[0];
    assert_eq!(record.params["text"], "public");
    assert_eq!(before(record), Some(BeforeHookResult::ArgsReplaced));
    assert!(record.log.is_empty(), "{:?}", record.log);
    assert_eq!(record.result, CallResult::Ok);
    let json = serde_json::to_string(&outcome).unwrap();
    assert!(!json.contains("s3cret"), "{json}");
    tab.close().await;
}

/// Done when 4: `afterAction` replaces `title`'s result (the script reads the replacement, the
/// record says `replaced` and keeps no log), its note reaches `RunOutcome::notes`, and one that
/// throws fails the call after the action ran.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_after_action_replaces_notes_and_fails_the_call_after_it_ran() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    let hooks = "neo.on('afterAction', { kinds: ['title'] }, () => ({ replace: { value: 'Hooked title' }, note: 'the title was redacted' }))
       neo.on('afterAction', { kinds: ['evaluate'] }, () => { throw new Error('s3cret failure') })";
    let code = "const title = await page.title()
       let evaluated
       try { await page.evaluate(() => { window.__ran = true }); evaluated = 'ok' }
       catch (error) { evaluated = error.message.includes('s3cret') ? 'leaked' : 'failed' }
       return { title, evaluated }";
    let outcome = run(&runtime, &tab, code, hooks).await;
    assert_eq!(
        outcome.value,
        Some(json!({ "title": "Hooked title", "evaluated": "failed" }))
    );
    assert_eq!(outcome.notes, ["the title was redacted"]);
    let ran = tab.evaluate("window.__ran ?? null").await;
    assert_eq!(
        ran,
        json!(true),
        "the evaluation ran before its hook failed"
    );
    let title = calls(&outcome, "title");
    assert_eq!(after(title[0]), Some(AfterHookResult::Replaced));
    assert!(title[0].log.is_empty(), "{:?}", title[0].log);
    let evaluated = calls(&outcome, "evaluateExpression");
    assert_eq!(after(evaluated[0]), Some(AfterHookResult::Failed));
    assert!(matches!(evaluated[0].result, CallResult::Error { .. }));
    let json = serde_json::to_string(&outcome).unwrap();
    assert!(!json.contains("s3cret"), "{json}");
    assert!(
        outcome
            .hook_trace
            .iter()
            .any(|line| line.contains("s3cret failure")),
        "the host reads why: {:?}",
        outcome.hook_trace
    );
    tab.close().await;
}

/// Done when 5: a `beforeAction` hook that itself calls `page.evaluate` while it blocks the
/// `pageScript` group for the agent: its own call runs, is not hooked (no recursion, no
/// deadlock) and is recorded as the hook's.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_hooks_own_page_script_runs_unhooked_while_the_agents_is_blocked() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    let hooks = "neo.on('beforeAction', { groups: ['pageScript'] }, async (event, ctx) => {
         ctx.trace('the hook read ' + await event.page.evaluate(() => document.title))
         return { block: 'agent page scripts are off' }
       })";
    let code = "try { await page.evaluate(() => { window.__agent = true }); return 'ran' }
       catch (error) { return error.message.includes('agent page scripts are off') ? 'blocked' : error.message }";
    let outcome = run(&runtime, &tab, code, hooks).await;
    assert_eq!(outcome.value, Some(json!("blocked")));
    assert!(
        outcome
            .hook_trace
            .iter()
            .any(|line| line.contains("the hook read Start")),
        "the hook's own evaluate ran: {:?}",
        outcome.hook_trace
    );
    let agent = tab.evaluate("window.__agent ?? null").await;
    assert_eq!(agent, Value::Null);
    let evaluations = calls(&outcome, "evaluateExpression");
    let hooks_own: Vec<&&CallRecord> = evaluations
        .iter()
        .filter(|call| call.origin == CallOrigin::Hook)
        .collect();
    assert_eq!(hooks_own.len(), 1, "{evaluations:?}");
    assert_eq!(hooks_own[0].hook, None, "a hook's own call is never hooked");
    assert_eq!(hooks_own[0].result, CallResult::Ok);
    let agents: Vec<&&CallRecord> = evaluations
        .iter()
        .filter(|call| call.origin == CallOrigin::Agent)
        .collect();
    assert_eq!(agents.len(), 1);
    assert_eq!(before(agents[0]), Some(BeforeHookResult::Blocked));
    tab.close().await;
}
