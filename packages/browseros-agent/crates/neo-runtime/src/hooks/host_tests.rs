//! Neo: host exchanges through the real SDK and hook actor, under paused time.

use std::borrow::Cow;
use std::future;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Barrier;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::host::Host;
use super::realm::{Bridge, Setup};
use super::{Gate, HookEvent, HookFailure, HookRun, HookSandbox, Invoked};
use crate::deadline::Deadline;
use crate::quickjs;
use crate::run::{HostCallback, RunOutcome, RunStatus};

/// Neo: short invocation budget for exact paused-clock assertions.
const BUDGET: Duration = Duration::from_millis(300);

fn far() -> Deadline {
    Deadline::new(Duration::from_hours(1), CancellationToken::new())
}

fn sandbox(
    code: &str,
    callback: Option<HostCallback>,
    deadline: Deadline,
) -> (HookRun, TaskTracker) {
    let tasks = TaskTracker::new();
    let setup = Setup {
        host: Host::new(callback, deadline, tasks.clone()),
        code: format!("{code}\nneo.on('afterRun', (_event, ctx) => ({{ replace: ctx.state }}))"),
        meta: Value::Null,
        page_guid: "page@T".to_string(),
        frame_guid: "frame@T".to_string(),
        url: "https://a.test/".to_string(),
        bridge: Bridge::Bare,
        sdk: Cow::Borrowed(quickjs::SDK_BUNDLE),
        realms: Arc::default(),
    };
    (HookSandbox::spawn(setup, BUDGET, &tasks), tasks)
}

async fn state(hooks: &HookRun) -> Value {
    hooks
        .after_run(RunOutcome::returned(None, vec![]))
        .await
        .value
        .unwrap()
}

fn pending_host() -> (HostCallback, CancellationToken, CancellationToken) {
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
                future::pending().await
            })
        })
    };
    (callback, started, dropped)
}

async fn action(hooks: &HookRun) -> Invoked {
    hooks
        .invoke(
            HookEvent::BeforeAction,
            json!({"context":"context@default"}).to_string(),
            None,
        )
        .await
}

#[tokio::test(start_paused = true)]
async fn tests_that_parallel_host_calls_both_return_json() {
    let barrier = Arc::new(Barrier::new(2));
    let callback: HostCallback = Arc::new(move |request| {
        let barrier = Arc::clone(&barrier);
        Box::pin(async move {
            barrier.wait().await;
            Ok(json!({"name": request.name, "args": request.args}))
        })
    });
    let (hooks, _tasks) = sandbox(
        "neo.on('beforeRun', async (_event, ctx) => {
           ctx.state.answers = await Promise.all([ctx.host('one', { n: 1 }), ctx.host('two')])
         })",
        Some(callback),
        far(),
    );
    assert!(matches!(hooks.before_run("", &far()).await, Gate::Start));
    assert_eq!(
        state(&hooks).await,
        json!({"answers": [
            {"name": "one", "args": {"n": 1}}, {"name": "two", "args": null}
        ]})
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_host_errors_and_json_limits_reject_and_block_the_gate() {
    let callbacks: Vec<Option<HostCallback>> = vec![
        None,
        Some(Arc::new(|_| {
            Box::pin(async { Err("host failed".to_string()) })
        })),
        Some(Arc::new(|_| {
            Box::pin(async { Ok(json!("x".repeat(1024 * 1024))) })
        })),
    ];
    for callback in callbacks {
        let (hooks, _tasks) = sandbox(
            "neo.on('beforeRun', async (_event, ctx) => {
               try { await ctx.host('vault.login') }
               catch (e) { ctx.state.error = e instanceof Error; throw e }
             })",
            callback,
            far(),
        );
        let Gate::End(outcome) = hooks.before_run("", &far()).await else {
            panic!("gate opened")
        };
        assert_eq!(outcome.status, RunStatus::Blocked);
        assert_eq!(state(&hooks).await, json!({"error": true}));
    }
    let callback: HostCallback = Arc::new(|_| panic!("oversize request reached the host"));
    let (hooks, _tasks) = sandbox(
        "neo.on('beforeRun', async (_event, ctx) => { await ctx.host('big', 'é'.repeat(524288)) })",
        Some(callback),
        far(),
    );
    let Gate::End(outcome) = hooks.before_run("", &far()).await else {
        panic!("gate opened")
    };
    assert_eq!(outcome.status, RunStatus::Blocked);
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_host_wait_spends_exactly_the_budget_and_drops_its_future() {
    let (callback, started, dropped) = pending_host();
    let (hooks, _tasks) = sandbox(
        "neo.on('beforeAction', async (_event, ctx) => { try { await ctx.host('wait') } catch {} })",
        Some(callback),
        far(),
    );
    assert!(matches!(hooks.before_run("", &far()).await, Gate::Start));
    let before = Instant::now();
    assert!(matches!(
        action(&hooks).await,
        Invoked::Failed(HookFailure::Timeout)
    ));
    assert_eq!(before.elapsed(), BUDGET);
    assert!(started.is_cancelled());
    dropped.cancelled().await;
    assert_eq!(before.elapsed(), BUDGET);
}

