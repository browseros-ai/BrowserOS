# Host seam: `ScriptHost` / `ScriptEngine` / `ScriptDispatch`, buildable tonight

Follow-up to `architecture-review.md`. Additive: `InnerCallHook`,
`ScriptInnerCallHook`, `helper_runtime.rs` and `run.rs` are untouched;
the new port lives beside them. No HTTP contract change. Both engine
shapes fit: an in-process engine calling from Rust with `PageId`/`TargetId`,
and a sidecar reporting over a stream with `targetId` (+ fork `tabId`).
Layering test stays green: new host modules are under `api/mcp/`, the port
types are in the shared crate. The 30 s cap and cancellation stay
dispatch-shaped through `RunControl`. Line numbers are from
`packages/browseros-agent` at `8d9a40820`.

**Ping facts.** Host-side pieces: 6 (5 required + 1 optional). Existing
function unreachable without a new `pub(crate)` seam: **none**. Every
function the dispatch needs is already `pub` or `pub(crate)` inside
`claw-server-rust`. The only visibility change anywhere is optional
(`observers/audit.rs::preview_callback`, see §2, row 1).

---

## 1. Final Rust signatures

### 1a. Port types: new `crates/browseros-mcp/src/script.rs`

A new module, not `framework.rs`. `framework.rs` is 728 lines of tool
registry plus JSON-schema normalisation; the port is a distinct seam with
its own tests, and the `run.rs` adapter must be able to import it without
pulling the QuickJS types the other way. `lib.rs` re-exports it.

```rust
//! The seam between a script engine and the host. The host implements
//! `ScriptHost` once (`ScriptDispatch` in claw-server-rust); every engine
//! implements `ScriptEngine` and only *reports* what it did. Audit rows,
//! page claims, tab grouping, ownership notices, screenshots, liveness,
//! helper preloading and redaction all live behind `ScriptHost`.

use crate::framework::{HelperSource, ToolCtx, ToolResult, text_result};
use browseros_core::{PageId, TabId, TargetId};
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// How an engine names a page. The host resolves every variant to a `PageId`
/// (§3). In-process engines pass `Page`; a sidecar passes `Target`, or `Tab`
/// when the fork's `Target.TargetInfo.tabId` is at hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageRef {
    Page(PageId),
    Tab(TabId),
    Target(TargetId),
}

/// What the engine knows about the input a typed value went into. `Unknown`
/// makes the host probe the page (§4). `Plain` means the engine looked and
/// the input is not sensitive; `Secret` forces masking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetSensitivity {
    Plain,
    Secret,
    Unknown,
}

/// Marks the argument that carries text typed into the page, so the host can
/// mask it before the audit row is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedValue {
    pub arg_index: usize,
    pub target: TargetSensitivity,
}

pub struct ActionStart<'a> {
    /// Engine-neutral method name shown in the audit: `input.click`,
    /// `page.goto`, `locator.fill`. The host strips a leading `tool:`.
    pub method: &'a str,
    pub page: Option<PageRef>,
    /// JSON array of arguments. The host bounds it to 4 KiB on write.
    pub args: &'a Value,
    pub from_helper: bool,
    pub typed_value: Option<TypedValue>,
}

pub struct ActionEnd<'a> {
    pub outcome: Result<&'a Value, &'a str>,
    pub output_token_estimate: i64,
}

/// Minted by `action_started`, consumed by `action_finished`. Engines pass it
/// through untouched; the fields are public only so the host can build it.
#[derive(Debug, Clone)]
pub struct ActionToken {
    pub started: Instant,
    pub method: String,
    pub page: Option<PageId>,
    pub args: Value,
    pub from_helper: bool,
    pub typed_value: Option<TypedValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperDialect {
    BrowserSdk,
    Playwright,
}

/// Helpers the host preloaded for the caller's owned-tab hosts, already
/// filtered to the engine's dialect.
#[derive(Debug, Clone, Default)]
pub struct HelperCatalog {
    pub dialect: Option<HelperDialect>,
    pub sources: Vec<HelperSource>,
}

/// The port an engine calls. Never refuses: ownership is a label.
pub trait ScriptHost: Send + Sync {
    fn action_started<'a>(&'a self, start: ActionStart<'a>) -> BoxFuture<'a, ActionToken>;
    fn action_finished<'a>(&'a self, token: ActionToken, end: ActionEnd<'a>) -> BoxFuture<'a, ()>;
    /// A page the script opened (newPage, popup). Idempotent. `None` when the
    /// target could not be resolved before it vanished.
    fn page_created<'a>(&'a self, page: PageRef) -> BoxFuture<'a, Option<PageId>>;
    /// Tri-bucket ownership for a `pages.list()` result, keyed by `pageId`.
    fn annotate_pages<'a>(&'a self, pages: &'a [Value]) -> BoxFuture<'a, Vec<Value>>;
    fn helpers(&self) -> &HelperCatalog;
}

/// Dispatch-shaped control: one tool call, one script, one deadline.
#[derive(Clone)]
pub struct RunControl {
    pub cancel: CancellationToken,
    pub deadline: Instant,
}

pub struct ScriptRequest {
    pub code: String,
    /// Already clamped by the host to the 30 s cap.
    pub timeout: Duration,
}

/// `run`'s envelope, so every engine yields the same MCP result and the
/// cockpit renders both without change.
#[derive(Debug, Clone, Default)]
pub struct ScriptOutcome {
    pub ok: bool,
    pub value: Option<Value>,
    pub return_text: Option<String>,
    pub logs: Vec<String>,
    pub error: Option<String>,
}

impl ScriptOutcome {
    /// Byte-for-byte the shape of `run.rs::RunOutcome::into_tool_result`
    /// (text: `ok`/`return:`/`logs:`; structured: `{ok, value?, logs, error?}`).
    #[must_use]
    pub fn into_tool_result(self) -> ToolResult { /* copy of run.rs:361-386 + format_outcome */ }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("syntax error: {0}")]
    Syntax(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Engine(String),
    /// Sidecar missing, failed to start, or crashed mid-run.
    #[error("engine unavailable: {0}")]
    Unavailable(String),
}

/// One adapter per engine. Reads `ctx.script` (host, control) and
/// `ctx.session`; must honour `control.cancel` and `control.deadline`.
pub trait ScriptEngine: Send + Sync {
    fn name(&self) -> &'static str;
    fn dialect(&self) -> HelperDialect;
    fn run<'a>(
        &'a self,
        request: ScriptRequest,
        ctx: &'a ToolCtx,
    ) -> BoxFuture<'a, Result<ScriptOutcome, EngineError>>;
}

/// Present on `ToolCtx` only while a `ScriptHost`-backed tool runs.
#[derive(Clone)]
pub struct ScriptContext {
    pub host: Arc<dyn ScriptHost>,
    pub engine: Arc<dyn ScriptEngine>,
    pub control: RunControl,
}
```

