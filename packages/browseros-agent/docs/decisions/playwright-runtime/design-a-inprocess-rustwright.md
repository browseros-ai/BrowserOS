# Design A: in-process Playwright facade over a Rust engine

Lens: minimise interface and footprint. Evidence: `packages/browseros-agent` at
`8d9a40820`, `/Users/shadowfax/code/oss/rustwright` at `dfb481b` (0.3.0, MIT),
`rquickjs-sys 0.12.1`. Line numbers are from those revisions.

## 1. Recommendation

1. Add one MCP tool, `playwright`, beside `run`: same `{code, timeout}` schema, same QuickJS runtime, same 30 s cap, same `InnerCallHook` audit path.
2. The script sees a Playwright-shaped facade (`browser`, `context`, `page`, locators, `expect`) written in JS; every leaf call funnels through the existing `__browserosCall` chokepoint.
3. Page lifecycle stays in `browseros-core` (`Browser.createTab background:true`, `PageId`, tab groups, ownership); element semantics come from `rustwright-core` linked into the binary behind one `PwEngine` trait.
4. rustwright is a pinned fork with `default-features = false`, about 20 `pub fn` twins of its PyO3 methods plus per-target adoption, and it never attaches to a tab the script did not address.
5. No new process, no Node or Bun, two extra threads, one extra CDP WebSocket, roughly 3 MB more per binary (ASSUMPTION, see §7).

**Rationale.** Everything `run` guarantees today comes from one chokepoint: `BrowserBridge::call` (`crates/browseros-mcp/src/tools/run.rs:600`) calls `hook.authorize`, dispatches, fires `on_page_created`, and records a child audit row; the host's `ScriptInnerCallHook` (`apps/claw-server-rust/src/api/mcp/script_hook.rs`) turns that into ownership notices, page claims, tab grouping, session-tab windows (which gate the replay recorder), per-step screenshots and token accounting. A Playwright surface is a different dialect over the same chokepoint, not a different runtime. The hard part is Playwright's element semantics: locator resolution (role and accessible name, strictness, `filter`/`nth`/chaining, frames), actionability waits, auto-retrying `expect`, trusted input. `rustwright-core` implements exactly that (`page_pointer_actionable_async`, `page_fill_actionable_async`, `assert_locator_sync`, `evaluate_locator_action_for_page` at `src/lib.rs:35959`, the spec grammar consumed by `locator_script` at `:63817`). `browseros-core` has none of it: `click_node` (`input/mod.rs:163`) scrolls, centres, checks cover and dispatches, with no wait-until-actionable loop, and `wait.rs` is a raw `document.querySelector`. What rustwright must not own is page identity and focus: its `new_page` sends `Target.createTarget` (`lib.rs:30062`) and its `pages()` attaches to every tab (`list_pages_raw`, `lib.rs:57647`), so creation and listing stay in `browseros-core` and rustwright adopts a target by id.

## 2. Agent-facing contract

**Tool** `playwright`, registered in `browseros_mcp::tools::catalog()` after `run`. `run` is untouched.

```json
{ "type": "object", "required": ["code"], "additionalProperties": false,
  "properties": {
    "code":    { "type": "string", "description": "Async JS body using the Playwright API (no imports). Use top-level await; return a value." },
    "timeout": { "type": "number", "description": "Max run time in ms. Default and hard cap 30000; larger values are clamped." } } }
```

Output schema is `RunOutput` from `run.rs:231` reused: `{ ok, value?, logs, error? }`.

**Globals** the script sees, and nothing else: `browser`, `context`, `page`, `expect`, `console`, `setTimeout`/`clearTimeout`/`sleep`. No `require`, `fetch`, `process`, `document`, `helpers`.

**Mapping.**
- `context` ⇔ this agent session (tab group + ownership key). `context.newPage(url?)` → `pages.new_page` with `background: Some(true)` → `on_page_created` claims and groups it. `context.pages()` returns every open tab as `Page` objects with non-standard `page.ownership` (`'mine' | 'user' | 'other-agent'`), `page.ownerLabel`, `page.id` (= `PageId`): more than Playwright's "pages of this context", labelled, never refused.
- `browser.contexts()` → `[context]`; `browser.newContext()` returns `context` and logs a warning (one shared profile, no isolation); `browser.close()` logs and does nothing.
- `page` global: the session's most recently active `'mine'` tab, created lazily if none. Stable across calls while that tab lives.
- Identity across calls: carry `page.url()`; re-derive with `context.pages()`; `context.page(id)` returns a known page or throws.

