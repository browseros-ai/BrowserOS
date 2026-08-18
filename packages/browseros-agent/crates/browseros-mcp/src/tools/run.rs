use crate::framework::{
    HelperSource, InnerCallRecord, ToolCtx, ToolDef, ToolError, ToolExecResult, ToolResult,
    execute_tool, page_json, parse_args, text_result,
};
use browseros_core::{
    PageId, Ref, SessionId, WindowId, input::ScrollDirection, pages::NewPageOptions,
};
use futures_util::future::BoxFuture;
use rquickjs::{
    Array, AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Ctx, Exception, FromJs,
    Function, IntoJs, Object, Promise, Value as JsValue,
    function::{Async, Func},
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::{Instant, sleep_until};

const DEFAULT_TIMEOUT_MS: f64 = 30_000.0;
const MAX_TIMEOUT_MS: u64 = 30_000;
const MIN_TIMEOUT_MS: u64 = 1;
const RUN_MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
const RUN_STACK_SIZE_BYTES: usize = 512 * 1024;
const MAX_LOG_ENTRIES: usize = 1_000;
const MAX_LOG_BYTES: usize = 1_000_000;
const MAX_RETURN_VALUE_BYTES: usize = 2_000_000;

const DESCRIPTION: &str = r#"The primary way to drive the browser - prefer run for any task; the granular tools are the fallback. Do multi-step flows, pagination, bulk extraction, and repeated act/read loops - in ONE call: async JavaScript against the `browser` SDK in the server runtime. console.log is captured; return a value to read it back; exceptions come back as a result, not thrown. Every call is `await`-able.

The return shapes below are stable. Do NOT probe them at runtime (no typeof / Object.keys / getOwnPropertyNames) and do NOT re-open a page to inspect what a call returned; that just piles up duplicate tabs. Reuse a pageId across steps.

Pages (pageId is a NUMBER):
  browser.pages.newPage(url)   -> pageId (number). Use it directly; it is not an object. Opens in the background so it does not steal the user's focus; pass { background: false } only when the user asks to bring the tab to the front.
  browser.pages.close(pageId)  -> undefined. Closes a page you own.
  browser.pages.list()         -> [{ pageId, url, title, ownership, ownerLabel, ... }] for EVERY open tab in the browser, including the user's and other agents'. `ownership` is "mine" | "user" | "other-agent"; "other-agent" tabs also carry ownerLabel. Act only on your own ("mine") tabs. Leave "user" and "other-agent" tabs alone unless the user explicitly asks you to work on one.
  browser.pages.getInfo(pageId)-> { pageId, url, title, ... } or null
Observe / act (refs eN come from a snapshot's text/refs):
  browser.observe(pageId).snapshot() -> { text, refs, url }
  browser.observe(pageId).diff()     -> { text, added, removed, changed }
  browser.observe(pageId).resolveRef(ref) -> { backendNodeId, sessionId }
  browser.input(pageId).click(ref) / fill(ref,value) / type(text) / press(key) / hover(ref) / selectOption(ref,value) / scroll(dir,amount,ref?)
  browser.nav(pageId).goto(url) / back() / forward() / reload()
Read / wait / capture:
  browser.read(pageId)               -> the page as a markdown STRING (large pages are truncated with a note pointing to a saved file)
  browser.grep(pageId, { pattern })  -> matching lines as a STRING
  browser.wait(pageId, { for: "text", value: "..." } | { for: "selector", value: "..." } | { value: ms }) -> resolves when ready. For content that loads in, wait on the thing itself with { for: "selector" } (or { for: "text" }); it resolves the moment it appears - e.g. await browser.wait(3, { for: "selector", value: 'div[data-component-type="s-search-result"]' }). Use { value: ms } only for a plain fixed pause. setTimeout(fn, ms) and `await sleep(ms)` also work for a fixed pause. Never poll in a loop (re-checking a count with a fixed wait between tries) - wait on the selector once instead.
  browser.screenshot(pageId) / evaluate(pageId, { code } | { func }) / pdf(pageId)
  browser.download(pageId, opts) / upload(pageId, opts)
  browser.tabGroups(opts) / windows(opts)
Reusable helpers (self-healing): saved helpers for a host, hot-loaded as helpers.<name>(browser, page).
  browser.saveHelper(name, source, { page } | { host }) - source is a function expression, e.g. async (browser, page) => { ... }
  browser.listHelpers({ page } | { host }) -> { host, helpers: [{ name, ageDays, candidate }] }; browser.readHelper(name, { page } | { host }) -> source string
Raw escape hatch: browser.cdp(method, params?, sessionId?) / browser.cdpJsonForPage(pageId, method, paramsJson).

Do the whole task in as few run calls as possible: loop over all the items in one call rather than one run per item. Parallelize independent work with Promise.all so N pages cost one wait cycle, not N. Keep steps on the same page sequential. Efficient pattern:
  const ids = await Promise.all(urls.map(u => browser.pages.newPage(u)));
  await Promise.all(ids.map(id => browser.wait(id, { value: 2500 })));
  const docs = await Promise.all(ids.map(id => browser.read(id)));
  return docs;"#;

const BOOTSTRAP_JS: &str = r#"
(() => {
  const AsyncFunction = Object.getPrototypeOf(async function() {}).constructor;

  // Timers: the runtime is a bare engine, so bridge setTimeout/sleep to a real
  // async sleep. For content that loads in, prefer browser.wait({ for: 'selector' }).
  globalThis.sleep = (ms) => __browserosSleep(Number(ms) || 0);
  globalThis.setTimeout = (fn, ms) => {
    __browserosSleep(Number(ms) || 0).then(() => { if (typeof fn === 'function') fn(); });
    return 0;
  };
  globalThis.clearTimeout = () => {};

  function safeStringify(value) {
    if (value === undefined) return 'undefined';
    try {
      const encoded = JSON.stringify(value, null, 2);
      return encoded ?? String(value);
    } catch {
      return String(value);
    }
  }

  function jsonSafeString(value) {
    const seen = new WeakSet();
    let encoded;
    try {
      encoded = JSON.stringify(value, (_key, next) => {
        if (typeof next === 'bigint') return next.toString();
        if (typeof next === 'function' || typeof next === 'symbol') {
          return String(next);
        }
        if (typeof next === 'number' && !Number.isFinite(next)) return null;
        if (typeof next === 'object' && next !== null) {
          if (seen.has(next)) return '[Circular]';
          seen.add(next);
        }
        return next;
      });
    } catch {
      return JSON.stringify(safeStringify(value));
    }
    return encoded;
  }

  function call(method, args) {
    // The third flag marks primitives that ran inside a hot-loaded helper, so
    // distillation can skip a successful reuse's replayed actions.
    return __browserosCall(method, JSON.stringify(args ?? []), (globalThis.__helperDepth || 0) > 0);
  }

  function scoped(prefix, pageId) {
    return (name, args) => call(`${prefix}.${name}`, [pageId, ...args]);
  }

  const browser = {
    pages: {
      list: () => call('pages.list', []),
      newPage: (url, opts) => call('pages.newPage', [url, opts]),
      close: (pageId) => call('pages.close', [pageId]),
      getInfo: (pageId) => call('pages.getInfo', [pageId]),
    },
    observe: (pageId) => {
      const run = scoped('observe', pageId);
      return {
        snapshot: () => run('snapshot', []),
        diff: () => run('diff', []),
        resolveRef: (ref) => run('resolveRef', [ref]),
      };
    },
    input: (pageId) => {
      const run = scoped('input', pageId);
      return {
        click: (ref) => run('click', [ref]),
        fill: (ref, value) => run('fill', [ref, value]),
        type: (text) => run('type', [text]),
        press: (key) => run('press', [key]),
        hover: (ref) => run('hover', [ref]),
        selectOption: (ref, value) => run('selectOption', [ref, value]),
        scroll: (dir, amount, ref) => run('scroll', [dir, amount, ref]),
      };
    },
    nav: (pageId) => {
      const run = scoped('nav', pageId);
      return {
        goto: (url) => run('goto', [url]),
        back: () => run('back', []),
        forward: () => run('forward', []),
        reload: () => run('reload', []),
      };
    },
    cdp: (method, params, sessionId) => call('cdp', [method, params, sessionId]),
    cdpJsonForPage: (pageId, method, paramsJson) =>
      call('cdpJsonForPage', [pageId, method, paramsJson]),
    read: (pageId, opts) => call('tool:read', [pageId, opts]),
    grep: (pageId, opts) => call('tool:grep', [pageId, opts]),
    wait: (pageId, opts) => call('tool:wait', [pageId, opts]),
    screenshot: (pageId, opts) => call('tool:screenshot', [pageId, opts]),
    evaluate: (pageId, opts) => call('tool:evaluate', [pageId, opts]),
    download: (pageId, opts) => call('tool:download', [pageId, opts]),
    pdf: (pageId, opts) => call('tool:pdf', [pageId, opts]),
    upload: (pageId, opts) => call('tool:upload', [pageId, opts]),
    tabGroups: (opts) => call('tool:tab_groups', [opts]),
    windows: (opts) => call('tool:windows', [opts]),
    saveHelper: (name, source, opts) => {
      if (typeof source !== 'string' || !source.trim()) {
        throw new Error('saveHelper: source must be a non-empty function-expression string');
      }
      let fn;
      try { fn = new Function('return (' + source + '\n);')(); }
      catch (e) { throw new Error('saveHelper: source must be valid JS (' + e + ')'); }
      if (typeof fn !== 'function') {
        throw new Error('saveHelper: source must evaluate to a function, e.g. async (browser, page) => { ... }');
      }
      return call('helpers.save', [String(name), source, opts || {}]);
    },
    listHelpers: (opts) => call('helpers.list', [opts || {}]),
    readHelper: (name, opts) => call('helpers.read', [String(name), opts || {}]),
  };

  const sink = (level) => (...parts) => {
    __browserosPushLog(
      `${level}${parts
        .map((part) => (typeof part === 'string' ? part : safeStringify(part)))
        .join(' ')}`
    );
  };

  globalThis.__browserosBrowser = browser;
  globalThis.__browserosConsole = {
    log: sink(''),
    info: sink(''),
    warn: sink('warn: '),
    error: sink('error: '),
    debug: sink(''),
  };
  globalThis.__browserosMakeRunFunction = (code) =>
    new AsyncFunction('browser', 'console', `"use strict";\n${code}`);
  globalThis.__browserosJsonSafeString = jsonSafeString;
  globalThis.__browserosSafeStringify = safeStringify;
})();
"#;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct RunArgs {
    /// Async-capable JS body. Use top-level await; `return` a value.
    code: String,
    /// Max run time in ms (default 30000).
    #[serde(default = "default_timeout")]
    timeout: f64,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
struct RunOutput {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    logs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def_with_output::<RunArgs, RunOutput>(
        "run",
        DESCRIPTION,
        Some(super::open_world_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: RunArgs = parse_args(raw)?;
        let outcome = match execute_run(args, ctx).await {
            Ok(outcome) => outcome,
            Err(RunError::Syntax(message)) => {
                RunOutcome::failure(format!("run: syntax error - {message}"), Vec::new())
            }
            Err(RunError::Cancelled) => return Err(ToolError::Cancelled),
            Err(RunError::Engine(message)) => return Err(ToolError::message(message)),
        };
        Ok(Some(outcome.into_tool_result()))
    })
}

fn default_timeout() -> f64 {
    DEFAULT_TIMEOUT_MS
}

#[derive(Clone)]
struct RunControl {
    cancel: tokio_util::sync::CancellationToken,
    deadline: Instant,
    timeout_message: Arc<str>,
}

impl RunControl {
    async fn race<F, T>(&self, future: F) -> Result<T, String>
    where
        F: Future<Output = Result<T, browseros_core::CoreError>>,
    {
        tokio::select! {
            () = self.cancel.cancelled() => Err("cancelled".to_string()),
            () = sleep_until(self.deadline) => Err(self.timeout_message.to_string()),
            result = future => result.map_err(|err| err.to_string()),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    fn timed_out(&self) -> bool {
        Instant::now() >= self.deadline
    }
}

#[derive(Clone)]
struct BrowserBridge {
    /// The full tool context: session-direct primitives use `ctx.session`,
    /// tool-backed primitives dispatch through `execute_tool` with this ctx,
    /// and the inner-call hook lives at `ctx.inner_call_hook`.
    ctx: ToolCtx,
    control: RunControl,
}

enum BrowserCallValue {
    Json(Value),
    Undefined,
}

#[derive(Debug)]
enum RunError {
    Syntax(String),
    Cancelled,
    Engine(String),
}

struct RunOutcome {
    ok: bool,
    value: Option<Value>,
    return_text: Option<String>,
    logs: Vec<String>,
    error: Option<String>,
}

#[derive(Default)]
struct CapturedLogs {
    entries: Vec<String>,
    bytes: usize,
    limit_message: Option<String>,
}

type SharedLogs = Arc<Mutex<CapturedLogs>>;

enum JsonValueError<'js> {
    Js(CaughtError<'js>),
    Limit(String),
}

impl RunOutcome {
    fn success(value: Option<Value>, return_text: Option<String>, logs: Vec<String>) -> Self {
        Self {
            ok: true,
            value,
            return_text,
            logs,
            error: None,
        }
    }

    fn failure(error: impl Into<String>, logs: Vec<String>) -> Self {
        Self {
            ok: false,
            value: None,
            return_text: None,
            logs,
            error: Some(error.into()),
        }
    }

    fn into_tool_result(self) -> ToolResult {
        let text = format_outcome(&self);
        let structured = if self.ok {
            let mut object = Map::new();
            object.insert("ok".to_string(), json!(true));
            if let Some(value) = self.value {
                object.insert("value".to_string(), value);
            }
            object.insert("logs".to_string(), json!(self.logs));
            Value::Object(object)
        } else {
            json!({
                "ok": false,
                "logs": self.logs,
                "error": self.error,
            })
        };
        let mut result = text_result(text, Some(structured));
        result.is_error = !self.ok;
        result
    }
}

async fn execute_run(args: RunArgs, ctx: &ToolCtx) -> Result<RunOutcome, RunError> {
    ctx.throw_if_cancelled().map_err(|_| RunError::Cancelled)?;
    let logs = Arc::new(Mutex::new(CapturedLogs::default()));
    let timeout_ms = normalized_timeout_ms(args.timeout);
    let duration = Duration::from_millis(timeout_ms);
    let deadline = Instant::now() + duration;
    let timeout_message: Arc<str> = Arc::from(format!("run exceeded {timeout_ms}ms"));
    let control = RunControl {
        cancel: ctx.cancel.clone(),
        deadline,
        timeout_message: timeout_message.clone(),
    };
    let run = execute_quickjs(
        args.code,
        ctx.clone(),
        logs.clone(),
        control.clone(),
        duration,
    );
    tokio::select! {
        () = ctx.cancel.cancelled() => Err(RunError::Cancelled),
        () = sleep_until(deadline) => Ok(RunOutcome::failure(timeout_message.to_string(), logs_snapshot(&logs))),
        result = run => result,
    }
}

/// Loads the host-provided helpers into the script context as `helpers.<name>`
/// after the SDK bootstrap. A broken helper is contained by its try/catch and
/// reported through the captured console; it never fails the run.
fn load_preloaded_helpers(ctx: &Ctx<'_>, helpers: &[HelperSource]) {
    // Always expose the namespace, even with nothing to load, so a script that
    // references `helpers.<name>` gets a clean `undefined` instead of a
    // `ReferenceError: helpers is not defined`.
    let _ = ctx
        .eval::<(), _>("globalThis.helpers = globalThis.helpers || {}; globalThis.__helperDepth = globalThis.__helperDepth || 0;")
        .catch(ctx);
    for helper in helpers {
        let name = serde_json::to_string(&helper.name).unwrap_or_else(|_| "\"helper\"".to_string());
        // Wrap so calls made inside the helper run at helperDepth > 0, tagging
        // their primitives as replayed; the depth is restored even if it throws.
        let snippet = format!(
            "try {{ const __fn = (\n{source}\n); helpers[{name}] = async (...args) => {{ globalThis.__helperDepth++; try {{ return await __fn(...args); }} finally {{ globalThis.__helperDepth--; }} }}; }} catch (e) {{ console.log('helper load failed: ' + {name} + ': ' + e); }}",
            source = helper.source,
        );
        let _ = ctx.eval::<(), _>(snippet).catch(ctx);
    }
}

async fn execute_quickjs(
    code: String,
    mut tool_ctx: ToolCtx,
    logs: SharedLogs,
    control: RunControl,
    duration: Duration,
) -> Result<RunOutcome, RunError> {
    let preloaded_helpers = std::mem::take(&mut tool_ctx.preloaded_helpers);
    let runtime = AsyncRuntime::new().map_err(engine_error)?;
    runtime.set_memory_limit(RUN_MEMORY_LIMIT_BYTES).await;
    runtime.set_max_stack_size(RUN_STACK_SIZE_BYTES).await;
    let interrupt_control = control.clone();
    let interrupt_deadline = std::time::Instant::now() + duration;
    runtime
        .set_interrupt_handler(Some(Box::new(move || {
            interrupt_control.is_cancelled() || std::time::Instant::now() >= interrupt_deadline
        })))
        .await;
    let context = AsyncContext::full(&runtime).await.map_err(engine_error)?;
    let result = context
        .async_with(async |ctx| {
            install_globals(&ctx, tool_ctx, logs.clone(), control.clone())?;
            ctx.eval::<(), _>(BOOTSTRAP_JS).catch(&ctx).map_err(|err| {
                RunError::Engine(format!(
                    "failed to initialize run runtime: {}",
                    js_error_message(&ctx, err)
                ))
            })?;
            load_preloaded_helpers(&ctx, &preloaded_helpers);

            let make_run: Function<'_> = ctx
                .globals()
                .get("__browserosMakeRunFunction")
                .catch(&ctx)
                .map_err(|err| RunError::Engine(js_error_message(&ctx, err)))?;
            let user_fn: Function<'_> = make_run
                .call((code,))
                .catch(&ctx)
                .map_err(|err| RunError::Syntax(js_error_message(&ctx, err)))?;
            let browser: Object<'_> = ctx
                .globals()
                .get("__browserosBrowser")
                .catch(&ctx)
                .map_err(|err| RunError::Engine(js_error_message(&ctx, err)))?;
            let console: Object<'_> = ctx
                .globals()
                .get("__browserosConsole")
                .catch(&ctx)
                .map_err(|err| RunError::Engine(js_error_message(&ctx, err)))?;
            let promise: Promise<'_> = match user_fn.call((browser, console)).catch(&ctx) {
                Ok(promise) => promise,
                Err(err) => {
                    if control.is_cancelled() {
                        return Err(RunError::Cancelled);
                    }
                    if control.timed_out() {
                        return Ok(RunOutcome::failure(
                            control.timeout_message.to_string(),
                            logs_snapshot(&logs),
                        ));
                    }
                    return Ok(RunOutcome::failure(
                        js_error_message(&ctx, err),
                        logs_snapshot(&logs),
                    ));
                }
            };

            match promise.into_future::<JsValue<'_>>().await.catch(&ctx) {
                Ok(value) => match json_safe_value(&ctx, value) {
                    Ok((value, return_text)) => {
                        if control.is_cancelled() {
                            return Err(RunError::Cancelled);
                        }
                        if control.timed_out() {
                            return Ok(RunOutcome::failure(
                                control.timeout_message.to_string(),
                                logs_snapshot(&logs),
                            ));
                        }
                        Ok(RunOutcome::success(
                            value,
                            return_text,
                            logs_snapshot(&logs),
                        ))
                    }
                    Err(JsonValueError::Limit(message)) => {
                        Ok(RunOutcome::failure(message, logs_snapshot(&logs)))
                    }
                    Err(JsonValueError::Js(err)) => {
                        Err(RunError::Engine(js_error_message(&ctx, err)))
                    }
                },
                Err(err) => {
                    if control.is_cancelled() {
                        Err(RunError::Cancelled)
                    } else if control.timed_out() {
                        Ok(RunOutcome::failure(
                            control.timeout_message.to_string(),
                            logs_snapshot(&logs),
                        ))
                    } else {
                        Ok(RunOutcome::failure(
                            js_error_message(&ctx, err),
                            logs_snapshot(&logs),
                        ))
                    }
                }
            }
        })
        .await;
    runtime.set_interrupt_handler(None).await;
    result
}

fn install_globals<'js>(
    ctx: &Ctx<'js>,
    tool_ctx: ToolCtx,
    logs: SharedLogs,
    control: RunControl,
) -> Result<(), RunError> {
    let bridge = BrowserBridge {
        ctx: tool_ctx,
        control,
    };
    let call_bridge = {
        let bridge = bridge.clone();
        move |ctx: Ctx<'js>, method: String, args_json: String, from_helper: bool| {
            let bridge = bridge.clone();
            async move {
                match bridge.call(&method, &args_json, from_helper).await {
                    Ok(BrowserCallValue::Json(value)) => json_to_js(&ctx, value),
                    Ok(BrowserCallValue::Undefined) => Ok(JsValue::new_undefined(ctx.clone())),
                    Err(message) => Err(Exception::throw_message(&ctx, &message)),
                }
            }
        }
    };
    let push_log = move |ctx: Ctx<'js>, line: String| {
        push_log(&logs, line).map_err(|message| Exception::throw_message(&ctx, &message))
    };
    // Backs the JS setTimeout/sleep shims: the runtime is a bare rquickjs engine
    // with no timers, so bridge a fixed pause to a real async sleep. Capped to the
    // run budget; the run deadline still bounds total wall time.
    let sleep_bridge = move |_ctx: Ctx<'js>, ms: f64| async move {
        let capped = ms.max(0.0).min(MAX_TIMEOUT_MS as f64) as u64;
        tokio::time::sleep(Duration::from_millis(capped)).await;
    };
    let globals = ctx.globals();
    globals
        .set("__browserosCall", Func::from(Async(call_bridge)))
        .catch(ctx)
        .map_err(|err| RunError::Engine(js_error_message(ctx, err)))?;
    globals
        .set("__browserosSleep", Func::from(Async(sleep_bridge)))
        .catch(ctx)
        .map_err(|err| RunError::Engine(js_error_message(ctx, err)))?;
    globals
        .set("__browserosPushLog", Func::from(push_log))
        .catch(ctx)
        .map_err(|err| RunError::Engine(js_error_message(ctx, err)))?;
    Ok(())
}

