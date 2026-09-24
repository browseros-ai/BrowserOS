# Design C (open lens): Playwright JS in `run`, in-process, with Playwright's own engines vendored

Designer: pw-design-c. Written 2026-09-23. Evidence paths are relative to
`packages/browseros-agent/` unless absolute.

## 1. Recommendation

1. Keep QuickJS and the `run` tool. Add a Playwright-shaped JS facade (`browser`, `context`, `page`, `expect`, `neo`) as a second bootstrap in `crates/browseros-mcp/src/tools/run_playwright/facade.js`, selected by a new `api: "playwright"` arg.
2. Resolve locators with **Playwright's own injected selector engine**, vendored (`lib/generated/injectedScriptSource.js`, 309,776 B, Apache-2.0) and run in an isolated world through the existing `browseros-core` CDP session. Compose selector strings with Playwright's own `locatorUtils` rules. Fidelity is then Playwright's code, not our reimplementation.
3. Actionability, trusted input, frames, dialogs, downloads, navigation, reconnection: reuse `browseros-core` (`Input::click_backend_node`, `FrameRegistry`, `PageSignals`, `PageManager`).
4. Audit, ownership, tab groups, screenshots, replay, telemetry, cancellation: **zero host changes**; every facade leaf goes through the existing `__browserosCall` bridge, so `InnerCallHook` / `ScriptInnerCallHook` fire exactly as today.
5. No new runtime ships. Binary grows by ~0.4 MB of embedded JS. Sidecar (B) and rustwright-core (A1) lose; reasons recorded below.

**Rationale.** The win the owner wants is *the API the model already knows*. That API is two things: locator/expect **semantics**, which live in Playwright's injected script (a self-contained JS bundle), and the **object shape** (`page.getByRole(...).click()`), ~600 lines of client JS. Neither needs Node. What Playwright's server does per action (scroll, check states, hit-target, `Input.dispatchMouseEvent`, poll `to.*`) is a short Rust loop `browseros-core` mostly has. The two invariants that bite a sidecar cannot be fixed from outside the browser: real `connectOverCDP` auto-attaches to *every* tab with `waitForDebuggerOnStart: true` and runs `Emulation.setFocusEmulationEnabled`, `Runtime.addBinding`, `Page.addScriptToEvaluateOnNewDocument`, `Log.enable`, `Network.enable` on the user's own tabs (`/Users/shadowfax/code/oss/playwright/packages/playwright-core/src/server/chromium/crBrowser.ts:78`, `crPage.ts`), and `context.newPage()` is `Target.createTarget {url, browserContextId}` with no `background` (`crBrowser.ts:376`), which the fork's background default in `Browser.createTab` (`chromium_patches/.../browser_handler.cc:829`) does not cover. The in-process path creates pages through `Browser.createTab background:true` as `run` does today.

**Why the others lose.**
- **B, sidecar with real `playwright-core`**: +55 MB (Bun single binary) or +75 MB `libnode` + 13 MB `playwright-core` per platform, a second signed executable; the user-tab intrusion and focus theft above; `node:vm` is not a security boundary; a second CDP client whose `Page` identity must be reconciled with `PageId`; an in-flight `await` is cancelled only by killing the process. What B alone offers (`route`, `storageState`, tracing, video) conflicts with a shared signed-in profile anyway.
- **A1, link `rustwright-core`**: synchronous API owning its own Tokio runtime (`block_on`, `src/lib.rs:29236`) → `spawn_blocking` per call; its own websocket → second client, second page identity; its locator engine is a hand-written JS template in `lib.rs` (`locator_script_for_root`, `:62889`) whose fast path excludes `selected/expanded/pressed/level` (`simple_role_fast_spec`, `:1777`), the exact "silently differs" risk; alpha, sanitized errors (`ConnectFailed`). It duplicates `browseros-core`.
- **A2, our own selector engine**: C is A2 with the engine vendored instead of written.
- **All-JS Playwright over CDP inside QuickJS**: no event transport; audit sees raw CDP, not intent.
- **Hybrid (real Playwright + Rust observing CDP for audit)**: B's costs plus worse rows (events cannot recover "which locator").

## 2. Agent-facing contract

Tool name stays `run`. Args:

```json
{
  "type": "object",
  "properties": {
    "code":    { "type": "string",  "description": "Async body. Top-level await; `return` a value." },
    "timeout": { "type": "number",  "description": "Wall-clock cap in ms, clamped to 30000." },
    "api":     { "type": "string",  "enum": ["playwright", "browseros"],
                 "description": "playwright (default): standard Playwright JS against `browser`/`context`/`page`/`expect`. browseros: the legacy `browser.*` SDK." }
  },
  "required": ["code"],
  "additionalProperties": false
}
```

Dialect selection: `api` omitted → `playwright`, unless the source matches the legacy namespace regex `\bbrowser\.(pages|input|observe|nav|read|grep|wait|evaluate|screenshot|download|pdf|upload|tabGroups|windows|cdp|saveHelper|listHelpers|readHelper)\b` or `\bhelpers\[`, in which case it runs as `browseros` and logs `note: legacy browseros SDK detected; pass api:"browseros" explicitly`. Existing skills and saved helpers keep working with no edits.

Globals in `playwright` mode:

| Global | Meaning |
|---|---|
| `browser` | Playwright `Browser` facade: `contexts()` → `[context]`, `newContext()` → `context` + warning (one shared profile), `newPage()` = `context.newPage()`, `close()` no-op + warning, `version()`. |
| `context` | **The agent session** ⇔ Playwright `BrowserContext`. `pages()` = tabs this conversation owns (ownership `mine`), `newPage()` = background tab claimed + grouped at creation, `on('page')`, `setDefaultTimeout`, `waitForEvent('page')`. |
| `page` | Lazy: the most recently used own tab that is still open, else a new background tab on first use. So `await page.goto(url)` works with zero ceremony. |
| `expect` | Locator/page matchers (`to.*` via the injected engine, polled) plus a pure-JS value subset (`toBe`, `toEqual`, `toContain`, `toBeTruthy`, `toBeGreaterThan`, `toMatch`, `.not`). |
| `neo` | BrowserOS extras: `neo.pages({ownership:'all'\|'mine'\|'user'\|'other-agent'})` → Pages annotated `{ownership, ownerLabel}`, `neo.page(pageId)`, `neo.snapshot(page)`, `neo.read(page)`, `neo.grep(page, opts)`, `neo.download(page, opts)`, `neo.tabGroups`, `neo.windows`, `neo.cdp`. |
| `console`, `setTimeout`, `clearTimeout`, `sleep` | as today. `require('playwright')`/`chromium.launch()`/`chromium.connectOverCDP()` return the same `browser`; `test(name, fn)` runs `fn({page, context, browser})` inline. Node-isms fail loudly with a hint. |