**Result envelope** is `run`'s: text `ok` / `return:` / `logs:` plus structured `{ ok, value, logs }`; failures `{ ok: false, error, logs }`. `error` keeps Playwright's error name and call log (`TimeoutError: locator.click: Timeout 3000ms exceeded. Call log: waiting for getByRole('button', { name: 'Submit' })`). No `helpersAvailable` on this tool (DECIDED, §7).

**Timeouts.** One call is capped at 30 s (`MAX_TIMEOUT_MS`), same reason as `run`: MCP client timeouts and cancellation. Inside the call, Playwright defaults are shrunk so a wrong locator fails fast with a Playwright error instead of eating the budget: action and `expect` 5 s, navigation 15 s, every per-op timeout clamped to `deadline - now`. `context.setDefaultTimeout(ms)` and per-call `{ timeout }` are honoured up to the remaining budget. Longer work is chunked per call, as today; a job model is a REVISIT.

**`api/mcp/prompt.rs`**, replace the paragraph starting "Reach for run first" with:

> Reach for playwright first; run and the granular tools are the fallback. playwright executes standard Playwright JavaScript in the server runtime: `context`, `page`, locators (`page.getByRole`, `getByText`, `getByLabel`, `locator(css)`), `expect`. Write it exactly as you would a Playwright script, minus imports. `context.newPage(url)` opens your own tab; `context.pages()` lists every tab with `page.ownership` saying whose it is. It composes the whole loop (navigate, act, wait, assert, extract) in one call. run is the older bespoke `browser` SDK; use it only when a saved helper or `browser.cdp` is needed. Use a single granular tool (tabs, navigate, snapshot, act, evaluate, read, grep) directly only for a one-off step, step-by-step debugging, or when a script genuinely cannot express it.

Replace the line "run first, granular tools as the fallback. Compose anything multi-step inside one run script" with "playwright first, run second, granular tools as the fallback. Compose anything multi-step inside one playwright script". Keep the helpers paragraph; prefix it with "Helpers are a run feature."

**`skills/browseros-neo/SKILL.md`**: in "Connecting", add `playwright` to the tool-name list. Rewrite "## Tool choice" first sentence to "Reach for `playwright` first; `run` is for saved helpers and CDP escape hatches; the granular tools are the fallback." Add after it:

> ## Writing a `playwright` script
> - Standard Playwright JS, no imports: `context`, `page`, `expect` are globals.
> - `context.newPage(url)` opens your own background tab. `context.pages()` lists every tab; `page.ownership` tells you whose it is. Leave other people's tabs as you found them.
> - Actions auto-wait up to 5 s, navigations 15 s, the whole call 30 s. A `TimeoutError` names the locator; fix the locator, do not raise the timeout.
> - Not available: `page.route`, `request`, cookies/storageState, `on('request'|'response'|'popup')`, tracing, video, viewport changes. A call to one throws `Unsupported:` with the alternative. After a click that opens a tab, read `context.pages()`.
> - `page.evaluate(fn, arg)` works with a real function; `arg` and the result must be JSON.

Rename the existing "## Writing a `run` script" heading to "## Writing a `run` script (legacy SDK)".

**Worked examples.**

```js
// 1. navigate, act, assert, extract
const page = await context.newPage('https://news.ycombinator.com');
await page.getByRole('link', { name: 'new', exact: true }).click();
await expect(page).toHaveURL(/newest/);
const titles = await page.locator('.titleline > a').allInnerTexts();
return titles.slice(0, 10);
```

```js
// 2. reuse a tab across calls by URL, fill a form, verify
const page = context.pages().find(p => p.url().startsWith('https://example.com/settings'))
  ?? await context.newPage('https://example.com/settings');
await page.getByLabel('Display name').fill('Nikhil');
await page.getByRole('button', { name: 'Save' }).click();
await expect(page.getByText('Saved')).toBeVisible();
return { url: page.url(), id: page.id, ownership: page.ownership };
```

```js
// 3. fails: no such button
const page = await context.newPage('https://example.com');
await page.getByRole('button', { name: 'Submit' }).click({ timeout: 3000 });
return 'never';
```

Result of 3 (text block, `isError: true`):

```
error: TimeoutError: locator.click: Timeout 3000ms exceeded.
Call log:
  - waiting for getByRole('button', { name: 'Submit' })
logs:
```

Audit shows the parent `playwright` row and two child rows: `pages.newPage` (ok, with screenshot) and `locator.click` (error, 3001 ms, args `[1, {"kind":"role","role":"button","name":"Submit","exact":false}, {"timeout":3000}]`).