impl BrowserBridge {
    async fn call(
        &self,
        method: &str,
        args_json: &str,
        from_helper: bool,
    ) -> Result<BrowserCallValue, String> {
        let args = parse_bridge_args(args_json)?;
        let page = target_page(method, &args);
        // Kept for the audit record and the self-healing distiller; dispatch
        // consumes the owned args below.
        let recorded_args = Value::Array(args.clone());
        if let Some(hook) = &self.ctx.inner_call_hook {
            hook.authorize(page).await?;
        }
        let started = Instant::now();
        let outcome = self.dispatch(method, args).await;
        if let Some(hook) = &self.ctx.inner_call_hook {
            if method == "pages.newPage"
                && let Ok(BrowserCallValue::Json(Value::Number(number))) = &outcome
                && let Some(page_id) = number.as_u64().and_then(|value| u32::try_from(value).ok())
            {
                hook.on_page_created(page_id).await;
            }
            let output_token_estimate = match &outcome {
                Ok(BrowserCallValue::Json(value)) => {
                    crate::token_estimate::estimate_json_output_tokens(value)
                }
                Ok(BrowserCallValue::Undefined) | Err(_) => 0,
            };
            hook.record(InnerCallRecord {
                method,
                page,
                args: &recorded_args,
                from_helper,
                is_error: outcome.is_err(),
                duration_ms: started.elapsed().as_millis() as i64,
                output_token_estimate,
            })
            .await;
        }
        outcome
    }

