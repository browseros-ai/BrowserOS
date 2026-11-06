//! Runner N: the real prelude (`browseros_playwright::PRELUDE_JS`) running a Playwright script
//! in QuickJS behind claw-sdk's real run bootstrap (`apps/claw-sdk/src/runtime/js/bootstrap.js`,
//! included as it is), evaluated the way claw-sdk's `run_script.rs` evaluates a run, with
//! every op going through an [`OpHost`] (`Playwright::invoke` on a live browser, in the live
//! tests). A prelude that drifts from the skeleton's bootstrap fails here.
//!
//! ```text
//!  agent fn ─ prelude.js ─ __neo.invoke (bootstrap.js) ─ __neo_invoke ─ RunDoor (async Rust)
//!                                                          ├─ the lazy default page, once
//!                                                          └─ OpHost::invoke ─ Playwright::invoke
//! ```
//!
//! [`RunDoor`] stands in for claw-sdk's door and `runtime/bridge.rs`, only as far as the
//! prelude can tell them apart: one reply envelope per op (`{"ok"|"err", "pages": []}`, plus
//! `defaultPage` once an op opened the run's page), and an op id per op so a thrown op error
//! names its failure. It reports no page facts, so the prelude learns URLs from results and
//! events alone.
//!
//! Two test-only natives stand in for the user. `__neo_test_gesture(page, tab, expression)` runs
//! `expression` in the page's own world with a user gesture, as a trusted click would, so a
//! script can open a popup before `locator.click` is built (the popup blocker refuses a
//! `window.open` without one; measured on BrowserOS neo, headless). It resolves to `""`, or to
//! what failed. `__neo_test_eval(page, tab, expression)` runs it the same way and resolves to
//! `{"ok": value}` or `{"err": "what failed"}` as JSON text, so a script can read what the page
//! recorded (conformance's `harness.pageEval`) without `page.evaluate`.
//!
//! Each run gets its own `AsyncRuntime` and one context in it, dropped in that order when the
//! run ends. A second context in one runtime deadlocks when a bridge call re-enters it (claw-sdk
//! hit it), and QuickJS aborts the process if a JS value outlives its runtime, so no JS value
//! leaves [`run_script`].

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use browseros_core::{PageId, TabId};
use browseros_playwright::{ErrorCode, Op, OpTarget, PRELUDE_JS, PlaywrightError, TimedOut};
use futures_util::future::BoxFuture;
use rquickjs::context::EvalOptions;
use rquickjs::function::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Function, Object, Promise};
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use tokio::time;

use crate::live_run::{LiveRun, TestPage};

/// claw-sdk's run bootstrap, read-only: Runner N runs the very file each run starts with.
const BOOTSTRAP: &str = include_str!("../../../../apps/claw-sdk/src/runtime/js/bootstrap.js");

/// The file name claw-sdk compiles the agent's function under, which the bootstrap's call
/// sites look for in stacks.
const AGENT_FILE: &str = "run";

/// claw-sdk's limits for one run's QuickJS (`config/defaults.rs` `JS_MEMORY_BYTES` and
/// `JS_STACK_BYTES`), so a value too deep or too large for the agent's realm fails here as it
/// does there.
const JS_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const JS_STACK_BYTES: usize = 512 * 1024;

/// What `__neo_invoke` hands each op to, as claw-sdk's dispatcher hands it to the crate.
pub(crate) trait OpHost: Send + Sync {
    /// Runs op `name` with the JSON the prelude sent.
    fn invoke<'a>(
        &'a self,
        name: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, PlaywrightError>>;

    /// Runs `expression` in `page`'s own world with a user gesture, as the user's click or the
    /// page's own script would, and returns its value by JSON.
    fn gesture(&self, page: TestPage, expression: String) -> BoxFuture<'_, Result<Value, String>>;
}

impl OpHost for LiveRun {
    fn invoke<'a>(
        &'a self,
        name: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, PlaywrightError>> {
        Box::pin(LiveRun::invoke(self, name, input))
    }

    fn gesture(&self, page: TestPage, expression: String) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(async move {
            let evaluated = self.eval(&page, &expression).await;
            evaluated.map_err(|error| error.to_string())
        })
    }
}

