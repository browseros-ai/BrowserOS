# Design B: real `playwright-core` in a supervised sidecar

Designer lens: maximise fidelity. Written 2026-09-23 against `main` at
`8d9a40820` (worktree `feat/pw-design-b`). Every path below was read; every
number is either measured by the playwright spike
(`spikes/playwright/REPORT.md`, `COMMANDS.log`) or marked `ASSUMPTION:`.

## 1. Recommendation

1. Ship **B**: claw-server-rust spawns one Bun-compiled `browseros-playwright`
   sidecar running unmodified `playwright-core` 1.63.0 plus `playwright`'s
   `expect`; a new MCP tool `playwright` beside the untouched `run`.
2. The host owns a **CDP relay** per conversation: the sidecar's
   `connectOverCDP` goes to `ws://127.0.0.1:<port>/cdp/<token>`, and the relay
   attaches only that conversation's tabs, rewrites `Target.createTarget` to
   `background:true`, and claims each new page before the script sees it.
3. Audit rows come from Playwright's own client instrumentation seam
   (`connection._instrumentation.addListener`), one child row per API call,
   through the existing `InnerCallHook`.
4. Cost: +64.1 MB per platform (measured darwin-arm64), ~160 ms cold start,
   ~120 MB RSS while warm, signed through the `bos_build` table that already
   signs a Bun binary for the sibling product.
5. Switch to A only if the Bun/Playwright pin cannot be held across five
   targets (gate: P6) **and** a Node SEA (+~90 MB) is refused.

**Rationale.** Fidelity is the owner's whole ask, and it is what in-process
facades cannot buy: the rustwright spike shows semantic locators exposed for
click only, `>>` chains and `:has-text()` broken, no `wait_for`, and a sync
core that panics on the Tokio executor. `playwright-core` over CDP passed the
whole locator, frame, keyboard, screenshot, PDF and popup matrix on BrowserOS
neo unchanged. Everything BrowserOS adds (audit, ownership, tab groups, focus,
replay, cancellation) already lives in the host keyed on `PageId`; the new
work is two host vantage points on the sidecar: the instrumentation stream
(what the script *did*) and the CDP relay (which tabs it *may see*, and
creation before use). Neither can be delegated to Playwright, so both are Rust.

## 2. Agent-facing contract

**Tool** `playwright` (new; `run` unchanged). Args mirror `run`:

```json
{ "type": "object", "required": ["code"], "additionalProperties": false,
  "properties": {
    "code":    { "type": "string", "description": "Async Playwright JS body; top-level await; `return` a JSON value." },
    "timeout": { "type": "number", "default": 30000, "description": "ms, hard cap 30000 (clamped)." } } }
```

**Globals** the script sees (nothing else; no `require`, `process`, `fetch`,
`window`): `browser` (Playwright `Browser`; `newContext()` throws
`BrowserOS neo has one signed-in profile; use context`), `context`
(`browser.contexts()[0]`, **the agent session**: `context.pages()` is exactly
the tabs this conversation owns), `page` (the session's most recently used
owned page, or `undefined` when it owns none), `expect` (from
`playwright/test`, web-first, 5 s retry), `console` (captured), `setTimeout`
/ `clearTimeout` / `sleep(ms)`, and `browseros`:

```ts
browseros.pages(): Promise<Array<{ pageId, url, title, ownership: "mine"|"user"|"other-agent", ownerLabel? }>>
browseros.page(pageId: number): Promise<Page>      // adopt any tab, including the user's; noticed, never refused
browseros.id(page: Page): number                    // stable PageId for the next call
browseros.snapshot(page: Page): Promise<{ text, refs }> // today's ref snapshot, for agents that like it
browseros.helpers: Record<string, Function>         // saved helpers, unchanged (page, not id, in Playwright dialect)
browseros.cdp(method, params?, page?): Promise<any> // raw escape hatch
```

**Page identity.** `BrowserContext ⇔ agent session ⇔ tab group`. Across
calls the agent carries `browseros.id(page)` (a `PageId`, same lifetime rules
as `run`) or the URL; within a call it holds `Page` objects. Tabs the script
opens are claimed and grouped before `newPage()` resolves.

