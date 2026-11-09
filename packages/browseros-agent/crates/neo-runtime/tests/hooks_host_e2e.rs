//! Neo: host calls through `Runtime::run` and a real page, including fail-closed gates and secrets.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the shared browser harness serves other suites too"
)]
mod browser;

use std::future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{
    CallOrigin, HostCallback, PageId, RunHooks, RunOutcome, RunRequest, RunStatus, Runtime,
};
use serde_json::{Value, json};
use tokio::time;
use tokio_util::sync::CancellationToken;

/// Neo: enough time for browser setup and page operations on a shared machine.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Neo: a host wait is bounded by this invocation budget, not a second timeout.
const BUDGET: Duration = Duration::from_millis(300);
/// Neo: a test secret that must not reach any serialized outcome field.
const PASSWORD: &str = "vault-only-password-5d9";
/// Neo: sets a page flag only if the gate allowed the script to start.
const TOUCH: &str = "await page.evaluate(() => { window.__agentStarted = true }); return 1";

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                slow_response: Duration::ZERO,
                pages: vec![(
                    "/login".to_string(),
                    "<!doctype html><input id=user><input id=pw type=password>".to_string(),
                )],
            },
            "/login",
        )
        .await,
    )
}

async fn run(
    runtime: &Runtime,
    tab: &Tab,
    hooks: RunHooks,
    cancel: CancellationToken,
) -> RunOutcome {
    runtime
        .run(RunRequest {
            code: TOUCH.to_string(),
            page: PageId::new(tab.target_id.clone()),
            timeout: TIMEOUT,
            cancel,
            artifacts: None,
            hooks: Some(hooks),
        })
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_hook_uses_vault_credentials_on_the_page_without_leaking_them() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let callback: HostCallback = {
        let seen = Arc::clone(&seen);
        Arc::new(move |request| {
            seen.lock().unwrap().push(request);
            Box::pin(async { Ok(json!({"username":"alice", "password":PASSWORD})) })
        })
    };
    let outcome = run(
        &runtime,
        &tab,
        RunHooks::new(
            "neo.on('beforeRun', async (event, ctx) => {
           const login = await ctx.host('vault.login', { origin: new URL(event.page.url()).origin })
           await event.page.evaluate(({ username, password }) => {
             document.querySelector('#user').value = username
             document.querySelector('#pw').value = password
           }, login)
         })",
        )
        .with_host(callback),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        tab.evaluate(
            "[document.querySelector('#user').value, document.querySelector('#pw').value]"
        )
        .await,
        json!(["alice", PASSWORD])
    );
    let origin = tab.evaluate("location.origin").await;
    {
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].name, "vault.login");
        assert_eq!(requests[0].args, json!({"origin":origin}));
    }
    assert_eq!(
        outcome.calls.len(),
        2,
        "only the hook and agent page evaluations are calls"
    );
    assert_eq!(outcome.calls[0].origin, CallOrigin::Hook);
    assert!(!serde_json::to_string(&outcome).unwrap().contains(PASSWORD));
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_host_rejections_block_the_script_and_keep_secrets_out_of_the_outcome() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    let callbacks: Vec<Option<HostCallback>> = vec![
        None,
        Some(Arc::new(|_| Box::pin(async { Err(PASSWORD.to_string()) }))),
        Some(Arc::new(|_| {
            Box::pin(async { Ok(json!(PASSWORD.repeat(100_000))) })
        })),
    ];
    for callback in callbacks {
        let mut hooks = RunHooks::new(
            "neo.on('beforeRun', async (_event, ctx) => { await ctx.host('vault.login') })",
        );
        hooks.host = callback;
        let outcome = run(&runtime, &tab, hooks, CancellationToken::new()).await;
        assert_eq!(outcome.status, RunStatus::Blocked, "{outcome:?}");
        assert!(outcome.calls.is_empty());
        assert_eq!(
            tab.evaluate("window.__agentStarted === true").await,
            json!(false)
        );
        assert!(!serde_json::to_string(&outcome).unwrap().contains(PASSWORD));
    }
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_host_timeout_or_run_cancel_drops_the_future_and_keeps_the_gate_closed() {
    let Some(tab) = open().await else { return };
    let runtime = tab.runtime().await;
    for cancel_run in [false, true] {
        let started = CancellationToken::new();
        let dropped = CancellationToken::new();
        let callback: HostCallback = {
            let started = started.clone();
            let dropped = dropped.clone();
            Arc::new(move |_| {
                let started = started.clone();
                let dropped = dropped.clone();
                Box::pin(async move {
                    let _drop = dropped.drop_guard();
                    started.cancel();
                    future::pending::<Result<Value, String>>().await
                })
            })
        };
        let cancel = CancellationToken::new();
        let hooks =
            RunHooks::new("neo.on('beforeRun', async (_event, ctx) => { await ctx.host('wait') })")
                .with_host(callback)
                .with_budget(if cancel_run { TIMEOUT } else { BUDGET });
        let stopping = async {
            time::timeout(TIMEOUT, started.cancelled()).await.unwrap();
            if cancel_run {
                cancel.cancel();
            }
        };
        let (outcome, ()) = tokio::join!(run(&runtime, &tab, hooks, cancel.clone()), stopping);
        assert_eq!(
            outcome.status,
            if cancel_run {
                RunStatus::Cancelled
            } else {
                RunStatus::Blocked
            },
            "{outcome:?}"
        );
        assert!(dropped.is_cancelled());
        assert!(outcome.calls.is_empty());
        assert_eq!(
            tab.evaluate("window.__agentStarted === true").await,
            json!(false)
        );
    }
    tab.close().await;
}