/// Runs `script`, the body of the agent's async function, after the bootstrap and the
/// prelude, with `init` as the run's init (`{"page": brand | null}`, `run_script.rs`, plus
/// `"pages"`, every tab's facts, which sdk-skeleton is to add). `Ok` is what the script
/// returned, as JSON; `Err` says what it threw, or what stopped the engine.
pub(crate) async fn run_script(
    host: &Arc<dyn OpHost>,
    init: &Value,
    script: &str,
) -> Result<Value, String> {
    let runtime = AsyncRuntime::new().map_err(|error| error.to_string())?;
    runtime.set_memory_limit(JS_MEMORY_BYTES).await;
    runtime.set_max_stack_size(JS_STACK_BYTES).await;
    let context = AsyncContext::full(&runtime)
        .await
        .map_err(|error| error.to_string())?;
    let door = Arc::new(RunDoor::new(Arc::clone(host)));
    let natives = Arc::clone(&door);
    let init = init.to_string();
    let code = script.to_owned();
    let settled = context
        .async_with(async move |ctx| evaluate(&ctx, &natives, &code, &init).await)
        .await;
    // The context before the runtime; an op the script left pending (the event pump's long
    // poll) is dropped with the runtime, as claw-sdk drops a run's background ops at its end.
    drop(context);
    drop(runtime);
    let settled: Value = serde_json::from_str(&settled?).map_err(|error| error.to_string())?;
    match settled.get("thrown") {
        Some(thrown) => Err(door.thrown(thrown)),
        None => Ok(settled.get("ok").cloned().unwrap_or(Value::Null)),
    }
}

/// Inside the runtime's lock, as `run_script.rs`'s `prepare`: the natives, the bootstrap, the
/// prelude, the agent's function with `__neo.parameters()` for its parameters, then
/// `__neo.run(main, init)` awaited to its settled JSON.
async fn evaluate<'js>(
    ctx: &Ctx<'js>,
    door: &Arc<RunDoor>,
    code: &str,
    init: &str,
) -> Result<String, String> {
    let fail = |stage: &str, caught: &dyn fmt::Display| format!("{stage}: {caught}");
    install(ctx, door)
        .catch(ctx)
        .map_err(|caught| fail("natives", &caught))?;
    for (file, source) in [("bootstrap.js", BOOTSTRAP), ("prelude.js", PRELUDE_JS)] {
        ctx.eval_with_options::<(), _>(source, options(file))
            .catch(ctx)
            .map_err(|caught| fail(file, &caught))?;
    }
    let neo: Object<'js> = ctx
        .globals()
        .get("__neo")
        .catch(ctx)
        .map_err(|caught| fail("__neo", &caught))?;
    let parameters: Function<'js> = neo
        .get("parameters")
        .catch(ctx)
        .map_err(|caught| fail("__neo.parameters", &caught))?;
    let parameters: String = parameters
        .call(())
        .catch(ctx)
        .map_err(|caught| fail("__neo.parameters", &caught))?;
    let main: Function<'js> = ctx
        .eval_with_options(
            format!("(async function ({parameters}) {{\n{code}\n}})"),
            options(AGENT_FILE),
        )
        .catch(ctx)
        .map_err(|caught| fail("script", &caught))?;
    let run: Function<'js> = neo
        .get("run")
        .catch(ctx)
        .map_err(|caught| fail("__neo.run", &caught))?;
    let promise: Promise<'js> = run
        .call((main, init.to_owned()))
        .catch(ctx)
        .map_err(|caught| fail("__neo.run", &caught))?;
    promise
        .into_future::<String>()
        .await
        .catch(ctx)
        .map_err(|caught| fail("script", &caught))
}

/// Global code in strict mode, named `file` in stack frames, as `run_script.rs` evaluates every
/// file of a run.
fn options(file: &str) -> EvalOptions {
    let mut options = EvalOptions::default();
    options.global = true;
    options.strict = true;
    options.filename = Some(file.to_owned());
    options
}