    async fn dispatch(&self, method: &str, args: Vec<Value>) -> Result<BrowserCallValue, String> {
        match method {
            "pages.list" => {
                let pages = self.control.race(self.ctx.session.pages.list()).await?;
                let mut values: Vec<Value> = pages.iter().map(page_json).collect();
                if let Some(hook) = &self.ctx.inner_call_hook {
                    values = hook.annotate_pages(&values).await;
                }
                Ok(BrowserCallValue::Json(Value::Array(values)))
            }
            "pages.newPage" => {
                let url = string_arg(&args, 0, "url")?;
                let opts = optional_object_arg(&args, 1)?;
                if opts.is_some_and(|options| options.contains_key("hidden")) {
                    return Err("pages.newPage: hidden is no longer supported".to_string());
                }
                let window_id = optional_i64_field(opts, "windowId")?
                    .map(WindowId)
                    .or_else(|| self.ctx.defaults.default_window_id.clone());
                let tab_group_id = optional_string_field(opts, "tabGroupId")?
                    .or_else(|| self.ctx.defaults.default_tab_group_id.clone());
                let page_id = self
                    .control
                    .race(self.ctx.session.pages.new_page(
                        &url,
                        NewPageOptions {
                            // Default to a background tab so a working agent does
                            // not steal the user's focus, matching the granular
                            // tabs-new default. An explicit background:false opens
                            // it active, which the agent should do only when the
                            // user asks to bring a tab to the front.
                            background: optional_bool_field(opts, "background")?.or(Some(true)),
                            window_id,
                            tab_group_id,
                        },
                    ))
                    .await?;
                Ok(BrowserCallValue::Json(json!(page_id.0)))
            }
            "pages.close" => {
                let page_id = page_arg(&args, 0)?;
                self.control
                    .race(self.ctx.session.pages.close(page_id))
                    .await?;
                Ok(BrowserCallValue::Undefined)
            }
            "pages.getInfo" => {
                let page_id = page_arg(&args, 0)?;
                let info = self
                    .control
                    .race(self.ctx.session.pages.refresh(page_id))
                    .await?;
                Ok(BrowserCallValue::Json(
                    info.map(|page| page_json(&page)).unwrap_or(Value::Null),
                ))
            }
            "observe.snapshot" => {
                let page_id = page_arg(&args, 0)?;
                let observer = self.ctx.session.observe(page_id).await;
                let snapshot = self.control.race(observer.snapshot()).await?;
                Ok(BrowserCallValue::Json(json!({
                    "text": snapshot.text,
                    "refs": refs_json(&snapshot.refs),
                    "url": snapshot.url,
                })))
            }
            "observe.diff" => {
                let page_id = page_arg(&args, 0)?;
                let observer = self.ctx.session.observe(page_id).await;
                let diff = self.control.race(observer.diff()).await?;
                Ok(BrowserCallValue::Json(diff_json(&diff)))
            }
            "observe.resolveRef" => {
                let page_id = page_arg(&args, 0)?;
                let ref_id = string_arg(&args, 1, "ref")?;
                let observer = self.ctx.session.observe(page_id).await;
                let resolved = self
                    .control
                    .race(observer.resolve_ref(&Ref(ref_id)))
                    .await?;
                Ok(BrowserCallValue::Json(json!({
                    "backendNodeId": resolved.backend_node_id,
                    "sessionId": resolved.session.session_id().map(ToString::to_string),
                })))
            }
            "input.click" => {
                let (page_id, ref_id) = page_ref_args(&args)?;
                let input = self.ctx.session.input(page_id).await;
                self.control
                    .race(input.click(&Ref(ref_id), Default::default()))
                    .await?;
                Ok(BrowserCallValue::Undefined)
            }
            "input.fill" => {
                let (page_id, ref_id) = page_ref_args(&args)?;
                let value = string_arg(&args, 2, "value")?;
                let input = self.ctx.session.input(page_id).await;
                self.control
                    .race(input.fill(&Ref(ref_id), &value, true))
                    .await?;
                Ok(BrowserCallValue::Undefined)
            }
            "input.type" => {
                let page_id = page_arg(&args, 0)?;
                let text = string_arg(&args, 1, "text")?;
                let input = self.ctx.session.input(page_id).await;
                self.control.race(input.type_text(&text)).await?;
                Ok(BrowserCallValue::Undefined)
            }
            "input.press" => {
                let page_id = page_arg(&args, 0)?;
                let key = string_arg(&args, 1, "key")?;
                let input = self.ctx.session.input(page_id).await;
                self.control.race(input.press(&key)).await?;
                Ok(BrowserCallValue::Undefined)
            }
            "input.hover" => {
                let (page_id, ref_id) = page_ref_args(&args)?;
                let input = self.ctx.session.input(page_id).await;
                self.control.race(input.hover(&Ref(ref_id))).await?;
                Ok(BrowserCallValue::Undefined)
            }
            "input.selectOption" => {
                let (page_id, ref_id) = page_ref_args(&args)?;
                let value = string_arg(&args, 2, "value")?;
                let input = self.ctx.session.input(page_id).await;
                let selected = self
                    .control
                    .race(input.select_option(&Ref(ref_id), &value))
                    .await?;
                Ok(BrowserCallValue::Json(json!(selected)))
            }
            "input.scroll" => {
                let page_id = page_arg(&args, 0)?;
                let direction = scroll_direction(&string_arg(&args, 1, "dir")?)?;
                let amount = optional_f64_arg(&args, 2).unwrap_or(3.0).round() as i64;
                let ref_id = optional_string_arg(&args, 3)?.map(Ref);
                let input = self.ctx.session.input(page_id).await;
                self.control
                    .race(input.scroll(direction, amount, ref_id.as_ref()))
                    .await?;
                Ok(BrowserCallValue::Undefined)
            }
            "nav.goto" => {
                let page_id = page_arg(&args, 0)?;
                let url = string_arg(&args, 1, "url")?;
                let nav = self.ctx.session.nav(page_id);
                self.control.race(nav.goto(&url)).await?;
                Ok(BrowserCallValue::Undefined)
            }
            "nav.back" => {
                let page_id = page_arg(&args, 0)?;
                let nav = self.ctx.session.nav(page_id);
                self.control.race(nav.back()).await?;
                Ok(BrowserCallValue::Undefined)
            }
            "nav.forward" => {
                let page_id = page_arg(&args, 0)?;
                let nav = self.ctx.session.nav(page_id);
                self.control.race(nav.forward()).await?;
                Ok(BrowserCallValue::Undefined)
            }
            "nav.reload" => {
                let page_id = page_arg(&args, 0)?;
                let nav = self.ctx.session.nav(page_id);
                self.control.race(nav.reload()).await?;
                Ok(BrowserCallValue::Undefined)
            }
            "cdp" => {
                let method = string_arg(&args, 0, "method")?;
                let params = optional_json_arg(&args, 1).unwrap_or_else(|| json!({}));
                let session_id = optional_string_arg(&args, 2)?.map(SessionId::from);
                let value = self
                    .control
                    .race(self.ctx.session.cdp(&method, params, session_id.as_ref()))
                    .await?;
                Ok(BrowserCallValue::Json(value))
            }
            "cdpJsonForPage" => {
                let page_id = page_arg(&args, 0)?;
                let method = string_arg(&args, 1, "method")?;
                let params_json = string_arg(&args, 2, "paramsJson")?;
                let raw = self
                    .control
                    .race(
                        self.ctx
                            .session
                            .cdp_json_for_page(page_id, &method, &params_json),
                    )
                    .await?;
                let value = serde_json::from_str(&raw).map_err(|err| err.to_string())?;
                Ok(BrowserCallValue::Json(value))
            }
            "helpers.save" => {
                let name = string_arg(&args, 0, "name")?;
                let source = string_arg(&args, 1, "source")?;
                let opts = optional_object_arg(&args, 2)?;
                let host = self.resolve_helper_host(opts).await?;
                self.helper_hook()?
                    .save_helper(&host, &name, &source)
                    .await?;
                Ok(BrowserCallValue::Json(
                    json!({ "saved": name, "host": host }),
                ))
            }
            "helpers.list" => {
                let opts = optional_object_arg(&args, 0)?;
                let host = self.resolve_helper_host(opts).await?;
                let helpers = self.helper_hook()?.list_helpers(&host).await;
                Ok(BrowserCallValue::Json(
                    json!({ "host": host, "helpers": helpers }),
                ))
            }
            "helpers.read" => {
                let name = string_arg(&args, 0, "name")?;
                let opts = optional_object_arg(&args, 1)?;
                let host = self.resolve_helper_host(opts).await?;
                match self.helper_hook()?.read_helper(&host, &name).await {
                    Some(source) => Ok(BrowserCallValue::Json(Value::String(source))),
                    None => Ok(BrowserCallValue::Json(Value::Null)),
                }
            }
            method if method.starts_with("tool:") => {
                let tool_name = &method["tool:".len()..];
                self.run_tool(tool_name, build_tool_args(&args)).await
            }
            _ => Err(format!("Unknown browser method {method}")),
        }
    }

