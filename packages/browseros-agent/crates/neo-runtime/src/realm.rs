//! One fresh QuickJS runtime per run: the script's whole world. It holds the host globals a
//! Playwright script expects (`console`, `setTimeout`, `clearTimeout`), `__neo_runtime`, and
//! the `page` and `context` objects neo-sdk builds, and nothing survives the run.
//!
//! The realm boundary is also where limits apply: a memory cap, an interrupt handler that
//! stops running JS at the deadline or on cancellation, and a `select!` that ends the run when
//! the script is parked on a timer or a promise instead of running.

use std::time::Duration;

use rquickjs::function::{Async, Func};
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Coerced, Constructor, Ctx, FromJs,
    Function, Object, Promise,
};
use serde_json::Value;

use crate::bridge::{Getters, Logs};
use crate::deadline::Deadline;
use crate::dispatch::Dispatcher;
use crate::run::{RunError, RunOutcome};
use crate::{bridge, world};

const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
// Well under a tokio worker's 2 MiB native stack, so deep JS recursion throws a RangeError
// before it can overflow the thread.
const MAX_STACK_BYTES: usize = 512 * 1024;

/// neo-sdk's bundle. Evaluating it defines no globals; its completion value is
/// `{ createRun }`. `scripts/neo-check.sh` fails when it is stale.
///
/// `ctx.eval` names its frames `eval_script`, and only the agent's code, built by the
/// `AsyncFunction` constructor, has frames named `<input>`. neo-sdk's `captureLocation` relies on
/// that to find the agent's call site in a stack (`tests/realm_limits.rs` pins it).
const SDK_BUNDLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-sdk/dist/sdk.js"
));

/// The host globals a Playwright script finds in Node. Evaluates to a function that takes the
/// native `sleep(ms)`, so the sleep itself never becomes a global.
const HOST_GLOBALS: &str = r"(sleep) => {
  let nextTimer = 1
  const timers = new Set()
  globalThis.setTimeout = (callback, delay, ...args) => {
    const id = nextTimer++
    timers.add(id)
    sleep(Math.max(0, Number(delay) || 0)).then(() => {
      if (timers.delete(id)) callback(...args)
    })
    return id
  }
  globalThis.clearTimeout = (id) => {
    timers.delete(id)
  }
  const format = (value) => {
    if (typeof value === 'string') return value
    try {
      return JSON.stringify(value) ?? String(value)
    } catch {
      return String(value)
    }
  }
  const log = (...values) => __neo_runtime.log(values.map(format).join(' '))
  globalThis.console = { log, info: log, warn: log, error: log, debug: log }
}";

/// How many lines QuickJS reports before the agent's line 1 in an `AsyncFunction`-constructed
/// function (`async function anonymous(page,context\n) {\n`). Measured by probe P4 and pinned by
/// `tests/realm_limits.rs`, which reads the line a call records, so a QuickJS upgrade that
/// changes the wrapper fails that test, not every recorded location.
const AGENT_LINE_OFFSET: u32 = 2;

/// What the realm needs to know about the run's page.
pub(crate) struct Script {
    pub(crate) code: String,
    pub(crate) page_guid: String,
    /// The page's main frame, built once by neo-sdk so `page.mainFrame()` is one object.
    pub(crate) frame_guid: String,
    /// The page's URL when the run started, which seeds `Page`'s memory of its last URL.
    pub(crate) url: String,
}

/// Runs `script` to completion, failure, its deadline or cancellation. Browser calls go to
/// `dispatcher`, whose tasks may outlive the realm; this returns with the realm dropped and
/// leaves ending those calls to the caller (`Dispatcher::finish`).
pub(crate) async fn run(
    script: Script,
    getters: Getters,
    dispatcher: Dispatcher,
    deadline: Deadline,
) -> RunOutcome {
    let logs = Logs::default();
    let evaluated = evaluate(script, getters, logs.clone(), dispatcher, deadline.clone());
    // Dropping `evaluated` here drops the whole realm mid-script, timers and pending
    // promises included; nothing outlives the run.
    let result = tokio::select! {
        stop = deadline.reached() => return RunOutcome::stopped(stop, deadline.timeout(), logs.take()),
        result = evaluated => result,
    };
    // A stop that fired during evaluation decides the outcome, however the script ended. It
    // reaches the script as a JS error from the interrupt handler or from a getter waiting on
    // CDP, and a getter's `TimeoutError` or cancellation error is catchable, so a script can
    // swallow it and return normally. Only the deadline knows the run did not finish in time.
    if let Some(stop) = deadline.stop() {
        return RunOutcome::stopped(stop, deadline.timeout(), logs.take());
    }
    match result {
        Ok(value) => RunOutcome::returned(value, logs.take()),
        Err(error) => RunOutcome::failed(error, logs.take()),
    }
}