## 3. Architecture

No new process. New thread boundary: rustwright's owned two-worker Tokio runtime (`connect_browser_over_cdp_cancelable`, `lib.rs:45636`) reached through `tokio::task::spawn_blocking`. New wire: one extra CDP WebSocket from the same binary.

```
QuickJS (pw/bootstrap.js)  --__browserosCall("pw", [pageId, op])-->  PwBridge (browseros-mcp)
   |                                                                      |  authorize / record / on_page_created  -> InnerCallHook (unchanged)
   |  pages.* / tool:screenshot|pdf|read|grep|download|upload  -----------+--> browseros-core / existing ToolDefs
   |                                                                      |  PwRequest{target_id, op, timeout, cancel}
   +----------------------------------------------------------------------+--> dyn PwEngine  --spawn_blocking-->  RustwrightEngine --ws--> BrowserOS neo
```

**Modules.**

1. `crates/browseros-mcp/src/tools/playwright.rs` (new). `definition()`, `PlaywrightArgs`, `DESCRIPTION` (the fidelity list from below, verbatim), handler → `script_runtime::execute_script`. Deletion test: it is the tool; nothing else exposes the facade.

2. `crates/browseros-mcp/src/script_runtime.rs` (new, extracted from `run.rs:272-597`, `1131-1283`). Owns the QuickJS lifecycle, interrupt handler, `RunControl`, log capture, `json_to_js`, outcome shaping.

```rust
pub(crate) struct ScriptSpec { pub bootstrap_js: &'static str, pub entry_globals: &'static [&'static str], pub code: String, pub timeout_ms: u64 }
pub(crate) trait ScriptBridge: Send + Sync + 'static {
    fn call<'a>(&'a self, method: &'a str, args_json: &'a str, from_helper: bool) -> BoxFuture<'a, Result<BrowserCallValue, String>>;
}
pub(crate) async fn execute_script(spec: ScriptSpec, bridge: Arc<dyn ScriptBridge>, ctx: &ToolCtx, helpers: Vec<HelperSource>) -> Result<RunOutcome, RunError>;
```

   Two adapters of `ScriptBridge`: `run.rs::BrowserBridge` and `pw::PwBridge`, so this is a real seam. Deletion test: delete it and ~350 lines of runtime code exist twice. In the 4-hour path it is a copy, not an extraction (§6).

3. `crates/browseros-mcp/src/pw/bootstrap.js` (new, `include_str!`). The facade. Ports the spec builders from `python/rustwright/sync_api.py` (`_selector_spec:2883`, `_role_selector_spec:2697`, `_text_selector_spec:2436`): `getByRole` → `{"kind":"role","role","name","exact"}`, `getByText` → `{"kind":"text_selector","text","exact"}`, `locator(css)` → `{"kind":"css","selector"}`, `nth/first/last` → `{"kind":"nth","base","index"}`, chaining → `descendant`, `filter` → `filtered`, `frameLocator` → `frame`, plus the `xpath=`/`text=`/`nth=`/`id=` prefixes. A `Locator` is a value (page id + spec); each action is one bridge call. `expect(locator)` builds matcher JSON in the `_native_matcher_base` shape (`sync_api.py:29701`) and sends `Assert`; `expect(page)` polls `Url`/`Title`; `expect(value)` runs in QuickJS. `page.evaluate(fn, arg)` sends `fn.toString()`: QuickJS keeps function source at parse (`quickjs.c:36939`) and `js_function_toString` returns it (`:41794`). `page.url()` is synchronous, so page-op responses are `{ value, url }` and the facade refreshes its cached URL after every call. `page.on('dialog', h)` stores `h`; on `PwErrorKind::Dialog` the facade runs `h`, sends `DialogHandle`, retries once.

4. `crates/browseros-mcp/src/pw/engine.rs` (new). The seam.