### 1b. `ToolCtx` addition (`framework.rs`), one field beside the existing two

```rust
pub struct ToolCtx {
    pub session: Arc<BrowserSession>,
    pub defaults: BrowserToolDefaults,
    pub cancel: CancellationToken,
    pub output_files: OutputFileAccess,
    pub inner_call_hook: Option<Arc<dyn InnerCallHook>>,   // unchanged
    pub preloaded_helpers: Vec<HelperSource>,               // unchanged
    /// Set by the host after construction for `ScriptHost`-backed tools;
    /// `None` everywhere else. `BrowserToolOptions` is not widened, so no
    /// existing constructor site changes.
    pub script: Option<crate::script::ScriptContext>,
}
// ToolCtx::new: `script: None,`
```

`lib.rs`: `pub mod script;` and
`pub use script::{ActionEnd, ActionStart, ActionToken, EngineError, HelperCatalog, HelperDialect, PageRef, RunControl, ScriptContext, ScriptEngine, ScriptHost, ScriptOutcome, ScriptRequest, TargetSensitivity, TypedValue};`

### 1c. `apps/claw-server-rust/src/api/mcp/script_dispatch/` (host, `ApiMcp` layer)

Split into four files so pieces 2–4 own disjoint files.

```rust
// script_dispatch/mod.rs
//! The host's view of one in-flight script dispatch. Accounts each action
//! once, whichever engine performed it. Reuses the effect and observer
//! helpers `ScriptInnerCallHook` already reuses; adds nothing to them.

pub(crate) mod actions;
pub(crate) mod pages;
pub(crate) mod redact;

pub struct ScriptDispatch {
    call: ToolCall,
    helpers: HelperCatalog,
    /// Tab ids this dispatch has already claimed, so repeated or late
    /// `page_created` reports are no-ops.
    claimed_tabs: tokio::sync::Mutex<std::collections::HashSet<i64>>,
}

impl ScriptDispatch {
    /// Preloads helpers for `dialect` (BrowserSdk only tonight, §2 row 7).
    pub async fn open(call: &ToolCall, dialect: HelperDialect) -> Arc<Self>;
    pub(crate) fn call(&self) -> &ToolCall;
}

impl ScriptHost for ScriptDispatch {
    fn action_started<'a>(&'a self, start: ActionStart<'a>) -> BoxFuture<'a, ActionToken> {
        Box::pin(actions::started(self, start))
    }
    fn action_finished<'a>(&'a self, token: ActionToken, end: ActionEnd<'a>) -> BoxFuture<'a, ()> {
        Box::pin(actions::finished(self, token, end))
    }
    fn page_created<'a>(&'a self, page: PageRef) -> BoxFuture<'a, Option<PageId>> {
        Box::pin(pages::created(self, page))
    }
    fn annotate_pages<'a>(&'a self, pages: &'a [Value]) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let Some(identity) = &self.call.identity else { return pages.to_vec() };
            tabs_list_view::annotate_pages_with_ownership(&self.call.state, &identity.ownership_key, pages, "pageId").await
        })
    }
    fn helpers(&self) -> &HelperCatalog { &self.helpers }
}
```

