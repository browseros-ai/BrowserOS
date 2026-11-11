//! One agent script, start to finish, in a QuickJS runtime of its own.

use std::future::Future;
use std::iter;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant as StdInstant};

use futures_util::future::BoxFuture;
use neo_trace::OperationId;
use rquickjs::context::EvalOptions;
use rquickjs::function::{Async, Func};
use rquickjs::runtime::InterruptHandler;
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Ctx, Function, Object, Promise,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time;

use super::install_web_globals::install_web_globals;
use super::{Bridge, RunLogs};
use crate::browser::PageRef;
use crate::builtins::PLAYWRIGHT_PRELUDE;
use crate::config::Defaults;
use crate::dispatch::{
    self, Dispatcher, ErrorCode, Observation, OpFailure, OperationError, PageFact, RunCtx,
    StopReason, TEXT_CHARS,
};

/// The bootstrap: `NeoError`, the codec, envelopes, call sites, console, timers, `__neo.run`.
const BOOTSTRAP: JsFile = JsFile {
    name: "bootstrap.js",
    source: include_str!("js/bootstrap.js"),
};
/// Evaluated after the bootstrap, in order. Each adds parameters of the agent's function through
/// `__neo.provide`: neo-playwright's prelude `page`, `context`, `browser` and `expect`,
/// and the neo prelude `neo`.
const PRELUDES: &[JsFile] = &[
    JsFile {
        name: "playwright_prelude.js",
        source: PLAYWRIGHT_PRELUDE,
    },
    JsFile {
        name: "neo_prelude.js",
        source: include_str!("js/neo_prelude.js"),
    },
];
/// The file name the agent's function compiles under. The bootstrap's call sites and the run's
/// error locations find the agent's own frames by it.
const AGENT_FILE: &str = "run";
/// Room for `{"ok":…}` around the value when the settled JSON is held to the result limit.
const SETTLED_OVERHEAD: usize = 16;
/// The bootstrap's prefix for a returned value, which the result limit applies to.
const SETTLED_OK: &str = r#"{"ok":"#;

/// One script to run. A plain data bag.
#[derive(Debug)]
pub(crate) struct Script {
    /// The body of the agent's async function, as the model wrote it.
    pub(crate) code: String,
    /// The `page` parameter's tab: the task's tab when the run service knows one, else `None`,
    /// and the run opens a tab on the first call that needs it.
    pub(crate) page: Option<PageRef>,
    /// Every tab open when the run started, as reply facts describe them, so `context.pages()`,
    /// which is synchronous in Playwright, lists each one before the script's first op. Empty
    /// when the run service could not list them.
    pub(crate) pages: Vec<PageFact>,
    /// The host observation that produced `pages`, shared with subsequent operation replies.
    pub(crate) observed: Option<Observation>,
}

/// How a script ended, and what it logged. A plain data bag with nothing JS-typed in it: the
/// runtime is gone by the time this exists.
#[derive(Debug)]
pub(crate) struct ScriptEnd {
    /// How it ended.
    pub(crate) outcome: ScriptOutcome,
    /// Its console output, within the run's limits.
    pub(crate) logs: RunLogs,
}

/// How a script ended.
#[derive(Debug)]
pub(crate) enum ScriptOutcome {
    /// It returned this value, JSON-safe and within `Defaults::MAX_RESULT_BYTES`.
    Returned(Value),
    /// A `NeoError` from one of its ops escaped: the run fails with that op's failure, exactly
    /// as the door decided it.
    OperationFailed(OpFailure),
    /// It failed by itself: it threw, did not compile, or hit a limit. The owner is named here.
    Failed(OperationError),
    /// Its run stopped (the caller left, the deadline, shutdown) before it settled. The run's
    /// stop reason says why.
    Stopped,
}

/// A JavaScript file the runtime evaluates, with the name its stack frames carry. A plain data
/// bag.
struct JsFile {
    name: &'static str,
    source: &'static str,
}

/// What the evaluation hands out of the runtime: strings only, so it can leave the `async_with`
/// future that every JS value lives in.
enum Evaluated {
    /// The bootstrap settled the run: `{"ok":…}` or `{"thrown":…}`.
    Settled(String),
    /// The agent's code did not compile; the message says where.
    Uncompiled(String),
    /// The run's promise rejected: QuickJS stopped the script (its interrupt, its heap), or a
    /// failure struck the bootstrap's own catch.
    Halted(String),
    /// QuickJS could not be set up.
    Engine(String),
    /// The script had not settled `Defaults::TEARDOWN_GRACE` after its run stopped.
    Unsettled,
}