```rust
pub trait PwEngine: Send + Sync {
    /// One Playwright-semantic operation against a page target. `target_id` is the CDP
    /// target the host resolved from a PageId; the engine adopts it on first use.
    fn call<'a>(&'a self, req: PwRequest<'a>) -> BoxFuture<'a, Result<Value, PwError>>;
    /// The host saw the page detach; drop any per-target state.
    fn forget_target<'a>(&'a self, target_id: &'a str) -> BoxFuture<'a, ()>;
}
pub struct PwRequest<'a> { pub target_id: &'a str, pub op: PwOp, pub timeout: Duration, pub cancel: CancellationToken }

#[derive(Deserialize)] #[serde(tag = "op", rename_all = "camelCase")]
pub enum PwOp {
    Goto { url: String, wait_until: Option<String> }, GoBack, GoForward, Reload,
    WaitForLoadState { state: String }, WaitForUrl { pattern: String },
    Title, Url, Content, Evaluate { expression: String, arg: Option<Value> },
    WaitForFunction { expression: String, arg: Option<Value>, polling_ms: u64 },
    LocatorAction { spec: Value, index: Option<usize>, action: LocatorAction },
    LocatorQuery  { spec: Value, index: Option<usize>, query: LocatorQuery },
    Assert { spec: Value, index: Option<usize>, matcher: Value, polling_ms: u64 },
    KeyboardPress { key: String }, KeyboardType { text: String, delay_ms: Option<u64> },
    MouseClick { x: f64, y: f64, button: String, click_count: u32 }, MouseMove { x: f64, y: f64 }, MouseWheel { dx: f64, dy: f64 },
    DialogHandle { accept: bool, prompt_text: Option<String> },
    SetInputFiles { spec: Value, index: Option<usize>, paths: Vec<String> },
}
#[derive(Deserialize)] #[serde(tag = "kind", rename_all = "camelCase")]
pub enum LocatorAction { Click { button: String, click_count: u32, modifiers: Vec<String>, force: bool }, Dblclick, Hover,
    Fill { value: String, force: bool }, Type { text: String, delay_ms: Option<u64> }, Press { key: String },
    Check { checked: bool }, SelectOption { values: Vec<String>, by_label: bool }, Focus, Blur, Clear, ScrollIntoView, Tap, DragTo { target: Value } }
#[derive(Deserialize)] #[serde(tag = "kind", rename_all = "camelCase")]
pub enum LocatorQuery { Count, TextContent, InnerText, InnerHtml, InputValue, GetAttribute { name: String },
    IsVisible, IsHidden, IsEnabled, IsChecked, IsEditable, BoundingBox, AllInnerTexts, AllTextContents,
    WaitFor { state: String }, Evaluate { expression: String, arg: Option<Value> }, EvaluateAll { expression: String, arg: Option<Value> }, AriaSnapshot }
pub struct PwError { pub kind: PwErrorKind, pub message: String }
pub enum PwErrorKind { Timeout, Strict, TargetClosed, NotFound, Dialog, Unsupported, Engine }
```

   Adapters: `RustwrightEngine` (production) and `FakePwEngine` (tests in `pw/bridge.rs`, records the `PwOp` stream and returns canned values). Two adapters, and the interface is the facade's test surface. Deletion test: delete the trait and the facade tests need a live browser; earns its keep.

5. `crates/browseros-mcp/src/pw/bridge.rs` (new). `PwBridge { ctx: ToolCtx, control: RunControl }` implements `ScriptBridge`. `pages.list|newPage|close|getInfo` reuse the bodies from `run.rs:645-696` (moved to `pw/pages.rs`, shared by both bridges); `tool:screenshot|pdf|read|grep|download|upload` reuse `run.rs::run_tool`; `pw` deserialises `[pageId, PwOp]`, resolves `target_id` via `ctx.session.pages.get_info`, calls `ctx.pw_engine`. Hook protocol identical to `BrowserBridge::call:600-641`. Audit `method` is the Playwright call (`page.goto`, `locator.click`, `expect.toBeVisible`) so the cockpit needs no translation. Deletion test: it could be a `"pw"` arm inside `BrowserBridge` (the 4-hour path does that); split out because it needs `pw_engine` and page→target resolution `run` never needs, and `run`'s 30 bridge tests should not churn.

6. `crates/browseros-pw-rustwright/src/lib.rs` (new crate; depends on `browseros-mcp` for the trait and on the fork).

```rust
pub struct RustwrightEngine { epoch: u64, browser: RustwrightBrowser, pages: Mutex<HashMap<String, RustwrightPage>>, gate: tokio::sync::Semaphore /* 8 */ }
impl RustwrightEngine {
    /// Connects a second CDP client to the browser the host is already attached to.
    /// Runs the connect on spawn_blocking: rustwright owns its own Tokio runtime.
    pub async fn connect(ws_url: &str, epoch: u64) -> Result<Arc<Self>, PwError>;
    pub fn epoch(&self) -> u64;
    pub fn is_connected(&self) -> bool;
}
impl PwEngine for RustwrightEngine { /* call: adopt target lazily via page_for_target; CancelToken watcher; spawn_blocking; RwError -> PwError */ }
```

   `call` creates a `rustwright_core::CancelToken`, spawns a watcher (`select! { req.cancel.cancelled(), sleep(req.timeout) } → token.cancel()`), then `spawn_blocking(move || page.<method>_with_cancel(.., Some(timeout_ms), Some(&token)))`. `RwError::ActionTimeoutError` → `Timeout`; `TargetClosed*` → `TargetClosed` and evict. Deletion test: without the crate, `playwright` compiles, its tests pass on the fake, production answers `Engine("playwright engine unavailable")`; it earns its keep by isolating a 64k-line alpha dependency behind ~300 lines.