Page identity: a `Page` object carries `page.pageId` (the server's `PageId(u32)`) and standard `page.url()`. Across `run` calls carry the URL; `context.pages()` re-derives own tabs; `neo.page(id)` re-attaches to any tab (ownership notice appended, never refused). `page.close()` closes; `page.bringToFront()` is a warning no-op (invariant 3).

Timeouts: the 30 s run cap stays (invariant 5, MCP clients). Defaults: action/navigation 10 s, `expect` 5 s, `waitForTimeout` clamped; every primitive is raced against the run deadline and cancel token (`RunControl::race`, `run.rs:280`). Long work = one bounded chunk per call, as today. DECIDED below on no job model.

Result envelope (unchanged `RunOutput` + one optional field):

```json
{ "ok": true,  "value": <json>, "logs": ["..."], "api": "playwright" }
{ "ok": false, "logs": ["..."], "error": "TimeoutError: locator.click: Timeout 3000ms exceeded.\nCall log:\n  - waiting for getByRole('button', { name: 'Submit' })\n  - 0 matches (strict) on https://example.com/ (page 7)", "api": "playwright" }
```

Errors keep Playwright's `name` (`TimeoutError`, `Error`) and message shape so the model's recovery habits transfer. `helpersAvailable` is appended only in `browseros` mode.

Unsupported (throw `Error: not available in BrowserOS neo: <api>. <hint>`): `route`/`unroute`, `addInitScript`, `setViewportSize`, `emulateMedia`, `cookies`/`addCookies`/`storageState` ("already signed in"), `tracing`, `video`, `request` (APIRequestContext), `context.close()`/`browser.close()` (no-op + warning).

**Prompt text changes** in `apps/claw-server-rust/src/api/mcp/prompt.rs` (`BROWSERCLAW_MCP_INSTRUCTIONS`), replace the second paragraph and the "run first" paragraph:

```
Reach for run first; the granular tools are the fallback. run executes standard
Playwright JavaScript in the server: you get `browser`, `context`, `page`,
`expect` and write exactly what you would in a Playwright script — `await
page.goto(url); await page.getByRole('button', { name: 'Submit' }).click();
await expect(page.getByText('Done')).toBeVisible();` — plus `neo` for BrowserOS
extras (neo.snapshot/read/grep/pages). `context` is your session: context.pages()
lists your own tabs, context.newPage() opens a background tab in your tab
group. No Node: no require, fetch, fs; no route/cookies/storageState (you are
already signed in). Keep one call under 30s; carry URLs, not page ids, between
calls. The legacy browser.* SDK still runs with api:"browseros".
```

and drop the "Reuse what already works … helpers" paragraph's SDK call forms (helpers stay `browseros`-mode only overnight). **`skills/browseros-neo/SKILL.md`**: replace the "Writing a `run` script" section with:

```
## Writing a `run` script (Playwright)
- Standard Playwright: `page`, `context`, `browser`, `expect` are in scope; write
  it like a Playwright test body with top-level await and `return` a value.
- `context` is your session. `context.pages()` are your tabs; `context.newPage()`
  opens a background tab in your group. Other tabs: `neo.pages({ ownership: 'all' })`.
- The sandbox is neither Node nor the page: no require/fetch/fs; page code runs
  via `page.evaluate`.
- No network interception, cookies, storage state, viewport or video; you are
  signed in already. Ask for a snapshot with `neo.snapshot(page)` when a locator
  is ambiguous.
- One bounded chunk per call: hard 30 s cap, action timeouts default 10 s,
  expect 5 s. Wait on the element (`locator.waitFor`, `expect(...).toBeVisible()`),
  never `waitForTimeout` loops.
- Carry URLs between calls, not page ids; re-derive with `context.pages()`.
```

**Worked examples.**

```js
// 1. Search and verify, returns data.
await page.goto('https://news.ycombinator.com');
await page.getByRole('link', { name: 'new', exact: true }).click();
await expect(page.getByRole('link', { name: 'Hacker News' })).toBeVisible();
const titles = await page.locator('.titleline > a').allTextContents();
return titles.slice(0, 10);
```

```js
// 2. Fan out over pages, then use a BrowserOS extra.
const urls = ['https://example.com', 'https://example.org'];
const pages = await Promise.all(urls.map(async (u) => {
  const p = await context.newPage();                  // background, claimed, grouped
  await p.goto(u, { waitUntil: 'domcontentloaded' });
  return p;
}));
const docs = await Promise.all(pages.map((p) => neo.read(p)));   // markdown
return docs.map((d) => d.slice(0, 1500));
```

```js
// 3. A failing script: strict-mode miss.
await page.goto('https://example.com');
await page.getByRole('button', { name: 'Submit' }).click({ timeout: 3000 });
```

Result text for 3 (is_error = true):

```
error: TimeoutError: locator.click: Timeout 3000ms exceeded.
Call log:
  - waiting for getByRole('button', { name: 'Submit' })
  - 0 matches (strict) on https://example.com/ (page 7)
```

Audit rows for 3: parent `run` dispatch; children `pw.goto` (page 7) and `pw.click` (page 7, `isError: true`), each with a screenshot.

## 3. Architecture

```
run tool (browseros-mcp, QuickJS)                       claw-server-rust host
┌──────────────────────────────┐                        ┌────────────────────────┐
│ facade.js (Playwright shape) │ __browserosCall("pw.*")│ ScriptInnerCallHook    │
│ page/locator/expect/neo      ├──────────────┐         │ authorize/record/      │
└──────────────────────────────┘              ▼         │ on_page_created        │
                                   BrowserBridge::call ─┼─► audit rows, screenshots,
                                   (run.rs, unchanged) │   ownership notice, tab groups
                                              │         └────────────────────────┘
                                              ▼
                     run_playwright/{mod,actions,waits,expect}.rs
                                              │
                          ┌───────────────────┼─────────────────────┐
                          ▼                   ▼                     ▼
          browseros-core::locator   browseros-core::input   PageManager/Navigation/
          (injected engine in an    (backend-node actions,  PageSignals/FrameRegistry
           isolated world)           trusted input)         (existing)
```

**Seam reused: `InnerCallHook` via `BrowserBridge::call`.** Every `pw.*` method is page-first, so `target_page()` (`run.rs`) authorizes and records it once `pw.` joins its `page_first` rule. `on_page_created` fires for `pw.newPage` and for `pw.waitForEvent('popup')`. Audit `tool_name` is the bridge method (`pw.click`), `args` the JSON array (`[7, "internal:role=button[name=\"Submit\"i]", {"timeout":3000}]`); page/url/title/duration/error, per-step screenshots (`persist_screenshot`), tab activity and session touch all come from `ScriptInnerCallHook::record` unchanged. Ownership notice: `authorize()` fills `foreign_pages` → `effects::page_ownership_notice`. Tab groups and replay: `on_page_created` → `ownership_claims::record_new_page` + `tab_groups::run_tab_group_work`; the session-tab claim there (`enqueue_claim_tab_for_session`) is what the recorder and `visuals.capture` key on. Run-failure telemetry: `observers/run_failure.rs` keys on tool `run` and `raw_args.code`; the `ress` scanner is dialect-agnostic. Cancellation: `RunControl` races every primitive against deadline and `CancellationToken`; the QuickJS interrupt handler stops pure-JS loops; engine retry loops take `Deadline`. Reconnect: all CDP goes through `PageManager::get_session`, which re-attaches on epoch change (`pages.rs:ensure_connected`); the world cache is epoch-keyed like `FrameRegistry::clear_sessions_from_prior_connection`.

### Module: `browseros-core::locator` (new, `crates/browseros-core/src/locator/`)

Interface (Rust):

```rust
pub struct LocatorEngine { pages: Arc<PageManager>, frames: Arc<FrameRegistry>, worlds: Mutex<WorldCache> }

#[derive(Clone, Copy)] pub enum ElementState { Visible, Hidden, Enabled, Disabled, Editable, Checked, Unchecked, Stable }
pub struct Resolved { pub session: ProtocolSession, pub backend_node_id: i64, pub object_id: String, pub frame_id: Option<FrameId> }
pub struct Deadline { pub at: tokio::time::Instant, pub cancel: CancellationToken }
pub enum Strictness { Strict, First }

impl LocatorEngine {
    /// Playwright selector string (`internal:role=…`, `css=…`, `>> nth=0`, `>> internal:control=enter-frame`).
    /// Polls [0,20,50,100,100,500] ms until one match (Strict errors on >1) or the deadline.
    pub async fn resolve(&self, page: PageId, selector: &str, strict: Strictness, wait: ElementState, dl: &Deadline) -> Result<Resolved, CoreError>;
    pub async fn count(&self, page: PageId, selector: &str) -> Result<usize, CoreError>;
    pub async fn resolve_all(&self, page: PageId, selector: &str) -> Result<Vec<Resolved>, CoreError>;
    pub async fn wait_for_states(&self, r: &Resolved, states: &[ElementState], dl: &Deadline) -> Result<(), CoreError>;
    /// `expression` is an injected `to.*` name; returns Playwright's {matches, received}.
    pub async fn expect(&self, page: PageId, selector: &str, expression: &str, options: Value, dl: &Deadline) -> Result<ExpectOutcome, CoreError>;
    /// Run a page function on the element (main world), used by locator.evaluate/inputValue/etc.
    pub async fn call_on(&self, r: &Resolved, fn_source: &str, arg: Value) -> Result<Value, CoreError>;
}
pub struct ExpectOutcome { pub matches: bool, pub received: Option<Value>, pub timed_out: bool }
```

`world.rs`: `async fn injected(&self, session: &ProtocolSession, frame_id: Option<&FrameId>) -> Result<ExecutionContextId, CoreError>` does `Page.createIsolatedWorld {frameId, worldName: "__browseros_pw"}` once per (epoch, session, frame), evaluates `assets/injected_script.js` + `globalThis.__pw = new InjectedScript(window, false, 'javascript', 'data-testid', 20, 'chromium', true, [])`, and drops the entry on `Runtime.executionContextsCleared` / `executionContextDestroyed` (retry once on "Cannot find context"). Frame chains: split on `internal:control=enter-frame`, resolve the frame element, get its `frameId` from `DOM.describeNode` (`observer.rs:DescribedNode` already parses it), then `FrameRegistry::resolve_frame_target` for the OOPIF session. Deletion test: delete this module and selector resolution reappears in every `pw.*` action and in `expect`; it earns its keep, and `act` can adopt selector strings later (second adapter → real seam).

### Module: `browseros-core::input` additions (`crates/browseros-core/src/input/mod.rs`, `fill.rs`)

```rust
impl Input {
    pub async fn fill_backend_node(&self, session: &ProtocolSession, backend_node_id: i64, value: &str) -> Result<(), CoreError>;
    pub async fn check_backend_node(&self, session: &ProtocolSession, backend_node_id: i64, checked: bool) -> Result<bool, CoreError>;
    pub async fn upload_backend_node(&self, session: &ProtocolSession, backend_node_id: i64, paths: &[PathBuf]) -> Result<(), CoreError>;
    pub async fn focus_backend_node(&self, session: &ProtocolSession, backend_node_id: i64) -> Result<(), CoreError>;
}
```

`click_backend_node`/`hover_backend_node`/`select_backend_node`/`press`/`type_text`/`ClickOptions{click_count}` exist. The cover check (`check_click_point` → `CoreError::ElementCovered`) is Playwright's `expectHitTarget` equivalent; keep it.

### Module: `browseros-mcp::tools::run_playwright` (new dir `crates/browseros-mcp/src/tools/run_playwright/`)

`mod.rs`:

```rust
pub const FACADE_JS: &str = include_str!("facade.js");
/// Dispatches a `pw.*` bridge method. Called from `BrowserBridge::dispatch` for `method.starts_with("pw.")`.
pub(crate) async fn dispatch(bridge: &BrowserBridge, method: &str, args: Vec<Value>) -> Result<BrowserCallValue, String>;
```

Bridge methods (JSON: `[pageId, ...]`; `opts` objects carry `timeout`, `strict`, `force`, `waitUntil`, `exact` …):

```
pw.newPage(url?, opts?) -> pageId            pw.pages(scope) -> [{pageId,url,title,ownership,ownerLabel}]
pw.close(page)                                pw.info(page) -> {url,title}
pw.goto(page,url,opts) pw.reload pw.goBack pw.goForward pw.waitForLoadState(page,state,opts) pw.waitForURL(page,pattern,opts)
pw.click(page,sel,opts) pw.dblclick pw.hover pw.fill(page,sel,value,opts) pw.type pw.press(page,sel|null,key,opts)
pw.check(page,sel,checked,opts) pw.selectOption(page,sel,values,opts) pw.setInputFiles(page,sel,paths,opts)
pw.focus pw.scrollIntoView pw.clear pw.dragTo(page,sel,targetSel,opts) pw.waitFor(page,sel,state,opts)
pw.query(page,sel,what,arg?)  what ∈ count|textContent|innerText|innerHTML|inputValue|getAttribute|isVisible|isEnabled|isChecked|boundingBox|allTextContents|allInnerTexts
pw.locatorEvaluate(page,sel,fnSource,arg,opts)   pw.evaluate(page,fnSource,arg,opts)   pw.waitForFunction(page,fnSource,arg,opts)
pw.expect(page,sel|null,expression,options,opts) -> {matches,received,timedOut}
pw.screenshot(page,opts) -> tool:screenshot   pw.pdf(page,opts) -> tool:pdf   pw.content(page)
pw.keyboard(page,op,args) pw.mouse(page,op,args)
pw.waitForEvent(page,kind,filter,opts)  kind ∈ download|dialog|popup|response|request|framenavigated
pw.dialog(page,accept,promptText?)      pw.frames(page) -> Page.getFrameTree
```

`actions.rs`: Playwright's order per action (`dom.ts:_performPointerAction`): resolve (poll) → scroll into view → `wait_for_states([Visible,Enabled,Stable])` (skip with `force`) → cover check → dispatch via `Input`, all under one `Deadline`; error text builds a `Call log`. `waits.rs`: load states from `Page.lifecycleEvent`/`Navigation::wait_for_load`; `waitForURL` polls `PageManager::refresh`; `waitForResponse/Request` do a lazy `Network.enable` per session and filter `Network.responseReceived` via `CdpClient::on_event`; `download` reuses the `Page.downloadWillBegin/downloadProgress` handling from `tools/download.rs`; `dialog` uses `PageSignals::pending_dialog` + `Input::handle_dialog`; `popup` diffs `pages.list()` and calls `hook.on_page_created`. `expect.rs`: poll `[100,250,500,1000]` ms until `matches` or deadline, with Playwright's expression names (`to.be.visible`, `to.have.text`, `to.contain.text`, `to.have.url`, `to.have.count`, `to.have.value`, `to.be.enabled`, `to.have.title`, `to.be.checked`, `to.have.attribute`, `to.have.class`, `to.be.hidden`, `to.be.attached`, `to.be.focused`, `to.be.empty`, `to.match.aria`). `facade.js`: immutable chainable `Locator` composing selector strings exactly like `client/locator.ts` (`>> nth=0`, `internal:has=`, `internal:has-text=`, `internal:and=`, `internal:or=`, `internal:control=enter-frame`) with `getBy*` from `locatorUtils.ts` (3 KB); `page.evaluate(fn, arg)` sends `String(fn)` (QuickJS keeps function source: `quickjs.c:41794`).

Deletion test: `run_playwright` is the *interface* the agent programs against; deleting it moves the API surface into the agent's head. It is not pass-through: each method carries a deadline, error formatting, and audit shape.

### Edits to existing files

- `crates/browseros-mcp/src/tools/run.rs`: `RunArgs.api: Option<RunApi>` + sniff; choose bootstrap (`BOOTSTRAP_JS` vs `FACADE_JS`); `dispatch` arm `m if m.starts_with("pw.") => run_playwright::dispatch(self, m, args).await`; `target_page` treats `pw.` as page-first; `on_page_created` also for `pw.newPage`; `RunOutput.api`. `LocatorEngine` is constructed lazily per `BrowserSession` (add `BrowserSession::locator()` in `crates/browseros-core/src/session.rs`, same pattern as `observe()`).
- `crates/browseros-core/src/lib.rs`: `pub mod locator;`. `crates/browseros-mcp/src/tools/mod.rs`: `pub mod run_playwright;`.
- `apps/claw-server-rust/src/api/mcp/helper_runtime.rs`: `preload_helpers`/`discovery` skip when `raw_args.api == "playwright"` (or sniff says playwright).
- `apps/claw-server-rust/src/api/mcp/prompt.rs`, `skills/browseros-neo/SKILL.md`: text above.
- `apps/claw-app/components/audit/Timeline.tsx`: nest rows by `parentDispatchId` under their `run`; highlight `code` with shiki (optional, cosmetic).
- Not touched: `dispatch.rs`, `script_hook.rs`, `observers/*`, `effects/*`, `db/*`, `claw-api` contract, Chromium.

## 4. Work pieces (dependency-ordered; ‖ = parallel after P0)

| # | Goal | Owned files | Interface honoured | Verify | Size |
|---|---|---|---|---|---|
| P0 | **Skeleton**: every file/type/fn above as compiling stubs (`todo!()`-free: return `Err("not implemented")`), `api` arg, bootstrap switch, `pw.` arm, vendored JS + NOTICE | `run.rs` (code, not `DESCRIPTION`), `run_playwright/{mod.rs,actions.rs,waits.rs,expect.rs,facade.js}` stubs, `locator/{mod.rs,world.rs,assets/*}`, `session.rs` (`locator()`), `lib.rs`, `tools/mod.rs` | signatures in §3 | `cargo build -p claw-server-rust && cargo test -p browseros-mcp run_` ; new test: `api:"playwright"`, code `return typeof page + typeof expect` → `"objectfunction"` | M |
| P1 ‖ | Locator engine over the injected script in an isolated world, frames | `locator/mod.rs`, `locator/world.rs` | `LocatorEngine` | `cargo test -p browseros-core locator::` with a fake `CdpConnection` asserting `Page.createIsolatedWorld` then `Runtime.callFunctionOn` returning a backendNodeId; live: isolated neo, `page.getByRole('heading',{name:'Example Domain'}).count()` on example.com → `1` | L |
| P2 ‖ | Backend-node input variants | `input/mod.rs`, `input/fill.rs` | §3 signatures | `cargo test -p browseros-core input::` (fake `DOM.setFileInputFiles`, `Input.insertText`) | S |
| P3 ‖ | Facade JS: Browser/Context/Page/Locator/FrameLocator/expect/neo, selector composition, error shaping, Node-ism shims | `run_playwright/facade.js` | bridge method list | `cargo test -p browseros-mcp pw_facade_` with `RunFakeConnection`: `getByRole('button',{name:'Submit'})` composes `internal:role=button[name="Submit"i]`; `newContext()` warns | L |
| P4 | Actions bridge | `run_playwright/mod.rs`, `run_playwright/actions.rs` | `dispatch`, `LocatorEngine`, `Input` | `cargo test -p browseros-mcp pw_actions_`: hook log records `("pw.goto",Some(1),false)`, `("pw.click",Some(1),false)`; live smoke: example 1 | L |
| P5 ‖P4 | Waits and events (load states, URL, function, network, download, dialog, popup) | `run_playwright/waits.rs` | `dispatch` | `cargo test -p browseros-mcp pw_waits_`; live: `page.waitForEvent('popup')` claims the tab (row `pages.newPage`-equivalent + group) | M |
| P6 ‖P4 | `expect` polling | `run_playwright/expect.rs` | `LocatorEngine::expect` | `cargo test -p browseros-mcp pw_expect_`: `toBeVisible` on missing element returns `TimeoutError` with `Received: hidden` | M |
| P7 ‖ | Text: tool `DESCRIPTION` block in `run.rs`, prompt, skill | `run.rs` `DESCRIPTION` const only, `prompt.rs`, `SKILL.md` | prompt tests in `prompt.rs` still pass | `cargo test -p claw-server-rust prompt` | S |
| P8 ‖ | Host gating of helpers by dialect | `helper_runtime.rs` | `ARBITRARY_SCRIPT_TOOLS` unchanged | `cargo test -p claw-server-rust helper_runtime` | S |
| P9 ‖ | Cockpit nesting + code highlight | `Timeline.tsx` | `parentDispatchId` on `Dispatch` | `bun run check && bun test` in `packages/browseros-agent` | M |
| P10 | Conformance cases | `contracts/claw-mcp/tests/cases-playwright.ts`, registration line in `cases.ts` | MCP `run` contract | `BROWSEROS_BINARY='/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo' bun contracts/claw-mcp/tests/run.ts --smoke` | M |
| P11 (morning) | Gates: `cargo clippy --package claw-server-rust --all-targets --locked -- -D warnings`, `cargo fmt --all --check`, `cargo test -p claw-server-rust --locked` | none | | all green | S |

## 5. Top five ways this fails

1. **Injected-script version drift** (constructor signature changed between 1.53 and 1.63; `packages/injected/src/injectedScript.ts:109`). Mitigation: vendor one pinned build with its SHA in `assets/NOTICE`, wrap construction in `world.rs`, and a `cargo test` that evaluates the bundle in QuickJS-free fashion (string contains `class InjectedScript`, constructor arity check via regex) plus the live conformance case.
2. **Stale isolated-world context after navigation** → `Cannot find context with specified id`. Mitigation: cache keyed by (epoch, session, frame), invalidated on `Runtime.executionContextsCleared`, one automatic retry; this is what Playwright itself does.
3. **Actionability retries burn the 30 s cap** and the agent sees only "run exceeded 30000ms". Mitigation: 10 s / 5 s defaults clamped to remaining budget; every error carries the `Call log`; the run error names the primitive that consumed the budget.
4. **Focus/ownership gaps on browser-created tabs** (`target=_blank`, `window.open`): the fork only keeps them in the background when the source tab is inactive (`browser.cc:AddNewContents` patch), and nobody claims them. Mitigation: `page.waitForEvent('popup')`/`context.on('page')` claim and group via `on_page_created`; `pw.click` on a `target=_blank` link reconciles `pages.list()` afterwards and claims new tabs. Flagged follow-up (Chromium): default `Target.createTarget` to background under the pref, for any third-party CDP client.
5. **Model writes Node/test-runner code** (`require`, `chromium.launch`, `test.describe`, `page.route`, `import`). Mitigation: shims for the harmless ones, `not available in BrowserOS neo` errors with the replacement named, and the skill/prompt examples. Run-failure telemetry (`run_failure.rs`) tells us which idioms fail most; iterate the shims from data.

## 6. If I only had 4 hours

P0 with the vendored bundle; P1 for main-frame only (no `enter-frame`); P3 with `locator/getByRole/getByText/getByLabel/getByPlaceholder/getByTestId`, `nth/first/last/filter({hasText})`, `click/fill/press/type/check/selectOption`, `goto/url/title/content/evaluate/screenshot/waitForLoadState/waitForTimeout`, `context.newPage/pages`, lazy `page`; P6 with `toBeVisible/toHaveText/toContainText/toHaveURL/toHaveCount/toHaveValue`; P7 prompt paragraph. Everything else throws "not available yet". Verify with example 1 and 3 against an isolated neo, then `cargo test -p browseros-mcp`. Skip frames, network waits, downloads/dialogs (use `neo.download`), keyboard/mouse, popups, cockpit.

## 7. Decisions and assumptions

DECIDED: `run` keeps its name; `api` arg with default `playwright`, legacy sniff keeps existing skills/helpers working.
WHY: Owner's goal is out-of-the-box Playwright; invariant 1 says don't break `run`; the legacy namespace is distinctive enough to sniff deterministically.
REVISIT: remove the sniff once `skills/neo-*` and saved helpers are migrated.

DECIDED: `context` ⇔ agent session; `browser.newContext()` returns the same context with a warning; `browser.close()`/`context.close()` are no-ops.
WHY: one shared signed-in profile; the `newContext(); newPage()` idiom is common (674 hits in the local corpus) and must not fail.
REVISIT: per-agent `browserContextId` if the fork ever exposes isolated profiles.

DECIDED: `page` global is lazy (last own tab, else new background tab).
WHY: `await page.goto(url)` is the single most common first line; zero ceremony wins.
REVISIT: if agents keep piling up tabs, bind `page` only to the last own tab and require `context.newPage()`.

DECIDED: no job model overnight; 30 s cap stays, defaults 10 s/5 s.
WHY: MCP client timeouts and cancellation semantics are built around the cap; chunking works today.
REVISIT: `run` with `detach: true` + `run_status` tool once audit rows can stream.

DECIDED: `page.bringToFront()` is a warning no-op; `pw.newPage` always `Browser.createTab background:true`.
WHY: invariant 3.
REVISIT: allow `bringToFront` for a tab in a window the user is not looking at.

DECIDED: helpers hot-load only in `browseros` mode overnight.
WHY: saved helpers are written against the SDK's `(browser, page)`.
REVISIT: Playwright-dialect helpers `(context, page, inputs)` and a converter.

DECIDED: vendor Playwright's injected script (Apache-2.0) rather than link rustwright-core or write our own engine.
WHY: fidelity is the point; Apache-2.0 is compatible with AGPL-3.0 with a NOTICE.
REVISIT: if the bundle's isolated-world assumptions break on Chromium 151+, fall back to rustwright's MIT template for role/text/label.

ASSUMPTION: plain `Target.createTarget` without `background` opens a foreground tab in the fork (upstream `ChromeDevToolsManagerDelegate::CreateNewTarget` maps it to `NEW_FOREGROUND_TAB`; the fork patch to that file only adds `GetTargetTabId`). The playwright spike measures this.
ASSUMPTION: rquickjs evaluates without `JS_EVAL_FLAG_STRIP`, so `Function.prototype.toString` returns source (QuickJS side verified at `quickjs.c:41794`; rquickjs flags not read).
ASSUMPTION: Playwright's injected bundle (309,776 B in `playwright-core@1.59.1/lib/generated/injectedScriptSource.js`; 1.63 inlines it in `lib/coreBundle.js`) evaluates standalone in a Chromium 151 isolated world without `utilityScriptSource.js`.
ASSUMPTION: `Page.createIsolatedWorld` on an OOPIF child session accepts that frame's `frameId` as `FrameRegistry` records it.
ASSUMPTION: binary growth ≤ 1 MB (today 22.6 MB dev release build, 24.4 MB shipped in `/Applications/BrowserOS neo.app`).
ASSUMPTION: rustwright's `new_page` issues `Target.createTarget` (not read; 64k-line file).
ASSUMPTION: the local corpus counts (Playwright's own tests dominate; agent repos stagehand, chrome-devtools-mcp, oss-skills, paperclip agree on the top ten: goto, evaluate, locator/getBy*, click, url, screenshot, waitForLoadState/Selector, newPage/pages, on) approximate what agents write.
ASSUMPTION: `Network.enable` per page session for `waitForResponse` does not measurably slow the existing observer path; enable lazily and only on pages a script asks about.
