//! Neo: the hook sandbox on its own, with a stand-in for the SDK's `createHooks` and a bare
//! `__neo_runtime` (only `log`), so no browser is needed. Time is paused except where a test
//! needs JS that spins, which only a real clock can stop.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::time;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::*;
use crate::run::{HOOK_BUDGET, RunBinary};

/// A stand-in for neo-quickjs-injected's `createHooks`, to the hooks contract: `neo.on` with
/// filters, registrations fixed by `finishLoading`, handlers run in order (the first block wins,
/// replacements chain, notes add up), a thrown handler is a `fail` decision, and `abort(id)`
/// aborts that invocation's `ctx.signal`. A handler that returns a string has it passed through
/// as the raw decision, so a test can hand Rust any decision text.
const STUB_SDK: &str = r"(() => {
  const EVENTS = ['beforeRun', 'afterRun', 'beforeAction', 'afterAction']
  const createHooks = (init) => {
    const registrations = []
    let loading = true
    const controllers = new Map()
    const state = {}
    let fail
    const uncaught = new Promise((_, reject) => { fail = reject })
    const neo = {
      on(event, filter, handler) {
        if (!loading) throw new TypeError('neo.on works only while the hook file loads')
        if (handler === undefined) { handler = filter; filter = undefined }
        if (!EVENTS.includes(event)) throw new TypeError(`no hook event ${event}`)
        if (filter !== undefined && !event.endsWith('Action'))
          throw new TypeError('a filter is only for action events')
        registrations.push({ event, filter, handler })
      },
      uncaught(error) { fail(error) },
    }
    const context = (signal) => ({
      meta: init.meta,
      state,
      signal,
      trace: (...values) => __neo_runtime.log(values.map(String).join(' ')),
    })
    const decide = async (event, payload, signal) => {
      const before = event.startsWith('before')
      const notes = []
      let replaced = false
      let value
      for (const { event: name, handler } of registrations) {
        if (name !== event) continue
        const result = await handler(payload, context(signal))
        if (typeof result === 'string') return result
        if (result === undefined) continue
        if (before && 'block' in result)
          return JSON.stringify({ decision: 'block', reason: result.block })
        if (before && 'replaceArgs' in result)
          return JSON.stringify({ decision: 'replaceArgs', args: result.replaceArgs })
        if (!before && 'replace' in result) {
          replaced = true
          value = result.replace
        }
        if (!before && 'note' in result) notes.push(result.note)
      }
      if (before) return JSON.stringify({ decision: 'allow' })
      return JSON.stringify(replaced ? { decision: 'replace', value, notes } : { decision: 'keep', notes })
    }
    return {
      neo,
      globals: {},
      uncaught,
      exportValue: (value) => JSON.stringify(value),
      finishLoading() {
        loading = false
        return JSON.stringify(registrations.map(({ event, filter }) => ({ event, ...filter })))
      },
      async invoke(id, event, payload) {
        const controller = new AbortController()
        controllers.set(id, controller)
        try {
          return await decide(event, JSON.parse(payload), controller.signal)
        } catch (error) {
          return JSON.stringify({ decision: 'fail', error: String(error) })
        } finally {
          controllers.delete(id)
        }
      },
      abort(id, reason) {
        controllers.get(id)?.abort(reason)
      },
    }
  }
  return { createHooks }
})()";

const BUDGET: Duration = Duration::from_millis(300);

fn sandbox_with(code: &str, budget: Duration, sdk: &'static str) -> (HookSandbox, TaskTracker) {
    let tasks = TaskTracker::new();
    let setup = Setup {
        code: code.to_string(),
        meta: json!({"who": "owner"}),
        page_guid: "page@T".to_string(),
        frame_guid: "frame@T".to_string(),
        url: "https://a.test/".to_string(),
        bridge: Bridge::Bare,
        sdk: Cow::Borrowed(sdk),
        realms: Arc::default(),
    };
    let sandbox = HookSandbox::spawn(setup, budget, &tasks);
    (sandbox, tasks)
}

fn sandbox(code: &str) -> (HookSandbox, TaskTracker) {
    sandbox_with(code, BUDGET, STUB_SDK)
}