```rust
// script_dispatch/pages.rs
pub(super) async fn resolve(dispatch: &ScriptDispatch, page: &PageRef) -> Option<PageInfo>;
pub(super) async fn resolve_fast(dispatch: &ScriptDispatch, page: &PageRef) -> Option<PageInfo>; // no list()
pub(super) async fn created(dispatch: &ScriptDispatch, page: PageRef) -> Option<PageId>;
```

```rust
// script_dispatch/actions.rs
pub(super) async fn started(dispatch: &ScriptDispatch, start: ActionStart<'_>) -> ActionToken;
pub(super) async fn finished(dispatch: &ScriptDispatch, token: ActionToken, end: ActionEnd<'_>);
/// "the user" / "another agent (label)"; `None` when the caller owns the page.
pub(super) async fn describe_foreign_owner(state: &AppState, identity: &ToolIdentity, page: PageId) -> Option<String>;
```

```rust
// script_dispatch/redact.rs
pub(super) struct Redaction { pub arg_index: usize, pub plaintext: String }
/// Returns the args to store and the redactions applied (§4).
pub(super) async fn mask_typed_value(
    dispatch: &ScriptDispatch, token: &ActionToken,
) -> (Value, Vec<Redaction>);
/// Replaces every recorded plaintext inside a JSON value (parent row).
pub(crate) fn redact_literals(value: &Value, secrets: &[String]) -> Value;
```

### 1d. Host tool, registry, `dispatch.rs`, `AppState`, `service.rs`

```rust
// api/mcp/script_tool.rs  (host-defined ToolDef; NOT in browseros_mcp::catalog())
pub const SCRIPT_TOOL_NAME: &str = "playwright";   // DECIDED: placeholder; mediator names it
pub fn definition() -> ToolDef {
    ToolDef {
        name: SCRIPT_TOOL_NAME, description: DESCRIPTION,
        input_schema: browseros_mcp::framework::input_schema::<ScriptArgs>(),   // {code, timeout}
        output_schema: Some(browseros_mcp::framework::output_schema::<ScriptOutput>()), // run's RunOutput shape
        annotations: Some(ToolAnnotations::new().open_world(true)),
        metadata: ToolMetadata::default(), handler,
    }
}
/// `browseros_mcp::catalog()` plus the host's script tools. Used by
/// `ClawMcpService::new` and `test_support::tool_call`.
pub fn host_catalog() -> Vec<ToolDef>;
fn handler<'a>(raw: Value, ctx: &'a ToolCtx, _r: &'a mut ToolResponse)
    -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>>
{
    Box::pin(async move {
        let args: ScriptArgs = parse_args(raw)?;
        let Some(script) = &ctx.script else {
            return Ok(Some(ToolResult::error("script host not attached")));
        };
        let request = ScriptRequest { code: args.code, timeout: script.control.deadline.saturating_duration_since(Instant::now()) };
        match script.engine.run(request, ctx).await {
            Ok(outcome) => Ok(Some(outcome.into_tool_result())),
            Err(EngineError::Cancelled) => Err(ToolError::Cancelled),
            Err(EngineError::Syntax(m)) => Ok(Some(ScriptOutcome { ok: false, error: Some(format!("syntax error - {m}")), ..Default::default() }.into_tool_result())),
            Err(e) => Err(ToolError::message(e.to_string())),
        }
    })
}
```