async fn evaluate(
    script: Script,
    getters: Getters,
    logs: Logs,
    dispatcher: Dispatcher,
    deadline: Deadline,
) -> Result<Option<Value>, RunError> {
    let runtime = AsyncRuntime::new().map_err(|error| engine_error(&error))?;
    runtime.set_memory_limit(MEMORY_LIMIT_BYTES).await;
    runtime.set_max_stack_size(MAX_STACK_BYTES).await;
    let interrupt = deadline.clone();
    runtime
        .set_interrupt_handler(Some(Box::new(move || interrupt.stop().is_some())))
        .await;
    let context = AsyncContext::full(&runtime)
        .await
        .map_err(|error| engine_error(&error))?;
    context
        .async_with(async |ctx| {
            let returned = start(&ctx, script, getters, logs, dispatcher, deadline)
                .catch(&ctx)
                .map_err(|error| script_error(&ctx, error))?;
            let value: rquickjs::Value<'_> = returned
                .into_future()
                .await
                .catch(&ctx)
                .map_err(|error| script_error(&ctx, error))?;
            to_json(&ctx, value)
        })
        .await
}

/// Sets up the realm and starts the script, returning the promise of its async function.
fn start<'js>(
    ctx: &Ctx<'js>,
    script: Script,
    getters: Getters,
    logs: Logs,
    dispatcher: Dispatcher,
    deadline: Deadline,
) -> rquickjs::Result<Promise<'js>> {
    bridge::install(ctx, getters, logs, dispatcher)?;
    let sleep = Func::from(Async(move |delay_ms: f64| {
        let deadline = deadline.clone();
        async move { deadline.sleep(delay_from_millis(delay_ms)).await }
    }));
    let install_host_globals: Function<'js> = ctx.eval(HOST_GLOBALS)?;
    install_host_globals.call::<_, ()>((sleep,))?;

    let sdk: Object<'js> = ctx.eval(SDK_BUNDLE)?;
    let create_run: Function<'js> = sdk.get("createRun")?;
    let init = Object::new(ctx.clone())?;
    init.set("page", script.page_guid)?;
    init.set("mainFrame", script.frame_guid)?;
    init.set("context", "context@default")?;
    init.set("url", script.url)?;
    init.set("testIdAttribute", world::TEST_ID_ATTRIBUTE)?;
    init.set("lineOffset", AGENT_LINE_OFFSET)?;
    let objects: Object<'js> = create_run.call((init,))?;
    let page: rquickjs::Value<'js> = objects.get("page")?;
    let context: rquickjs::Value<'js> = objects.get("context")?;

    // The AsyncFunction constructor, as Playwright users write `new AsyncFunction(...)`: the
    // code is a function body that cannot close the wrapper early, and a syntax error in it
    // throws a SyntaxError here.
    let async_function: Constructor<'js> = ctx.eval("(async () => {}).constructor")?;
    let agent_function: Function<'js> =
        async_function.construct(("page", "context", script.code))?;
    agent_function.call((page, context))
}

/// `setTimeout` delays in JS milliseconds. A delay too large for a `Duration` is due after
/// any deadline, which [`Deadline::sleep`] treats as never.
fn delay_from_millis(delay_ms: f64) -> Duration {
    Duration::try_from_secs_f64(delay_ms / 1000.0).unwrap_or(Duration::MAX)
}

/// The script's return value as JSON, through the realm's own `JSON.stringify` so the
/// conversion is exactly JavaScript's. `undefined` (and anything else JSON drops) is `None`.
fn to_json<'js>(ctx: &Ctx<'js>, value: rquickjs::Value<'js>) -> Result<Option<Value>, RunError> {
    let text = match ctx.json_stringify(value).catch(ctx) {
        Ok(Some(text)) => text
            .to_string()
            .map_err(|error| RunError::new("Error", error.to_string()))?,
        Ok(None) => return Ok(None),
        Err(error) => return Err(script_error(ctx, error)),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| RunError::new("Error", format!("return value is not JSON: {error}")))
}

/// Names a thrown value the way the script saw it: an `Error`'s own `name` and `message`,
/// or `Error` plus the text of a thrown non-Error value.
fn script_error<'js>(ctx: &Ctx<'js>, error: CaughtError<'js>) -> RunError {
    match error {
        CaughtError::Exception(exception) => {
            let name: Option<Coerced<String>> = exception.as_object().get("name").ok();
            let name = name.map_or_else(|| "Error".to_string(), |name| name.0);
            RunError::new(&name, exception.message().unwrap_or_default())
        }
        CaughtError::Value(value) => {
            let text = Coerced::<String>::from_js(ctx, value)
                .map_or_else(|error| error.to_string(), |text| text.0);
            RunError::new("Error", text)
        }
        CaughtError::Error(error) => RunError::new("Error", error.to_string()),
    }
}

fn engine_error(error: &rquickjs::Error) -> RunError {
    RunError::new("Error", format!("QuickJS failed to start: {error}"))
}