/// The bootstrap's settled JSON.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Settled {
    Ok(Value),
    Thrown(Thrown),
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Thrown {
    /// A genuine `NeoError`, by the op it came from.
    Operation {
        #[serde(rename = "operationId")]
        operation_id: OperationId,
    },
    /// Anything else the script threw.
    Error {
        name: String,
        message: String,
        stack: String,
    },
}

/// Runs `code` in a fresh runtime and reports how it ended. It never panics on script input,
/// and it returns at most `Defaults::TEARDOWN_GRACE` after the run stops, with the runtime and
/// every JS value already freed.
///
/// The caller stops the run (`RunCtx::stop`) for the caller leaving, the deadline or shutdown;
/// this function only turns that stop into the end of the script.
///
/// Boxed as `Send` here, where the compiler can prove it: inside a spawned task it cannot see
/// through rquickjs's `for<'js>` closure state, and the run task is spawned.
pub(crate) fn run_script<'a>(
    run: &'a Arc<RunCtx>,
    dispatcher: &'a Arc<Dispatcher>,
    script: Script,
) -> BoxFuture<'a, ScriptEnd> {
    Box::pin(async move {
        let bridge = Arc::new(Bridge::new(Arc::clone(run), Arc::clone(dispatcher)));
        let cause = Arc::new(HaltCause::default());
        let outcome = match evaluate(run, &bridge, &cause, script).await {
            Evaluated::Settled(json) => settle(&json, &bridge),
            Evaluated::Uncompiled(message) => {
                ScriptOutcome::Failed(OperationError::script(ErrorCode::SYNTAX_ERROR, message))
            }
            Evaluated::Halted(message) => halted(run, cause.get(), &message),
            Evaluated::Engine(message) => ScriptOutcome::Failed(OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                format_args!("the JavaScript runtime failed: {message}"),
            )),
            Evaluated::Unsettled => ScriptOutcome::Stopped,
        };
        ScriptEnd {
            outcome,
            logs: bridge.take_logs(),
        }
    })
}

/// The runtime's whole life, in the order QuickJS needs: the evaluation future, which holds
/// every JS value, ends or is dropped inside the `select!`; then the context goes, then the
/// runtime. QuickJS aborts the process if a value outlives its runtime, so nothing JS-typed may
/// escape this function.
async fn evaluate(
    run: &Arc<RunCtx>,
    bridge: &Arc<Bridge>,
    cause: &Arc<HaltCause>,
    script: Script,
) -> Evaluated {
    let runtime = match AsyncRuntime::new() {
        Ok(runtime) => runtime,
        Err(error) => return Evaluated::Engine(error.to_string()),
    };
    runtime.set_memory_limit(Defaults::JS_MEMORY_BYTES).await;
    runtime.set_max_stack_size(Defaults::JS_STACK_BYTES).await;
    let clock = Arc::new(SliceClock::new());
    runtime
        .set_interrupt_handler(Some(interrupt_handler(run, &clock, cause)))
        .await;
    // full uses JS_NewContext, which installs QuickJS-ng's atob/btoa/DOMException.
    // rquickjs's intrinsic::All omits them; web-global regression tests pin this contract.
    let context = match AsyncContext::full(&runtime).await {
        Ok(context) => context,
        Err(error) => return Evaluated::Engine(error.to_string()),
    };
    // The prelude's `RunInit`: `page` is the run's own tab, `pages` every tab, whose order is
    // `context.pages()`'s (sdk-playwright's bridge.ts).
    let init = json!({ "page": script.page, "pages": script.pages, "observed": script.observed })
        .to_string();
    let code = script.code;
    let natives = Arc::clone(bridge);
    let evaluation = SliceTimed::new(
        clock,
        context.async_with(async move |ctx| evaluate_in(&ctx, &natives, &code, &init).await),
    );
    let evaluated = tokio::select! {
        biased;
        evaluated = evaluation => evaluated,
        () = unsettled_after_stop(run) => Evaluated::Unsettled,
    };
    runtime.set_interrupt_handler(None).await;
    drop(context);
    drop(runtime);
    evaluated
}

/// Inside the runtime's lock, one poll at a time: the natives, the bootstrap, the preludes, the
/// agent's function, then the bootstrap's `__neo.run` awaited to its settled JSON.
async fn evaluate_in<'js>(
    ctx: &Ctx<'js>,
    bridge: &Arc<Bridge>,
    code: &str,
    init: &str,
) -> Evaluated {
    let (neo_run, main) = match prepare(ctx, bridge, code) {
        Ok(prepared) => prepared,
        Err(evaluated) => return evaluated,
    };
    let promise: Promise<'js> = match neo_run.call((main, init.to_owned())).catch(ctx) {
        Ok(promise) => promise,
        Err(caught) => return Evaluated::Halted(caught.to_string()),
    };
    match promise.into_future::<String>().await.catch(ctx) {
        Ok(json) => Evaluated::Settled(json),
        Err(caught) => Evaluated::Halted(caught.to_string()),
    }
}

