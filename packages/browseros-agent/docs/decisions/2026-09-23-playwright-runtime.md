# Decision: agents write real Playwright JS; BrowserOS neo runs it in-process

Date: 2026-09-23 (overnight). Status: decided for the first build; every
`DECIDED:` below can be overturned in the morning. Inputs: three design
documents (in-process rustwright, sidecar `playwright-core`, open), an
architecture scan of the script path, and three spikes (rustwright
in-process, `playwright-core` sidecar, Playwright's injected script
standalone). Those live in the shared store under `playwright-runtime/`.

## What is being built

A new MCP tool, **`playwright`**, beside the untouched `run`. The agent sends
ordinary Playwright JavaScript (`context`, `page`, locators, `expect`) as the
`code` argument; the server runs it in the **existing QuickJS runtime** with a
Playwright-shaped facade. Locator resolution, element states, actionability
checks and `expect` matchers are **Playwright's own injected script**
(Apache-2.0, vendored, pinned), evaluated in an isolated world through the
server's existing CDP session. Trusted input, navigation, page lifecycle,
screenshots, downloads and dialogs are `browseros-core`. Every facade leaf
goes through the existing `__browserosCall` bridge, so `InnerCallHook` and
`ScriptInnerCallHook` fire unchanged: child audit rows, per-step screenshots,
page claiming, tab grouping, ownership notices, replay attribution and
cancellation are inherited, not re-implemented.

```
agent ──MCP──▶ tool "playwright" {code, timeout}
                 │ (browseros-mcp/src/tools/playwright.rs)
                 ▼
        QuickJS runtime shared with `run` (script_runtime in run.rs)
        bootstrap = pw/facade.js  ── every leaf: __browserosCall("page.goto"|"locator.click"|…)
                 │
                 ▼
        BrowserBridge::call (run.rs, unchanged choreography)
          hook.authorize ─▶ dispatch ─▶ [hook.on_page_created] ─▶ hook.record (+ redaction)
                 │  method starts with page./locator./expect./context./keyboard./mouse./frame.
                 ▼
        pw::dispatch (browseros-mcp/src/pw/{mod,actions,waits,expect}.rs)
                 │
    ┌────────────┼──────────────────────────┐
    ▼            ▼                          ▼
 browseros-core::locator   browseros-core::input    PageManager / Navigation /
 (injected script in an     (backend-node actions,   PageSignals / FrameRegistry /
  isolated world)            trusted CDP input)      tools: screenshot, pdf, download
```

Nothing new ships: no Node, no Bun, no sidecar, no second CDP client, no
Chromium change. Binary growth is the vendored bundle (~310 KB).

## Why this and not the others