```rust
// api/mcp/engines.rs
pub struct ScriptEngineRegistry { engines: HashMap<&'static str, Arc<dyn ScriptEngine>> }
impl ScriptEngineRegistry {
    pub fn new() -> Self;                                              // empty
    pub fn register(&mut self, tool: &'static str, engine: Arc<dyn ScriptEngine>);
    pub fn get(&self, tool: &str) -> Option<Arc<dyn ScriptEngine>>;
}
/// Adapter that answers `EngineError::Unavailable` until a real engine is
/// registered, so the tool can ship dark.
pub struct UnavailableEngine;
```

```rust
// api/mcp/dispatch.rs — additions only
pub(crate) const SCRIPT_HOST_TOOLS: &[&str] = &[crate::api::mcp::script_tool::SCRIPT_TOOL_NAME];

pub struct ToolCall {
    // ...all existing fields unchanged...
    /// Plaintext values the script dispatch masked in child rows, so the
    /// parent row's script text is masked the same way (§4). Initialised in
    /// `ToolCall::new`; no signature change.
    pub redactions: Arc<std::sync::Mutex<Vec<String>>>,
}

// execute_with_cancellation, inside `Some(browser_session) =>`, after the
// existing `preloaded_helpers` block and `let ctx = ToolCtx::new(...)`:
let mut ctx = ToolCtx::new(BrowserToolOptions { /* unchanged */ });
if SCRIPT_HOST_TOOLS.contains(&call.tool().name) && call.identity.is_some() {
    let engine = call.state.script_engines.get(call.tool().name)
        .unwrap_or_else(|| Arc::new(crate::api::mcp::engines::UnavailableEngine));
    let host = crate::api::mcp::script_dispatch::ScriptDispatch::open(call, engine.dialect()).await;
    let timeout_ms = browseros_mcp::framework::clamp_timeout(
        call.raw_args.get("timeout").and_then(Value::as_f64), 30_000, 30_000);
    ctx.script = Some(ScriptContext {
        host, engine,
        control: RunControl { cancel: call.cancel.clone(), deadline: Instant::now() + Duration::from_millis(timeout_ms) },
    });
}
// existing `execute_tool(call.tool(), ...)` call unchanged
```

The existing `is_script` branch (`ARBITRARY_SCRIPT_TOOLS`, `ScriptInnerCallHook`,
`preload_helpers`) is not touched; the new tool is not in
`ARBITRARY_SCRIPT_TOOLS`, so it gets `inner_call_hook: None`.

```rust
// app.rs (Composition layer): one field, built empty in new_with_home
pub script_engines: Arc<crate::api::mcp::engines::ScriptEngineRegistry>,

// service.rs:106  `catalog: Arc::new(catalog())`  →
catalog: Arc::new(crate::api::mcp::script_tool::host_catalog()),

// observers/audit.rs build_event, args_json line (two lines):
let secrets = call.redactions.lock().map(|s| s.clone()).unwrap_or_default();
args_json: bounded_args_json(&crate::api::mcp::script_dispatch::redact::redact_literals(&call.raw_args, &secrets)),
```

`page_ownership_notice` needs nothing: `ScriptDispatch` writes into
`call.foreign_pages`, the field the effect already reads (`dispatch.rs:74`,
`page_ownership_notice.rs:82`). `run_failure` needs nothing tonight: it is
keyed on `"run"`; adding the new tool name is a one-line follow-up with
candidate 3 of the scan.

---

## 2. The eight concerns, each served by an existing function