/// The realm, set up: `__neo.run` and the agent's compiled function. The function's parameters
/// are the agent's API, named by the bootstrap and the preludes (`__neo.parameters()`), so the
/// API never sits on the global object.
fn prepare<'js>(
    ctx: &Ctx<'js>,
    bridge: &Arc<Bridge>,
    code: &str,
) -> Result<(Function<'js>, Function<'js>), Evaluated> {
    install(ctx, bridge)
        .catch(ctx)
        .map_err(|caught| engine(&caught))?;
    install_web_globals(ctx, Defaults::JS_MEMORY_BYTES)
        .catch(ctx)
        .map_err(|caught| Evaluated::Engine(format!("web_globals.js: {caught}")))?;
    for file in iter::once(&BOOTSTRAP).chain(PRELUDES) {
        ctx.eval_with_options::<(), _>(file.source, options(file.name))
            .catch(ctx)
            .map_err(|caught| Evaluated::Engine(format!("{}: {caught}", file.name)))?;
    }
    let neo: Object<'js> = ctx
        .globals()
        .get("__neo")
        .catch(ctx)
        .map_err(|caught| engine(&caught))?;
    let parameters: Function<'js> = neo
        .get("parameters")
        .catch(ctx)
        .map_err(|caught| engine(&caught))?;
    let parameters: String = parameters
        .call(())
        .catch(ctx)
        .map_err(|caught| engine(&caught))?;
    // The body starts on line 2, which the bootstrap's call sites and `agent_site` subtract.
    let source = format!("(async function ({parameters}) {{\n{code}\n}})");
    let main: Function<'js> = ctx
        .eval_with_options(source, options(AGENT_FILE))
        .catch(ctx)
        .map_err(|caught| Evaluated::Uncompiled(compile_error(&caught)))?;
    let neo_run: Function<'js> = neo
        .get("run")
        .catch(ctx)
        .map_err(|caught| engine(&caught))?;
    Ok((neo_run, main))
}

/// The only natives on the global object (runtime/tests.rs pins the realm's additions). Each
/// closure owns an `Arc<Bridge>` and owned strings, never a JS value, so nothing JS-typed is
/// held across an await or outlives the runtime.
fn install(ctx: &Ctx<'_>, bridge: &Arc<Bridge>) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let invoke = {
        let bridge = Arc::clone(bridge);
        move |name: String, input: String, site: String| {
            // rquickjs calls this closure at the script's call and spawns the future it returns,
            // which first runs when the realm next yields to Rust. So the door admits the call
            // here: a script that settles before it yields again, as one that never awaits an op
            // does, would otherwise leave that op with no id and no record.
            let pending = bridge.submit(name, &input, &site);
            let bridge = Arc::clone(&bridge);
            async move { bridge.answer(pending).await }
        }
    };
    globals.set("__neo_invoke", Func::from(Async(invoke)))?;
    let log = {
        let bridge = Arc::clone(bridge);
        move |level: String, text: String| bridge.log(&level, text)
    };
    globals.set("__neo_log", Func::from(log))?;
    let sleep = {
        let bridge = Arc::clone(bridge);
        move |ms: f64| {
            let bridge = Arc::clone(&bridge);
            async move { bridge.sleep(ms).await }
        }
    };
    globals.set("__neo_sleep", Func::from(Async(sleep)))?;
    Ok(())
}

/// Global code in strict mode, named `file` in stack frames. Strict here, not by directive: the
/// repo's formatter treats the `.js` files as modules and strips their `"use strict"`.
fn options(file: &str) -> EvalOptions {
    let mut options = EvalOptions::default();
    options.global = true;
    options.strict = true;
    options.filename = Some(file.to_owned());
    options
}

fn engine(caught: &CaughtError<'_>) -> Evaluated {
    Evaluated::Engine(caught.to_string())
}

/// `SyntaxError: unexpected token in expression: ')' (run:3:12)`.
fn compile_error(caught: &CaughtError<'_>) -> String {
    match caught {
        CaughtError::Exception(exception) => {
            let name: String = exception
                .as_object()
                .get("name")
                .unwrap_or_else(|_| "SyntaxError".to_owned());
            let message = exception.message().unwrap_or_default();
            let site = exception.stack().as_deref().and_then(agent_site);
            located(&format!("{name}: {message}"), site.as_deref())
        }
        CaughtError::Error(_) | CaughtError::Value(_) => caught.to_string(),
    }
}