7. rustwright fork: `browseros-ai/rustwright`, branch `browseros`, consumed as `rustwright-core = { git, rev, default-features = false }`. Additive changes in `src/lib.rs` only: (a) `pub fn RustwrightBrowser::page_for_target(&self, target_id: &str, timeout: Duration) -> RwResult<RustwrightPage>` wrapping the private `attach_existing_page` (`:49702`), so the engine never runs `list_pages_raw`; (b) `pub fn` twins on `RustwrightPage` of the `#[cfg(feature = "python")]` `PyPage` methods the shim uses (each is already `browser.block_on(cancelable(cancel, <private async fn>))`, `:42380`, `:42571`, so a twin is a copy minus `py.detach`):

```
locator_fast_path locator_probe_state locator_fill_apply locator_fill_actionable locator_select_apply
locator_check_apply locator_eval locator_eval_handle assert_locator dispatch_locator_pointer_action count
wait_for_selector evaluate_with_call_arguments keyboard_press_native keyboard_type_native
dispatch_mouse_click_sequence dispatch_mouse_events handle_dialog set_file_input_files content pdf
already pub: click_locator_json_with_cancel(:47255) goto title text_content inner_text is_visible screenshot close
```

8. Host wiring (`apps/claw-server-rust`).
   - `src/api/mcp/dispatch.rs`: `ARBITRARY_SCRIPT_TOOLS = &["run", "evaluate", "playwright"]`; `execute_with_cancellation` passes `pw_engine: call.state.pw_engines.for_browser(browser_session).await` in `BrowserToolOptions`. `framework.rs` gains `pub pw_engine: Option<Arc<dyn PwEngine>>` on `BrowserToolOptions` and `ToolCtx`, `None` outside the host, the same pattern as `inner_call_hook`.
   - `src/services/browser/pw_engines.rs` (new): `PwEngineCache { cdp_port: u16, current: RwLock<Option<Arc<RustwrightEngine>>> }` with `pub async fn for_browser(&self, session: &BrowserSession) -> Option<Arc<dyn PwEngine>>`. Keyed by `CdpConnection::connection_epoch()`; on a new epoch it connects again with `browseros_cdp::discovery::discover_websocket_url` and drops the old engine (rustwright's `OwnedRuntime::drop` moves the drop to a thread when inside Tokio, `lib.rs:29156`). This is how the `BrowserService` reconnect loop (`services/browser/connection.rs:145`) propagates.
   - `src/api/mcp/helper_runtime.rs`: `preload_helpers` and `discovery` gate on `call.tool().name == "run"`.
   - `src/api/mcp/observers/run_failure.rs`: `REPORTED_TOOLS: &[&str] = &["run", "playwright"]`.
   - `src/analytics/script_fingerprint.rs`: `SDK_SURFACE` gains every name in the COVERED list above plus `page context expect newPage pages keyboard mouse`.
   - `src/api/mcp/prompt.rs`, `skills/browseros-neo/SKILL.md`: §2 text.
   - `apps/claw-app/components/audit/Timeline.tsx:39`: `NOTABLE_TOOLS` gains `'playwright'`. Nesting child rows under `parentDispatchId` is a follow-up; rows already carry it.

**How each guarantee gets its data** (all existing code, unchanged unless noted):
- Audit rows: `PwBridge::call` → `hook.record` → `ScriptInnerCallHook::record` → `audit_log.record_tool_dispatch` with `parent_dispatch_id` → `persist_screenshot`.
- Claim + tab group: `context.newPage` → `pages.newPage` arm → `hook.on_page_created` → `ownership_claims::record_new_page` + `tab_groups::run_tab_group_work`.
- Ownership notice: `hook.authorize(page)` before every op fills `ToolCall.foreign_pages`; `page_ownership_notice::run_script_notice` reports it.
- Screenshots: per child row via `visual.rs capture` (session's active owned tab). Replay: `recorder.content.ts` records tabs holding a session-tab claim, made by `record_new_page`.
- Run-failure telemetry: `run_failure.rs` for both tools; the allowlisted fingerprint keeps agent-chosen names out (changed: §3 item 8).
- Cancellation: `ctx.cancel` drives `RunControl.race`, the QuickJS interrupt handler, and `PwRequest.cancel` → rustwright `CancelToken.cancel()`, so a blocked click returns instead of stranding a `spawn_blocking` thread.
- Focus: creation goes through `Browser.createTab background:true`; rustwright's per-page attach enables focus emulation so a background tab behaves focused for the script (ASSUMPTION, §7).

**Seams reused:** `InnerCallHook` (no change), the `ToolDef` handler, the `BrowserToolOptions` injection pattern. **New seams:** `ScriptBridge`, `PwEngine`. No new effect or observer.

**Fidelity.** The tool description carries these three lists verbatim, so the model never guesses.

```
COVERED  page: goto goBack goForward reload waitForLoadState waitForURL waitForFunction title url content evaluate
              screenshot pdf close keyboard.press/type mouse.click/move/wheel on('dialog')
         locators: locator getByRole getByText getByLabel getByPlaceholder getByTestId getByAltText getByTitle
              first last nth filter and or chaining frameLocator
         actions: click dblclick hover fill type press check uncheck setChecked selectOption focus blur clear
              scrollIntoViewIfNeeded setInputFiles dragTo tap
         queries: count textContent innerText innerHTML inputValue getAttribute isVisible isHidden isEnabled
              isChecked isEditable boundingBox allInnerTexts allTextContents waitFor evaluate evaluateAll ariaSnapshot
         expect(locator): toBeVisible/Hidden/Attached/Enabled/Disabled/Checked/Editable/Empty/Focused/InViewport
              toHaveText/ContainText/HaveValue/HaveValues/HaveAttribute/HaveClass/ContainClass/HaveCount/HaveId/HaveCSS
              toHaveRole/HaveAccessibleName, .not; expect(page): toHaveURL toHaveTitle
DIFFERS  context.pages() includes non-owned tabs (labelled); newContext() = session context, no isolation;
         screenshot() -> { path, base64 } not Buffer; page.url() = value after the last bridged call;
         evaluate args/results JSON only (no handles); action default timeout 5 s (Playwright 30 s)
THROWS   Unsupported: page.route/unroute/request, context.cookies/addCookies/storageState,
         page.on('request'|'response'|'console'|'popup'), waitForEvent('popup') (read context.pages() after the click),
         tracing, video, setViewportSize, addInitScript, exposeFunction, ElementHandle ($/$$ map to locators), page.pause
```

Coverage of what agents actually write: ASSUMPTION, above 90 % by call count (§7).

**B and C.** B (sidecar `playwright-core` in Node or Bun): full fidelity, but it ships a runtime (tens of MB per target, five targets, macOS notarisation and Windows signing of a second executable), a supervised child process, an IPC protocol, and `node:vm` is not a sandbox for untrusted agent code; `on_page_created` becomes an inference from `Target` events, so claims race the script's next action. Switch to B if the rustwright spike shows the locator engine diverging on more than two of the ten conformance scripts, if the fork surface passes about 1,000 lines, or if the owner accepts shipping a runtime. C1 (real Playwright, host observes CDP events for audit) fails auditability parity: CDP shows `Input.dispatchMouseEvent`, not `getByRole('button', {name})`. C2 (rustwright's Python-shim semantics as a JS shim) is this design. C3 (Playwright over raw CDP inside QuickJS) makes every selector probe a bridge round trip and still has no actionability engine.

## 4. Work pieces

| # | Goal | Owned files | Interface | Verify | Size |
|---|---|---|---|---|---|
| P0 | **Skeleton**: every file, type and fn stub compiles; `playwright` listed; returns `Engine("playwright engine unavailable")` | `tools/playwright.rs`, `pw/{mod,engine,bridge,pages}.rs`, `pw/bootstrap.js` (globals only), `tools/mod.rs` catalog line, `framework.rs` `pw_engine` field, `crates/browseros-pw-rustwright/{Cargo.toml,src/lib.rs}`, workspace `Cargo.toml` `[workspace.dependencies]` + `members`, `services/browser/pw_engines.rs`, `AppState.pw_engines` in `lib.rs` | §3 signatures | `cargo build -p claw-server-rust`; `cargo test -p browseros-mcp playwright::lists_tool` | S |
| P1 | Extract `script_runtime.rs`; `run.rs` uses it | `script_runtime.rs`, `run.rs` | `ScriptSpec`, `ScriptBridge`, `execute_script` | `cargo test -p browseros-mcp run::` all green, no test edited | M |
| P2 | Facade JS: spec builders, locators, `expect`, `page`/`context` objects, dialog retry | `pw/bootstrap.js`, `pw/bridge.rs` tests (`FakePwEngine`) | emits `PwOp` JSON exactly as §3 | `cargo test -p browseros-mcp pw::` with 12 canonical scripts asserting the op stream | M |
| P3 | rustwright fork: `page_for_target` + `pub fn` twins | fork repo; workspace `rustwright-core` dependency line | §3 item 7 | `cargo test` in fork; `cargo check -p browseros-pw-rustwright --target <each of the five>` | M |
| P4 | `RustwrightEngine` + `PwEngineCache` + dispatch injection | `browseros-pw-rustwright/src/lib.rs`, `pw_engines.rs`, `dispatch.rs` (`execute_with_cancellation` block + `ARBITRARY_SCRIPT_TOOLS`) | `PwEngine` | isolated BrowserOS neo: example 1 returns ten titles; child audit rows nested; example 3 returns `TimeoutError` in ~3 s | M |
| P5 | Host gating and telemetry | `helper_runtime.rs`, `observers/run_failure.rs`, `analytics/script_fingerprint.rs` | none new | `cargo test -p claw-server-rust --locked`; clippy `-D warnings` | S |
| P6 | Prompt, skill, cockpit | `prompt.rs`, `skills/browseros-neo/SKILL.md`, `Timeline.tsx` | §2 text | `cargo test -p claw-server-rust prompt`; `bun run check` | S |
| P7 | Conformance: `cases-playwright.ts`, ten scripts | `contracts/claw-mcp/tests/cases-playwright.ts` + registration in `rust-conformance.test.ts` | tool contract §2 | `BROWSEROS_BINARY=... bun contracts/claw-mcp/tests/run.ts --smoke` | M |

Order: P0 → {P1, P2, P3, P6} in parallel → P4 (needs P3, P2) → P5 (needs P0) ∥ P7 (needs P2) → gates. P1 can be skipped for the morning; P2 then targets a copied runtime.

## 5. Top five ways this fails

1. **Spec grammar mismatch.** The facade emits JSON that `locator_script` rejects (key names for `descendant`, `filtered`, `label`, `placeholder` are only partly read tonight). Mitigation: P2 fixtures are generated by running `sync_api.py`'s `_selector_spec` on the same inputs and pinned as test data; day one falls back to `{"kind":"css"}` when a builder is unsure.
2. **The second CDP client touches the user's tabs.** `pages()` attaches to every target (`list_pages_raw`). Mitigation: the engine only ever calls `page_for_target`; connect performs only the service-worker stealth auto-attach with pages excluded (`lib.rs:54064`); the spike counts `Target.attachToTarget` on an isolated browser with ten user tabs and one script tab, expecting one.
3. **Blocking-call leak.** A rustwright call outlives the run and starves `spawn_blocking`. Mitigation: every op carries a `CancelToken` and `timeout = remaining`; the engine's `Semaphore(8)` bounds `Promise.all` fan-out; the watcher fires on cancel or deadline, whichever first.
4. **Browser restart mid-session.** Stale `RustwrightBrowser`, ops fail with `ConnectFailed`. Mitigation: the cache keys on `connection_epoch`; `is_connected()` is checked before each op; `TargetClosed` evicts the page; the script sees `TargetClosed: page 3 was closed` and re-derives.
5. **Build and size.** rustwright-core does not build on `x86_64-pc-windows-msvc` or the glibc-pinned Linux targets, or adds more than the budget. Mitigation: P3 runs `cargo check` per target before P4 starts; `scripts/build/claw-server-rust/compiler.ts` gains a size assertion (fail above +6 MB per binary); the fork drops `reqwest` and `tempfile` paths that only `launch_chromium` needs if size bites.

## 6. If I only had 4 hours

0:00–0:40 P0 with `playwright.rs` as a copy of `run.rs` minus helpers, `BOOTSTRAP_JS` swapped, a `"pw"` arm in its bridge. 0:40–1:40 fork: `page_for_target` and eight twins (`locator_fill_actionable`, `locator_probe_state`, `assert_locator`, `count`, `evaluate_with_call_arguments`, `keyboard_press_native`, `text_content`, `handle_dialog`; `click_locator_json_with_cancel`, `goto`, `title` exist). 1:40–2:40 `RustwrightEngine` for `Goto/Title/Url/Evaluate/LocatorAction{Click,Fill,Press}/LocatorQuery{Count,TextContent,IsVisible}/Assert{visible,text,url,title}` plus `PwEngineCache` and the `dispatch.rs` injection. 2:40–3:20 facade with `context.newPage/pages`, `page.goto/url/title/evaluate/getByRole/getByText/getByLabel/locator`, `click/fill/press/count/textContent`, `expect` four matchers, one paragraph in `prompt.rs`. 3:20–4:00 drive the isolated browser with the three §2 scripts, check nested audit rows in the cockpit, commit.

## 7. Decisions and assumptions

DECIDED: tool name `playwright`, args identical to `run`.
WHY: a model that knows Playwright recognises the name; identical args mean clients and telemetry treat both tools alike.
REVISIT: at mediation, if another design's name wins.

DECIDED: engine is rustwright-core behind `PwEngine`; `browseros-core` keeps page creation, listing, closing, screenshots, tab groups.
WHY: rustwright has the locator engine, actionability and assertions; `browseros-core` has the fork-aware `Browser.createTab background:true`, `PageId`, and every host hook.
REVISIT: if the spike shows rustwright's engine diverging on the conformance scripts, implement a `CorePwEngine` adapter over `browseros-core` for the css/role subset behind the same trait.

DECIDED: `context.pages()` returns every tab with `page.ownership`; `newContext()` is the session context.
WHY: ownership is a label, not a permission (commit 756b9572a); the product's `browser.pages.list()` already does this.
REVISIT: if agents keep acting on user tabs by accident, split into `browser.contexts()` per owner with `context.label`.

DECIDED: no helpers on `playwright`; `helpersAvailable` and hot-loading stay `run`-only.
WHY: saved helpers take `(browser, page)` from the old SDK and would throw under the facade.
REVISIT: Playwright-flavoured helpers once the facade is stable.

DECIDED: action default 5 s, navigation 15 s, per-op timeout clamped to the remaining run budget; 30 s call cap unchanged.
WHY: the cap is a client-timeout and cancellation guarantee; shrunk defaults make a wrong locator fail with a Playwright error inside the budget.
REVISIT: a `job` tool with `playwright_status` polling for long flows.

DECIDED: the engine is connected per browser epoch and cached in `AppState`, not per run.
WHY: connect opens a WebSocket, builds a two-worker runtime and arms the service-worker stealth auto-attach (`lib.rs:45626-45700`); per-run would add latency and churn threads.
REVISIT: never, unless memory of an idle engine matters.

DECIDED: `page.evaluate(fn)` serialises the function with `Function.prototype.toString`.
WHY: QuickJS keeps the source (`quickjs.c:36939`, `:41794`); it is what agents write.
REVISIT: never.

ASSUMPTION: rustwright-core with `default-features = false` adds about 3 MB to a release binary; `reqwest 0.12.28` and `tokio-tungstenite 0.29` are already in `Cargo.lock`, so few new crates. Not measured (no build tonight).
ASSUMPTION: the rustwright spec JSON keys for `descendant`, `filtered`, `label`, `placeholder`, `test_id`, `frame` are as `sync_api.py` emits them; only `role`, `text_selector`, `css`, `nth`, `xpath`, `attribute` were read.
ASSUMPTION: the `assert_locator` matcher JSON accepts the `_native_matcher_base` names (`visible`, `hidden`, `text`, `attribute`, `count`, ...); read only at `sync_api.py:29701`.
ASSUMPTION: rustwright's per-page attach sends `Emulation.setFocusEmulationEnabled`; seen only in its tests (`lib.rs:19080`), not in the attach body.
ASSUMPTION: `attach_existing_page` enables `Page`, `Runtime`, `Network` domains on the adopted target only; a second client's `Network.enable` on a page the user is also using has no visible effect.
ASSUMPTION: rustwright-core builds on all five targets in `scripts/build/claw-server-rust/compiler.ts`; it publishes wheels for them but our glibc pin was not checked.
ASSUMPTION: connect latency for `rustwright_connect_over_cdp` against a local browser is under 200 ms.
ASSUMPTION: agents' Playwright usage is over 90 % locators, actions, `expect`, `goto`, `evaluate`; the unsupported list is rarely hit.
ASSUMPTION: `Target.createTarget` without the fork's `background` flag steals focus; the design avoids it regardless.