| # | Concern | `ScriptDispatch` call | Existing function (visibility) |
|---|---|---|---|
| 1 | Child audit row + per-step screenshot | `actions::finished`: builds `RecordToolDispatchInput { parent_dispatch_id: Some(call.dispatch_id), created_at: None, page/tab/target/url/title from `pages::resolve_fast`, args_json: masked (§4), result_meta: `child_result_meta`-shaped }` then `state.audit_log.record_tool_dispatch(input).await` → `row_id`; if the action had a page, `observers::audit::persist_screenshot(&state, session_id, &child_id, row_id).await` | `AuditLog::record_tool_dispatch` (pub, `audit_log.rs:257`); `persist_screenshot` (`pub(crate)`, `observers/audit.rs:184`). DECIDED: direct write, same as `script_hook.rs:135-174`, for parity (every page-targeting step gets its screenshot; the worker coalesces previews to the latest per session, `services/audit.rs:323`). Follow-up: worker route needs `preview_callback` made `pub(crate)` and a non-coalesced mode. Parent row unchanged: `observers::audit::apply` via `audit_worker.submit`, `created_at = started_at_ms`, so it sorts first. |
| 2 | Page claim + tab group | `pages::created`: resolve (§3) → `ownership_claims::record_new_page(&state, identity, Some(&browser), session_id, page_id.0, call.started_at_ms).await` → `tokio::spawn(tab_groups::run_tab_group_work(call.clone(), Some(page_id.0)))` | both `pub(crate)` (`ownership_claims.rs:68`, `tab_groups.rs:48`); identical to `script_hook.rs:183-199` |
| 3 | Ownership notice | `actions::started`: `describe_foreign_owner` (copy of `script_hook.rs:55-79` logic over `state.sessions.owner_of_page` + `snapshot()`), insert into `call.foreign_pages` | `Sessions::owner_of_page` (pub, `manager.rs:458`); `ToolCall.foreign_pages` (pub field) read by `page_ownership_notice::run_script_notice` |
| 4 | Screenshots | row 1 | `persist_screenshot` → `state.visuals.capture` + `state.screenshots.write` + `audit_log.mark_screenshot` (all pub) |
| 5 | Replay recorder | row 2: `record_new_page` → `session_tabs.enqueue_claim_tab_for_session(tab_id, …)`; recorder attribution keys on ledger windows (`recordings/store.rs:338`) | pub |
| 6 | Run-failure telemetry | none tonight (observer keyed on `"run"`); follow-up: extend `REPORTED_TOOL` to a list containing `SCRIPT_TOOL_NAME` | `run_failure.rs:16` |
| 7 | Helper preloading | `ScriptDispatch::open`: `dialect == BrowserSdk` → `helper_runtime::preload_helpers(&state, &identity.ownership_key, &browser).await`; any other dialect → empty catalog (the store has no dialect field yet; scan candidate 5) | `preload_helpers` (`pub(crate)`, `helper_runtime.rs:40`) |
| 8 | Cancellation + 30 s cap | host builds `RunControl { cancel: call.cancel, deadline }` in `execute_with_cancellation`; `call.cancel` already folds session, client and operator tokens (`linked_cancel_token`, `dispatch.rs:585`). Engine obligation: honour both; a sidecar adapter sends abort, then kills on deadline + grace | `clamp_timeout` (pub, `framework.rs`) |

Also in `actions::finished`, before the row: `identity.session.touch(Instant::now()).await`
(pub, liveness) and `state.tab_activity.record_tool(RecordToolInput { target_id, tab_id, page_id, session_id, agent_id, slug, tool_name })`
(pub, `cockpit/activity.rs:114`), exactly as `script_hook.rs:106-125`.

---

## 3. Page identity resolution for `PageRef`

**Mint point.** `PageManager::list()` (`pages.rs:78-130`) calls the fork's
`Browser.getTabs` and mints a `PageId` for every tab it has not seen,
whoever created the tab. So a target created by a sidecar or by rustwright's
own socket gets a `PageId` on the next `list()`. No core change is needed;
there is no public targetId lookup, so the host lists and matches.

```rust
// pages.rs (script_dispatch)
pub(super) async fn resolve(d: &ScriptDispatch, page: &PageRef) -> Option<PageInfo> {
    let browser = d.call().browser_session.as_ref()?;
    match page {
        PageRef::Page(id) => match browser.pages.get_info(id.clone()).await {
            Some(info) => Some(info),
            None => browser.pages.refresh(id.clone()).await.ok().flatten(),
        },
        PageRef::Tab(tab) => {
            let map = browser.pages.resolve_tab_ids(std::slice::from_ref(tab)).await.ok()?; // lists
            browser.pages.get_info(map.get(tab)?.clone()).await
        }
        PageRef::Target(target) => {
            // Creation is in flight for a moment after Target.targetCreated;
            // mirror new_page's own poll (pages.rs:301-318): 10 × 100 ms.
            for attempt in 0..10 {
                if let Some(info) = browser.pages.list().await.ok()?.into_iter()
                    .find(|p| p.target_id == *target) { return Some(info); }
                if attempt == 9 { break; }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            // Last resort: the registry saw the target (Target.* events) even
            // if Browser.getTabs has not listed it yet.
            let tab_id = d.call().state.tab_registry.tab_for_target(target.as_str()).await?;
            let map = browser.pages.resolve_tab_ids(&[TabId(tab_id)]).await.ok()?;
            browser.pages.get_info(map.get(&TabId(tab_id))?.clone()).await
        }
    }
}
/// For per-action lookups: cache only (`get_info`), or `tab_registry.tab_for_target`
/// + cached pages for Target/Tab. Never lists; an unresolved page records a row
/// with `page_id: None`.
pub(super) async fn resolve_fast(d: &ScriptDispatch, page: &PageRef) -> Option<PageInfo>;
```