/// The bootstrap's natives (`__neo_invoke`, `__neo_log`, `__neo_sleep`) and the two test-only
/// ones. Each closure owns an `Arc` and owned strings, never a JS value, so nothing JS-typed is
/// held across an await.
fn install(ctx: &Ctx<'_>, door: &Arc<RunDoor>) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let invoke = {
        let door = Arc::clone(door);
        move |name: String, input: String, _site: String| {
            let door = Arc::clone(&door);
            async move { door.invoke(&name, &input).await }
        }
    };
    globals.set("__neo_invoke", Func::from(Async(invoke)))?;
    globals.set("__neo_log", Func::from(|_level: String, _text: String| {}))?;
    let sleep = |ms: f64| async move {
        time::sleep(Duration::try_from_secs_f64(ms.max(0.0) / 1000.0).unwrap_or_default()).await;
    };
    globals.set("__neo_sleep", Func::from(Async(sleep)))?;
    let gesture = {
        let door = Arc::clone(door);
        move |page: u32, tab: f64, expression: String| {
            let door = Arc::clone(&door);
            async move {
                door.host
                    .gesture(test_page(page, tab), expression)
                    .await
                    .err()
                    .unwrap_or_default()
            }
        }
    };
    globals.set("__neo_test_gesture", Func::from(Async(gesture)))?;
    let eval = {
        let door = Arc::clone(door);
        move |page: u32, tab: f64, expression: String| {
            let door = Arc::clone(&door);
            async move {
                let reply = match door.host.gesture(test_page(page, tab), expression).await {
                    Ok(value) => json!({ "ok": value }),
                    Err(error) => json!({ "err": error }),
                };
                reply.to_string()
            }
        }
    };
    globals.set("__neo_test_eval", Func::from(Async(eval)))?;
    Ok(())
}

/// The tab a script names by a page handle's `_id` and `_tab`.
fn test_page(page: u32, tab: f64) -> TestPage {
    // A tab id is an integer the prelude read from a brand; JS hands it over as a number.
    #[expect(clippy::cast_possible_truncation, reason = "tab ids are integers")]
    let tab = TabId(tab as i64);
    TestPage {
        id: PageId(page),
        tab,
    }
}

/// claw-sdk's door and bridge for one run, as the prelude sees them.
struct RunDoor {
    host: Arc<dyn OpHost>,
    /// The run's own page brand, opened by the first op whose target names
    /// `{"$neo":"page","id":null}`. Single-flight, as the door's: racing ops share one tab.
    default_page: OnceCell<Value>,
    /// The last op id handed out; ids read `run.1`, `run.2`, ...
    operations: AtomicU64,
    /// What each failed op said, by op id. The bootstrap settles an uncaught op error as its
    /// id alone, and claw-sdk reports the failure it recorded for that id.
    failures: Mutex<HashMap<String, String>>,
}

impl RunDoor {
    fn new(host: Arc<dyn OpHost>) -> Self {
        Self {
            host,
            default_page: OnceCell::new(),
            operations: AtomicU64::new(0),
            failures: Mutex::default(),
        }
    }

    /// `__neo_invoke`: one op through the door, always answered with an envelope.
    async fn invoke(&self, name: &str, input: &str) -> String {
        let operation = format!(
            "run.{}",
            self.operations.fetch_add(1, Ordering::Relaxed) + 1
        );
        let mut input: Value = serde_json::from_str(input).unwrap_or(Value::Null);
        // A prelude's refusal (`__neo.refuse`) is refused under the name it gives, as claw-sdk's
        // door refuses it at admission: never run, never resolving a page, so it opens no tab,
        // and remembered as any failed op is.
        if let Some(refused) = Refused::of(name, &input) {
            lock(&self.failures).insert(operation.clone(), refused.said());
            return refused.envelope(&operation);
        }
        let (outcome, default_page) = match self.default_page(name, &mut input).await {
            Ok(default_page) => (self.host.invoke(name, input).await, default_page),
            Err(error) => (Err(error), None),
        };
        if let Err(error) = &outcome {
            let said = format!("{}: {}", error.code(), error.message());
            lock(&self.failures).insert(operation.clone(), said);
        }
        envelope(outcome, default_page, &operation)
    }

    /// The door's lazy default page: an op whose target (`page`, or `target.page`, per the
    /// op's spec) names the run's page before it has a tab gets the tab, opened once with
    /// `context.newPage`. Its brand replaces every unopened brand in the input, as claw-sdk's
    /// adapter hands `Playwright::invoke` the page the door resolved, and the reply's
    /// `defaultPage` names it. Any other op passes its input through untouched, so an
    /// unopened brand the door would not resolve reaches the crate, which refuses it.
    async fn default_page(
        &self,
        name: &str,
        input: &mut Value,
    ) -> Result<Option<Value>, PlaywrightError> {
        let named = match Op::from_name(name).map(|op| op.spec().target) {
            Some(OpTarget::Field("page")) => input.get("page"),
            Some(OpTarget::Field(field)) => input.get(field).and_then(|target| target.get("page")),
            _ => None,
        };
        if !named.is_some_and(unopened) {
            return Ok(None);
        }
        let brand = self
            .default_page
            .get_or_try_init(|| self.open())
            .await?
            .clone();
        substitute(input, &brand);
        let id = brand.get("id").cloned().unwrap_or(Value::Null);
        let tab = brand.get("tab").cloned().unwrap_or(Value::Null);
        Ok(Some(json!({ "id": id, "tab": tab })))
    }