    /// The injected hook, or an error surfaced to the script when helpers are
    /// unavailable (no host attached, e.g. a unit-test context).
    fn helper_hook(&self) -> Result<&Arc<dyn crate::framework::InnerCallHook>, String> {
        self.ctx
            .inner_call_hook
            .as_ref()
            .ok_or_else(|| "helpers are not available in this context".to_string())
    }

    /// Resolves the helper host bucket from `{ host }` (explicit) or `{ page }`
    /// (the page's URL, host-side). One is required.
    async fn resolve_helper_host(
        &self,
        opts: Option<&Map<String, Value>>,
    ) -> Result<String, String> {
        if let Some(host) = optional_string_field(opts, "host")? {
            return Ok(host);
        }
        if let Some(page) = optional_i64_field(opts, "page")? {
            let page = u32::try_from(page).map_err(|_| "page id is out of range".to_string())?;
            if let Some(host) = self.helper_hook()?.resolve_host(page).await {
                return Ok(host);
            }
            return Err(format!(
                "no host for page {page}; navigate to a site first or pass an explicit host"
            ));
        }
        Err("helpers need a host: pass { host } or { page }".to_string())
    }

    /// Dispatches a tool-backed primitive through the real tool handler so the
    /// SDK reaches full parity without reimplementing each tool. The inner-call
    /// hook already wrapped this via `call`, so `execute_tool` runs the handler
    /// with the same ctx.
    async fn run_tool(&self, tool_name: &str, args: Value) -> Result<BrowserCallValue, String> {
        let def = tool_def(tool_name).ok_or_else(|| format!("Unknown tool {tool_name}"))?;
        let result = execute_tool(&def, args, &self.ctx)
            .await
            .map_err(|err| err.to_string())?;
        if result.is_error {
            return Err(tool_result_text(&result));
        }
        // Text-producing primitives return their text, not the metadata: a
        // script wants the markdown from read and the matching lines from grep,
        // not { format, path, contentLength }.
        if matches!(tool_name, "read" | "grep") {
            return Ok(BrowserCallValue::Json(Value::String(tool_result_text(
                &result,
            ))));
        }
        Ok(match result.structured_content {
            Some(value) => BrowserCallValue::Json(value),
            None => {
                let text = tool_result_text(&result);
                if text.is_empty() {
                    BrowserCallValue::Undefined
                } else {
                    BrowserCallValue::Json(Value::String(text))
                }
            }
        })
    }
}

/// Resolves a tool-backed primitive name to its definition. Only the tools that
/// are not already covered by the session-direct SDK are routable.
fn tool_def(name: &str) -> Option<ToolDef> {
    use crate::tools;
    Some(match name {
        "read" => tools::read::definition(),
        "grep" => tools::grep::definition(),
        "wait" => tools::wait::definition(),
        "screenshot" => tools::screenshot::definition(),
        "evaluate" => tools::evaluate::definition(),
        "download" => tools::download::definition(),
        "pdf" => tools::pdf::definition(),
        "upload" => tools::upload::definition(),
        "tab_groups" => tools::tab_groups::definition(),
        "windows" => tools::windows::definition(),
        _ => return None,
    })
}