**`tabId`.** Comes with the `PageInfo` (`tab_id`, from `Browser.getTabs`),
so no `Target.getTargetInfo` call is needed. `TabRegistry::tab_for_target`
(`tab_registry.rs:173`) is the fallback map, fed by `Target.targetCreated`
(`:305-338`).

**Idempotency.** `TabRegistry` never writes `PageOwnership` or tab groups;
it writes the ledger only for popups (`enqueue_inherit_tab_ownership`,
`:318-332`) and releases on `targetDestroyed`. The ledger's
`claim_tab_for_session` (`session_tabs.rs:525-549`) returns the existing
row when the open claim has the same `session_id` and `agent_id`, and
supersedes another owner's open claim; `inherit_tab_ownership` (`:565`)
calls the same function on the opener's behalf. `PageOwnership::claim_page`
is a map insert. `run_tab_group_work` returns early when the page is
already in the agent's group (`tab_groups.rs:87-97`). Therefore:

- registry first (popup inherited), engine `page_created` later: ledger
  no-op, map + group added; converge.
- engine first, registry inherit later: ledger no-op; converge.
- engine reports the same target twice: `claimed_tabs` makes the second a
  no-op before any I/O.
- target destroyed before resolution: `resolve` returns `None`,
  `page_created` returns `None`, nothing claimed.
- popup the engine never reports: ledger-owned (screenshots, replay,
  cleanup) but not in the map or group, exactly today's gap. Follow-up: a
  `TabRegistry` inherit hook that also calls `record_new_page`.

---

## 4. Redaction rule for action args (added at the mediator's request)

**Where.** `redact::mask_typed_value`, called in `actions::finished`
before `bounded_args_json`. One place; both engines; the parent row is
masked from the same list via `ToolCall.redactions`.

**What is masked.** The argument at `TypedValue.arg_index` of an action
that typed text into the page, when the input is sensitive. Sensitive =
`type="password"` at minimum, plus `autocomplete` containing any of
`current-password`, `new-password`, `one-time-code`, `cc-number`,
`cc-csc`, `cc-exp`, `cc-exp-month`, `cc-exp-year`. Stored as the string
`"[redacted]"`; the child's `result_meta` gains `"redacted": [arg_index]`;
the plaintext is pushed to `call.redactions` so the parent row's script
text (`raw_args.code`, which embeds the literal) is masked in
`build_event`. Children are written before the parent, so the parent
write sees every redaction. Log lines are not stored (`result_meta` keeps
no content), and `run_failure` already strips literals
(`analytics/script_fingerprint.rs`).

**How the host learns the input type.** Decided at `action_finished`,
after the action, because both engines leave focus on the target:
core's `fill` clicks then inserts (`input/fill.rs:28-109`), Playwright's
`fill` focuses first.

1. `TargetSensitivity::Secret` → mask. `Plain` → keep (the engine looked).
2. `Unknown`, and the args carry a snapshot ref (bridge `input.fill(page, ref, value)`):
   `browser.observe(page).resolve_ref(&Ref(ref))` → `backend_node_id`, then
   `DOM.describeNode` on that session; read `type`/`autocomplete` from
   `attributes`. Exact node, no focus assumption.
3. `Unknown`, no ref (Playwright locator, `input.type`, `keyboard.type`):
   probe the focused element through the host's own page session:
   `browser.cdp_json_for_page(page, "Runtime.evaluate", r#"{"expression":"(()=>{const e=document.activeElement;return e?{type:String(e.type||'').toLowerCase(),ac:String(e.autocomplete||'').toLowerCase()}:null})()","returnByValue":true}"#)`.
4. Probe error, no page, or `null` → **mask** (fail closed for typed values).