    /// Opens the run's page, as the door does through `context.newPage`.
    async fn open(&self) -> Result<Value, PlaywrightError> {
        let input = json!({ "call": { "api": "context.newPage", "timeoutMs": 0 } });
        let opened = self.host.invoke("context.newPage", input).await?;
        opened.get("page").cloned().ok_or_else(|| {
            PlaywrightError::new(
                ErrorCode::Internal,
                "context.newPage returned no page".to_owned(),
            )
        })
    }

    /// What the script threw, with the recorded failure when it was an op's own error.
    fn thrown(&self, thrown: &Value) -> String {
        let operation = thrown
            .get("operationId")
            .and_then(Value::as_str)
            .filter(|_| thrown.get("kind").and_then(Value::as_str) == Some("operation"));
        match operation.and_then(|operation| lock(&self.failures).get(operation).cloned()) {
            Some(failure) => format!("the script threw the error of an op: {failure}"),
            None => format!("the script threw {thrown}"),
        }
    }
}

/// The refusal brand a prelude's `__neo.refuse` sends, `{"$neo":"refused","code","reason"}`, as
/// claw-sdk's door reads it (`dispatch/refusal_brand.rs`): a script-owned failure, its code one
/// of the two a script may claim, any other code or a missing reason `invalid_input`.
struct Refused {
    code: &'static str,
    message: String,
}

impl Refused {
    fn of(name: &str, input: &Value) -> Option<Self> {
        if input.get("$neo").and_then(Value::as_str) != Some("refused") {
            return None;
        }
        let reason = input.get("reason").and_then(Value::as_str);
        Some(match (input.get("code").and_then(Value::as_str), reason) {
            (Some(code @ ("method_unavailable" | "invalid_input")), Some(reason)) => Self {
                code: if code == "method_unavailable" {
                    "method_unavailable"
                } else {
                    "invalid_input"
                },
                message: reason.to_owned(),
            },
            _ => Self {
                code: "invalid_input",
                message: format!(
                    "`{name}`: a refusal may name only method_unavailable or invalid_input"
                ),
            },
        })
    }

    /// The failure as `RunDoor::thrown` reports it.
    fn said(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }

    /// The door's answer: refused with nothing sent, retryable as a run that changed nothing.
    fn envelope(&self, operation: &str) -> String {
        json!({
            "err": {
                "code": self.code,
                "owner": "script",
                "delivery": "not_sent",
                "retryable": true,
                "operationId": operation,
                "timeout": false,
                "message": self.message,
            },
            "pages": [],
        })
        .to_string()
    }
}

/// `{"$neo":"page","id":null}`: the run's page before it has a tab.
fn unopened(value: &Value) -> bool {
    value.get("$neo").and_then(Value::as_str) == Some("page")
        && value.get("id").is_none_or(Value::is_null)
}

/// Replaces every unopened page brand in `value` with `brand`.
fn substitute(value: &mut Value, brand: &Value) {
    if unopened(value) {
        brand.clone_into(value);
        return;
    }
    match value {
        Value::Array(items) => items.iter_mut().for_each(|item| substitute(item, brand)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| substitute(field, brand)),
        _ => {}
    }
}

/// bridge.rs's envelope for one op's outcome. The runner tracks no delivery, so that field
/// is fixed.
fn envelope(
    outcome: Result<Value, PlaywrightError>,
    default_page: Option<Value>,
    operation: &str,
) -> String {
    let mut envelope = match outcome {
        Ok(ok) => json!({ "ok": ok, "pages": [] }),
        Err(error) => json!({
            "err": {
                "code": error.code(),
                "owner": error.owner().as_str(),
                "delivery": "unknown",
                "retryable": false,
                "operationId": operation,
                "timeout": error.timed_out() == TimedOut::Yes,
                "message": error.message(),
            },
            "pages": [],
        }),
    };
    if let (Some(page), Some(fields)) = (default_page, envelope.as_object_mut()) {
        fields.insert("defaultPage".to_owned(), page);
    }
    envelope.to_string()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
