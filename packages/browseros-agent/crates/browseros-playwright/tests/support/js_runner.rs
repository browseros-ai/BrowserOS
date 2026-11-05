//! Runner N: the real prelude (`browseros_playwright::PRELUDE_JS`) running a Playwright script
//! in QuickJS, the engine claw-sdk ships it in, with every op going through an [`OpHost`]
//! (`Playwright::invoke` on a live browser, in the live tests).
//!
//! ```text
//!  script ─ prelude.js ─ __neo.invoke (mock_bootstrap.js) ─ __neo_invoke (async Rust)
//!                                                            └─ OpHost::invoke ─ Playwright::invoke
//! ```
//!
//! Only strings cross the bridge, as in claw-sdk (sdk-skeleton D8): the input arrives as JSON
//! text and leaves as one envelope, `{"ok": value, "pages": []}` or `{"err": {..}, "pages": []}`.
//! This mock reports no page facts, so the prelude learns URLs from results and events alone.
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

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::{PageId, TabId};
use browseros_playwright::{PRELUDE_JS, PlaywrightError, TimedOut};
use futures_util::future::BoxFuture;
use rquickjs::function::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Promise};
use serde_json::{Value, json};
use tokio::time;

use crate::live_run::{LiveRun, TestPage};

/// The bootstrap the prelude runs on: claw-sdk's documented `__neo` surface, over the natives
/// [`install`] defines.
const BOOTSTRAP: &str = include_str!("mock_bootstrap.js");

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

/// Runs `script`, an async function body, after the bootstrap and the prelude, with `seed` as
/// `__neoPlaywrightSeed`. `Ok` is what the script returned, as JSON; `Err` says what it threw,
/// or what stopped the engine.
pub(crate) async fn run_script(
    host: &Arc<dyn OpHost>,
    seed: &Value,
    script: &str,
) -> Result<Value, String> {
    let runtime = AsyncRuntime::new().map_err(|error| error.to_string())?;
    let context = AsyncContext::full(&runtime)
        .await
        .map_err(|error| error.to_string())?;
    let host = Arc::clone(host);
    let seed = seed.to_string();
    let code = format!(
        "(async () => {{\n\
           try {{ return JSON.stringify({{ ok: (await (async () => {{\n{script}\n}})()) ?? null }}) }}\n\
           catch (e) {{ return JSON.stringify({{ thrown: {{ name: String(e?.name), message: String(e?.message ?? e), code: e?.code ?? null }} }}) }}\n\
         }})()"
    );
    let settled = context
        .async_with(async move |ctx| evaluate(&ctx, &host, &seed, &code).await)
        .await;
    // The context before the runtime; an op the script left pending (the event pump's long
    // poll) is dropped with the runtime, as claw-sdk drops a run's background ops at its end.
    drop(context);
    drop(runtime);
    let settled: Value = serde_json::from_str(&settled?).map_err(|error| error.to_string())?;
    match settled.get("thrown") {
        Some(thrown) => Err(format!("the script threw {thrown}")),
        None => Ok(settled.get("ok").cloned().unwrap_or(Value::Null)),
    }
}

/// Inside the runtime's lock: the natives, the bootstrap, the seed, the prelude, then the
/// script's promise awaited to its JSON.
async fn evaluate<'js>(
    ctx: &Ctx<'js>,
    host: &Arc<dyn OpHost>,
    seed: &str,
    code: &str,
) -> Result<String, String> {
    let fail = |stage: &str, caught: &dyn fmt::Display| format!("{stage}: {caught}");
    install(ctx, host)
        .catch(ctx)
        .map_err(|caught| fail("natives", &caught))?;
    ctx.eval::<(), _>(BOOTSTRAP)
        .catch(ctx)
        .map_err(|caught| fail("mock_bootstrap.js", &caught))?;
    let seed = ctx
        .json_parse(seed)
        .catch(ctx)
        .map_err(|caught| fail("seed", &caught))?;
    ctx.globals()
        .set("__neoPlaywrightSeed", seed)
        .catch(ctx)
        .map_err(|caught| fail("seed", &caught))?;
    ctx.eval::<(), _>(PRELUDE_JS)
        .catch(ctx)
        .map_err(|caught| fail("prelude.js", &caught))?;
    let promise: Promise<'js> = ctx
        .eval(code)
        .catch(ctx)
        .map_err(|caught| fail("script", &caught))?;
    promise
        .into_future::<String>()
        .await
        .catch(ctx)
        .map_err(|caught| fail("script", &caught))
}

/// `__neo_invoke`, `__neo_sleep`, `__neo_test_gesture` and `__neo_test_eval`. Each closure owns
/// an `Arc` and owned strings, never a JS value, so nothing JS-typed is held across an await.
fn install(ctx: &Ctx<'_>, host: &Arc<dyn OpHost>) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let invoke = {
        let host = Arc::clone(host);
        move |name: String, input: String, _site: String| {
            let host = Arc::clone(&host);
            async move {
                let input = serde_json::from_str(&input).unwrap_or(Value::Null);
                envelope(host.invoke(&name, input).await)
            }
        }
    };
    globals.set("__neo_invoke", Func::from(Async(invoke)))?;
    let sleep = |ms: f64| async move {
        time::sleep(Duration::try_from_secs_f64(ms.max(0.0) / 1000.0).unwrap_or_default()).await;
    };
    globals.set("__neo_sleep", Func::from(Async(sleep)))?;
    let gesture = {
        let host = Arc::clone(host);
        move |page: u32, tab: f64, expression: String| {
            let host = Arc::clone(&host);
            async move {
                host.gesture(test_page(page, tab), expression)
                    .await
                    .err()
                    .unwrap_or_default()
            }
        }
    };
    globals.set("__neo_test_gesture", Func::from(Async(gesture)))?;
    let eval = {
        let host = Arc::clone(host);
        move |page: u32, tab: f64, expression: String| {
            let host = Arc::clone(&host);
            async move {
                let reply = match host.gesture(test_page(page, tab), expression).await {
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

/// The skeleton's envelope for one op's outcome. The mock tracks no delivery and no operation
/// ids, so those fields are fixed.
fn envelope(outcome: Result<Value, PlaywrightError>) -> String {
    let envelope = match outcome {
        Ok(ok) => json!({ "ok": ok, "pages": [] }),
        Err(error) => json!({
            "err": {
                "code": error.code(),
                "owner": error.owner().as_str(),
                "delivery": "unknown",
                "retryable": false,
                "operationId": "",
                "hostCode": error.host_code(),
                "timeout": error.timed_out() == TimedOut::Yes,
                "message": error.message(),
            },
            "pages": [],
        }),
    };
    envelope.to_string()
}