**Engine obligations.** Set `typed_value` on every action that types text:
in-process rustwright adapter `page.fill`/`locator.fill`/`locator.type`/
`keyboard.type` → `Some(TypedValue { arg_index, target: Unknown })`, or
`Secret` when the script passed `{ secret: true }`; a sidecar adapter may
pre-check with `locator.evaluate(el => el.type)` and send `Plain`/`Secret`
to skip the host probe. For the bridge names (optional piece 6) the host
keeps a built-in table: `input.fill` → 2, `input.type` → 1, `cdp` with
`Input.insertText` → `params.text`.

Out of scope tonight, noted: secrets embedded in `evaluate` code strings;
values typed key-by-key with `press`.

---

## 5. FakeEngine and the seam test

`api/mcp/test_support.rs` (cfg(test)) gains:

```rust
pub enum FakeStep {
    NewPage { target: &'static str },
    Action { method: &'static str, page: PageRef, args: Value, typed_value: Option<TypedValue>, ok: bool },
    ListPages,
}
pub struct FakeEngine { pub steps: Vec<FakeStep> }
impl ScriptEngine for FakeEngine {
    fn name(&self) -> &'static str { "fake" }
    fn dialect(&self) -> HelperDialect { HelperDialect::Playwright }
    fn run<'a>(&'a self, _req: ScriptRequest, ctx: &'a ToolCtx) -> BoxFuture<'a, Result<ScriptOutcome, EngineError>> {
        Box::pin(async move {
            let script = ctx.script.as_ref().ok_or_else(|| EngineError::Unavailable("no host".into()))?;
            let mut created = Vec::new();
            for step in &self.steps {
                script.control.cancel.is_cancelled().then(|| ()).map_or(Ok(()), |()| Err(EngineError::Cancelled))?;
                match step {
                    FakeStep::NewPage { target } => created.push(script.host.page_created(PageRef::Target(TargetId::from((*target).to_string()))).await),
                    FakeStep::Action { method, page, args, typed_value, ok } => {
                        let token = script.host.action_started(ActionStart { method, page: Some(page.clone()), args, from_helper: false, typed_value: *typed_value }).await;
                        let value = json!(null);
                        script.host.action_finished(token, ActionEnd { outcome: if *ok { Ok(&value) } else { Err("boom") }, output_token_estimate: 0 }).await;
                    }
                    FakeStep::ListPages => { let _ = script.host.annotate_pages(&[json!({"pageId": 1})]).await; }
                }
            }
            Ok(ScriptOutcome { ok: true, value: Some(json!({ "created": created })), ..Default::default() })
        })
    }
}
```

`test_support::tool_call` switches its catalog to
`crate::api::mcp::script_tool::host_catalog()` (one line) so
`tool_call("playwright", …)` resolves. The browser fake is the
`GroupDispatchRecorder` pattern from `effects/tab_groups.rs` tests: a
`CdpConnection` whose `Browser.getTabs` answers tab 11/`target-a` and,
once a flag is set by the test, also tab 12/`target-b`; it records
`Browser.*` group calls.

`api/mcp/script_dispatch/tests.rs`, `seam_end_to_end` (marked `#[ignore]`
in piece 1, enabled in piece 5):

1. Build the call, attach the fake browser, register `FakeEngine` with
   steps: `NewPage { target: "target-b" }`, `Action { method: "locator.fill", page: Target("target-b"), args: ["#pw", "hunter2"], typed_value: Some(TypedValue { arg_index: 1, target: Secret }), ok: true }`,
   `Action { method: "page.goto", page: Page(PageId(1)), args: ["https://user.test"], typed_value: None, ok: true }`,
   `Action { method: "locator.click", page: Target("target-b"), args: ["#go"], typed_value: None, ok: false }`, `ListPages`.
2. `dispatch_tool_call(call.clone()).await?`, then `audit_worker.flush_session`.
3. Assert audit: one parent row `playwright` with `created_at == started_at_ms`
   and `args_json` not containing `hunter2`; three child rows with
   `parent_dispatch_id == parent`, `page_id` set, `locator.fill` args
   containing `"[redacted]"` and `result_meta` containing `"redacted"`,
   `locator.click` with `isError:true`.
4. Assert claims: `sessions.owner_of_page(PageId(2)) == mine`;
   `session_tabs.drain_writes()` then `first_session_tab()` is tab 12 /
   `target-b`; the recorder saw a `Browser.*` group create containing page 2.
5. Assert notice: result text contains `page 1 belongs to the user`.
6. Assert idempotency: a second `page_created(Target("target-b"))` through
   the same dispatch adds no ledger row and no group call.