/// Builds a tool's argument object from the SDK call arguments. Page-scoped
/// primitives pass `[pageId, opts]`; page-less ones pass `[opts]`.
fn build_tool_args(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::Number(page)) => {
            let mut object = args
                .get(1)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            object.insert("page".to_string(), Value::Number(page.clone()));
            Value::Object(object)
        }
        Some(object @ Value::Object(_)) => object.clone(),
        _ => Value::Object(Map::new()),
    }
}

fn tool_result_text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| match block {
            rmcp::model::ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_bridge_args(args_json: &str) -> Result<Vec<Value>, String> {
    serde_json::from_str::<Vec<Value>>(args_json)
        .map_err(|err| format!("Invalid browser call arguments: {err}"))
}

/// The page a bridge primitive targets, for the ownership hook. Page-scoped
/// SDK helpers (`observe`/`input`/`nav` and the page-first `pages`/`cdp`
/// variants) carry the page id as their first argument; the rest address no
/// specific page.
fn target_page(method: &str, args: &[Value]) -> Option<u32> {
    let page_first = method.starts_with("observe.")
        || method.starts_with("input.")
        || method.starts_with("nav.")
        || method.starts_with("tool:")
        || matches!(method, "pages.close" | "pages.getInfo" | "cdpJsonForPage");
    if !page_first {
        return None;
    }
    args.first()
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn page_arg(args: &[Value], index: usize) -> Result<PageId, String> {
    let raw = args
        .get(index)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("pageId argument {index} is required"))?;
    let page_id = u32::try_from(raw).map_err(|_| format!("pageId {raw} is out of range"))?;
    Ok(PageId(page_id))
}

fn page_ref_args(args: &[Value]) -> Result<(PageId, String), String> {
    Ok((page_arg(args, 0)?, string_arg(args, 1, "ref")?))
}

fn string_arg(args: &[Value], index: usize, name: &str) -> Result<String, String> {
    args.get(index)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| format!("{name} argument is required"))
}

fn optional_string_arg(args: &[Value], index: usize) -> Result<Option<String>, String> {
    match args.get(index) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("argument {index} must be a string")),
    }
}

fn optional_f64_arg(args: &[Value], index: usize) -> Option<f64> {
    args.get(index).and_then(Value::as_f64)
}

fn optional_json_arg(args: &[Value], index: usize) -> Option<Value> {
    match args.get(index) {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.clone()),
    }
}

fn optional_object_arg(
    args: &[Value],
    index: usize,
) -> Result<Option<&Map<String, Value>>, String> {
    match args.get(index) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(value)) => Ok(Some(value)),
        Some(_) => Err(format!("argument {index} must be an object")),
    }
}

fn optional_bool_field(
    object: Option<&Map<String, Value>>,
    name: &str,
) -> Result<Option<bool>, String> {
    match object.and_then(|object| object.get(name)) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("{name} must be a boolean")),
    }
}

fn optional_i64_field(
    object: Option<&Map<String, Value>>,
    name: &str,
) -> Result<Option<i64>, String> {
    match object.and_then(|object| object.get(name)) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("{name} must be an integer")),
        Some(_) => Err(format!("{name} must be an integer")),
    }
}

fn optional_string_field(
    object: Option<&Map<String, Value>>,
    name: &str,
) -> Result<Option<String>, String> {
    match object.and_then(|object| object.get(name)) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{name} must be a string")),
    }
}

fn scroll_direction(value: &str) -> Result<ScrollDirection, String> {
    match value {
        "up" => Ok(ScrollDirection::Up),
        "down" => Ok(ScrollDirection::Down),
        "left" => Ok(ScrollDirection::Left),
        "right" => Ok(ScrollDirection::Right),
        _ => Err(format!("Unknown scroll direction {value}")),
    }
}

fn refs_json(refs: &browseros_core::snapshot::RefMap) -> Value {
    Value::Array(
        refs.entries_in_order()
            .into_iter()
            .map(|entry| {
                let mut value = json!({
                    "ref": entry.ref_id.as_str(),
                    "backendNodeId": entry.backend_node_id,
                    "role": entry.role,
                    "name": entry.name,
                    "nth": entry.nth,
                });
                if let (Value::Object(object), Some(frame_id)) = (&mut value, &entry.frame_id) {
                    object.insert("frameId".to_string(), json!(frame_id.as_str()));
                }
                value
            })
            .collect(),
    )
}

fn diff_json(diff: &browseros_core::snapshot::SnapshotDiff) -> Value {
    let mut value = json!({
        "text": diff.text,
        "added": diff.added,
        "removed": diff.removed,
        "changed": diff.changed,
    });
    if let Value::Object(object) = &mut value {
        if let Some(before_url) = &diff.before_url {
            object.insert("beforeUrl".to_string(), json!(before_url));
        }
        if let Some(after_url) = &diff.after_url {
            object.insert("afterUrl".to_string(), json!(after_url));
        }
    }
    value
}

fn json_to_js<'js>(ctx: &Ctx<'js>, value: Value) -> rquickjs::Result<JsValue<'js>> {
    match value {
        Value::Null => Ok(JsValue::new_null(ctx.clone())),
        Value::Bool(value) => Ok(JsValue::new_bool(ctx.clone(), value)),
        Value::Number(value) => Ok(JsValue::new_number(
            ctx.clone(),
            value.as_f64().unwrap_or_default(),
        )),
        Value::String(value) => value.into_js(ctx),
        Value::Array(values) => {
            let array = Array::new(ctx.clone())?;
            for (index, value) in values.into_iter().enumerate() {
                array.set(index, json_to_js(ctx, value)?)?;
            }
            Ok(array.into_value())
        }
        Value::Object(values) => {
            let object = Object::new(ctx.clone())?;
            for (key, value) in values {
                object.set(key, json_to_js(ctx, value)?)?;
            }
            Ok(object.into_value())
        }
    }
}

fn json_safe_value<'js>(
    ctx: &Ctx<'js>,
    value: JsValue<'js>,
) -> Result<(Option<Value>, Option<String>), JsonValueError<'js>> {
    let encode: Function<'_> = ctx
        .globals()
        .get("__browserosJsonSafeString")
        .catch(ctx)
        .map_err(JsonValueError::Js)?;
    let encoded: Option<String> = encode
        .call((value.clone(),))
        .catch(ctx)
        .map_err(JsonValueError::Js)?;
    let Some(encoded) = encoded else {
        return Ok((None, None));
    };
    if encoded.len() > MAX_RETURN_VALUE_BYTES {
        return Err(JsonValueError::Limit(format!(
            "run return value exceeded {MAX_RETURN_VALUE_BYTES} byte limit"
        )));
    }
    let display: Function<'_> = ctx
        .globals()
        .get("__browserosSafeStringify")
        .catch(ctx)
        .map_err(JsonValueError::Js)?;
    let return_text: String = display
        .call((value,))
        .catch(ctx)
        .map_err(JsonValueError::Js)?;
    let value = serde_json::from_str(&encoded).map_err(|err| {
        JsonValueError::Js(CaughtError::Error(rquickjs::Error::new_from_js_message(
            "string",
            "JSON",
            err.to_string(),
        )))
    })?;
    Ok((Some(value), Some(return_text)))
}

fn js_error_message<'js>(ctx: &Ctx<'js>, error: CaughtError<'js>) -> String {
    match error {
        CaughtError::Error(error) => error.to_string(),
        CaughtError::Exception(exception) => {
            exception.message().unwrap_or_else(|| exception.to_string())
        }
        CaughtError::Value(value) => js_value_string(ctx, value),
    }
}

fn js_value_string<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> String {
    if value.is_undefined() {
        return "undefined".to_string();
    }
    if value.is_null() {
        return "null".to_string();
    }
    if let Some(value) = value.as_bool() {
        return value.to_string();
    }
    if let Some(value) = value.as_number() {
        return value.to_string();
    }
    if let Ok(value) = String::from_js(ctx, value.clone()) {
        return value;
    }
    let string_constructor: rquickjs::Result<Function<'_>> = ctx.globals().get("String");
    match string_constructor.and_then(|func| func.call((value,))) {
        Ok(value) => value,
        Err(err) => err.to_string(),
    }
}

fn format_outcome(outcome: &RunOutcome) -> String {
    let mut sections = Vec::new();
    if let Some(error) = &outcome.error {
        sections.push(format!("error: {error}"));
    } else {
        sections.push("ok".to_string());
        if let Some(value) = &outcome.return_text {
            sections.push(format!("return: {value}"));
        }
    }
    if !outcome.logs.is_empty() {
        sections.push(format!("logs:\n{}", outcome.logs.join("\n")));
    }
    sections.join("\n")
}

fn normalized_timeout_ms(timeout_ms: f64) -> u64 {
    if !timeout_ms.is_finite() || timeout_ms <= 0.0 {
        MIN_TIMEOUT_MS
    } else {
        timeout_ms.ceil().min(MAX_TIMEOUT_MS as f64) as u64
    }
}