/// Resolves `TEARDOWN_GRACE` after the run stops. The interrupt handler ends running JS at once
/// and a stopped op answers at once, so only a script awaiting a promise that never settles
/// gets here; dropping its evaluation ends it.
async fn unsettled_after_stop(run: &RunCtx) {
    run.cancel_token().cancelled().await;
    time::sleep(Defaults::TEARDOWN_GRACE).await;
}

/// The one interrupt handler. QuickJS calls it every few thousand instructions from inside its
/// own C stack, so it must not panic, and it reads only a clock, atomics and a token.
fn interrupt_handler(
    run: &Arc<RunCtx>,
    clock: &Arc<SliceClock>,
    cause: &Arc<HaltCause>,
) -> InterruptHandler {
    let run = Arc::clone(run);
    let clock = Arc::clone(clock);
    let cause = Arc::clone(cause);
    let deadline = run.deadline().into_std();
    Box::new(move || {
        // The timer that stops a run at its deadline cannot fire while every worker runs JS, so
        // the handler watches the clock too. Whoever notices first stops the run, with the same
        // reason either way.
        if StdInstant::now() >= deadline {
            run.stop(StopReason::Deadline);
        }
        if run.cancel_token().is_cancelled() {
            cause.record(Halt::RunStopped);
            return true;
        }
        if clock.slice() > Defaults::JS_SLICE_BUDGET {
            cause.record(Halt::LongTask);
            return true;
        }
        false
    })
}

fn settle(json: &str, bridge: &Bridge) -> ScriptOutcome {
    // Before parsing, so a huge value is refused without building its tree. A thrown error's
    // text is not held to it: it parses into three strings, clipped below.
    if json.starts_with(SETTLED_OK) && json.len() > Defaults::MAX_RESULT_BYTES + SETTLED_OVERHEAD {
        return ScriptOutcome::Failed(OperationError::script(
            ErrorCode::RESULT_TOO_LARGE,
            format_args!(
                "the script returned {} bytes of JSON; a run returns at most {} KiB, so return less \
                 (a summary, or the fields you need)",
                json.len(),
                Defaults::MAX_RESULT_BYTES / 1024
            ),
        ));
    }
    match serde_json::from_str::<Settled>(json) {
        Ok(Settled::Ok(value)) => ScriptOutcome::Returned(value),
        Ok(Settled::Thrown(Thrown::Operation { operation_id })) => {
            match bridge.failure(operation_id) {
                Some(failure) => ScriptOutcome::OperationFailed(failure),
                None => ScriptOutcome::Failed(OperationError::neo(
                    ErrorCode::INTERNAL_ERROR,
                    format_args!(
                        "the script threw the error of operation {operation_id}, which this run has no record of"
                    ),
                )),
            }
        }
        Ok(Settled::Thrown(Thrown::Error {
            name,
            message,
            stack,
        })) => ScriptOutcome::Failed(thrown_error(name, message, &stack)),
        Err(error) => ScriptOutcome::Failed(OperationError::neo(
            ErrorCode::INTERNAL_ERROR,
            format_args!("the bootstrap settled the run with JSON Neo cannot read: {error}"),
        )),
    }
}

/// What the script threw, as the run's error, its name and message each cut to [`TEXT_CHARS`].
/// QuickJS reports a full heap as a catchable `InternalError`, which the bootstrap caught like any
/// other throw.
fn thrown_error(name: String, message: String, stack: &str) -> OperationError {
    if name == "InternalError" && message == "out of memory" {
        return out_of_memory();
    }
    let (name, message) = (
        dispatch::clip(name, TEXT_CHARS),
        dispatch::clip(message, TEXT_CHARS),
    );
    let site = agent_site(stack);
    let text = if name.is_empty() {
        message
    } else {
        format!("{name}: {message}")
    };
    OperationError::script(ErrorCode::SCRIPT_ERROR, located(&text, site.as_deref()))
}

fn halted(run: &RunCtx, cause: Option<Halt>, message: &str) -> ScriptOutcome {
    match cause {
        Some(Halt::LongTask) => ScriptOutcome::Failed(OperationError::script(
            ErrorCode::LONG_TASK,
            format_args!(
                "the script ran JavaScript for {} s without awaiting; split the work, or await \
                 between its parts",
                Defaults::JS_SLICE_BUDGET.as_secs()
            ),
        )),
        Some(Halt::RunStopped) => ScriptOutcome::Stopped,
        None if run.stop_reason().is_some() => ScriptOutcome::Stopped,
        None if message.contains("out of memory") => ScriptOutcome::Failed(out_of_memory()),
        None => ScriptOutcome::Failed(OperationError::neo(
            ErrorCode::INTERNAL_ERROR,
            format_args!("the script stopped unexpectedly: {message}"),
        )),
    }
}

