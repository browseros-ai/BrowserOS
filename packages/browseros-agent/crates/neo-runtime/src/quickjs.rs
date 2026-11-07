//! One fresh QuickJS runtime per run: the script's whole world. It holds the host globals a
//! Playwright script expects (`console`, `setTimeout`, `setInterval` and their `clear…`,
//! `AbortController`, `AbortSignal`, and neo-quickjs-injected's `URL`, `URLSearchParams`, `Buffer`,
//! `TextEncoder`, `TextDecoder`, `structuredClone` and `crypto`), `__neo_runtime`, and the `page`,
//! `context` and `playwright` objects neo-quickjs-injected builds, and nothing survives the run.
//!
//! The realm boundary is also where limits apply: a memory cap, an interrupt handler that
//! stops running JS at the deadline or on cancellation, and a `select!` that ends the run when
//! the script is parked on a timer or a promise instead of running.

use std::hash::{BuildHasher, RandomState};
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rquickjs::function::{Async, Func};
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Coerced, Constructor, Ctx, FromJs,
    Function, Object, Promise, TypedArray,
};
use serde_json::Value;

use crate::chromium_bridge::playwright_injected_methods;
use crate::deadline::Deadline;
use crate::events::EventHub;
use crate::method_router::Dispatcher;
use crate::playwright_methods::browser_methods;
use crate::quickjs_bridge::{Getters, Logs};
use crate::run::{RunBinary, RunError, RunOutcome};
#[cfg(feature = "test-host")]
use crate::test_host::TestHost;
use crate::{abort, quickjs_bridge};

const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
// Well under a tokio worker's 2 MiB native stack, so deep JS recursion throws a RangeError
// before it can overflow the thread.
const MAX_STACK_BYTES: usize = 512 * 1024;

/// neo-quickjs-injected's bundle. Evaluating it defines no globals; its completion value is
/// `{ createRun }`. `scripts/neo-check.sh` fails when it is stale.
///
/// `ctx.eval` names its frames `eval_script`, and only the agent's code, built by the
/// `AsyncFunction` constructor, has frames named `<input>`. neo-quickjs-injected's
/// `captureLocation` relies on that to find the agent's call site in a stack
/// (`tests/realm_limits.rs` pins it).
const SDK_BUNDLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-quickjs-injected/dist/sdk.js"
));

/// The host globals a Playwright script finds in Node. Evaluates to a function that takes the
/// native `sleep(ms)`, so the sleep itself never becomes a global, and returns a promise that
/// rejects with the first error a timer callback throws: Node reports that as an uncaught
/// exception and exits, and the realm races the promise with the script (`RACE_UNCAUGHT`), so
/// the run ends with that error at once instead of running on without it.
///
/// Every timer waits on that `sleep`, which never wakes at or after the run's deadline
/// ([`Deadline::sleep`]), so no timer fires once the run is over; dropping the realm drops the
/// pending sleeps with it. An interval re-arms only after its callback returned and only while
/// its id is live, so `clearInterval` from inside the callback stops it, and it never piles up
/// ticks; a callback that throws stops its interval. Timer ids are one sequence, so `clearTimeout`
/// clears an interval and `clearInterval` a timeout, as in Node.
///
/// Platform: Node's timers refuse a callback that is not a function with a `TypeError`, and
/// `setInterval` repeats after `delay` ms, where a `delay` below 1, above 2^31 - 1 or not a
/// number is 1 (Node `lib/internal/timers.js` `Timeout`), so a 0 ms interval never spins.
const HOST_GLOBALS: &str = r"(sleep) => {
  let fail
  const uncaught = new Promise((_, reject) => {
    fail = reject
  })
  const report = (error) => {
    const reject = fail
    fail = undefined
    if (reject) reject(error)
  }
  const callable = (callback) => {
    if (typeof callback !== 'function')
      throw new TypeError(`The callback argument must be of type function. Received ${typeof callback}`)
  }
  let nextTimer = 1
  const timers = new Set()
  globalThis.setTimeout = (callback, delay, ...args) => {
    callable(callback)
    const id = nextTimer++
    timers.add(id)
    sleep(Math.max(0, Number(delay) || 0)).then(() => {
      if (!timers.delete(id)) return
      try {
        callback(...args)
      } catch (error) {
        report(error)
      }
    })
    return id
  }
  globalThis.clearTimeout = (id) => {
    timers.delete(id)
  }
  globalThis.setInterval = (callback, delay, ...args) => {
    callable(callback)
    const id = nextTimer++
    const ms = Number(delay)
    const every = ms >= 1 && ms <= 2147483647 ? ms : 1
    timers.add(id)
    const tick = () => {
      sleep(every).then(() => {
        if (!timers.has(id)) return
        try {
          callback(...args)
        } catch (error) {
          timers.delete(id)
          report(error)
          return
        }
        if (timers.has(id)) tick()
      })
    }
    tick()
    return id
  }
  globalThis.clearInterval = globalThis.clearTimeout
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
  return uncaught
}";