**Result envelope** (text mirrors `run`'s `format_outcome`; structured):

```json
{ "ok": true,  "value": ..., "logs": ["..."], "pages": [{ "pageId": 7, "url": "...", "title": "..." }] }
{ "ok": false, "logs": ["..."], "error": { "name": "TimeoutError", "message": "locator.click: Timeout 10000ms exceeded.\nCall log:\n  - waiting for getByRole('button', { name: 'Submit' })", "step": { "api": "locator.click", "pageId": 7, "url": "..." } } }
```

`helpersAvailable` and the ownership notice are appended by the existing
effects (`helper_runtime::discovery`, `page_ownership_notice`) because the
tool name is added to `ARBITRARY_SCRIPT_TOOLS`. Errors come back as results
(`is_error: true`), never thrown, exactly like `run`.

**Timeout policy.** 30 s wall cap per call, unchanged (MCP client timeouts,
cancellation). Inside it: `context.setDefaultTimeout(min(10 s, remaining))`
and `setDefaultNavigationTimeout(min(15 s, remaining))` re-applied at every
`onApiCallBegin`, so a stuck step fails with a normal Playwright
`TimeoutError` before the run cap; long jobs remain "one bounded chunk per
call" (REVISIT: a `playwright_job` tool once the sidecar has proven stable).

**Text changes.** `api/mcp/prompt.rs` `BROWSERCLAW_MCP_INSTRUCTIONS`, second
paragraph becomes: "Reach for `playwright` first: write standard Playwright
JavaScript (`context`, `page`, `expect`, locators) and it runs in one call
against this browser. `run` (the `browser.*` SDK) and the granular tools are
the fallback." Add under "Shared with other agents": "`context.pages()` are
your tabs; `browseros.pages()` lists everyone's; `browseros.page(id)` adopts
one." Keep the `run_failure`/`save_skill` paragraphs. Add test
`prompt_prefers_playwright_tool`. `skills/browseros-neo/SKILL.md`: rename
"Writing a `run` script" to "Writing a `playwright` script", replace its five
bullets with: sandbox is Node-less and page-less (`page.evaluate` for the
DOM); one bounded chunk per call, 30 s; `context.newPage()` always opens a
background tab in your group; carry `browseros.id(page)` or the URL between
calls; assertions with `expect` retry for 5 s, so do not poll. Keep a short
"`run` (legacy SDK)" note pointing at the tool description.

**Worked examples.**

```js
// 1. Search and extract, one call.
const p = await context.newPage();
await p.goto('https://news.ycombinator.com');
await p.getByRole('link', { name: 'new' }).click();
await expect(p.getByRole('heading')).toBeVisible();
const titles = await p.locator('.titleline > a').allInnerTexts();
return { pageId: browseros.id(p), titles: titles.slice(0, 10) };
```

```js
// 2. Continue on a page from the last call, act on the user's tab when asked.
const p = await browseros.page(7);            // notice appended: "page 7 belongs to the user"
await p.getByLabel('Search').fill('invoice 2026-09');
await p.keyboard.press('Enter');
await p.getByText('1 result').waitFor();
return await p.getByRole('row').nth(1).innerText();
```

```js
// 3. Fails: wrong accessible name.
const p = await context.newPage();
await p.goto('https://example.com');
await p.getByRole('button', { name: 'Submit' }).click();
```

Result of 3 (`ok:false`): `TimeoutError: locator.click: Timeout 10000ms
exceeded. Call log: - waiting for getByRole('button', { name: 'Submit' })`
with `step.api = "locator.click"`, `step.pageId`, `logs`, and the audit shows
`context.newPage`, `page.goto` (ok) and `locator.click` (error) as children.

## 3. Architecture

```
agent ──MCP──▶ ClawMcpService.call_tool ──▶ dispatch_tool_call (guards ▸ execute ▸ effects ▸ observers)
                                               │ execute_with_cancellation: ToolCtx{ inner_call_hook: ScriptInnerCallHook,
                                               │                                     script_runtime: SidecarScriptRuntime }
                                               ▼
                             browseros_mcp::tools::playwright::handler ──▶ ctx.script_runtime.run(ScriptRunRequest)
                                               │
        ┌──────────────────── claw-server-rust process ─────────────────────┐
        │ services/playwright/runtime.rs  ──stdio NDJSON──▶ supervisor.rs ──spawn──▶ browseros-playwright (Bun+playwright-core)
        │        ▲ CallBegin/CallEnd/RunLog/RunDone/HostRequest              │            │ chromium.connectOverCDP(ws://127.0.0.1:R/cdp/<tok>)
        │        └── InnerCallHook::{authorize,record,on_page_created}      │            ▼
        │ services/playwright/relay.rs  ◀──ws per convo──────────────────────┘   (target filter, createTarget→background, claim)
        │        └── ws to browser /devtools/browser/<id>  ◀── same CDP the BrowserService uses, a second client
        └─────────────────────────────────────────────────────────────────────┘
```

### Modules and seams

**M1. `ScriptRuntime` port** (seam, `crates/browseros-mcp/src/framework.rs`).
Same shape as `InnerCallHook`: the crate cannot see the host, so the host
injects the runtime through `ToolCtx`.

```rust
pub struct ScriptRunRequest<'a> { pub code: &'a str, pub timeout: Duration,
    pub helpers: &'a [HelperSource], pub cancel: CancellationToken }
pub struct ScriptRunOutcome { pub ok: bool, pub value: Option<Value>, pub logs: Vec<String>,
    pub error: Option<ScriptError>, pub pages: Vec<Value> }
pub struct ScriptError { pub name: String, pub message: String, pub step: Option<Value> }
pub trait ScriptRuntime: Send + Sync {
    fn run<'a>(&'a self, req: ScriptRunRequest<'a>) -> BoxFuture<'a, Result<ScriptRunOutcome, ToolError>>;
}
// ToolCtx / BrowserToolOptions gain: pub script_runtime: Option<Arc<dyn ScriptRuntime>>
```

Deletion test: delete it and the `playwright` handler must reach the host's
sidecar through a name-sniffed bypass in `execute_with_cancellation`,
duplicating `execute_tool`'s cancellation and response building. Earns its
keep; one production adapter tonight (hypothetical seam) but the test fake is
the second, as with `InnerCallHook`.

**M2. `tools/playwright.rs`** (`crates/browseros-mcp/src/tools/`): `ToolDef`
via `def_with_output::<PlaywrightArgs, RunOutput>("playwright", …)`, handler
calls `ctx.script_runtime` or returns `ToolResult::error("playwright runtime
unavailable in this context")`. `RunOutcome`/`format_outcome`/log limits move
to `tools/script_outcome.rs` (mechanical, shared with `run.rs`). Deletion test:
pass-through by design; it *is* the tool.

**M3. `InnerCallHook` (reused, `script_hook.rs`)** gets a second adapter
caller. One addition so the sidecar can report by target rather than `u32`:

```rust
// crates/browseros-core/src/pages.rs
impl PageManager { pub async fn page_id_for_target(&self, target_id: &TargetId) -> Result<PageId, CoreError> } // wraps ensure_page_id_for_target
```

`SidecarScriptRuntime` resolves `targetId → PageId` with it, then calls the
unchanged `authorize`, `record`, `on_page_created`, `annotate_pages`,
`resolve_host`, `save_helper`… Two real adapters now (QuickJS bridge, sidecar
runtime): a real seam. REVISIT the architecture review's `ScriptDispatch`
refactor after both engines have run for a week.

**M4. Supervisor** (`apps/claw-server-rust/src/services/playwright/supervisor.rs`).

```rust
pub struct PlaywrightSidecar { … }
pub struct SidecarConfig { pub binary: PathBuf, pub dev_entry: Option<PathBuf>, pub idle_after: Duration,
    pub relay: Arc<CdpRelay>, pub crash_budget: (u32, Duration) /* 3 per 5 min */ }
impl PlaywrightSidecar {
    pub fn new(cfg: SidecarConfig) -> Arc<Self>;
    /// Spawns on first use; returns a live handle or the last crash reason.
    pub async fn ensure(&self) -> Result<SidecarHandle, SidecarError>;
    pub async fn stop(&self);                            // server shutdown: RunCancel all, close stdin, wait 2 s, kill
    pub async fn kill_and_restart(&self, why: &str);     // wedge/crash backstop
}
pub struct SidecarHandle { pub generation: u64 }
impl SidecarHandle {
    pub async fn request(&self, msg: HostToSidecar) -> Result<(), SidecarError>;
    pub fn subscribe(&self, run_id: &RunId) -> mpsc::Receiver<SidecarToHost>;
}
```

Process facts: `tokio::process::Command` (feature already on), `kill_on_drop
(true)`, stdin/stdout piped, stderr to `tracing`; env cleared except `PATH`
and `BROWSEROS_PLAYWRIGHT_PROTOCOL=1`; cwd = an empty temp dir; the sidecar
**exits on stdin EOF** (the cross-platform "die with the server" rule, since
`BrowserOSServerManager` SIGKILLs the server on restart). Binary located at
`config.resources_dir/bin/browseros-playwright[.exe]`; `config.dev_mode`
falls back to `bun apps/claw-playwright-sidecar/src/main.ts` (no compile in
dev). Crash: in-flight runs fail with `sidecar exited (code N)`; respawn is
lazy with 0.5 s→8 s backoff; over budget → tool error + run-failure telemetry
kind `sidecar_crash`. Idle: stopped 10 min after the last live session.
Deletion test: without it each call would spawn a process (~200 ms) and lose
the warm per-conversation `Browser`; restart policy would scatter into the
tool handler. Keeps.

**M5. Protocol** (`services/playwright/protocol.rs` ⇔
`apps/claw-playwright-sidecar/src/protocol.ts`; NDJSON over stdio, both
directions carry `id` for request/response, `protocolVersion: 1`):

```jsonc
// host → sidecar
{ "t": "hello",     "protocolVersion": 1, "serverVersion": "0.0.58" }
{ "t": "run.start", "runId": "01J…", "convo": "c-…", "relayUrl": "ws://127.0.0.1:53211/cdp/7f…",
                    "code": "…", "timeoutMs": 30000, "helpers": [{ "name": "…", "source": "…" }],
                    "pages": [{ "targetId": "…", "pageId": 7, "url": "…", "ownership": "mine" }], "lastPageId": 7 }
{ "t": "run.cancel", "runId": "01J…", "reason": "timeout" | "cancelled" }
{ "t": "convo.drop", "convo": "c-…" }                 // session ended: disconnect that Browser
// sidecar → host
{ "t": "hello.ack", "protocolVersion": 1, "playwright": "1.63.0", "runtime": "bun/1.3.6" }
{ "t": "call.begin", "runId": "…", "callId": 12, "api": "locator.click", "targetId": "…", "params": {...}, "fromHelper": false }
{ "t": "call.end",   "runId": "…", "callId": 12, "ok": false, "durationMs": 10004, "error": { "name": "TimeoutError", "message": "…" }, "resultTokens": 0 }
{ "t": "page.created", "runId": "…", "targetId": "…" }   // informational; the relay already claimed it
{ "t": "run.log",  "runId": "…", "line": "…" }
{ "t": "run.done", "runId": "…", "ok": true, "value": …, "logs": [...], "error": null, "pages": [{ "targetId": "…" }] }
{ "t": "host.req", "id": 3, "runId": "…", "op": "pages.list" | "page.adopt" | "helpers.save" | "helpers.list" | "helpers.read" | "snapshot", "args": {...} }
```

Host answers `host.req` with `{ "t": "host.res", "id": 3, "ok": true, "value": … }`.
Deletion test: types only; deleting it means two hand-parsed JSON dialects.

**M6. CDP relay** (`services/playwright/relay.rs`). The module that makes B
correct rather than merely working. One loopback listener
(`tokio::net::TcpListener` port 0, `tokio_tungstenite::accept_hdr_async`;
`tokio-tungstenite` is already a dev-dep and a `browseros-cdp` dep), one
upstream browser socket per conversation (`browseros_cdp::discovery` gives the
`/devtools/browser/<id>` URL), no axum change.

```rust
pub struct CdpRelay { … }
pub struct RelayGrant { pub url: String /* ws://127.0.0.1:P/cdp/<token> */ }
pub struct RelayHooks {
    /// Called with the new target *before* the createTarget response is forwarded; awaited.
    pub on_created: Arc<dyn Fn(ConvoId, TargetId) -> BoxFuture<'static, ()> + Send + Sync>,
    pub allowed: Arc<dyn Fn(ConvoId) -> BoxFuture<'static, BTreeSet<TargetId>> + Send + Sync>, // owned_pages → target ids
}
impl CdpRelay {
    pub async fn start(cdp_port: u16, tab_registry: Arc<TabRegistry>, hooks: RelayHooks) -> Result<Arc<Self>, AppError>;
    pub async fn grant(&self, convo: &ConvoId) -> RelayGrant;     // one token per convo; reused while live
    pub async fn adopt(&self, convo: &ConvoId, target: &TargetId); // attach a foreign tab (browseros.page)
    pub async fn drop_convo(&self, convo: &ConvoId);              // closes the ws: every pending Playwright promise rejects
}
```

Rules, all on the root session (no `sessionId`): `Target.setAutoAttach` is
answered `{}` locally and never forwarded; the relay attaches allowed targets
itself with `Target.attachToTarget {targetId, flatten:true}` (CRBrowser's
`_onAttachedToTarget` creates a `CRPage` regardless of `waitingForDebugger`,
verified in `coreBundle.js` 38423). `Target.createTarget` is forwarded with
`background: true` added; on the response the relay attaches the target,
awaits `on_created` (host claims + groups + opens the session-tab window),
then forwards the response, so `newPage()` resolves only after the claim.
Popups: `TabRegistry`'s `Target.targetCreated` stream (it already runs
`setDiscoverTargets`) with `openerId` in the allowed set → attach. Rejected
with a CDP error: `Target.createBrowserContext`, `Browser.close`,
`Target.attachToTarget`/`closeTarget` on non-allowed targets. Per-page
sessions (iframes, workers, `Target.setAutoAttach` inside a page session)
pass through untouched. Deletion test: without the relay Playwright's
`setAutoAttach {autoAttach:true, waitForDebuggerOnStart:true}` attaches a
debugger to **every** tab, so the fork's `ShouldSuppressAutomationFocus`
(`IsDebuggerAttached`) suppresses the *user's* own popups and `window.focus()`,
every user tab pays `Runtime/Network/Page.enable`, and claiming races the
script. Keeps.