#[tokio::test(start_paused = true)]
async fn tests_that_run_cancel_and_deadline_drop_the_host_and_reject_in_the_hook() {
    for cancel_run in [false, true] {
        let cancel = CancellationToken::new();
        let deadline = Deadline::new(Duration::from_millis(100), cancel.clone());
        let (callback, started, dropped) = pending_host();
        let (hooks, _tasks) = sandbox(
            "neo.on('beforeAction', async (_event, ctx) => {
               try { await ctx.host('wait'); ctx.state.resolved = true }
               catch (e) { ctx.state.rejected = e instanceof Error }
             })",
            Some(callback),
            deadline,
        );
        assert!(matches!(hooks.before_run("", &far()).await, Gate::Start));
        let stopping = async {
            started.cancelled().await;
            if cancel_run {
                cancel.cancel();
            }
        };
        let (invoked, ()) = tokio::join!(action(&hooks), stopping);
        assert!(matches!(invoked, Invoked::Decided(_)), "{invoked:?}");
        dropped.cancelled().await;
        assert_eq!(state(&hooks).await, json!({"rejected": true}));
    }
}

#[tokio::test(start_paused = true)]
async fn tests_that_host_calls_after_a_run_stop_reject_without_calling_the_host() {
    let cancel = CancellationToken::new();
    let deadline = Deadline::new(Duration::from_hours(1), cancel.clone());
    let callback: HostCallback = Arc::new(|_| panic!("stopped run called host"));
    let (hooks, _tasks) = sandbox(
        "neo.on('beforeAction', async (_event, ctx) => {
           try { await ctx.host('late') } catch (e) { ctx.state.rejected = e instanceof Error }
         })",
        Some(callback),
        deadline,
    );
    assert!(matches!(hooks.before_run("", &far()).await, Gate::Start));
    cancel.cancel();
    assert!(matches!(action(&hooks).await, Invoked::Decided(_)));
    assert_eq!(state(&hooks).await, json!({"rejected": true}));
}

#[tokio::test(start_paused = true)]
async fn tests_that_abandoning_a_job_drops_its_host_while_the_realm_is_frozen() {
    let (callback, started, dropped) = pending_host();
    let (hooks, _tasks) = sandbox(
        "neo.on('beforeAction', async (_event, ctx) => {
           try { await ctx.host('wait'); ctx.state.resolved = true } catch {}
         })",
        Some(callback),
        far(),
    );
    assert!(matches!(hooks.before_run("", &far()).await, Gate::Start));
    let mut pending = Box::pin(action(&hooks));
    tokio::select! {
        () = started.cancelled() => {},
        _ = &mut pending => panic!("host wait completed"),
    }
    drop(pending);
    let before = Instant::now();
    dropped.cancelled().await;
    assert_eq!(before.elapsed(), Duration::ZERO);
    assert_eq!(state(&hooks).await, json!({}));
}

#[tokio::test(start_paused = true)]
async fn tests_that_an_unawaited_host_call_ends_with_its_successful_job() {
    let (callback, started, dropped) = pending_host();
    let (hooks, _tasks) = sandbox(
        "neo.on('beforeRun', async (_event, ctx) => {
           ctx.host('wait').then(() => { ctx.state.resolved = true }, () => {})
           await new Promise(resolve => setTimeout(resolve, 1))
         })",
        Some(callback),
        far(),
    );
    assert!(matches!(hooks.before_run("", &far()).await, Gate::Start));
    assert!(started.is_cancelled());
    let before = Instant::now();
    dropped.cancelled().await;
    assert_eq!(before.elapsed(), Duration::ZERO);
    assert_eq!(state(&hooks).await, json!({}));
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_buffered_host_answer_cannot_resolve_in_a_later_job() {
    let tasks = TaskTracker::new();
    let callback: HostCallback = Arc::new(|_| Box::pin(async { Ok(json!("secret")) }));
    let host = Host::new(Some(callback), far(), tasks.clone());
    let close = CancellationToken::new();
    let job = host.arm(Instant::now() + BUDGET, &close);
    let reply = host.call(r#"{"name":"x"}"#);
    tasks.close();
    tasks.wait().await;
    drop(job);
    let _next = host.arm(Instant::now() + BUDGET, &close);
    assert!(reply.await.is_err());
}