/// Neo: puts neo-quickjs-injected's standard globals (`createRun`'s `globals`: `URL`, `Buffer`,
/// `crypto`, …) on the realm's global object the way Node defines its own: writable, configurable,
/// not enumerable. neo-quickjs-injected's bundle defines no globals when it is evaluated; this is
/// the one place they appear.
const INSTALL_SDK_GLOBALS: &str = r"(globals) => {
  for (const [name, value] of Object.entries(globals))
    Object.defineProperty(globalThis, name, {
      value,
      writable: true,
      configurable: true,
      enumerable: false,
    })
}";

/// Deviation: `client/eventEmitter.ts` `EventEmitter._handleListenerError`: `throw error;`
/// A listener's throw, or its rejection with no `error` listener, is an uncaught exception in
/// Node, which exits. QuickJS has no such exit, so neo-quickjs-injected reports the first one on
/// its `uncaught` promise, and the run ends with that error at once instead of running on. The
/// realm's own `abort` listeners and timer callbacks report theirs the same way
/// (`abort::REALM_GLOBALS`, `HOST_GLOBALS`).
const RACE_UNCAUGHT: &str =
    "(script, uncaught, aborts, timers) => Promise.race([script, uncaught, aborts, timers])";

/// How many lines QuickJS reports before the agent's line 1 in an `AsyncFunction`-constructed
/// function (`async function anonymous(page,context,playwright\n) {\n`). Measured by probe P4 and pinned by
/// `tests/realm_limits.rs`, which reads the line a call records, so a QuickJS upgrade that
/// changes the wrapper fails that test, not every recorded location.
const AGENT_LINE_OFFSET: u32 = 2;

/// What the realm needs to know about the run's page.
pub(crate) struct Script {
    /// The optional opaque host belongs to this realm alone, never to the runtime.
    #[cfg(feature = "test-host")]
    pub test_host: Option<Box<dyn TestHost>>,
    pub(crate) code: String,
    pub(crate) page_guid: String,
    /// The page's main frame, built once by neo-quickjs-injected so `page.mainFrame()` is one
    /// object.
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
    events: EventHub,
) -> RunOutcome {
    let logs = Logs::default();
    let evaluated = evaluate(script, getters, logs.clone(), dispatcher, deadline.clone());
    // Dropping `evaluated` here drops the whole realm mid-script, timers and pending
    // promises included; nothing outlives the run.
    let ended = tokio::select! {
        stop = deadline.reached() => Err(stop),
        result = evaluated => Ok(result),
    };
    // The realm is gone, and its listeners with it: no event is delivered from now on, and a
    // drop period the script never drained still leaves its note.
    events.close();
    for note in events.take_notes() {
        logs.push(note);
    }
    let result = match ended {
        Ok(result) => result,
        Err(stop) => return RunOutcome::stopped(stop, deadline.timeout(), logs.take()),
    };
    // A stop that fired during evaluation decides the outcome, however the script ended. It
    // reaches the script as a JS error from the interrupt handler or from a getter waiting on
    // CDP, and a getter's `TimeoutError` or cancellation error is catchable, so a script can
    // swallow it and return normally. Only the deadline knows the run did not finish in time.
    if let Some(stop) = deadline.stop() {
        return RunOutcome::stopped(stop, deadline.timeout(), logs.take());
    }
    match result {
        Ok((value, binaries)) => RunOutcome::returned(value, logs.take()).with_binaries(binaries),
        Err(error) => RunOutcome::failed(error, logs.take()),
    }
}

/// The script's return value as JSON, and the bytes of each binary in it ([`to_json`]).
type Returned = (Option<Value>, Vec<RunBinary>);

async fn evaluate(
    script: Script,
    getters: Getters,
    logs: Logs,
    dispatcher: Dispatcher,
    deadline: Deadline,
) -> Result<Returned, RunError> {
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
            let (returned, export) = start(&ctx, script, getters, logs, dispatcher, deadline)
                .catch(&ctx)
                .map_err(|error| script_error(&ctx, error))?;
            let value: rquickjs::Value<'_> = returned
                .into_future()
                .await
                .catch(&ctx)
                .map_err(|error| script_error(&ctx, error))?;
            to_json(&ctx, &export, value)
        })
        .await
}