DECIDED: in-process facade over Playwright's vendored injected script (design C's engine) behind a new `playwright` tool (designs A and B's contract).
WHY: fidelity where it matters is Playwright's own code (selectors, strict mode, states, `expect`), with zero runtime shipped and the existing sandbox, audit and ownership paths untouched. The sidecar (B) measured +64.1 MB per platform, needs a Bun/Playwright pin with source rewrites, a CDP relay that fakes `Target.setAutoAttach`, per-platform signing, and `node:vm` is not a boundary (escape demonstrated). rustwright in-process (A) measured: locator specs public for click only, no public waits or spec-based fill, `>>`/`:has-text`/`nth=` broken, own Tokio runtime (panics on the executor), `pages()` debugger-attaches every user tab, alpha code with `unwrap` in live paths; closing those gaps means maintaining a fork of a 64k-line file.
REVISIT: if the injected-script spike reports the bundle cannot run standalone in a Chromium 151 isolated world, the fallback engine behind the same bridge is rustwright's MIT locator template for the role/text/label subset (design C §7) or a rustwright fork (design A §3.7), not a sidecar. If the owner later wants `page.route`, `storageState`, tracing or video, that is the sidecar's territory and design B is the spec for it; the `ScriptEngine` seam in `host-seam.md` is how it plugs in.

DECIDED: a new tool named `playwright`, not `run` with an `api` switch.
WHY: additive-only overnight; `run`'s contract, tests and saved helpers stay byte-identical; models recognise the name; run-failure telemetry and helper gating key on the tool name with no sniffing.
REVISIT: fold into `run` behind `api: "playwright"` once the new tool has a week of sessions.

DECIDED: reuse `InnerCallHook` through the shared `BrowserBridge`; do not build the `ScriptHost`/`ScriptDispatch` seam tonight.
WHY: with both tools on one bridge the choreography exists once already; the scan's refactor moves working code the night before the owner reads it. Its one urgent finding, unredacted typed secrets in audit rows, is taken (below).
REVISIT: `host-seam.md` is the spec for the day a second engine (sidecar or rustwright) is added.

DECIDED: no Chromium changes; `context.newPage()` goes through the fork's `Browser.createTab background:true` as `run` does.
WHY: measured: plain `Target.createTarget` (what Playwright and rustwright send) selects the new tab; the fork's command does not.
REVISIT: a Chromium follow-up to default `Target.createTarget` to background under the never-steal-focus pref would make any third-party CDP client safe.

## Agent-facing contract

Tool `playwright`, catalog entry after `run`. Input schema (`deny_unknown_fields`):

```json
{ "type": "object", "required": ["code"], "additionalProperties": false,
  "properties": {
    "code":    { "type": "string", "description": "Standard Playwright JavaScript body, no imports. Top-level await; `return` a JSON value." },
    "timeout": { "type": "number", "description": "Max run time in ms. Default and hard cap 30000; larger values are clamped." } } }
```

Output is `run`'s `RunOutput` (`{ ok, value?, logs, error? }`) and `run`'s text
envelope (`ok` / `return:` / `logs:`), produced by the same code.

Globals the script sees, and nothing else:

| Global | Meaning |
|---|---|
| `browser` | `contexts()` → `[context]`; `newContext()` → `context` plus a console warning (one signed-in profile, no isolation); `newPage()` = `context.newPage()`; `close()` → warning, no-op; `version()`. |
| `context` | **The agent session** ⇔ Playwright `BrowserContext`. `pages()` = tabs this conversation owns (ownership `mine`); `newPage()` = new background tab, claimed and grouped before it resolves; `setDefaultTimeout`, `setDefaultNavigationTimeout`; `on('page', h)` / `waitForEvent('page')` for popups; `close()` → warning, no-op. |
| `page` | Lazy: the most recently used own tab that is still open, else a new background tab on first use. `await page.goto(url)` works with no ceremony. |
| `expect` | Web-first matchers on locators and pages via the injected engine (`toBeVisible`, `toBeHidden`, `toBeAttached`, `toBeEnabled`, `toBeDisabled`, `toBeChecked`, `toBeEditable`, `toBeEmpty`, `toBeFocused`, `toHaveText`, `toContainText`, `toHaveValue`, `toHaveCount`, `toHaveAttribute`, `toHaveClass`, `toHaveId`, `toHaveCSS`, `toHaveTitle`, `toHaveURL`, `.not`), 5 s retry; plus a pure-JS value subset (`toBe`, `toEqual`, `toContain`, `toBeTruthy`, `toBeGreaterThan`, `toMatch`, `.not`). |
| `neo` | BrowserOS extras: `neo.pages({ ownership: 'all' \| 'mine' \| 'user' \| 'other-agent' })` → `[{ pageId, url, title, ownership, ownerLabel }]`; `neo.page(pageId)` → a `Page` for any tab (ownership notice appended, never refused); `neo.snapshot(page)` → `{ text, refs }`; `neo.read(page)` → markdown; `neo.grep(page, opts)`; `neo.download(page, opts)`; `neo.cdp(method, params?, page?)`. |
| `console`, `setTimeout`, `clearTimeout`, `sleep(ms)` | as in `run`. |

`Page` objects carry `page.pageId` (the server `PageId`) and standard
`page.url()`. Between calls carry the URL or `page.pageId`; re-derive with
`context.pages()` or `neo.page(id)`. `page.bringToFront()` is a warning
no-op (agents never steal focus). `page.evaluate(fn, arg)` sends
`String(fn)`; `arg` and the result are JSON.

Timeouts: the 30 s call cap is unchanged. Inside it, action and navigation
default to 10 s, `expect` to 5 s, each clamped to the remaining budget, so a
wrong locator fails with a Playwright `TimeoutError` (with `Call log`) rather
than eating the run. No job model overnight; one bounded chunk per call.

Not available (throw `Error: not available in BrowserOS neo: <api>. <hint>`):
`route`/`unroute`, `request` (APIRequestContext), `cookies`/`addCookies`/
`storageState` ("you are already signed in"), `addInitScript`,
`setViewportSize`, `emulateMedia`, `tracing`, `video`, `exposeFunction`,
`page.pause`. `require`, `import`, `fetch`, `process`, `fs` fail with a
one-line hint. `test(name, fn)`, `chromium.launch()`, `chromium.connectOverCDP()`
resolve to the same `browser` so pasted test bodies run.

Audit child rows carry Playwright-shaped method names: `context.newPage`,
`page.goto`, `page.evaluate`, `locator.click`, `locator.fill`,
`expect.toBeVisible`, … with args `[pageId, selector, …]`. Values typed into
password or secret-autocomplete inputs are recorded as `"[redacted]"` in the
child row and masked in the parent row's script text.

No saved helpers in `playwright` overnight (they are written against the
`browser.*` SDK); `helpersAvailable` is not appended.

## Interfaces at the seams (build against these)

### `browseros-core/src/locator/` (new; `LocatorEngine`)

```rust
pub struct LocatorEngine { /* pages, frames, world cache keyed by (epoch, session, frame) */ }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementState { Visible, Hidden, Enabled, Disabled, Editable, Checked, Unchecked, Stable }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strictness { Strict, First }

pub struct Resolved { pub session: ProtocolSession, pub backend_node_id: i64, pub object_id: String, pub frame_id: Option<FrameId> }
pub struct Deadline { pub at: tokio::time::Instant, pub cancel: CancellationToken }
pub struct ExpectOutcome { pub matches: bool, pub received: Option<Value>, pub timed_out: bool, pub log: Vec<String> }

impl LocatorEngine {
    pub fn new(pages: Arc<PageManager>, frames: Arc<FrameRegistry>) -> Self;
    /// Playwright selector string (`internal:role=…`, `css=…`, `>> nth=0`, `>> internal:control=enter-frame`).
    /// Polls [0,20,50,100,100,500] ms until one match (Strict errors on >1) or the deadline.
    pub async fn resolve(&self, page: PageId, selector: &str, strict: Strictness, wait: Option<ElementState>, dl: &Deadline) -> Result<Resolved, CoreError>;
    pub async fn resolve_all(&self, page: PageId, selector: &str) -> Result<Vec<Resolved>, CoreError>;
    pub async fn count(&self, page: PageId, selector: &str) -> Result<usize, CoreError>;
    pub async fn wait_for_states(&self, r: &Resolved, states: &[ElementState], dl: &Deadline) -> Result<(), CoreError>;
    /// `expression` is an injected `to.*` name (`to.be.visible`, `to.have.text`, …); options as Playwright's `ExpectedTextValue`/count/attribute payload.
    pub async fn expect(&self, page: PageId, selector: Option<&str>, expression: &str, options: Value, dl: &Deadline) -> Result<ExpectOutcome, CoreError>;
    /// Run a page function on the element in the main world (locator.evaluate, inputValue, getAttribute, …).
    pub async fn call_on(&self, r: &Resolved, fn_source: &str, arg: Value) -> Result<Value, CoreError>;
    /// Attributes the host needs for redaction and error text: `{ tag, type, autocomplete }`.
    pub async fn describe(&self, r: &Resolved) -> Result<Value, CoreError>;
}
```

`world.rs` owns the bootstrap: `Page.createIsolatedWorld { frameId, worldName: "__browseros_pw", grantUniveralAccess: false }`,
evaluate `assets/injected_script.js`, construct `InjectedScript` with the
arguments the vendored version expects (the spike report has the exact
sequence), cache per `(connection epoch, session, frame)`, invalidate on
`Runtime.executionContextsCleared` / `executionContextDestroyed`, retry once
on "Cannot find context". Frame chains split on `internal:control=enter-frame`,
resolve the frame element, `DOM.describeNode` → `frameId`,
`FrameRegistry::resolve_frame_target` for OOPIFs. `assets/NOTICE` records the
bundle's origin, version, SHA-256 and Apache-2.0 licence. `BrowserSession`
gains `pub fn locator(&self) -> Arc<LocatorEngine>` (lazy, same pattern as
`observe()`).

### `browseros-core/src/input/` (additions)

```rust
impl Input {
    pub async fn fill_backend_node(&self, session: &ProtocolSession, backend_node_id: i64, value: &str) -> Result<(), CoreError>;
    pub async fn check_backend_node(&self, session: &ProtocolSession, backend_node_id: i64, checked: bool) -> Result<bool, CoreError>;
    pub async fn upload_backend_node(&self, session: &ProtocolSession, backend_node_id: i64, paths: &[PathBuf]) -> Result<(), CoreError>;
    pub async fn focus_backend_node(&self, session: &ProtocolSession, backend_node_id: i64) -> Result<(), CoreError>;
}
```

`click_backend_node`, `hover_backend_node`, `select_backend_node`, `press`,
`type_text` exist and keep their cover check (`CoreError::ElementCovered`).

### `browseros-mcp/src/tools/run.rs` (shared runtime, additive edits only)

```rust
pub(crate) struct ScriptSpec { pub bootstrap_js: &'static str, pub code: String, pub timeout_ms: u64, pub helpers: bool }
/// The QuickJS lifecycle `run` has today (limits, interrupt, logs, marshalling), parameterised by bootstrap.
pub(crate) async fn execute_script(spec: ScriptSpec, ctx: &ToolCtx) -> Result<RunOutcome, RunError>;
// BrowserBridge::dispatch gains one arm:
//   m if pw::is_pw_method(m) => pw::dispatch(self, m, args).await
// target_page treats pw methods as page-first (args[0] is the pageId) except `context.*`/`neo.*` without a page.
// BrowserBridge::call records `audit_args` from a PwCallOutcome when present (redaction), otherwise the raw args.
```

`run`'s handler becomes a two-line caller of `execute_script` with
`BOOTSTRAP_JS`; all existing `run` tests pass unedited.

### `browseros-mcp/src/pw/` (new)

```rust
// mod.rs
pub const FACADE_JS: &str = include_str!("facade.js");
pub(crate) fn is_pw_method(method: &str) -> bool;   // page. locator. expect. context. keyboard. mouse. frame. neo.
pub(crate) struct PwCallOutcome { pub value: BrowserCallValue, pub audit_args: Option<Value>, pub created_page: Option<u32> }
pub(crate) async fn dispatch(bridge: &BrowserBridge, method: &str, args: Vec<Value>) -> Result<PwCallOutcome, String>;
pub(crate) struct Opts { pub timeout_ms: Option<u64>, pub strict: bool, pub force: bool, pub wait_until: Option<String>, pub exact: Option<bool> }
pub(crate) fn opts(args: &[Value], index: usize) -> Opts;
pub(crate) fn deadline(bridge: &BrowserBridge, opts: &Opts, default_ms: u64) -> Deadline; // clamped to the run's remaining budget

// actions.rs — Playwright's per-action order: resolve (poll) → scroll into view → wait_for_states → cover check → dispatch
pub(crate) async fn dispatch(bridge: &BrowserBridge, method: &str, args: &[Value]) -> Result<PwCallOutcome, String>;
// waits.rs — page.goto/reload/goBack/goForward/waitForLoadState/waitForURL/waitForFunction/waitForEvent(download|dialog|popup|response|request)/locator.waitFor
pub(crate) async fn dispatch(bridge: &BrowserBridge, method: &str, args: &[Value]) -> Result<PwCallOutcome, String>;
// expect.rs — expect.<matcher> polled [100,250,500,1000] ms until matches or deadline; returns {matches, received, log}
pub(crate) async fn dispatch(bridge: &BrowserBridge, method: &str, args: &[Value]) -> Result<PwCallOutcome, String>;
```

Bridge method table (JSON args `[pageId, …]`; `opts` objects carry `timeout`,
`strict`, `force`, `waitUntil`, `exact`, `noWaitAfter`):

```
context.newPage(url?, opts?) -> pageId        context.pages() -> [{pageId,url,title}]      context.close(pageId)
page.info(pageId) -> {url,title}              page.content(pageId)                          page.title / page.url via info
page.goto(pageId,url,opts) page.reload page.goBack page.goForward page.waitForLoadState(pageId,state,opts) page.waitForURL(pageId,pattern,opts)
page.evaluate(pageId,fnSource,arg,opts)  page.waitForFunction(pageId,fnSource,arg,opts)  page.screenshot(pageId,opts) page.pdf(pageId,opts)
page.waitForEvent(pageId,kind,filter,opts)  page.dialog(pageId,accept,promptText?)  page.frames(pageId)
keyboard.press(pageId,key,opts) keyboard.type(pageId,text,opts) keyboard.insertText  mouse.click(pageId,x,y,opts) mouse.move mouse.wheel
locator.click(pageId,sel,opts) locator.dblclick locator.hover locator.fill(pageId,sel,value,opts) locator.type locator.press(pageId,sel,key,opts)
locator.check(pageId,sel,checked,opts) locator.selectOption(pageId,sel,values,opts) locator.setInputFiles(pageId,sel,paths,opts)
locator.focus locator.blur locator.clear locator.scrollIntoViewIfNeeded locator.dragTo(pageId,sel,targetSel,opts) locator.waitFor(pageId,sel,state,opts)
locator.query(pageId,sel,what,arg?)   what ∈ count|textContent|innerText|innerHTML|inputValue|getAttribute|isVisible|isHidden|isEnabled|isChecked|isEditable|boundingBox|allTextContents|allInnerTexts|ariaSnapshot
locator.evaluate(pageId,sel,fnSource,arg,opts) locator.evaluateAll(pageId,sel,fnSource,arg,opts)
expect.<matcher>(pageId, sel|null, expected, opts) -> {matches, received, log}
neo.pages(scope) neo.page(pageId) neo.snapshot(pageId) neo.read(pageId,opts) neo.grep(pageId,opts) neo.download(pageId,opts) neo.cdp(method,params,pageId?)
```

`facade.js` composes selector strings exactly as Playwright's client does
(`packages/playwright-core/src/utils/isomorphic/locatorUtils.ts` and
`client/locator.ts` in `/Users/shadowfax/code/oss/playwright`): `getByRole`
→ `internal:role=button[name="Submit"i]`, `getByText` → `internal:text="…"i|s`,
`getByLabel` → `internal:label=…`, `getByPlaceholder`/`getByAltText`/`getByTitle`
→ `internal:attr=[placeholder="…"i]`, `getByTestId` → `internal:testid=[data-testid="…"s]`,
`{ hasText }` → `>> internal:has-text="…"i`, `{ has }` → `>> internal:has="…"`,
`{ hasNot }`/`{ hasNotText }`, `nth(n)`/`first()`/`last()` → `>> nth=n|0|-1`,
`and()` → `internal:and=`, `or()` → `internal:or=`, chaining → ` >> `,
`frameLocator(sel)` → `sel >> internal:control=enter-frame`, with the same
escaping helpers. `Locator` is an immutable value (pageId + selector string);
each action is one bridge call; errors keep Playwright's `name` and message
shape (`TimeoutError: locator.click: Timeout 10000ms exceeded.\nCall log:\n  - waiting for getByRole('button', { name: 'Submit' })`).

### `browseros-mcp/src/tools/playwright.rs` (new)

`definition()` → `def_with_output::<PlaywrightArgs, RunOutput>("playwright", DESCRIPTION, open_world, handler)`;
handler = `parse_args` → `execute_script(ScriptSpec { bootstrap_js: pw::FACADE_JS, code, timeout_ms, helpers: false }, ctx)`
→ `RunOutcome::into_tool_result()`. `DESCRIPTION` carries the COVERED /
DIFFERS / NOT AVAILABLE lists from this document so the model never guesses.
`metadata_for_tool` keeps `accepts_page_arg: false`.

### Host (`claw-server-rust`) edits, all small

- `api/mcp/dispatch.rs`: `ARBITRARY_SCRIPT_TOOLS = &["run", "evaluate", "playwright"]` (gives the tool `ScriptInnerCallHook`); `ToolCall.redactions: Arc<Mutex<Vec<String>>>` (initialised in `ToolCall::new`).
- `api/mcp/helper_runtime.rs`: `preload_helpers` and `discovery` only for `call.tool().name == "run"`.
- `api/mcp/script_hook.rs`: `record` pushes `record.secrets` into `call.redactions`.
- `api/mcp/observers/audit.rs`: parent `args_json` masks every literal in `call.redactions` (`"[redacted]"`) before `bounded_args_json`.
- `api/mcp/observers/run_failure.rs`: `REPORTED_TOOLS = &["run", "playwright"]`.
- `analytics/script_fingerprint.rs`: `SDK_SURFACE` gains the Playwright names (`page context browser expect neo locator getByRole getByText getByLabel getByPlaceholder getByTestId getByAltText getByTitle frameLocator newPage pages goto click fill press type check uncheck selectOption hover waitFor waitForURL waitForLoadState waitForEvent evaluate screenshot pdf keyboard mouse first last nth filter toBeVisible toHaveText toContainText toHaveURL toHaveCount toHaveValue toBeEnabled toBeChecked toHaveTitle toHaveAttribute`).
- `api/mcp/prompt.rs` and both skill copies (`resources/skills/browserclaw/SKILL.md`, `skills/browseros-neo/SKILL.md`): teach `playwright` first, `run` for helpers and raw CDP, granular tools as the fallback.
- Catalog pins: `crates/browseros-mcp/src/tests.rs` (`catalog_order_matches_typescript_registry`, page metadata), `apps/claw-server-rust/tests/routes.rs` (tools/list), `contracts/claw-mcp/tests/cases-transport.ts`.
- `crates/browseros-mcp/src/framework.rs`: `InnerCallRecord` gains `pub secrets: &'a [String]` (values the bridge masked in `args`).

Redaction rule (from the architecture scan): after `locator.fill`,
`locator.type`, `keyboard.type`, `keyboard.insertText`, the bridge asks
`LocatorEngine::describe` (or, for keyboard methods, probes
`document.activeElement` through the page session) for `type` and
`autocomplete`; when `type == "password"` or `autocomplete` contains
`current-password`, `new-password`, `one-time-code`, `cc-number`, `cc-csc`,
`cc-exp`, the typed value is replaced by `"[redacted]"` in `audit_args` and
listed in `secrets`. Probe failure masks (fail closed).

## Work pieces

Ownership is exact; no two pieces share a file. Everyone codes against the
signatures above; if a module you import is not implemented yet, import it
from the documented path anyway, let the typecheck fail exactly there, and
say so in your commit message. That pattern is accepted here.

| # | Goal | Owned files | Verify | Size |
|---|---|---|---|---|
| P0 skeleton | Every file, type and fn above exists and compiles; `playwright` listed and callable, every `pw.*` method returns `Err("not implemented yet: <method>")`; catalog tests updated; vendored bundle + NOTICE in place (from the injected spike) | all new files as stubs; the one-line edits listed under "Host edits" except prompt/skills; `run.rs` `ScriptSpec`/`execute_script` extraction; `tools/mod.rs`; `browseros-core/src/lib.rs`, `session.rs`; `crates/browseros-mcp/src/lib.rs` | `cargo build -p claw-server-rust`, `cargo test -p browseros-mcp`, `cargo test -p claw-server-rust --locked`, clippy `-D warnings`, `cargo fmt --all --check`; new test: `playwright` with `return typeof page + typeof expect` → `"objectfunction"` | M |
| P1 locator engine | `LocatorEngine` over the injected script in an isolated world, frames, world invalidation, `expect`, `describe` | `crates/browseros-core/src/locator/{mod.rs,world.rs,frames.rs}`, `assets/injected_script.js`, `assets/NOTICE` | `cargo test -p browseros-core locator::` with fake `CdpConnection`s (assert `Page.createIsolatedWorld` then `Runtime.callFunctionOn`); live: isolated neo, `internal:role=heading[name="Example Domain"i]` on example.com resolves | L |
| P2 input additions | fill/check/upload/focus by backend node | `crates/browseros-core/src/input/mod.rs`, `input/fill.rs` | `cargo test -p browseros-core input::` | S |
| P3 facade | `facade.js`: Browser/Context/Page/Locator/FrameLocator/Keyboard/Mouse/expect/neo, selector composition with Playwright's escaping, lazy `page`, error shaping, Node-ism shims; tests against `RunFakeConnection` asserting the exact bridge calls | `crates/browseros-mcp/src/pw/facade.js`, `crates/browseros-mcp/src/pw/facade_tests.rs` | `cargo test -p browseros-mcp pw::facade` | L |
| P4 actions | `pw::dispatch` routing + `actions.rs` (click, dblclick, hover, fill, type, press, check, selectOption, setInputFiles, focus, blur, clear, scrollIntoViewIfNeeded, dragTo, query, evaluate, evaluateAll, keyboard, mouse, context.newPage/pages/close, page.info/content/screenshot/pdf/evaluate, neo.*) with redaction | `crates/browseros-mcp/src/pw/mod.rs`, `pw/actions.rs`, `pw/actions_tests.rs` | `cargo test -p browseros-mcp pw::actions` (hook log records `("locator.click", Some(1), false)`); live smoke script 1 of the corpus | L |
| P5 waits | navigation waits, `waitForLoadState`, `waitForURL`, `waitForFunction`, `locator.waitFor`, `waitForEvent` (download, dialog, popup → `on_page_created`, response, request), `page.dialog` | `crates/browseros-mcp/src/pw/waits.rs`, `pw/waits_tests.rs` | `cargo test -p browseros-mcp pw::waits`; live: popup claimed and grouped | M |
| P6 expect | `expect.rs`: matcher table → injected expression + options, polling, `received` formatting, `.not` | `crates/browseros-mcp/src/pw/expect.rs`, `pw/expect_tests.rs` | `cargo test -p browseros-mcp pw::expect`; `toBeVisible` on a missing element → `TimeoutError` with `Received: hidden` | M |
| P7 text | tool `DESCRIPTION` const, MCP instructions, both skill files | `crates/browseros-mcp/src/tools/playwright.rs` (`DESCRIPTION` block only), `apps/claw-server-rust/src/api/mcp/prompt.rs`, `resources/skills/browserclaw/SKILL.md`, `skills/browseros-neo/SKILL.md` | `cargo test -p claw-server-rust prompt`, `cargo test -p browseros-mcp` schema tests | S |
| P8 host | helper gating, run-failure list, fingerprint surface, redaction plumbing (`ToolCall.redactions`, `script_hook.rs`, `observers/audit.rs`) | `apps/claw-server-rust/src/api/mcp/{helper_runtime.rs,script_hook.rs,dispatch.rs,observers/audit.rs,observers/run_failure.rs}`, `src/analytics/script_fingerprint.rs` | `cargo test -p claw-server-rust --locked`; a `run`-style test proving a child row's password value is `"[redacted]"` | M |
| P9 cockpit | nest children under parents, code block for scripts, `playwright` verbs | `apps/claw-app/components/audit/Timeline.tsx`, `Timeline.test.tsx`, `screens/replay/replay.data.ts` (+ test), `screens/task-detail/TabView.tsx` | `bun run check`, `bun test` | M |
| P10 conformance | real-browser cases from the conformance corpus | `contracts/claw-mcp/tests/cases-playwright.ts`, one registration line in `cases.ts`, fixtures under `contracts/claw-mcp/fixtures/playwright/` | `BROWSEROS_BINARY='/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo' bun contracts/claw-mcp/tests/run.ts --smoke` | M |

Order: P0 alone → P1, P2, P3, P7, P8, P9 in parallel → P4, P5, P6 (code
against P1/P2/P3 signatures; may start once P0 merges) → P10 → gates and the
end-to-end drive.

## If only four hours remain

P0; P1 for the main frame only; P3 with `locator/getByRole/getByText/getByLabel/getByPlaceholder/getByTestId`, `nth/first/last/filter({hasText})`, `click/fill/press/check/selectOption`, `goto/url/title/evaluate/screenshot/waitForLoadState`, `context.newPage/pages`, lazy `page`; P6 with `toBeVisible/toHaveText/toContainText/toHaveURL/toHaveCount/toHaveValue`; P7's prompt paragraph. Everything else throws "not available yet".

## Assumptions

ASSUMPTION: the vendored injected script evaluates standalone in a Chromium 151 isolated world and `InjectedScript` can be constructed with the arguments Playwright's server passes (the injected-script spike verifies this before P1 starts).
ASSUMPTION: rquickjs evaluates without `JS_EVAL_FLAG_STRIP`, so `Function.prototype.toString` returns source for `page.evaluate(fn)`.
ASSUMPTION: `Page.createIsolatedWorld` on an OOPIF child session accepts that frame's `frameId` as `FrameRegistry` records it.
ASSUMPTION: agents' Playwright usage is dominated by navigation, locators, actions, waits, `expect`, `evaluate` and screenshots; `route`, `storageState`, tracing and video are rare and are named as unavailable.