fn logs_snapshot(logs: &SharedLogs) -> Vec<String> {
    logs.lock()
        .map(|logs| logs.entries.clone())
        .unwrap_or_else(|_| Vec::new())
}

fn push_log(logs: &SharedLogs, line: String) -> Result<(), String> {
    let mut logs = logs
        .lock()
        .map_err(|_| "run log capture unavailable".to_string())?;
    if let Some(message) = &logs.limit_message {
        return Err(message.clone());
    }
    if logs.entries.len() >= MAX_LOG_ENTRIES
        || logs.bytes.saturating_add(line.len()) > MAX_LOG_BYTES
    {
        let message = format!(
            "run console output exceeded limit (max {MAX_LOG_ENTRIES} entries, {MAX_LOG_BYTES} bytes)"
        );
        logs.limit_message = Some(message.clone());
        return Err(message);
    }
    logs.bytes = logs.bytes.saturating_add(line.len());
    logs.entries.push(line);
    Ok(())
}

fn engine_error(error: rquickjs::Error) -> RunError {
    RunError::Engine(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::InnerCallHook;
    use crate::{
        framework::{BrowserToolDefaults, BrowserToolOptions, ToolCtx, execute_tool},
        output_file::create_browser_output_file_access,
    };
    use browseros_cdp::{CdpError, CdpEvent};
    use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection, WindowId};
    use futures_util::future::BoxFuture;
    use serde_json::json;
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;

    struct RunFakeConnection {
        sender: broadcast::Sender<CdpEvent>,
        state: Arc<Mutex<RunFakeState>>,
    }

    #[derive(Default)]
    struct RunFakeState {
        create_tab_params: Vec<Value>,
        add_group_params: Vec<Value>,
    }

    impl RunFakeConnection {
        fn new() -> Self {
            let (sender, _receiver) = broadcast::channel(8);
            Self {
                sender,
                state: Arc::new(Mutex::new(RunFakeState::default())),
            }
        }

        fn create_tab_params(&self) -> Vec<Value> {
            self.state
                .lock()
                .map(|state| state.create_tab_params.clone())
                .unwrap_or_default()
        }

        fn add_group_params(&self) -> Vec<Value> {
            self.state
                .lock()
                .map(|state| state.add_group_params.clone())
                .unwrap_or_default()
        }
    }

    impl CdpConnection for RunFakeConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            let state = self.state.clone();
            Box::pin(async move {
                match method {
                    "Browser.getTabs" => Ok(json!({
                        "tabs": [fake_tab_json(7, "target-7", "https://example.com", "Example", 1, 0)]
                    })),
                    "Browser.getWindows" => Ok(json!({ "windows": [] })),
                    "Browser.hang" => {
                        futures_util::future::pending::<Result<Value, CdpError>>().await
                    }
                    "Browser.createTab" => {
                        if let Ok(mut state) = state.lock() {
                            state.create_tab_params.push(params.clone());
                        }
                        Ok(json!({
                            "tab": fake_tab_json(
                                9,
                                "target-9",
                                params.get("url").and_then(Value::as_str).unwrap_or("about:blank"),
                                "Created",
                                params.get("windowId").and_then(Value::as_i64).unwrap_or(1),
                                1
                            )
                        }))
                    }
                    "Browser.getTabInfo" => {
                        let tab_id = params.get("tabId").and_then(Value::as_i64).unwrap_or(7);
                        let tab = if tab_id == 9 {
                            fake_tab_json(9, "target-9", "https://new.example", "Created", 42, 1)
                        } else {
                            fake_tab_json(7, "target-7", "https://example.com", "Example", 1, 0)
                        };
                        Ok(json!({ "tab": tab }))
                    }
                    "Target.attachToTarget" => Ok(json!({ "sessionId": "session-target-7" })),
                    "Page.enable"
                    | "DOM.enable"
                    | "Runtime.enable"
                    | "Accessibility.enable"
                    | "Runtime.runIfWaitingForDebugger"
                    | "Target.setAutoAttach"
                    | "Page.reload" => Ok(json!({})),
                    "Browser.addTabsToGroup" => {
                        if let Ok(mut state) = state.lock() {
                            state.add_group_params.push(params.clone());
                        }
                        Ok(json!({
                            "group": {
                                "groupId": params
                                    .get("groupId")
                                    .and_then(Value::as_str)
                                    .unwrap_or("group-1"),
                                "windowId": 42,
                                "title": "group",
                                "color": "blue",
                                "collapsed": false,
                                "tabIds": params
                                    .get("tabIds")
                                    .cloned()
                                    .unwrap_or_else(|| json!([]))
                            }
                        }))
                    }
                    _ => Err(CdpError::Protocol {
                        code: -1,
                        message: format!("unexpected fake CDP call: {method}"),
                    }),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async move {
                match method {
                    "Runtime.evaluate" => Ok(json!({ "result": { "value": 3 } }).to_string()),
                    _ => Err(CdpError::Protocol {
                        code: -1,
                        message: format!("unexpected fake CDP raw call: {method}"),
                    }),
                }
            })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.sender.subscribe()
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    fn test_ctx() -> ToolCtx {
        test_ctx_for(
            Arc::new(RunFakeConnection::new()),
            BrowserToolDefaults::default(),
        )
    }

    fn test_ctx_for(connection: Arc<RunFakeConnection>, defaults: BrowserToolDefaults) -> ToolCtx {
        ToolCtx::new(BrowserToolOptions {
            session: BrowserSession::new(connection, BrowserSessionHooks::default()),
            defaults,
            cancel: CancellationToken::new(),
            output_files: create_browser_output_file_access(),
            inner_call_hook: None,
            preloaded_helpers: Vec::new(),
        })
    }

    async fn run_tool(code: &str, timeout: Option<f64>) -> anyhow::Result<ToolResult> {
        let ctx = test_ctx();
        run_tool_with_ctx(code, timeout, &ctx).await
    }

    async fn run_tool_with_ctx(
        code: &str,
        timeout: Option<f64>,
        ctx: &ToolCtx,
    ) -> anyhow::Result<ToolResult> {
        let mut args = json!({ "code": code });
        if let (Value::Object(object), Some(timeout)) = (&mut args, timeout) {
            object.insert("timeout".to_string(), json!(timeout));
        }
        let def = definition();
        execute_tool(&def, args, ctx)
            .await
            .map_err(|err| anyhow::anyhow!(err.to_string()))
    }

    #[derive(Default)]
    struct HookLog {
        authorized: Vec<Option<u32>>,
        recorded: Vec<(String, Option<u32>, bool)>,
        created: Vec<u32>,
        reject: Option<String>,
        annotated: usize,
        saved: Vec<(String, String, String)>,
        from_helper: Vec<(String, bool)>,
    }

    struct MockHook(Arc<Mutex<HookLog>>);

    impl InnerCallHook for MockHook {
        fn authorize<'a>(&'a self, page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
            let mut log = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            log.authorized.push(page);
            let result = match &log.reject {
                Some(message) => Err(message.clone()),
                None => Ok(()),
            };
            Box::pin(async move { result })
        }

        fn record<'a>(&'a self, record: InnerCallRecord<'a>) -> BoxFuture<'a, ()> {
            let mut log = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            log.recorded
                .push((record.method.to_owned(), record.page, record.is_error));
            log.from_helper
                .push((record.method.to_owned(), record.from_helper));
            Box::pin(async move {})
        }

        fn on_page_created<'a>(&'a self, page_id: u32) -> BoxFuture<'a, ()> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .created
                .push(page_id);
            Box::pin(async move {})
        }

        fn annotate_pages<'a>(&'a self, pages: &'a [Value]) -> BoxFuture<'a, Vec<Value>> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .annotated += 1;
            let tagged = pages
                .iter()
                .map(|page| {
                    let mut page = page.clone();
                    if let Value::Object(fields) = &mut page {
                        fields.insert("ownership".to_owned(), Value::String("mine".to_owned()));
                    }
                    page
                })
                .collect();
            Box::pin(async move { tagged })
        }

        fn resolve_host<'a>(&'a self, _page: u32) -> BoxFuture<'a, Option<String>> {
            Box::pin(async move { Some("resolved.example".to_string()) })
        }

        fn save_helper<'a>(
            &'a self,
            host: &'a str,
            name: &'a str,
            source: &'a str,
        ) -> BoxFuture<'a, Result<(), String>> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .saved
                .push((host.to_owned(), name.to_owned(), source.to_owned()));
            Box::pin(async move { Ok(()) })
        }

        fn list_helpers<'a>(&'a self, _host: &'a str) -> BoxFuture<'a, Vec<Value>> {
            Box::pin(
                async move { vec![json!({ "name": "greet", "ageDays": 2, "candidate": false })] },
            )
        }

        fn read_helper<'a>(
            &'a self,
            _host: &'a str,
            _name: &'a str,
        ) -> BoxFuture<'a, Option<String>> {
            Box::pin(async move { Some("async () => 42".to_string()) })
        }
    }

    fn ctx_with_hook(log: Arc<Mutex<HookLog>>) -> ToolCtx {
        let mut ctx = test_ctx();
        ctx.inner_call_hook = Some(Arc::new(MockHook(log)));
        ctx
    }

    #[tokio::test]
    async fn run_tags_primitives_called_inside_a_helper() -> anyhow::Result<()> {
        let log = Arc::new(Mutex::new(HookLog::default()));
        let mut ctx = ctx_with_hook(log.clone());
        ctx.preloaded_helpers = vec![HelperSource {
            name: "act".to_string(),
            source: "async (browser) => browser.pages.getInfo(1)".to_string(),
        }];
        // One direct primitive, then one via the helper.
        let result = run_tool_with_ctx(
            "await browser.pages.getInfo(1); await helpers.act(browser); return 'ok';",
            None,
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        let log = log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The direct call is not from a helper; the one inside act() is.
        assert_eq!(
            log.from_helper,
            vec![
                ("pages.getInfo".to_string(), false),
                ("pages.getInfo".to_string(), true),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn settimeout_and_sleep_shims_resolve() -> anyhow::Result<()> {
        // The runtime is a bare engine with no native timers; the bootstrap
        // bridges setTimeout/sleep to a real async sleep, so the model's natural
        // `await new Promise(r => setTimeout(r, ms))` idiom resolves instead of
        // throwing `setTimeout is not defined`.
        let result = run_tool(
            "await new Promise((r) => setTimeout(r, 5)); await sleep(5); return 'ok';",
            None,
        )
        .await?;
        assert!(!result.is_error);
        Ok(())
    }

    #[tokio::test]
    async fn run_exposes_an_empty_helpers_namespace_without_preloads() -> anyhow::Result<()> {
        // No preloaded_helpers: referencing helpers.<name> must not throw.
        let ctx = test_ctx();
        let result = run_tool_with_ctx(
            "return { kind: typeof helpers, missing: typeof helpers.nope };",
            None,
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        let structured = result
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("structured content"))?;
        assert_eq!(structured["value"]["kind"], json!("object"));
        assert_eq!(structured["value"]["missing"], json!("undefined"));
        Ok(())
    }

    #[tokio::test]
    async fn run_hot_loads_preloaded_helpers_and_skips_a_broken_one() -> anyhow::Result<()> {
        let mut ctx = test_ctx();
        ctx.preloaded_helpers = vec![
            HelperSource {
                name: "double".to_string(),
                source: "async (x) => x * 2".to_string(),
            },
            // A syntax-broken helper must be skipped without failing the run.
            HelperSource {
                name: "broken".to_string(),
                source: "async (x) => {".to_string(),
            },
        ];
        let result = run_tool_with_ctx(
            "const ok = typeof helpers.broken; return { doubled: await helpers.double(21), broken: ok };",
            None,
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        let structured = result
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("structured content"))?;
        assert_eq!(structured["value"]["doubled"], json!(42));
        assert_eq!(structured["value"]["broken"], json!("undefined"));
        Ok(())
    }

    #[tokio::test]
    async fn run_save_helper_routes_to_the_hook_with_explicit_host() -> anyhow::Result<()> {
        let log = Arc::new(Mutex::new(HookLog::default()));
        let ctx = ctx_with_hook(log.clone());
        let result = run_tool_with_ctx(
            "return await browser.saveHelper('greet', 'async (browser, page) => 1', { host: 'linkedin.com' })",
            None,
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        let log = log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(
            log.saved,
            vec![(
                "linkedin.com".to_string(),
                "greet".to_string(),
                "async (browser, page) => 1".to_string()
            )]
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_save_helper_resolves_the_host_from_a_page() -> anyhow::Result<()> {
        let log = Arc::new(Mutex::new(HookLog::default()));
        let ctx = ctx_with_hook(log.clone());
        let result = run_tool_with_ctx(
            "return await browser.saveHelper('greet', 'async () => 1', { page: 1 })",
            None,
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        let log = log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(log.saved[0].0, "resolved.example");
        Ok(())
    }

    #[tokio::test]
    async fn run_save_helper_rejects_a_non_function_source() -> anyhow::Result<()> {
        let ctx = test_ctx();
        let result = run_tool_with_ctx(
            "try { await browser.saveHelper('x', '123', { host: 'h' }); return 'saved'; } catch (e) { return String(e); }",
            None,
            &ctx,
        )
        .await?;
        let structured = result
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("structured content"))?;
        let value = structured["value"].as_str().unwrap_or_default();
        assert!(
            value.contains("saveHelper"),
            "expected rejection, got: {value}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_list_and_read_helpers_route_to_the_hook() -> anyhow::Result<()> {
        let log = Arc::new(Mutex::new(HookLog::default()));
        let ctx = ctx_with_hook(log);
        let listed = run_tool_with_ctx(
            "return (await browser.listHelpers({ host: 'h' })).helpers.map((x) => x.name)",
            None,
            &ctx,
        )
        .await?;
        assert_eq!(
            listed.structured_content,
            Some(json!({ "ok": true, "value": ["greet"], "logs": [] }))
        );
        let read = run_tool_with_ctx(
            "return await browser.readHelper('greet', { host: 'h' })",
            None,
            &ctx,
        )
        .await?;
        assert_eq!(
            read.structured_content,
            Some(json!({ "ok": true, "value": "async () => 42", "logs": [] }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_hook_authorizes_and_records_page_scoped_primitive() -> anyhow::Result<()> {
        let log = Arc::new(Mutex::new(HookLog::default()));
        let ctx = ctx_with_hook(log.clone());
        let result = run_tool_with_ctx("return await browser.pages.getInfo(1)", None, &ctx).await?;
        assert!(!result.is_error);
        let log = log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(log.authorized, vec![Some(1)]);
        assert_eq!(
            log.recorded,
            vec![("pages.getInfo".to_owned(), Some(1), false)]
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_hook_rejection_blocks_primitive_before_dispatch() -> anyhow::Result<()> {
        let log = Arc::new(Mutex::new(HookLog {
            reject: Some("page 1 is not owned by this agent".to_owned()),
            ..HookLog::default()
        }));
        let ctx = ctx_with_hook(log.clone());
        let result = run_tool_with_ctx("return await browser.pages.getInfo(1)", None, &ctx).await?;
        assert!(result.is_error);
        let text = result_text(&result)?;
        assert!(text.contains("not owned by this agent"));
        let log = log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(log.authorized, vec![Some(1)]);
        // Rejected before dispatch, so nothing is recorded.
        assert!(log.recorded.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn run_routes_tool_backed_primitive_through_handler_and_hooks_it() -> anyhow::Result<()> {
        let log = Arc::new(Mutex::new(HookLog::default()));
        let ctx = ctx_with_hook(log.clone());
        let result = run_tool_with_ctx(
            "return await browser.windows({ action: 'list' })",
            None,
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        let structured = result
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("structured content"))?;
        // The windows tool ran and its structured output is the script's return value.
        assert_eq!(structured["value"]["action"], json!("list"));
        // A page-less tool primitive authorizes with no page and is recorded.
        let log = log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(log.authorized, vec![None]);
        assert_eq!(log.recorded, vec![("tool:windows".to_owned(), None, false)]);
        Ok(())
    }

    #[tokio::test]
    async fn run_returns_json_safe_value() -> anyhow::Result<()> {
        let result = run_tool("return 1 + 1", None).await?;
        assert!(!result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": true,
                "value": 2,
                "logs": []
            }))
        );
        let text = result_text(&result)?;
        assert!(text.contains("ok"));
        assert!(text.contains("return: 2"));
        Ok(())
    }

    #[tokio::test]
    async fn run_captures_console_output() -> anyhow::Result<()> {
        let result = run_tool(
            r#"
console.log('a', { b: 1 });
console.info('i');
console.warn('w');
console.error('e');
return undefined;
"#,
            None,
        )
        .await?;
        assert!(!result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": true,
                "logs": [
                    "a {\n  \"b\": 1\n}",
                    "i",
                    "warn: w",
                    "error: e"
                ]
            }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_reports_runtime_exception_as_structured_error() -> anyhow::Result<()> {
        let result = run_tool(
            r#"
console.log('before');
throw new Error('boom');
"#,
            None,
        )
        .await?;
        assert!(result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": false,
                "logs": ["before"],
                "error": "boom"
            }))
        );
        let text = result_text(&result)?;
        assert!(text.contains("error: boom"));
        Ok(())
    }

    #[tokio::test]
    async fn run_reports_syntax_error_as_structured_error() -> anyhow::Result<()> {
        let result = run_tool("return (", None).await?;
        assert!(result.is_error);
        let structured = result
            .structured_content
            .as_ref()
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("missing structured object"))?;
        assert_eq!(structured.get("ok"), Some(&json!(false)));
        assert_eq!(structured.get("logs"), Some(&json!([])));
        let error = structured
            .get("error")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing structured error"))?;
        assert!(
            error.starts_with("run: syntax error - "),
            "unexpected error: {error}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_reports_timeout_with_logs_so_far() -> anyhow::Result<()> {
        let result = run_tool(
            r#"
console.log('before');
while (true) {}
"#,
            Some(10.0),
        )
        .await?;
        assert!(result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": false,
                "logs": ["before"],
                "error": "run exceeded 10ms"
            }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_timeout_cannot_be_caught_as_success() -> anyhow::Result<()> {
        let result = run_tool(
            r#"
try {
  await browser.cdp('Browser.hang');
} catch (_err) {
  return 'caught';
}
"#,
            Some(10.0),
        )
        .await?;
        assert!(result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": false,
                "logs": [],
                "error": "run exceeded 10ms"
            }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_clamps_pathological_timeout_values() -> anyhow::Result<()> {
        let result = run_tool("return 'ok'", Some(f64::MAX)).await?;
        assert!(!result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": true,
                "value": "ok",
                "logs": []
            }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_rejects_excessive_console_output_as_structured_error() -> anyhow::Result<()> {
        let result = run_tool(
            &format!("console.log('x'.repeat({}));", MAX_LOG_BYTES + 1),
            None,
        )
        .await?;
        assert!(result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": false,
                "logs": [],
                "error": format!(
                    "run console output exceeded limit (max {MAX_LOG_ENTRIES} entries, {MAX_LOG_BYTES} bytes)"
                )
            }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_rejects_excessive_return_output_as_structured_error() -> anyhow::Result<()> {
        let result = run_tool(
            &format!("return 'x'.repeat({});", MAX_RETURN_VALUE_BYTES + 1),
            None,
        )
        .await?;
        assert!(result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": false,
                "logs": [],
                "error": format!("run return value exceeded {MAX_RETURN_VALUE_BYTES} byte limit")
            }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_proxies_browser_pages_list() -> anyhow::Result<()> {
        let result = run_tool("return await browser.pages.list()", None).await?;
        assert!(!result.is_error);
        let page = result
            .structured_content
            .as_ref()
            .and_then(|structured| structured.pointer("/value/0"))
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("missing page value"))?;
        assert_eq!(page.get("pageId"), Some(&json!(1)));
        assert_eq!(page.get("tabId"), Some(&json!(7)));
        assert!(!page.contains_key("isHidden"));
        Ok(())
    }

    #[tokio::test]
    async fn run_pages_list_routes_through_the_hook_annotation() -> anyhow::Result<()> {
        let log = Arc::new(Mutex::new(HookLog::default()));
        let ctx = ctx_with_hook(log.clone());
        let result = run_tool_with_ctx(
            "const pages = await browser.pages.list(); return pages.map((p) => p.ownership);",
            None,
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({ "ok": true, "value": ["mine"], "logs": [] }))
        );
        assert_eq!(
            log.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .annotated,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_proxies_browser_pages_get_info_with_refresh() -> anyhow::Result<()> {
        let result = run_tool(
            r#"
const page = await browser.pages.getInfo(1);
return { pageId: page.pageId, tabId: page.tabId, url: page.url, title: page.title };
"#,
            None,
        )
        .await?;
        assert!(!result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": true,
                "value": {
                    "pageId": 1,
                    "tabId": 7,
                    "url": "https://example.com",
                    "title": "Example"
                },
                "logs": []
            }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_pages_new_page_forwards_default_placement() -> anyhow::Result<()> {
        let connection = Arc::new(RunFakeConnection::new());
        let ctx = test_ctx_for(
            connection.clone(),
            BrowserToolDefaults {
                default_window_id: Some(WindowId(42)),
                default_tab_group_id: Some("group-1".to_string()),
            },
        );
        let result = run_tool_with_ctx(
            "return await browser.pages.newPage('https://new.example')",
            None,
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": true,
                "value": 1,
                "logs": []
            }))
        );

        let create_params = connection.create_tab_params();
        assert_eq!(create_params.len(), 1);
        assert_eq!(
            create_params.first().and_then(|params| params.get("url")),
            Some(&json!("https://new.example"))
        );
        assert_eq!(
            create_params
                .first()
                .and_then(|params| params.get("windowId")),
            Some(&json!(42))
        );

        let group_params = connection.add_group_params();
        assert_eq!(group_params.len(), 1);
        assert_eq!(
            group_params
                .first()
                .and_then(|params| params.get("groupId")),
            Some(&json!("group-1"))
        );
        assert_eq!(
            group_params.first().and_then(|params| params.get("tabIds")),
            Some(&json!([9]))
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_pages_new_page_forwards_options_object() -> anyhow::Result<()> {
        let connection = Arc::new(RunFakeConnection::new());
        let ctx = test_ctx_for(
            connection.clone(),
            BrowserToolDefaults {
                default_window_id: Some(WindowId(42)),
                default_tab_group_id: Some("default-group".to_string()),
            },
        );
        let result = run_tool_with_ctx(
            r#"
return await browser.pages.newPage('https://new.example', {
  background: true,
  windowId: 88,
  tabGroupId: 'group-opts',
});
"#,
            None,
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({
                "ok": true,
                "value": 1,
                "logs": []
            }))
        );

        let create_params = connection.create_tab_params();
        assert_eq!(create_params.len(), 1);
        assert_eq!(
            create_params.first().and_then(|params| params.get("url")),
            Some(&json!("https://new.example"))
        );
        assert_eq!(
            create_params
                .first()
                .and_then(|params| params.get("background")),
            Some(&json!(true))
        );
        assert_eq!(
            create_params
                .first()
                .and_then(|params| params.get("windowId")),
            Some(&json!(88))
        );
        assert!(
            create_params
                .first()
                .and_then(|params| params.get("hidden"))
                .is_none()
        );

        let group_params = connection.add_group_params();
        assert_eq!(group_params.len(), 1);
        assert_eq!(
            group_params
                .first()
                .and_then(|params| params.get("groupId")),
            Some(&json!("group-opts"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_pages_new_page_rejects_hidden_option() -> anyhow::Result<()> {
        let connection = Arc::new(RunFakeConnection::new());
        let ctx = test_ctx_for(connection.clone(), BrowserToolDefaults::default());
        let result = run_tool_with_ctx(
            "return await browser.pages.newPage('https://new.example', { hidden: true })",
            None,
            &ctx,
        )
        .await?;

        assert!(result.is_error);
        assert!(result_text(&result)?.contains("pages.newPage: hidden is no longer supported"));
        assert!(connection.create_tab_params().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn run_exercises_remaining_browser_namespaces() -> anyhow::Result<()> {
        let result = run_tool(
            r#"
const seen = {};
seen.cdp = (await browser.cdp('Browser.getTabs')).tabs.length;
seen.cdpJsonForPage = await browser.cdpJsonForPage(
  1,
  'Runtime.evaluate',
  '{"expression":"1+1"}'
);
for (const [name, action] of Object.entries({
  observe: () => browser.observe(999).snapshot(),
  input: () => browser.input(999).type('x'),
  nav: () => browser.nav(999).reload(),
})) {
  try {
    await action();
    seen[name] = 'ok';
  } catch (err) {
    seen[name] = String(err && err.message ? err.message : err);
  }
}
return seen;
"#,
            None,
        )
        .await?;
        assert!(!result.is_error);
        let value = result
            .structured_content
            .as_ref()
            .and_then(|structured| structured.get("value"))
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("missing structured value"))?;
        assert_eq!(value.get("cdp"), Some(&json!(1)));
        assert_eq!(
            value.get("cdpJsonForPage"),
            Some(&json!({ "result": { "value": 3 } }))
        );
        for namespace in ["observe", "input", "nav"] {
            let error = value
                .get(namespace)
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing {namespace} result"))?;
            assert!(
                error.contains("Unknown page 999"),
                "unexpected {namespace} error: {error}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn run_browser_api_failure_rejects_into_structured_error() -> anyhow::Result<()> {
        let result = run_tool("await browser.cdp('Browser.nope')", None).await?;
        assert!(result.is_error);
        let structured = result
            .structured_content
            .as_ref()
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("missing structured object"))?;
        assert_eq!(structured.get("ok"), Some(&json!(false)));
        assert_eq!(structured.get("logs"), Some(&json!([])));
        let error = structured
            .get("error")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing structured error"))?;
        assert!(
            error.contains("unexpected fake CDP call: Browser.nope"),
            "unexpected error: {error}"
        );
        Ok(())
    }

    fn result_text(result: &ToolResult) -> anyhow::Result<&str> {
        result
            .content
            .first()
            .and_then(|content| content.as_text())
            .map(|content| content.text.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing text result"))
    }

    fn fake_tab_json(
        tab_id: i64,
        target_id: &str,
        url: &str,
        title: &str,
        window_id: i64,
        index: i64,
    ) -> Value {
        json!({
            "tabId": tab_id,
            "targetId": target_id,
            "url": url,
            "title": title,
            "isActive": true,
            "isLoading": false,
            "loadProgress": 1.0,
            "isPinned": false,
            "isHidden": false,
            "windowId": window_id,
            "index": index
        })
    }
}