**M7. `SidecarScriptRuntime`** (`services/playwright/runtime.rs`, adapter of
M1): builds `run.start` from `ToolCall` (identity, `owned_pages` → page index
with URLs, `preloaded_helpers`), streams events into `InnerCallHook`
(`call.begin → authorize(page)`, `call.end → record(InnerCallRecord{ method:
api, page, args: params, is_error, duration_ms, output_token_estimate:
resultTokens })`), answers `host.req` (`pages.list → session.pages.list +
annotate_pages`; `page.adopt → relay.adopt + authorize`; helpers → hook).
Cancellation/timeout: `run.cancel` → 1 s → `relay.drop_convo` (spike: a
closed connection rejects the pending `locator.click` cleanly and the
`Browser` stays usable for the next connect) → 2 s → `kill_and_restart`
(spike: SIGKILL in 500 ms, the tab survives in the browser, host closes it).

**M8. Sidecar** (`apps/claw-playwright-sidecar/`, Bun workspace app;
`playwright-core` and `playwright` pinned `1.63.0` exact):
`src/main.ts` (stdio loop, `hello`), `src/browsers.ts` (convo →
`chromium.connectOverCDP(relayUrl)`, `contexts()[0]`), `src/instrument.ts`
(`browser._connection._instrumentation.addListener({ onApiCallBegin(apiZone,
{type, method, params}), onApiCallEnd(apiZone) })` → `call.*`; `targetId`
per page cached from `context.newCDPSession(page)` + `Target.getTargetInfo`,
spike-verified 33 ms; overrides `browser.newContext`), `src/sandbox.ts`
(`vm.createContext(globals, { codeGeneration: { strings: false, wasm: false }
})`, `runInContext` with a 250 ms sync `timeout` per slice re-armed on each
awaited call, console capture with `run`'s 1 000-line / 1 MB limits),
`src/membrane.ts` (every host function reaches the script as a
**sandbox-realm** closure created by `vm.runInContext('f => (...a) => f(...a)')`
and every returned object as a `Proxy` with `getPrototypeOf` → sandbox
prototypes; this closes the spike's `page.goto.constructor('return
process')()` escape), `src/helpers.ts`, `build.ts` (Bun.build plugin with the
spike's three literal rewrites and its `version drift` throw; `external:
['chromium-bidi','ws']`; `--compile --target=<bunTarget>` from
`packages/build-server-tools/src/targets.ts`). Deletion test for the
membrane: without it `process.getBuiltinModule('fs')` is one property access
away (measured). Keeps.

### Where each guarantee gets its data

- **Child audit rows**: `call.end` → `ScriptInnerCallHook::record` →
  `RecordToolDispatchInput{ parent_dispatch_id: call.dispatch_id }` +
  `persist_screenshot` per page-targeting call, unchanged code path.
- **Claim + tab group**: relay `on_created` → `page_id_for_target` →
  `ScriptInnerCallHook::on_page_created` → `record_new_page` +
  `run_tab_group_work`, before `newPage()` returns. Popups: `TabRegistry
  .inherit_owner` (exists) plus the relay's opener rule.
- **Ownership notice**: `call.begin` → `authorize` fills `foreign_pages`;
  `effects/page_ownership_notice::run_script_notice` reads it.
- **Focus**: `createTarget` rewrite (spike: `changed:false`); `bringToFront`
  and `window.focus()` are funnelled through `Browser::ActivateContents`,
  which the fork suppresses for attached tabs (spike: no selection change).
- **Screenshots**: `persist_screenshot` on each child row (existing).
- **Replay**: `recorder.content.ts` records every document; attribution is
  the session-tab window opened by `record_new_page`. Nothing new.
- **Run-failure telemetry**: `observers/run_failure.rs` `REPORTED_TOOL`
  becomes `["run", "playwright"]`; `script_fingerprint::SDK_SURFACE` gains
  the Playwright names (`page`, `context`, `locator`, `getByRole`, `expect`…).
- **Cancellation / 30 s**: `ctx.cancel` and the deadline race in M7.
- **Session end**: the retained-group closure in `api/mcp/mod.rs` also calls
  `state.playwright.relay.drop_convo(&key)` and sends `convo.drop`.

### Packaging, five targets

| Target | Bun target | Sidecar size | Sign |
|---|---|---:|---|
| darwin-arm64 | `bun-darwin-arm64` | 64.1 MB measured | `codesign --options runtime` + `browseros-executable-entitlements.plist` (already has `allow-jit`) |
| darwin-x64 | `bun-darwin-x64` | ASSUMPTION ~66 MB | same |
| linux-x64 / arm64 | `bun-linux-x64-baseline` / `bun-linux-arm64` | ASSUMPTION ~95 MB each | none today |
| windows-x64 | `bun-windows-x64-baseline` | ASSUMPTION ~110 MB | `sign_with_codesigntool` (existing) |

The Bun compile pipeline (`packages/build-server-tools/src/compile.ts`),
target table, ad-hoc signing and Windows exe patching already exist for
`browseros_server`. Changes: `scripts/build/claw-server-rust/compiler.ts`
also compiles the sidecar and stages `resources/bin/browseros-playwright`;
`descriptor.ts` `expectedArtifactFiles` gains it;
`bos_build/products/browserclaw/product.py` gains a `macos_binaries` entry
`"browseros-playwright": SignSpec("browseros_playwright", "runtime",
"browseros-executable-entitlements.plist")` and the `.exe` in
`windows_binaries`. OTA ships the whole `resources/` zip, so server and
sidecar move together; `hello` rejects a `protocolVersion` mismatch. The
sidecar spawns on the first `playwright` call (≈160 ms + 34 ms attach).

### Security

The script is untrusted. Layers: (1) process boundary, clean env, empty cwd,
no secrets; (2) `vm` with string code generation off plus the membrane; (3)
`--smol`/`--max-old-space-size=256`, a host RSS watchdog (`ps -o rss=` every
2 s, kill above 512 MB), the 30 s cap with 250 ms sync slices; (4) macOS:
`sandbox-exec` profile allowing only the sidecar binary, system libraries,
`<browserclaw_dir>/tool-output` and loopback network; Linux: `bwrap
--unshare-net` when present; Windows: layers 1–3 (REVISIT: Job object).
`context.request` (Node-side fetch with the profile's cookies) is audited as
`apiRequestContext.*` and is the one net capability worth a REVISIT.

## 4. Work pieces

| # | Goal | Owns (no overlap) | Interface honoured | Verify | Size | Par. |
|---|---|---|---|---|---|---|
| P0 skeleton | Every file/type/fn compiles; `playwright` tool listed and returns "runtime unavailable"; sidecar prints `hello.ack` | `framework.rs` (`ScriptRuntime`, `ToolCtx` field), `tools/playwright.rs`, `tools/script_outcome.rs` (move), `tools/mod.rs` catalog line, `run.rs` imports, `services/playwright/{mod,protocol,supervisor,relay,runtime}.rs` stubs, `services/mod.rs`, `app.rs` field `playwright`, `dispatch.rs` (`ARBITRARY_SCRIPT_TOOLS`, `ToolCtx` wiring), `observers/run_failure.rs` const, `api/mcp/mod.rs` hook line, `pages.rs` `page_id_for_target`, `apps/claw-playwright-sidecar/{package.json,tsconfig.json,src/*.ts stubs,build.ts}` | §3 signatures verbatim | `cargo build -p claw-server-rust && cargo clippy --package claw-server-rust --all-targets --locked -- -D warnings && cargo fmt --all --check && bun run check`; `bun apps/claw-playwright-sidecar/src/main.ts <<< '{"t":"hello","protocolVersion":1}'` prints `hello.ack` | M | — |
| P1 protocol | serde/TS types + round-trip tests | `protocol.rs`, `protocol.ts` | M5 | `cargo test -p claw-server-rust protocol && bun test protocol` | S | ∥ |
| P2 supervisor | spawn, hello, stdin-EOF exit, crash budget, idle stop | `supervisor.rs` | M4 | `cargo test … supervisor` with `sh -c` fake sidecar (unix) | M | ∥ |
| P3 relay | listener, per-convo upstream, filter rules, `createTarget` rewrite, `on_created` ordering | `relay.rs` | M6 | `cargo test … relay` against an in-process fake browser ws (tokio-tungstenite dev-dep): asserts setAutoAttach answered locally, background injected, foreign attach rejected, response after `on_created` | L | ∥ |
| P4 runtime adapter | events → hook, host.req answers, cancel ladder | `runtime.rs` | M1/M3/M7 | `cargo test … runtime` with a channel `FakeSidecar`: child rows linked to parent (mirror `record_writes_child_row_linked_to_parent`), foreign page noticed, timeout → drop → kill | M | ∥ |
| P5 sidecar | browsers, instrumentation, sandbox, membrane, helpers, expect | `src/{browsers,instrument,sandbox,membrane,helpers,main}.ts` | M5/M8 | `bun test` fake host over stdio; with `BROWSEROS_BINARY` set, isolated browser: goto/click/expect pass, `page.goto.constructor` throws EvalError | L | ∥ |
| P6 packaging | compile five targets, stage, sign tables | `build.ts`, `scripts/build/claw-server-rust/compiler.ts`, `descriptor.ts`, `bos_build/products/browserclaw/product.py` | §3 packaging | `bun scripts/build/claw-server-rust.ts --targets darwin-arm64` yields `resources/bin/browseros-playwright`; `./browseros-playwright --version` prints protocol 1; `stat` size logged | M | ∥ |
| P7 text | prompt, skill, tool `DESCRIPTION` | `prompt.rs`, `skills/browseros-neo/SKILL.md`, the `DESCRIPTION` const block in `tools/playwright.rs` | §2 | `cargo test -p claw-server-rust prompt` | S | ∥ |
| P8 contract | real-browser conformance | `contracts/claw-mcp/tests/cases-playwright.ts`, line in `cases.ts` | §2 envelope | `BROWSEROS_BINARY=… bun contracts/claw-mcp/tests/run.ts --smoke` | M | after P4+P5 |
| P9 cockpit | `NOTABLE_TOOLS` + group children under `parentDispatchId` | `apps/claw-app/components/audit/Timeline.tsx` | `ToolDispatchRow` | `bun run check`; open a task with a `playwright` dispatch | S | ∥ |

P1–P7 and P9 run in parallel after P0; P8 last.

## 5. Top five failure modes

1. **Playwright/Bun pin drift.** The three literal rewrites and the
   `_instrumentation` seam are internals. Mitigation: exact version pin,
   `build.ts` throws on drift, P8 in CI on macOS, and the Node SEA path
   documented as the escape (`node --experimental-sea-config`, +~90 MB).
2. **Relay breaks a Playwright expectation** (workers, OOPIF, popups,
   `waitForEvent('page')`). Mitigation: P3 fixtures for each; env
   `BROWSEROS_PLAYWRIGHT_RELAY=passthrough` for triage (attach-all, still
   background-rewritten).
3. **Sandbox escape** past the membrane. Mitigation: layers in §3; treat any
   new escape as a P5 test; no secrets reachable from the sidecar process.
4. **Wedged sidecar on cancel.** Mitigation: the ladder (cancel → drop relay
   → kill), each step spike-verified; crash budget prevents loops.
5. **Claim/focus race** (a page is used before the host claimed it, or a
   script forces focus). Mitigation: claim inside the relay before the
   `createTarget` response; `TabRegistry` reconciliation is idempotent; fork
   suppresses `bringToFront` (measured).

## 6. If I only had 4 hours

Dev mode only, macOS: P0 minus relay; supervisor spawns `bun
apps/claw-playwright-sidecar/src/main.ts`; sidecar connects straight to
`http://127.0.0.1:<cdp>` and overrides `context.newPage` with
`browser.newBrowserCDPSession()` → `Target.createTarget {url, background:true}`
→ `context.waitForEvent('page')` matched by `targetId`, then a `page.created`
host round-trip before returning; instrumentation → child audit rows; `expect`
from `playwright/test`; no membrane. Caveat to state in the morning:
Playwright attaches to every tab in this mode, so the user's own tabs inherit
the never-steal-focus rule while a session is live.

## 7. Decisions and assumptions

DECIDED: new tool `playwright`, `run` untouched.
WHY: additive-only order and STATUS.md; no schema change to `run`.
REVISIT: fold into `run` behind `api: "playwright"` after a week of sessions.

DECIDED: `BrowserContext` = agent session, via relay filtering, not JS filtering.
WHY: the relay is the only place that can stop debugger attachment to user tabs.
REVISIT: if P3 fixtures show a Playwright feature that needs attach-all.

DECIDED: one sidecar process, one `Browser` per conversation inside it.
WHY: warm reuse; per-convo connections make `drop_convo` the cancel and teardown primitive.
REVISIT: one process per conversation if the membrane proves insufficient.

DECIDED: Bun-compiled, not Node SEA.
WHY: 64 MB vs ~90 MB, existing compile/sign pipeline, measured working.
REVISIT: any Bun incompatibility on Linux/Windows in P6.

DECIDED: reuse `InnerCallHook` plus `page_id_for_target`; defer `ScriptDispatch`.
WHY: two adapters tonight with zero host-side choreography changes.
REVISIT: after both engines run, per the architecture review.

DECIDED: 30 s cap stays; deadline-aware Playwright default timeouts.
WHY: MCP client timeouts; agents already chunk.
REVISIT: `playwright_job` tool.

ASSUMPTION: `expect` from `playwright/test` works outside the test runner (its `expectConfig().testInfo` guard is `undefined`-tolerant); verify in P5 with one `toBeVisible`.
ASSUMPTION: Chromium delivers `Target.attachedToTarget` for a relay-issued `attachToTarget` before the `createTarget` response is forwarded; the relay orders it explicitly.
ASSUMPTION: Linux and Windows Bun sizes and the `bun-*` cross-compile from macOS work for this bundle (only darwin-arm64 measured).
ASSUMPTION: `sandbox-exec` remains functional on macOS 26 for a Bun binary with JIT entitlements.
ASSUMPTION: bundling `playwright` (5 MB unpacked) adds under 10 MB to the compiled sidecar.
ASSUMPTION: the compressed resources zip grows by roughly 25 MB per platform (Bun binaries compress about 2.5×).
ASSUMPTION: `Target.detachFromTarget` is never needed for user tabs because the relay never attaches them; the momentary attach window of design R1 is avoided entirely.