/// Sets up the realm and starts the script, returning the promise of its async function and
/// neo-quickjs-injected's `exportValue`, which [`to_json`] serializes its result with.
fn start<'js>(
    ctx: &Ctx<'js>,
    script: Script,
    getters: Getters,
    logs: Logs,
    dispatcher: Dispatcher,
    deadline: Deadline,
) -> rquickjs::Result<(Promise<'js>, Function<'js>)> {
    quickjs_bridge::install(
        ctx,
        getters,
        logs,
        dispatcher,
        #[cfg(feature = "test-host")]
        script.test_host,
    )?;
    let sleep = Func::from(Async(move |delay_ms: f64| {
        let deadline = deadline.clone();
        async move { deadline.sleep(delay_from_millis(delay_ms)).await }
    }));
    let install_host_globals: Function<'js> = ctx.eval(HOST_GLOBALS)?;
    let timer_uncaught: rquickjs::Value<'js> = install_host_globals.call((sleep,))?;
    // After the host globals: `AbortSignal.timeout` schedules with their `setTimeout`.
    let install_abort_globals: Function<'js> = ctx.eval(abort::REALM_GLOBALS)?;
    let abort_uncaught: rquickjs::Value<'js> = install_abort_globals.call(())?;

    let sdk: Object<'js> = ctx.eval(SDK_BUNDLE)?;
    let create_run: Function<'js> = sdk.get("createRun")?;
    let init = Object::new(ctx.clone())?;
    init.set("page", script.page_guid)?;
    init.set("mainFrame", script.frame_guid)?;
    init.set("context", "context@default")?;
    init.set("browser", browser_methods::BROWSER_GUID)?;
    init.set("url", script.url)?;
    init.set(
        "testIdAttribute",
        playwright_injected_methods::TEST_ID_ATTRIBUTE,
    )?;
    init.set("lineOffset", AGENT_LINE_OFFSET)?;
    let objects: Object<'js> = create_run.call((init,))?;
    // Before the agent's function is built, so its code sees them as Node globals.
    let install_sdk_globals: Function<'js> = ctx.eval(INSTALL_SDK_GLOBALS)?;
    install_sdk_globals.call::<_, ()>((objects.get::<_, Object<'js>>("globals")?,))?;
    let page: rquickjs::Value<'js> = objects.get("page")?;
    let context: rquickjs::Value<'js> = objects.get("context")?;
    // Playwright's `Playwright` object, the script's third parameter: `chromium` and the other
    // browser types, `devices`, `errors`, `selectors`, `request`.
    let playwright: rquickjs::Value<'js> = objects.get("playwright")?;
    let uncaught: rquickjs::Value<'js> = objects.get("uncaught")?;
    let export: Function<'js> = objects.get("exportValue")?;

    // The AsyncFunction constructor, as Playwright users write `new AsyncFunction(...)`: the
    // code is a function body that cannot close the wrapper early, and a syntax error in it
    // throws a SyntaxError here.
    let async_function: Constructor<'js> = ctx.eval("(async () => {}).constructor")?;
    let agent_function: Function<'js> =
        async_function.construct(("page", "context", "playwright", script.code))?;
    let script: Promise<'js> = agent_function.call((page, context, playwright))?;
    let race: Function<'js> = ctx.eval(RACE_UNCAUGHT)?;
    Ok((
        race.call((script, uncaught, abort_uncaught, timer_uncaught))?,
        export,
    ))
}

/// `setTimeout` delays in JS milliseconds. A delay too large for a `Duration` is due after
/// any deadline, which [`Deadline::sleep`] treats as never.
fn delay_from_millis(delay_ms: f64) -> Duration {
    Duration::try_from_secs_f64(delay_ms / 1000.0).unwrap_or(Duration::MAX)
}

/// The script's return value as JSON, through the realm's own `JSON.stringify` so the
/// conversion is exactly JavaScript's. `undefined` (and anything else JSON drops) is `None`.
///
/// Except binaries: neo-quickjs-injected's `exportValue` hands each `Uint8Array` to `ship`, which
/// copies its bytes out of the realm here, outside its memory limit, and marks its place with this
/// run's random marker key; [`lift_binaries`] then takes the marks out of the JSON. A page cannot
/// forge a mark: it never sees the marker.
fn to_json<'js>(
    ctx: &Ctx<'js>,
    export: &Function<'js>,
    value: rquickjs::Value<'js>,
) -> Result<Returned, RunError> {
    let shipped = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let sink = Arc::clone(&shipped);
    let ship = Function::new(
        ctx.clone(),
        move |bytes: TypedArray<'js, u8>| -> rquickjs::Result<f64> {
            let Some(bytes) = bytes.as_bytes() else {
                return Err(rquickjs::Error::new_from_js_message(
                    "Uint8Array",
                    "bytes",
                    "its buffer is detached",
                ));
            };
            let mut shipped = sink.lock().map_err(|_| {
                rquickjs::Error::new_from_js_message("Uint8Array", "bytes", "poisoned")
            })?;
            shipped.push(bytes.to_vec());
            // Exact: a realm cannot hold 2^53 binaries.
            #[expect(clippy::cast_precision_loss, reason = "a count far below 2^53")]
            Ok((shipped.len() - 1) as f64)
        },
    )
    .map_err(|error| RunError::new("Error", error.to_string()))?;
    let marker = format!("neo:binary:{:016x}", RandomState::new().hash_one(0_u8));
    let text: Option<String> = match export.call((value, ship, marker.as_str())).catch(ctx) {
        Ok(text) => text,
        Err(error) => return Err(script_error(ctx, error)),
    };
    let Some(text) = text else {
        return Ok((None, Vec::new()));
    };
    let mut value: Value = serde_json::from_str(&text)
        .map_err(|error| RunError::new("Error", format!("return value is not JSON: {error}")))?;
    let shipped = shipped
        .lock()
        .map(|mut shipped| mem::take(&mut *shipped))
        .map_err(|_| RunError::new("Error", "returned binaries were lost".to_string()))?;
    let binaries = lift_binaries(&mut value, &marker, shipped);
    Ok((Some(value), binaries))
}