fn far() -> Deadline {
    Deadline::new(Duration::from_secs(3600), CancellationToken::new())
}

fn action(method: &str, args: Value) -> ActionEvent {
    ActionEvent {
        kind: None,
        group: None,
        method: method.to_string(),
        guid: "frame@T".to_string(),
        args,
        location: None,
        page: Some("page@T".to_string()),
        frame: Some("frame@T".to_string()),
        context: "context@default".to_string(),
        locator: None,
        element: None,
    }
}

/// Loads `code`, which registers no `beforeRun` that blocks, and returns the loaded sandbox.
async fn loaded(code: &str) -> (HookSandbox, TaskTracker) {
    let (hooks, tasks) = sandbox(code);
    let gate = hooks.before_run("return 1", &far()).await;
    assert!(matches!(gate, Gate::Start), "{gate:?}");
    (hooks, tasks)
}

/// The outcome a gate ended the run with.
fn ended(gate: Gate) -> RunOutcome {
    match gate {
        Gate::End(outcome) => *outcome,
        Gate::Start => panic!("the run was expected to end before its script"),
    }
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_waiting_hook_fails_at_exactly_its_budget() {
    let (hooks, _tasks) = sandbox("neo.on('beforeRun', () => new Promise(() => {}))");
    let started = Instant::now();
    let outcome = ended(hooks.before_run("return 1", &far()).await);
    assert_eq!(started.elapsed(), BUDGET);
    assert_eq!(outcome.status, RunStatus::Blocked);
    assert!(outcome.value.is_none());

    let (hooks, _tasks) = loaded("neo.on('beforeAction', () => new Promise(() => {}))").await;
    let started = Instant::now();
    let decided = hooks.before_action(action("click", json!({}))).await;
    assert_eq!(decided, Err(HookFailure::Timeout));
    assert_eq!(started.elapsed(), BUDGET);
}

#[tokio::test(start_paused = true)]
async fn tests_that_invocations_queue_in_the_order_they_were_sent() {
    let (hooks, _tasks) = loaded(
        "neo.on('beforeAction', async (event, ctx) => {
           ctx.trace('start', event.method)
           await new Promise((resolve) => setTimeout(resolve, event.args.wait))
           ctx.trace('end', event.method)
         })",
    )
    .await;
    let started = Instant::now();
    let first = async {
        let decided = hooks
            .before_action(action("first", json!({"wait": 100})))
            .await;
        (decided, started.elapsed())
    };
    let second = async {
        let decided = hooks
            .before_action(action("second", json!({"wait": 10})))
            .await;
        (decided, started.elapsed())
    };
    let ((first, first_at), (second, second_at)) = tokio::join!(first, second);
    assert_eq!(first, Ok(ActionDecision::Allow));
    assert_eq!(second, Ok(ActionDecision::Allow));
    assert_eq!(first_at, Duration::from_millis(100));
    assert_eq!(
        second_at,
        Duration::from_millis(110),
        "the second waits for the first"
    );
    assert_eq!(
        hooks.take_trace(),
        ["start first", "end first", "start second", "end second"]
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_before_run_allows_or_blocks_the_script() {
    for code in [
        "",
        "neo.on('afterRun', () => {})",
        "neo.on('beforeRun', () => {})",
        "neo.on('beforeRun', () => ({ allow: true }))",
        "neo.on('beforeRun', (event) => event.script.includes('checkout') ? { block: 'no' } : undefined)",
    ] {
        let (hooks, _tasks) = sandbox(code);
        let gate = hooks.before_run("return 1", &far()).await;
        assert!(matches!(gate, Gate::Start), "{code}: {gate:?}");
    }
    let (hooks, _tasks) = sandbox(
        "neo.on('beforeRun', (event) => event.script.includes('checkout') ? { block: 'owner approval needed' } : undefined)
         neo.on('beforeRun', (event, ctx) => { ctx.trace('second handler ran') })",
    );
    let outcome = ended(hooks.before_run("await checkout()", &far()).await);
    assert_eq!(outcome.status, RunStatus::Blocked);
    let error = outcome.error.expect("a blocked run says why");
    assert_eq!(
        (error.name.as_str(), error.message.as_str()),
        ("Error", "owner approval needed")
    );
    assert!(outcome.value.is_none());
    assert!(
        hooks.take_trace().is_empty(),
        "the first block wins and later handlers do not run"
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_failing_before_run_blocks_without_its_detail() {
    let failing = [
        "neo.on('beforeRun', () => { throw new Error('secret-1234') })",
        "neo.on('beforeRun', async () => { throw new Error('secret-1234') })",
        r#"neo.on('beforeRun', () => '{"decision":"keep","notes":["secret-1234"]}')"#,
        r#"neo.on('beforeRun', () => '{"decision":"block","reason":""}')"#,
        r"neo.on('beforeRun', () => 'secret-1234')",
    ];
    for code in failing {
        let (hooks, _tasks) = sandbox(code);
        let outcome = ended(hooks.before_run("return 1", &far()).await);
        assert_eq!(outcome.status, RunStatus::Blocked, "{code}");
        let error = outcome.error.expect("a blocked run has an error");
        assert_eq!(error.name, "Error");
        assert!(
            !error.message.contains("secret-1234"),
            "{code}: the hook's text never reaches the model-visible error"
        );
        assert!(!hooks.take_trace().is_empty(), "{code}: the trace says why");
    }
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_hook_file_that_cannot_load_blocks_the_run() {
    let broken = [
        "this is not javascript",
        "throw new Error('at load')",
        "await Promise.reject(new Error('at load'))",
        "neo.on('beforeLunch', () => {})",
        "neo.on('beforeRun', { kinds: ['fill'] }, () => {})",
        "await new Promise(() => {})",
    ];
    for code in broken {
        let (hooks, _tasks) = sandbox(code);
        let started = Instant::now();
        let outcome = ended(hooks.before_run("return 1", &far()).await);
        assert_eq!(outcome.status, RunStatus::Blocked, "{code}");
        assert!(started.elapsed() <= BUDGET, "{code}");
        assert!(!hooks.take_trace().is_empty(), "{code}");
    }
    let (hooks, _tasks) = sandbox_with("", BUDGET, "({})");
    let outcome = ended(hooks.before_run("return 1", &far()).await);
    assert_eq!(
        outcome.status,
        RunStatus::Blocked,
        "an SDK without createHooks"
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_the_run_deadline_and_cancel_end_loading_and_before_run_first() {
    let deadline = Deadline::new(Duration::from_millis(100), CancellationToken::new());
    let (hooks, _tasks) = sandbox_with("await new Promise(() => {})", HOOK_BUDGET, STUB_SDK);
    let started = Instant::now();
    let outcome = ended(hooks.before_run("return 1", &deadline).await);
    assert_eq!(outcome.status, RunStatus::Timeout);
    assert_eq!(started.elapsed(), Duration::from_millis(100));

    let deadline = Deadline::new(Duration::from_millis(100), CancellationToken::new());
    let (hooks, _tasks) = sandbox_with(
        "neo.on('beforeRun', () => new Promise(() => {}))
         neo.on('afterRun', (event) => ({ note: event.status }))",
        HOOK_BUDGET,
        STUB_SDK,
    );
    let started = Instant::now();
    let outcome = ended(hooks.before_run("return 1", &deadline).await);
    assert_eq!(outcome.status, RunStatus::Timeout);
    assert_eq!(started.elapsed(), Duration::from_millis(100));
    // afterRun still runs, on the timed-out run.
    let outcome = hooks.after_run(outcome).await;
    assert_eq!(outcome.status, RunStatus::Timeout);
    assert_eq!(outcome.notes, ["timeout"]);

    let cancel = CancellationToken::new();
    let deadline = Deadline::new(Duration::from_secs(60), cancel.clone());
    let (hooks, _tasks) = sandbox("neo.on('beforeRun', () => new Promise(() => {}))");
    let gate = async { hooks.before_run("return 1", &deadline).await };
    let cancelling = async {
        time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
    };
    let (gate, ()) = tokio::join!(gate, cancelling);
    assert_eq!(ended(gate).status, RunStatus::Cancelled);
}

#[tokio::test(start_paused = true)]
async fn tests_that_after_run_keeps_replaces_or_notes() {
    let ok = || {
        RunOutcome::returned(Some(json!({"secret": "s3"})), vec!["log".to_string()]).with_binaries(
            vec![RunBinary {
                pointer: "/shot".to_string(),
                bytes: vec![1, 2],
            }],
        )
    };
    let (hooks, _tasks) = loaded("neo.on('afterRun', () => {})").await;
    assert_eq!(hooks.after_run(ok()).await, ok(), "undefined keeps");

    let (hooks, _tasks) = loaded(
        "neo.on('afterRun', (event) => ({ replace: 'redacted', note: `was ${event.status}` }))
         neo.on('afterRun', (event, ctx) => ({ note: JSON.stringify(ctx.meta) }))",
    )
    .await;
    let outcome = hooks.after_run(ok()).await;
    assert_eq!(outcome.status, RunStatus::Ok);
    assert_eq!(outcome.value, Some(json!("redacted")));
    assert!(
        outcome.binaries.is_empty(),
        "the replaced value's binaries go"
    );
    assert_eq!(outcome.notes, ["was ok", r#"{"who":"owner"}"#]);
    assert_eq!(outcome.logs, ["log"]);

    let (hooks, _tasks) = loaded("neo.on('afterRun', () => ({ replace: undefined }))").await;
    let outcome = hooks.after_run(ok()).await;
    assert_eq!((outcome.status, outcome.value), (RunStatus::Ok, None));

    let (hooks, _tasks) =
        loaded("neo.on('afterRun', (event) => ({ note: event.value.secret }))").await;
    assert_eq!(hooks.after_run(ok()).await.notes, ["s3"]);
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_failing_after_run_withholds_the_result() {
    let failed = RunOutcome::failed(RunError::new("TypeError", "x".to_string()), Vec::new());
    let ok = RunOutcome::returned(Some(json!("secret-1234")), Vec::new());
    let cases = [
        (
            "neo.on('afterRun', () => { throw new Error('secret-1234') })",
            &ok,
        ),
        ("neo.on('afterRun', () => new Promise(() => {}))", &ok),
        (r#"neo.on('afterRun', () => '{"decision":"allow"}')"#, &ok),
        (
            r#"neo.on('afterRun', () => '{"decision":"replace","value":1,"extra":2}')"#,
            &ok,
        ),
        ("neo.on('afterRun', () => ({ replace: 'swap' }))", &failed),
    ];
    for (code, outcome) in cases {
        let (hooks, _tasks) = loaded(code).await;
        let outcome = hooks.after_run(outcome.clone()).await;
        assert_eq!(outcome.status, RunStatus::Error, "{code}");
        assert!(outcome.value.is_none(), "{code}");
        let error = outcome.error.expect("a withheld result has an error");
        assert_eq!(error.name, "Error", "{code}");
        assert!(!error.message.contains("secret-1234"), "{code}");
        assert_ne!(
            error.message, "x",
            "{code}: the script's own error is replaced too"
        );
        assert!(!hooks.take_trace().is_empty(), "{code}");
    }
}

#[tokio::test(start_paused = true)]
async fn tests_that_after_run_skips_a_blocked_run_and_runs_without_a_handler_never() {
    let (hooks, _tasks) = sandbox(
        "neo.on('beforeRun', () => ({ block: 'stop' }))
         neo.on('afterRun', () => { throw new Error('must not run') })",
    );
    let outcome = ended(hooks.before_run("return 1", &far()).await);
    let after = hooks.after_run(outcome.clone()).await;
    assert_eq!(after, outcome);
    assert!(hooks.take_trace().is_empty());
}

#[tokio::test(start_paused = true)]
async fn tests_that_action_hooks_decide_and_fold() {
    let (hooks, _tasks) = loaded(
        "neo.on('beforeAction', (event) => event.method === 'fill'
           ? { replaceArgs: { ...event.args, value: 'real' } }
           : event.method === 'click' ? { block: 'no clicks' } : undefined)
         neo.on('afterAction', (event) => event.method === 'textContent'
           ? { replace: { value: '***' }, note: 'redacted' } : undefined)",
    )
    .await;
    assert_eq!(
        hooks
            .before_action(action("fill", json!({"value": "{{password}}"})))
            .await,
        Ok(ActionDecision::ReplaceArgs(json!({"value": "real"})))
    );
    assert_eq!(
        hooks.before_action(action("click", json!({}))).await,
        Ok(ActionDecision::Block("no clicks".to_string()))
    );
    assert_eq!(
        hooks.before_action(action("goto", json!({}))).await,
        Ok(ActionDecision::Allow)
    );
    assert_eq!(
        hooks
            .after_action(
                action("textContent", json!({})),
                ActionEnd::Result(json!({"value": "secret"}))
            )
            .await,
        Ok(AfterDecision {
            replace: Some(Replaced {
                value: Some(json!({"value": "***"})),
                binaries: Vec::new(),
            }),
            notes: vec!["redacted".to_string()],
        })
    );
    assert_eq!(
        hooks
            .after_action(
                action("goto", json!({})),
                ActionEnd::Error(RunError::new("TimeoutError", "late".to_string()))
            )
            .await,
        Ok(AfterDecision {
            replace: None,
            notes: Vec::new(),
        })
    );
    let (hooks, _tasks) =
        loaded(r#"neo.on('beforeAction', () => '{"decision":"keep","notes":[]}')"#).await;
    assert!(matches!(
        hooks.before_action(action("goto", json!({}))).await,
        Err(HookFailure::BadDecision(_))
    ));
    let (hooks, _tasks) = loaded("neo.on('afterAction', () => { throw new Error('x') })").await;
    assert!(matches!(
        hooks
            .after_action(action("goto", json!({})), ActionEnd::Result(json!({})))
            .await,
        Err(HookFailure::Threw(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn tests_that_wants_reads_the_registrations_without_the_realm() {
    let code = "neo.on('beforeAction', { kinds: ['fill'] }, () => {})
                neo.on('afterAction', { groups: ['pointer'] }, () => {})";
    let (hooks, _tasks) = sandbox(code);
    assert!(
        !hooks.wants(ActionHook::Before, Some("fill"), None),
        "nothing is wanted before the hooks load"
    );
    let (hooks, _tasks) = loaded(code).await;
    assert!(hooks.wants(ActionHook::Before, Some("fill"), Some("type")));
    assert!(!hooks.wants(ActionHook::Before, Some("click"), Some("pointer")));
    assert!(!hooks.wants(ActionHook::Before, None, None));
    assert!(hooks.wants(ActionHook::After, Some("click"), Some("pointer")));
    assert!(!hooks.wants(ActionHook::After, Some("fill"), Some("type")));
    let (hooks, _tasks) = loaded("neo.on('beforeAction', () => {})").await;
    assert!(hooks.wants(ActionHook::Before, None, None));
    assert!(!hooks.wants(ActionHook::After, None, None));
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_job_without_a_decision_aborts_its_signal() {
    let (hooks, _tasks) = loaded(
        "neo.on('beforeAction', (event, ctx) => {
           ctx.signal.addEventListener('abort', () => ctx.trace('aborted', event.method))
           return new Promise(() => {})
         })",
    )
    .await;
    assert_eq!(
        hooks.before_action(action("spent", json!({}))).await,
        Err(HookFailure::Timeout)
    );
    // A caller that stops waiting abandons its invocation, and the next one runs at once.
    let abandoned = time::timeout(
        Duration::from_millis(50),
        hooks.before_action(action("dropped", json!({}))),
    )
    .await;
    assert!(abandoned.is_err());
    let started = Instant::now();
    let next = hooks.before_action(action("next", json!({}))).await;
    assert_eq!(next, Err(HookFailure::Timeout));
    assert_eq!(
        started.elapsed(),
        BUDGET,
        "the next invocation starts when it is sent"
    );
    assert_eq!(
        hooks.take_trace(),
        ["aborted spent", "aborted dropped", "aborted next"]
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_an_uncaught_error_in_the_realm_fails_the_job_it_runs_in() {
    let (hooks, _tasks) = loaded(
        "neo.on('beforeAction', async () => {
           setTimeout(() => { throw new Error('from a timer') }, 0)
           await new Promise((resolve) => setTimeout(resolve, 50))
         })",
    )
    .await;
    assert!(matches!(
        hooks.before_action(action("goto", json!({}))).await,
        Err(HookFailure::Threw(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn tests_that_closing_ends_the_actor_and_fails_later_jobs() {
    let (hooks, tasks) = loaded("neo.on('beforeAction', () => {})").await;
    hooks.close();
    tasks.close();
    tasks.wait().await;
    assert!(matches!(
        hooks.before_action(action("goto", json!({}))).await,
        Err(HookFailure::Realm(_))
    ));
    let (hooks, tasks) = sandbox("neo.on('beforeAction', () => {})");
    let guard = hooks.close_on_drop();
    drop(guard);
    tasks.close();
    tasks.wait().await;
    let outcome = ended(hooks.before_run("return 1", &far()).await);
    assert_eq!(
        outcome.status,
        RunStatus::Blocked,
        "a closed sandbox never loads"
    );
}

/// Only a real clock stops JS that spins: the interrupt handler reads it while JS runs.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_spinning_hook_is_stopped_at_its_budget_and_the_realm_lives_on() {
    let margin = Duration::from_secs(3);
    let (hooks, _tasks) = sandbox("while (true) {}");
    let started = Instant::now();
    let outcome = ended(hooks.before_run("return 1", &far()).await);
    assert_eq!(outcome.status, RunStatus::Blocked);
    assert!(started.elapsed() >= BUDGET && started.elapsed() < BUDGET + margin);

    let (hooks, _tasks) = sandbox(
        "neo.on('beforeAction', (event) => { if (event.method === 'spin') while (true) {} })",
    );
    assert!(matches!(
        hooks.before_run("return 1", &far()).await,
        Gate::Start
    ));
    let started = Instant::now();
    assert_eq!(
        hooks.before_action(action("spin", json!({}))).await,
        Err(HookFailure::Timeout)
    );
    assert!(started.elapsed() >= BUDGET && started.elapsed() < BUDGET + margin);
    assert_eq!(
        hooks.before_action(action("goto", json!({}))).await,
        Ok(ActionDecision::Allow),
        "the realm still answers after an interrupt"
    );
}

/// neo-quickjs-injected's own `createHooks` loads in the hook realm, blocks, notes and traces,
/// and a `replace` value's binaries leave the realm as the agent's result's do.
#[tokio::test(start_paused = true)]
async fn tests_that_the_sdks_create_hooks_decides_through_the_realm() {
    let (hooks, _tasks) = sandbox_with(
        "neo.on('beforeRun', (event, ctx) => {
           ctx.trace('saw', event.script)
           if (event.script.includes('checkout')) return { block: 'owner approval needed' }
         })
         neo.on('afterRun', (event, ctx) => ({
           replace: { shot: Buffer.from([1, 2, 3]), seen: event.value, who: ctx.meta.who },
           note: 'swapped',
         }))",
        BUDGET,
        quickjs::SDK_BUNDLE,
    );
    let outcome = ended(hooks.before_run("await checkout()", &far()).await);
    assert_eq!(outcome.status, RunStatus::Blocked);
    assert_eq!(
        outcome.error.map(|error| error.message),
        Some("owner approval needed".to_string())
    );
    assert_eq!(hooks.take_trace(), ["saw await checkout()"]);

    let (hooks, _tasks) = sandbox_with(
        "neo.on('afterRun', (event, ctx) => ({
           replace: { shot: Buffer.from([1, 2, 3]), seen: event.value, who: ctx.meta.who },
           note: 'swapped',
         }))",
        BUDGET,
        quickjs::SDK_BUNDLE,
    );
    assert!(matches!(
        hooks.before_run("return 7", &far()).await,
        Gate::Start
    ));
    let outcome = hooks
        .after_run(RunOutcome::returned(Some(json!(7)), Vec::new()))
        .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", hooks.take_trace());
    assert_eq!(
        outcome.value,
        Some(json!({"shot": {"type": "Buffer", "bytes": 3}, "seen": 7, "who": "owner"}))
    );
    assert_eq!(
        outcome.binaries,
        [RunBinary {
            pointer: "/shot".to_string(),
            bytes: vec![1, 2, 3],
        }]
    );
    assert_eq!(outcome.notes, ["swapped"]);
}