fn out_of_memory() -> OperationError {
    OperationError::script(
        ErrorCode::OUT_OF_MEMORY,
        format_args!(
            "the script used all of its run's {} MiB of JavaScript memory",
            Defaults::JS_MEMORY_BYTES / (1024 * 1024)
        ),
    )
}

fn located(text: &str, site: Option<&str>) -> String {
    match site {
        Some(site) => format!("{text} ({site})"),
        None => text.to_owned(),
    }
}

/// The first `run:LINE:COL` frame in a stack, as a line of the agent's own code: the compiled
/// source has one line of wrapper before the body.
fn agent_site(stack: &str) -> Option<String> {
    let mut rest = stack;
    while let Some(at) = rest.find("run:") {
        let (before, after) = rest.split_at(at);
        rest = after.get("run:".len()..).unwrap_or_default();
        if !(before.is_empty() || before.ends_with(' ') || before.ends_with('(')) {
            continue;
        }
        let mut numbers = rest.split(|char: char| !char.is_ascii_digit());
        let line: u32 = match numbers.next().map(str::parse) {
            Some(Ok(line)) => line,
            _ => continue,
        };
        let column = numbers.next().and_then(|column| column.parse::<u32>().ok());
        let line = line.saturating_sub(1).max(1);
        return Some(match column {
            Some(column) => format!("run:{line}:{column}"),
            None => format!("run:{line}"),
        });
    }
    None
}

/// Stamps the start of every poll of the evaluation, so the interrupt handler can stop one
/// synchronous stretch of JS that runs past `Defaults::JS_SLICE_BUDGET`. Without it a
/// `for(;;){}` holds a tokio worker until the run's deadline (up to 900 s), and with no limit on
/// concurrent runs a handful of them stall the server, health checks included. Every JS
/// instruction runs inside one of these polls, so the time since the stamp bounds the slice.
struct SliceTimed<F> {
    inner: Pin<Box<F>>,
    clock: Arc<SliceClock>,
}

impl<F: Future> SliceTimed<F> {
    fn new(clock: Arc<SliceClock>, inner: F) -> Self {
        Self {
            inner: Box::pin(inner),
            clock,
        }
    }
}

impl<F: Future> Future for SliceTimed<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        self.clock.stamp();
        self.inner.as_mut().poll(cx)
    }
}

/// When the current poll of the evaluation began, as nanoseconds since the runtime was made.
struct SliceClock {
    origin: StdInstant,
    started: AtomicU64,
}

impl SliceClock {
    fn new() -> Self {
        Self {
            origin: StdInstant::now(),
            started: AtomicU64::new(0),
        }
    }

    fn stamp(&self) {
        let nanos = u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.started.store(nanos, Ordering::Relaxed);
    }

    /// How long the current poll has run.
    fn slice(&self) -> Duration {
        self.origin
            .elapsed()
            .saturating_sub(Duration::from_nanos(self.started.load(Ordering::Relaxed)))
    }
}

/// Which check stopped the script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Halt {
    /// The run stopped: its reason is on the `RunCtx`.
    RunStopped,
    /// One slice of JS ran past the budget.
    LongTask,
}

/// The first `Halt` the interrupt handler returned, read once after the evaluation. An atomic
/// because QuickJS calls the handler from whichever worker polls the run.
#[derive(Default)]
struct HaltCause(AtomicU8);

impl HaltCause {
    const NONE: u8 = 0;
    const RUN_STOPPED: u8 = 1;
    const LONG_TASK: u8 = 2;

    fn record(&self, halt: Halt) {
        let code = match halt {
            Halt::RunStopped => Self::RUN_STOPPED,
            Halt::LongTask => Self::LONG_TASK,
        };
        // The first cause wins: the handler keeps returning true until the script unwinds.
        let _ = self
            .0
            .compare_exchange(Self::NONE, code, Ordering::Relaxed, Ordering::Relaxed);
    }

    fn get(&self) -> Option<Halt> {
        match self.0.load(Ordering::Relaxed) {
            Self::RUN_STOPPED => Some(Halt::RunStopped),
            Self::LONG_TASK => Some(Halt::LongTask),
            _ => None,
        }
    }
}