/// Takes `marker` out of every object of `value` that carries it, pairing each with the bytes
/// `shipped` holds at its index and its JSON pointer, in the order JSON wrote them. Recursion is
/// bounded by `serde_json`'s parse depth (128).
fn lift_binaries(value: &mut Value, marker: &str, shipped: Vec<Vec<u8>>) -> Vec<RunBinary> {
    fn walk(
        value: &mut Value,
        marker: &str,
        shipped: &mut [Option<Vec<u8>>],
        pointer: &mut String,
        out: &mut Vec<RunBinary>,
    ) {
        match value {
            Value::Object(map) => {
                if let Some(index) = map.remove(marker) {
                    let bytes = index
                        .as_u64()
                        .and_then(|index| usize::try_from(index).ok())
                        .and_then(|index| shipped.get_mut(index))
                        .and_then(Option::take);
                    if let Some(bytes) = bytes {
                        out.push(RunBinary {
                            pointer: pointer.clone(),
                            bytes,
                        });
                    }
                    return;
                }
                for (key, child) in map.iter_mut() {
                    let at = pointer.len();
                    pointer.push('/');
                    pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
                    walk(child, marker, shipped, pointer, out);
                    pointer.truncate(at);
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter_mut().enumerate() {
                    let at = pointer.len();
                    pointer.push('/');
                    pointer.push_str(&index.to_string());
                    walk(child, marker, shipped, pointer, out);
                    pointer.truncate(at);
                }
            }
            _ => {}
        }
    }
    let mut shipped: Vec<Option<Vec<u8>>> = shipped.into_iter().map(Some).collect();
    let mut out = Vec::new();
    walk(value, marker, &mut shipped, &mut String::new(), &mut out);
    out
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Marks leave the JSON, their bytes pair with escaped pointers in document order, and a
    /// mark naming no shipped binary (or one already taken) yields nothing.
    #[test]
    fn tests_that_binaries_are_lifted_with_their_pointers() {
        let marker = "neo:binary:test";
        let mut value = json!({
            "a/b": {"type":"Buffer","bytes":1,"neo:binary:test":1},
            "list": [{"type":"Uint8Array","bytes":2,"neo:binary:test":0}, "kept"],
            "m~": {"type":"Buffer","bytes":0,"neo:binary:test":7},
            "again": {"type":"Buffer","bytes":2,"neo:binary:test":0},
        });
        let binaries = lift_binaries(&mut value, marker, vec![vec![1, 2], vec![3]]);
        assert_eq!(
            binaries,
            [
                RunBinary {
                    pointer: "/a~1b".to_string(),
                    bytes: vec![3],
                },
                RunBinary {
                    pointer: "/again".to_string(),
                    bytes: vec![1, 2],
                },
            ]
        );
        assert!(!value.to_string().contains(marker), "{value}");
        assert_eq!(value["list"][1], "kept");
        assert_eq!(value["list"][0], json!({"type":"Uint8Array","bytes":2}));
        let mut whole = json!({"type":"Buffer","bytes":1,"neo:binary:test":0});
        let top = lift_binaries(&mut whole, marker, vec![vec![9]]);
        assert_eq!(top[0].pointer, "");
    }
}