7. Cancellation: a second test cancels `call.cancel` before dispatch and
   asserts the engine returned `Cancelled` and no child rows were written.

---

## 6. Host-side pieces (dependency-ordered)

Verification for every piece: `cargo build -p claw-server-rust`,
`cargo test -p claw-server-rust --locked <filter>`,
`cargo clippy --package claw-server-rust --all-targets --locked -- -D warnings`,
`cargo fmt --all --check`. Piece 1 also runs `cargo test -p browseros-mcp --locked`
and `cargo test -p claw-server-rust --locked dependency_boundaries`.

| # | Piece | Owns (disjoint after piece 1) | Size |
|---|---|---|---|
| 1 | Compiling skeleton: `script.rs` + `lib.rs` re-export + `ToolCtx.script` in browseros-mcp; `script_dispatch/{mod,pages,actions,redact}.rs` with default-returning bodies (no `todo!()`); `engines.rs` with `UnavailableEngine`; `mod.rs` module lines; `dispatch.rs` (`SCRIPT_HOST_TOOLS`, `ToolCall.redactions`, the branch); `app.rs` field; `test_support.rs` FakeEngine + `tests.rs` scaffold `#[ignore]` | those files | M |
| 2 | Page identity + claims: `resolve`, `resolve_fast`, `created`, `claimed_tabs`; unit tests with the two-tab fake | `script_dispatch/pages.rs` | M |
| 3 | Action accounting: `started` (foreign-owner note), `finished` (liveness, activity, child row, screenshot); unit tests mirroring `script_hook.rs` tests | `script_dispatch/actions.rs` | M |
| 4 | Redaction: `mask_typed_value`, `redact_literals`, probe + describeNode paths, fail-closed; parent-row masking in `observers/audit.rs` (two lines); tests for each rule in §4 | `script_dispatch/redact.rs`, `observers/audit.rs` | S/M |
| 5 | Host tool + wiring: `script_tool.rs` (definition, handler, `host_catalog`), `service.rs` catalog line, unignore `seam_end_to_end` + cancellation test | `script_tool.rs`, `service.rs`, `script_dispatch/tests.rs` | M |
| 6 (optional) | `ScriptInnerCallHook` delegates to `ScriptDispatch` (`authorize`→`action_started`, `record`→`action_finished` with the bridge `typed_value` table, `on_page_created`→`page_created(Page)`, `annotate_pages`); helpers methods unchanged. Verification: existing `script_hook.rs` tests and `cargo test -p browseros-mcp --locked` unchanged, plus `run` conformance smoke | `script_hook.rs` | M |

Pieces 2, 3, 4 can run concurrently after 1; 5 after 2–4; 6 after 5 and
only on the owner's say-so.

---

## 7. Decisions and assumptions

- DECIDED: port types in a new `browseros-mcp/src/script.rs`, not
  `framework.rs` (§1a). `ToolCtx.script` is set after construction so
  `BrowserToolOptions` and its five constructor sites stay untouched.
- DECIDED: child rows use `audit_log.record_tool_dispatch` + inline
  `persist_screenshot`, the same path as `ScriptInnerCallHook`, for
  screenshot parity; the audit-worker route is a follow-up.
- DECIDED: the tool name placeholder is `playwright`; the mediator
  decides the final name in one const.
- DECIDED: `ScriptOutcome::into_tool_result` copies `run.rs`'s private
  formatting rather than changing `run.rs` visibility; a follow-up makes
  `run.rs` use it.
- DECIDED: redaction is fail-closed for typed values when the host cannot
  determine the input type.
- DECIDED: `evaluate` stays on `ARBITRARY_SCRIPT_TOOLS`; nothing here
  changes its path.
- ASSUMPTION: `Browser.getTabs` lists a target created by another CDP
  client within the 10 × 100 ms poll (rustwright spike measured native
  adoption of a raw-created target in ~70 ms).
- ASSUMPTION: Chrome does not reuse `tabId` within a browser run
  (`tab_registry.rs:46-47`), so `claimed_tabs` keyed on `i64` is safe.
- ASSUMPTION: `document.activeElement` is the filled input at
  `action_finished` for both engines; the ref path (rule 2) does not
  depend on this.
- ASSUMPTION: `dependency_boundaries.rs` accepts `api/mcp/script_dispatch/*.rs`
  (classified `ApiMcp` by the first two path parts) and `app.rs` importing
  `crate::api::mcp::engines` (Composition may import anything).
