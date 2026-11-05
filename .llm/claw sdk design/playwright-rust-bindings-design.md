# Playwright API, Rust operations, and JavaScript bindings — proposed design

Updated: 2026-11-04. Worktree branch: `feat/playwright-bindings-design`. Parent: [Claw SDK — working design](claw-sdk-design.md).

**Status: proposal for review. Nothing here is approved.** Choices are labeled **Recommendation** or **Unresolved**. All snippets are illustrative and **untested**. Facts are labeled by how they were checked: *repo* (read in this repository), *docs* (current primary documentation), *note* (a research note in Appendix A), or *unverified* (prior knowledge, still to be checked; see Appendix D).

Code paths are relative to `packages/browseros-agent/` at commit `ee2c29451`. Playwright facts refer to **Playwright 1.64.1**, the latest stable release, published 2026-10-14 (*note*: pw-page-frame-context, pw-network-misc-testrunner).

## Requirements

The parent design's requirements, restated without change:

1. **Clear ownership boundary.** Distinguish BrowserOS execution failures from failures in the customer's agent or code.
2. **Better browser controls.** Give customer agents capable, reliable browser controls.
3. **Room for Jev-like operations and reusable skills.** The foundation must let them build on the same layer. Their details and V1 scope remain open.

## HLD

### Agreed constraints

These come from the parent design's agreed direction. They are restated here, not newly approved.

- **One tool.** The customer adds `neo.run`, which takes JavaScript plus a target tab or session reference.
- **Stateless JavaScript.** Every call starts with fresh variables, functions, objects, and globals. Chromium keeps tabs, navigation, cookies, and logins; the SDK is its sidecar.
- **Familiar Playwright.** Support a large subset of Playwright APIs with their expected behaviour. This is a design choice, not a separate requirement.
- **New codebase, Rust over CDP.** A new SDK codebase and binary implement browser operations in Rust that drive Chromium through CDP. `claw-app` and `claw-server` continue independently.
- **BrowserOS extras.** Non-Playwright operations live in the same code mode and are described by `.d.ts`.
- **REST, separately.** Screencasts, screenshots, recordings, tool traces, and audit traces are REST endpoints outside `neo.run`.
- **Engine open.** QuickJS (`rquickjs`) and V8 (`deno_core`) are both explored here. Neither is selected.
- **Two kinds of return.** A Playwright method's return type is separate from the `neo.run` result envelope. This document fixes neither a generic method type nor an envelope format.

### Scope and assumptions

Out of scope: the customer's agent loop, a plugin framework, the Playwright Test runner, launching browsers, and the REST endpoints themselves. This design only feeds them.

- **A1.** The SDK connects to an already-running BrowserOS Chromium over CDP, as today.
- **A2.** A target reference resolves to a stable page identity like today's `PageId` (`crates/browseros-core/src/types.rs:1-3`).
- **A3.** Agent-written scripts are untrusted. The runtime enforces memory, time, and host-access limits.

### The picture

```
customer agent (customer-owned loop)
   │  neo.run({ code, target })
   ▼
┌──────────────────────── SDK binary (new, Rust) ───────────────────────────┐
│ request ─► resolve target ─► RunScope (deadline, cancel, trace, handles)  │
│                 │                                                         │
│                 ▼                                                         │
│   fresh JS realm   (engine adapter: QuickJS or V8)                        │
│     prelude.js = Playwright-shaped classes + browseros.* extras           │
│     user code  = body of one async function                               │
│                 │   op(name, args) → Promise        ◄── the only seam     │
│                 ▼                                                         │
│   Rust browser ops (engine-agnostic crate)                                │
│     locator engine · actionability · input · navigation/waits · events    │
│     network · files · screenshots/PDF · evaluate · context                │
│                 │                                                         │
│   CDP client ── WebSocket ──► BrowserOS Chromium                          │
│   trace recorder (every op) ──► store ──► REST: traces, screenshots, …    │
└───────────────────────────────────────────────────────────────────────────┘
JS state dies with the call. Chromium state stays.
```

### Three layers, one narrow seam

| Layer | Owns | Does not own |
|---|---|---|
| JS facade (`prelude.js` + `.d.ts`) | Playwright names, argument shapes, synchronous builders such as `getByRole`, wrapping returned handles in classes, turning error data into JS errors | Browser behaviour, waiting, retries |
| Engine adapter (QuickJS or V8) | Fresh realm, `op()` bridge, Promise ↔ Rust future, JSON and byte conversion, memory and time limits | Browser semantics |
| Rust ops (engine-agnostic) | Locating, waiting, actionability, input, CDP, events, error classes, traces | Any JS engine type |

**Recommendation:** keep the adapter thin. Errors, handles, and events cross the seam as plain data. Then the same prelude and the same Rust ops run on either engine, and one conformance suite tests both. The engine choice stays reversible.

### One call, start to finish

1. **Admit.** Validate the request, resolve the target to a page, compute the deadline, create a `RunScope`.
2. **Fresh realm.** Install `op`, console, and timers; evaluate the prelude; wrap the code in an async function; call it with the agreed globals (`page`, `context`, `browser`, `browseros`; names unresolved).
3. **Drive.** Run the event loop until the script's promise settles, the deadline passes, or the caller cancels. Ops run as Rust futures.
4. **Convert.** Turn the returned value, logs, and error into the envelope. The format is unresolved.
5. **Tear down, always.** Cancel pending ops. Undo call-scoped browser state: listeners, routes, remote-object groups. Drop the realm. Leave tabs, cookies, and logins alone.

### Where today's code fits

Today's `run` tool already creates fresh JavaScript state per call. It builds a new QuickJS runtime and context each time, with a 64 MiB memory limit, a 512 KiB stack, and an interrupt for deadline or cancel (`crates/browseros-mcp/src/tools/run.rs:33-34`, `run.rs:806-816`). Its API is ref-based (`page.click("e12")`) and explicitly reports locators, frames, routes, and events as absent (`run.rs:201-235`). The new SDK can reuse logic and patterns from the lower Rust layers and this runtime, not the current JavaScript API. Reusing logic is not a decision to depend on the old crates: the parent asks for a new codebase, and whether the new crates import, copy, or rewrite that code is **unresolved**.

## Misc LLD points

1. **Page identity.** A JS `Page` holds an opaque reference. Rust resolves it on every op and re-checks admission, as tools do today (`crates/browseros-mcp/src/framework.rs:600-612`, `crates/browseros-core/src/target.rs:1014`). `PageId` stays stable when Chromium rebinds the tab (`types.rs:1-3`), but it is a process-local counter that restarts with the process (`crates/browseros-core/src/pages.rs:169-170`). Today's clients continue a task with an 8-hex session handle (`apps/claw-server/src/api/mcp/session_handle.rs:31-32`). **Unresolved:** the request field and a page reference that survives an SDK restart (parent open item).
2. **Synchronous Playwright members.** About 30 Page, Frame, and context members, such as `url()`, `frames()`, and `pages()`, return without a Promise; so do 62 of 149 members of the network and event classes (*note*). **Recommendation:** Rust keeps a per-page mirror (URL, frame tree, closed flag, page list) fed by CDP events; the facade reads it through one synchronous host function. Event objects (Request, Response, Dialog, Download, ConsoleMessage) carry their own fields.
3. **Call-scoped versus browser state.**

   | Kind | Members | Rule |
   |---|---|---|
   | Call-scoped | `on/once/off`, `waitForEvent`, `route/unroute` (when added), `setDefaultTimeout`, `exposeFunction`/`exposeBinding` (if added) | Removed at teardown. A paused request is continued before its route is removed. |
   | Browser state (persists by design) | navigation, storage, opened and closed tabs, downloaded files | Kept in Chromium. |
   | Persists and changes the user's profile, or exposes secrets | `cookies`, `addCookies`, `clearCookies`, `storageState`, `grantPermissions`, `setGeolocation`, `setExtraHTTPHeaders`, `setViewportSize`, `emulateMedia`, `addInitScript` | Policy-gated. **Unresolved:** allow, scope to the call, or defer each one. |

4. **Timeouts.** Each method keeps its own timeout option. Used as a library, Playwright defaults to 30 s for most operations; the "0 = no timeout" wording in the API pages describes the test runner (https://playwright.dev/docs/library#key-differences). Effective timeout = min(method timeout, remaining run budget). The run budget defaults to 60 s and callers may only shorten it (`crates/browseros-mcp/src/run_policy.rs:12-18`, `run_policy.rs:40-54`). Timeouts throw an error named `TimeoutError`.
5. **Cancellation.** A cancel stops waits and interrupts JavaScript (QuickJS interrupt handler; V8 `terminate_execution`). Input already sent cannot be undone.
6. **Concurrency.** Within a call, ops on different pages run in parallel. **Recommendation:** Rust serializes input per page with a lock; reads may overlap. Across calls, each call has its own realm; same-page conflicts follow the host's existing guard hook (`framework.rs:231-243`).
7. **Retries.** Locator actions retry inside the op until their timeout, as Playwright does. An action whose delivery is unknown is never repeated automatically (existing rule: `crates/browseros-core/src/error.rs:76-80`).
8. **Dialogs. Unresolved.** A `dialog` listener must accept or dismiss, or the page freezes; with no listener, Playwright dismisses dialogs automatically (https://playwright.dev/docs/api/class-page#page-event-dialog). Today's run returns `{ dialogOpen: true }` and waits for an explicit accept or dismiss (`run.rs:75`). Between calls no listener exists, so a dialog opened by a page timer stays open until the next call. **Recommendation:** listeners behave like Playwright. With no listener, do not auto-dismiss; reject the action with a dialog error that describes the dialog.
9. **Click recovery. Unresolved.** Today's click falls back once to a DOM press when the target is covered (`run.rs:70`; commit `ee2c29451`). Playwright instead waits for actionability and times out unless `force` is set. **Recommendation:** Playwright-named methods keep Playwright behaviour; the BrowserOS fallback becomes an explicit extra.
10. **`evaluate` world. Unresolved.** Playwright's `page.evaluate` runs in the page's main world. Today's SDK renamed it `evaluateInPage` so agents know the page can watch it (`run.rs:209`). **Recommendation:** provide `page.evaluate` with Playwright behaviour and record each call in the trace. Locator internals always run in an isolated world.
11. **Secure fill.** Secrets never enter JavaScript. A BrowserOS extra such as `browseros.credentials.fill(locator, { id })` resolves and types the secret in Rust; JavaScript receives only a status. Today nothing stops a script from reading the filled value back afterwards (*note*: core-map). **Unresolved:** block or redact reads such as `inputValue()` and `evaluate()` on fields filled from a secret.
12. **Traces.** Every op records run ID, op name, page, redacted arguments, timing, outcome, and error code at the one dispatch point, like today's `InnerCallRecord` (`framework.rs:291-314`). Today's store keeps arguments up to 4 KiB (`apps/claw-server/src/db/audit_log.rs:30`) and redacts script source (`apps/claw-server/src/api/mcp/observers/audit.rs:124-131`). REST trace endpoints read that store. Today's screenshots and recordings follow only tabs the session owns (`apps/claw-server/src/services/cockpit/visual.rs:385-390`), so a call on a user's tab would not appear. **Unresolved:** whether that should change.
13. **`.d.ts`.** The agent reads `.d.ts`; it does not implement anything. **Recommendation:** a hand-maintained subset of Playwright's published types plus a `browseros` namespace. A test checks that each declared member exists in the prelude and maps to a registered op.
14. **Version pinning. Recommendation:** pin Playwright 1.64.x behaviour and types (`playwright-core@1.64.1`, https://unpkg.com/playwright-core@1.64.1/types/types.d.ts ) and state the version in the `.d.ts` header.
15. **Node-only types.** Playwright signatures use `Buffer`, `AbortSignal` (the `signal` option, 1.62), `FormData`, and `Readable`; several members return a `Disposable` for `await using` (1.59) (*note*). Neither engine has Node built-ins. **Recommendation:** the prelude ships a `Buffer` shim on `Uint8Array`, plus `AbortController`/`AbortSignal`; `Readable` APIs (`createReadStream`) are not offered. deno_core switches on V8's explicit resource management, so `await using` works there (*note*: engine-deno-core); QuickJS support is unverified.
16. **URL policy in the op.** Today's scheme guard refuses `javascript:`, `file:`, and `data:` URLs, but only for the `navigate` tool (`apps/claw-server/src/api/mcp/guards/navigate_scheme.rs:7`, `navigate_scheme.rs:12`). A `run` script's `page.goto` goes straight to navigation (`run.rs:1393-1403`). **Recommendation:** put URL policy inside the Rust `page.goto` op, so every path is covered.
17. **Call limits.** The claw-server map found no cap on concurrent `run` calls (*note*). **Recommendation:** cap concurrent calls per process and per page.
18. **Refs between calls.** Playwright 1.64 adds `page.getByRef(ref)`, which resolves `e2`-style refs from the latest `page.ariaSnapshot({ mode: 'ai' })` (https://playwright.dev/docs/api/class-page#page-get-by-ref). In a stateless runtime that ref table must live outside JavaScript: in the page's isolated world (option A below) or in Rust, as today's snapshot refs already do.

## Design of individual components

### Playwright API surface

**What counts.** The browser library is `playwright-core`, which `playwright` re-exports. The test runner is `playwright/test`, and `@playwright/test` only re-exports it (https://playwright.dev/docs/library ; package files in the pw-network-misc-testrunner note). The library's types contain no `expect` and no assertion classes; `playwright.errors` holds only `TimeoutError`. Only the library is in scope. Runner-only APIs — `test`, fixtures, `expect` and its assertion classes (`LocatorAssertions`, `PageAssertions`, `APIResponseAssertions`, `GenericAssertions`, `SnapshotAssertions`), `TestInfo`, `TestConfig`, reporters — are not provided.

**Assertions as waits. Unresolved.** Agents often write `await expect(locator).toBeVisible()`. Some assertions already have library twins: `toBeVisible`, `toBeHidden`, `toBeAttached` ≈ `locator.waitFor`, and `toHaveURL` ≈ `page.waitForURL`. Candidates worth adding as BrowserOS waits: `toContainText`, `toHaveText`, `toHaveCount`, `toBeEnabled`, `toHaveValue`, `toHaveTitle` (*note*). Screenshot and snapshot-file assertions are excluded; they need the runner's stored files.

**Proposed initial subset (recommendation, not approved).** Appendix B lists every proposed method with its Playwright return type, JS binding, Rust responsibility, and reuse status.

| Family | Proposed for V1 | Deferred | Not applicable |
|---|---|---|---|
| Browser | `contexts`, `version`, `isConnected` | `newContext`, `newPage` | `close`, `browserType`, `startTracing`, `newBrowserCDPSession` |
| BrowserContext | `pages`, `newPage`, `browser`, `request`, `waitForEvent('page')`, `on('page')`, `setDefaultTimeout` | policy-gated: `cookies`, `addCookies`, `clearCookies`, `storageState`, `setStorageState`, `grantPermissions`, `clearPermissions`, `setGeolocation`, `setExtraHTTPHeaders`, `addInitScript`, `credentials`; call-scoped later: `route`, `exposeFunction`, `exposeBinding`; `newCDPSession` | `close`, `setOffline`, `routeFromHAR`, `tracing`, `clock`, `serviceWorkers`, `backgroundPages` (deprecated) |
| Page | navigation, `locator` + `getBy*`, `getByRef`, `frameLocator`, frames, waits, events, `evaluate`, `ariaSnapshot`, `screenshot`, `pdf`, `keyboard`, `mouse`, `touchscreen`, `title`, `url`, `content`, `setContent`, `close`, `opener`, `setDefaultTimeout`; selector-string shortcuts (`page.click(selector)` …) and `$eval`/`$$eval` as thin wrappers | `route`, `addInitScript`, `exposeFunction`, `setViewportSize`, `emulateMedia`, `addLocatorHandler`, `routeWebSocket`, `$`, `$$`, `waitForSelector`, `evaluateHandle`, `localStorage`/`sessionStorage`, `webmcp` | `pause`, `video`, `screencast`, `coverage`, `requestGC`, `routeFromHAR`; `accessibility` (removed in 1.57); `type` and `waitForNavigation` (deprecated) |
| Frame / FrameLocator | Page-like read, locate, evaluate, and wait members; `name`, `parentFrame`, `childFrames`, `isDetached`, `page`; `FrameLocator.owner` | `frameElement` (returns a handle) | `FrameLocator.first/last/nth` (deprecated: use `locator.first().contentFrame()`) |
| Locator | actions, reads, state checks, `filter`/`and`/`or`/`nth`, `waitFor`, `evaluate`, `evaluateAll`, `screenshot`, `ariaSnapshot`, `setInputFiles`, `dragTo` | `elementHandle(s)`, `evaluateHandle`, `highlight` | — |
| Keyboard / Mouse / Touchscreen | all members | — | — |
| Events / network objects | `Request`, `Response`, `Dialog`, `Download`, `FileChooser`, `ConsoleMessage`, `WebError` | `Route`, `WebSocket`, `WebSocketRoute`, `Worker`, `WebStorage`, `Credentials`, `WebMCP` | `Video`, `Screencast`, `Coverage`, `Tracing`, `Clock`, `Debugger` |
| Request API | `APIRequestContext` `get/post/put/patch/delete/head/fetch`, `APIResponse` (policy-gated) | `storageState`, `dispose` semantics | — |
| Handles | — | `ElementHandle`, `JSHandle` (call-scoped only) | — |

**Why deferred or not applicable:**

- **User-owned browser.** We attach to the user's running BrowserOS. Launching, closing, and throwaway contexts are not ours (`BrowserType.launch`, `browser.close`, `context.close`).
- **Profile-changing or secret-revealing calls.** Cookies, storage state, permissions, geolocation, extra headers, viewport, and init scripts outlive the call or expose session secrets to agent code. They need a policy first.
- **Needs live JS between calls.** `exposeFunction`, `exposeBinding`, `route` handlers, and custom selector engines call back into JavaScript that is gone after the call. They can only be call-scoped.
- **Needs lossless events.** A route pauses real network requests. Under load the current CDP client's event ring can overflow; the subscriber sees a lag error, but the events are gone (Rust ops section). A lost `Fetch.requestPaused` leaves the request hanging. `route` waits for lossless event routing.
- **Test and debugging tools.** `pause`, `video`, `screencast`, `tracing`, `coverage`, `clock`, and `Debugger` belong to test runs. BrowserOS exposes screencasts, recordings, and traces through REST instead.
- **Handle APIs.** `ElementHandle` and `JSHandle` pin remote objects. In a stateless runtime they can live for one call only. Locators cover the same needs, so handles wait for demand.
- **Discouraged members.** The docs mark 30 Page members as discouraged: `$`, `$$`, `$eval`, `$$eval`, `waitForSelector`, `waitForTimeout`, and the selector-string actions and reads such as `page.click(selector)`. `page.type` and `page.waitForNavigation` are deprecated (*note*: pw-page-frame-context). Agents still write the discouraged ones often. **Recommendation:** ship the cheap non-handle wrappers in V1, skip the deprecated ones, and let telemetry decide the rest. Today's analytics already counts which absent members agents reach for (`apps/claw-server/src/analytics/error_allowlist.rs:101-132`).

### Rust browser operations

**Crate shape (illustrative names):**

```
crates/neo-ops/            engine-agnostic: RunScope, op registry, OpValue/OpError
  locator/                 selector chain, frame hops, strictness, waiting
  action/                  actionability checks + input dispatch
  nav/                     goto/reload/back/forward, load states, URL waits
  events/                  per-call subscriptions, waitForEvent
  mirror/                  per-page URL, frame tree, page list for synchronous getters
crates/neo-engine-quickjs/ adapter
crates/neo-engine-v8/      adapter (feasibility spike)
apps/neo-server/           binary: neo.run + REST
```

**Op contract (pseudocode, untested):**

```rust
/// Crosses the seam. Knows nothing about QuickJS or V8.
pub enum OpValue {
    Undefined,                // Promise<void>          -> undefined
    Json(serde_json::Value),  // number, string, bool, null, plain objects
    Bytes(bytes::Bytes),      // screenshot(), pdf(), body() -> Uint8Array
}                             // handles travel as tagged JSON: {"$handle":"Response","id":"r3",...}

pub struct OpError {
    pub name: &'static str,   // JS error name: "TimeoutError" or "Error"
    pub code: ErrorCode,      // stable: timeout, strict_mode_violation, target_closed, ...
    pub origin: Origin,       // Script | Page | Limits | Policy | BrowserOs
    pub message: String,      // "locator.click: Timeout 30000ms exceeded."
    pub call_log: Vec<String>,// "waiting for getByRole('button', { name: 'Continue' })"
    pub retryable: bool,
}

pub async fn dispatch(scope: &RunScope, name: &str, args: serde_json::Value)
    -> Result<OpValue, OpError>
{
    let op = REGISTRY.get(name).ok_or_else(|| OpError::unknown_op(name))?;
    let span = scope.trace.start(name, &args);          // feeds tool/audit traces
    let result = scope.race(op.run(scope, args)).await; // run deadline + cancel
    span.finish(&result);
    result
}
```

**Locator click op (pseudocode, untested):**

```rust
async fn locator_click(scope: &RunScope, a: LocatorArgs<ClickOptions>) -> Result<OpValue, OpError> {
    let page = scope.admit(&a.page).await?;                        // closed/out of scope -> error
    let mut progress = scope.progress("locator.click", a.options.timeout);
    progress.log(format!("waiting for {}", a.selector.describe()));
    loop {
        let frame = page.resolve_frames(&a.selector, &mut progress).await?;   // frame hops
        let el = match frame.query_strict(&a.selector.last_part()).await? {    // isolated world
            Matches::None => { progress.wait_retry().await?; continue; }       // timeout -> TimeoutError
            Matches::Many(n) => return Err(OpError::strict(&a.selector, n)),   // fails at once
            Matches::One(el) => el,
        };
        if !a.options.force {
            if let Err(why) = el.check_states(&[Visible, Stable, Enabled]).await? {
                progress.log(why); progress.wait_retry().await?; continue;
            }
        }
        el.scroll_into_view_if_needed().await?;
        let point = el.click_point(a.options.position).await?;   // page coordinates, OOPIF-translated
        if !a.options.force && !el.receives_events_at(point).await? {
            progress.log("another element would receive the click"); progress.wait_retry().await?; continue;
        }
        if a.options.trial { return Ok(OpValue::Undefined); }
        page.input_lock().await.mouse_click(point, &a.options).await?;     // move, press, release
        page.wait_for_initiated_navigation(&progress).await?;              // (unverified: Playwright rule)
        return Ok(OpValue::Undefined);
    }
}
```

**Op families and Rust responsibilities:**

| Family | Rust responsibility | Main CDP methods |
|---|---|---|
| Locate | Split the selector at frame hops; run the selector engine in the frame's isolated world; enforce strictness | `Runtime.callFunctionOn`, `Page.createIsolatedWorld`, `DOM.describeNode` |
| Act | Wait for actionability; scroll; compute the point; check the hit target; send input | `DOM.scrollIntoViewIfNeeded`, `DOM.getContentQuads`, `Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`, `Input.insertText`, `Input.dispatchTouchEvent` |
| Read | Text, attributes, values, states, boxes, ARIA snapshot | `Runtime.callFunctionOn`, `DOM.getBoxModel` |
| Navigate / wait | goto/reload/history; lifecycle events; URL and function waits | `Page.navigate`, `Page.lifecycleEvent`, `Page.frameNavigated`, `Page.navigateToHistoryEntry` |
| Events | Call-scoped subscriptions, queued delivery, `waitForEvent` | `Network.*`, `Page.javascriptDialogOpening`, `Runtime.consoleAPICalled`, `Runtime.exceptionThrown`, `Target.*` |
| Files | Upload via chooser or input; downloads to a managed folder | `DOM.setFileInputFiles`, `Page.setInterceptFileChooserDialog`, `Browser.setDownloadBehavior` |
| Capture | Screenshots and PDF as bytes | `Page.captureScreenshot`, `Page.printToPDF` |
| Evaluate | Playwright-compatible argument and result serialization | `Runtime.callFunctionOn` (`awaitPromise`, `serializationOptions`) |
| Request API | HTTP with the browser context's cookies | none for the request itself; `Storage.getCookies` for the jar |

All CDP methods above exist in the vendored protocol (`crates/browseros-cdp/protocol/protocol.json`). The generated types cover nine domains only — Accessibility, Bookmarks, Browser, DOM, History, Input, Page, Runtime, Target — and Fetch, Network, Emulation, and `Accessibility.queryAXTree` are untyped (`crates/browseros-cdp/protocol/surface.json`; *note:* cdp-map). Untyped calls still work through the client's generic send.

**CDP client constraints (*repo*, spot-checked from the cdp-map note):**

- All events from all pages share one 4,096-slot broadcast ring (`crates/browseros-cdp/src/client.rs:124`). Subscribers filter by method name only (`crates/browseros-cdp/src/events.rs:25-32`). When a subscriber falls behind, the ring overwrites old events and `recv` returns the broadcast `RecvError::Lagged` (`events.rs:25-29`). The loss is observable, not silent inside the client, but the events are gone, so every consumer must handle the gap.
- The lossless path, `events_targeted`, allows one subscriber per method per client (`client.rs:320-334`).
- One timeout applies to every command: 60 s (`client.rs:85`).
- The default reconnect policy exits the process (`client.rs:88`, `client.rs:713`).

**Recommendation:** give the SDK its own CDP client with lossless per-session event routing before shipping network events or `route`. Locators, input, and navigation do not depend on that change.

**Locator engine — the biggest choice.** Playwright resolves locators with a matcher injected into the page. CDP's DOM and Accessibility domains are a real alternative that runs no page script (option B). **Recommendation:** use the injected matcher (option A), because it gives Playwright fidelity.

| Option | How | Fit |
|---|---|---|
| **A. Vendor Playwright's injected script** | Embed the pinned release's selector engine and actionability helpers; run them in a BrowserOS isolated world (`crates/browseros-core/src/page_js.rs:95`, `page_js.rs:347`) | Closest to Playwright behaviour: role and name rules, text matching, strictness messages, ARIA snapshots, `aria-ref`. Source: `packages/injected/src/injectedScript.ts` and siblings, Apache-2.0, bundled by `utils/generate_injected.js` into `injectedScriptSource.ts` (https://github.com/microsoft/playwright/tree/f0eadb9783f97ea495cb8bd55860c5d5b99fe1c1/packages/injected/src ; *note*). Costs: an internal, unversioned API; bundle upkeep; licence notice duties in an AGPL-3.0 product (F2) |
| **B. Own engine over CDP** | CSS and text via page script; roles and names via Chromium's accessibility tree (`Accessibility.queryAXTree`) | Matches today's snapshot, which reads `Accessibility.getFullAXTree` (`crates/browseros-core/src/observer.rs:776`). Diverges from Playwright on matching rules, hidden elements, and regular-expression names |
| **C. Snapshot refs as locators** | `e12` → backend node | Refs are eager, go stale, and number duplicates instead of failing strictly. Playwright 1.64 has its own ref locator, `page.getByRef(ref)`, fed by `page.ariaSnapshot({ mode: 'ai' })` (https://playwright.dev/docs/api/class-page#page-get-by-ref) |

The isolated world matters: core runs page scripts in the page's own world unless a host switches the isolated world on (`crates/browseros-core/src/page_js.rs:53`); claw-server does (`apps/claw-server/src/config/behavior.rs:47`). Locator internals should use the variant that refuses to run without an isolated world (`page_js.rs:103`), never the silent fallback.

**Recommendation:** A for Playwright-named methods, because faithful behaviour is the goal. With A, `page.ariaSnapshot({ mode: 'ai' })` and `page.getByRef()` come from the same vendored engine, and the ref table lives in the page's isolated world, so it survives stateless calls. Keep today's snapshot refs as a BrowserOS extra (`browseros.snapshot()` plus `browseros.refLocator(page, "e12")`). **Unresolved:** whether to retire the BrowserOS snapshot in favour of Playwright's AI-mode ARIA snapshot, so that refs, roles, and names come from one engine.

**Reuse.** Appendix C maps existing functions to reuse, adapt, or gap. "Reuse" means reusing logic and patterns in the new codebase, not an approved dependency on today's crates. Short version: navigation, input dispatch, keyboard tables, geometry, frames, isolated-world evaluation, dialogs, console, downloads, uploads, screenshots, and page admission all exist. Selector engine, strictness, Playwright actionability, network observation, request interception, the sync mirror, and the request API are new.

### JS API wrappers and locators

The prelude is plain JavaScript evaluated into every fresh realm. It holds no browser state.

- **Locators are pure data.** A locator is `{ scope, selector }`. `getByRole`, `filter`, `nth`, and `frameLocator` build a new locator synchronously and call nothing.
- **Selectors use Playwright's string syntax.** For example, `getByRole('button', { name: 'Continue' })` becomes `internal:role=button[name="Continue"i]`; `i` means case-insensitive substring and `s` means exact (*note*: pw-locator-input-semantics). The same syntax feeds the vendored engine and lets error messages match Playwright's.
- **Every action re-sends the whole selector.** Rust re-resolves it each time. That is what "lazy" means here.
- **Handles are thin.** `Response`, `Download`, `Dialog`, and `Request` wrap an ID plus the fields that arrived with the event.

```js
// prelude.js (pseudocode, untested). The only engine-specific line is how __op is bound.
const op = async (name, args) => {
  const r = await __op(name, JSON.stringify(args ?? {}));  // adapter: Promise<{ok, value|error}>
  if (r.ok) return wrap(r.value);                           // tagged handles -> classes
  throw makeError(r.error);                                 // TimeoutError, etc.
};

class Locator {
  #scope; #selector;
  constructor(scope, selector) { this.#scope = scope; this.#selector = selector; }
  getByRole(role, o) { return new Locator(this.#scope, chain(this.#selector, roleSel(role, o))); }
  filter(o)          { return new Locator(this.#scope, chain(this.#selector, filterSel(o))); }
  nth(i)             { return new Locator(this.#scope, chain(this.#selector, `nth=${i}`)); }
  first()            { return this.nth(0); }
  contentFrame()     { return new FrameLocator(this.#scope, chain(this.#selector, 'internal:control=enter-frame')); }
  click(o)           { return op('locator.click', this.#t(o)); }            // Promise<void>
  fill(value, o)     { return op('locator.fill', this.#t({ ...o, value })); }
  count()            { return op('locator.count', this.#t()); }             // Promise<number>
  textContent(o)     { return op('locator.textContent', this.#t(o)); }      // Promise<string|null>
  async all()        { const n = await this.count(); return [...Array(n)].map((_, i) => this.nth(i)); }
  #t(options = {})   { return { scope: this.#scope, selector: this.#selector, options }; }
}

class Page {
  #ref;
  constructor(ref) { this.#ref = ref; }
  url()              { return __opSync('page.url', this.#ref); }            // sync: reads Rust mirror
  getByRole(role, o) { return new Locator({ page: this.#ref }, roleSel(role, o)); }
  goto(url, o)       { return op('page.goto', { page: this.#ref, url, options: o }); }   // Response|null
  screenshot(o)      { return op('page.screenshot', { page: this.#ref, options: o }); } // Uint8Array
  click(sel, o)      { return op('locator.click', { scope: { page: this.#ref }, selector: sel, // Playwright
                                                    options: { ...o, strict: false } }); } // selector syntax; discouraged shortcut
}
```

```ts
// neo.d.ts excerpt (pseudocode). Types copied from the pinned Playwright release, trimmed
// (1.64 getByRole also takes `description`, added in 1.60; type to copy from the release).
interface Locator {
  getByRole(role: AriaRole, options?: { name?: string | RegExp; exact?: boolean;
    checked?: boolean; disabled?: boolean; expanded?: boolean; includeHidden?: boolean;
    level?: number; pressed?: boolean; selected?: boolean }): Locator;
  click(options?: { button?: 'left' | 'right' | 'middle'; clickCount?: number; delay?: number;
    force?: boolean; modifiers?: Array<'Alt' | 'Control' | 'ControlOrMeta' | 'Meta' | 'Shift'>;
    position?: { x: number; y: number }; timeout?: number; trial?: boolean }): Promise<void>;
  count(): Promise<number>;
}
declare namespace browseros {                 // BrowserOS extras, same code mode
  function snapshot(page: Page, options?: { mode?: 'full' | 'interactive' }): Promise<Snapshot>;
  function refLocator(page: Page, ref: string): Locator;
}
```

**BrowserOS extras** come from today's SDK where they still make sense: snapshot and diff, markdown read, grep, history, tab groups, windows, profiles, credentials, payment cards, saved helpers, raw CDP (`run.rs:376-446`). Each becomes a registered op with a `.d.ts` entry.

### Async bridge

**Walk-through: `await page.getByRole("button", { name: "Continue" }).click()`**

```
JS (realm)                  adapter                 Rust op                       Chromium
──────────                  ───────                 ───────                       ────────
getByRole(...) → Locator    (no call)
.click()       → op(...) ─► __op("locator.click", json) ─► dispatch(...)
  Promise pending                                    admit page ref ─────────────► session check
                                                     loop until deadline:
                                                       engine in isolated world ─► Runtime.callFunctionOn
                                                       query: 0 wait, >1 strict error, 1 go on
                                                       visible, stable, enabled ─► Runtime.callFunctionOn
                                                       scroll + point ───────────► DOM.scrollIntoViewIfNeeded,
                                                                                   DOM.getContentQuads
                                                       hit-target check ─────────► Runtime.callFunctionOn
                                                       press ────────────────────► Input.dispatchMouseEvent ×3
                                                     release object group ───────► Runtime.releaseObjectGroup
  resolve(undefined) ◄───── Ok(OpValue::Undefined) ◄─┘
```

Step by step:

1. **Build (synchronous).** `getByRole` returns a new `Locator` holding the page reference and `internal:role=button[name="Continue"i]`. Nothing touches the page.
2. **Call.** `click()` calls `op("locator.click", …)` and gets a pending Promise.
3. **Bridge.** The adapter hands JSON arguments to `dispatch`. The engine's event loop polls the Rust future.
4. **Admit.** Rust resolves the page reference to its current CDP session (`target.rs:1014`, `target.rs:1026`). A closed or out-of-scope page fails here.
5. **Locate.** Rust runs the selector engine in the frame's isolated world. Zero matches: wait and retry. More than one: fail with `strict mode violation: … resolved to N elements` (https://playwright.dev/docs/locators#strictness). Exactly one: continue with that element.
6. **Check and act.** Wait until visible, stable, and enabled; scroll; pick a point; confirm the element receives the event at that point; send mouse move, press, release. Any failed check logs a line and retries until the timeout.
7. **Finish.** Release remote objects. Return `OpValue::Undefined`.
8. **Settle.** The adapter resolves the Promise; the prelude returns `undefined`. On failure the prelude throws a JS error built from `OpError`.

**Actionability checks per action** (https://playwright.dev/docs/actionability ; *note*):

| Action | Visible | Stable | Receives events | Enabled | Editable |
|---|---|---|---|---|---|
| `click`, `dblclick`, `check`, `uncheck`, `setChecked`, `tap` | yes | yes | yes | yes | — |
| `hover`, `dragTo` | yes | yes | yes | — | — |
| `screenshot` | yes | yes | — | — | — |
| `fill`, `clear` | yes | — | — | yes | yes |
| `selectOption` | yes | — | — | yes | — |

"Enabled" also fails inside `<fieldset disabled>` and under `aria-disabled=true`; today's core checks `:disabled` only (*note*: core-map). `force` skips the non-essential checks; `trial` runs the checks without acting.

**Frame and locator gaps compared with today's refs:**

| Topic | Today's snapshot refs (*repo*) | Playwright `getByRole` (*docs*, via *note*) |
|---|---|---|
| Lifetime | Eager: an ID captured at snapshot time; can go stale (`error.rs:43-50`) | Lazy: re-queried on every action |
| Role and name | Chromium's accessibility tree (`observer.rs:776`) | Computed in page script by Playwright's ARIA rules |
| Duplicates | Numbered with `nth`, used to recover a stale ref (`crates/browseros-core/src/snapshot/refs.rs:17-19`) | Strict: more than one match is an error unless `nth`, `first`, or `last` is used |
| Frames | A ref already reaches into iframes; it carries a frame ID and session (`run.rs:1791-1793`, `run.rs:1244-1247`) | Does not cross iframes; needs `frameLocator()` or `contentFrame()`. Pierces open shadow roots |
| Actionability | Scroll, probes, hit tests, then one fallback press (`run.rs:70`, `crates/browseros-core/src/input/click.rs:121`). Enabled means `:disabled` only (*note*: core-map) | Visible, stable, enabled, receives events; retries until timeout; no fallback |
| Timing | Click has a fixed 1.5 s recovery budget (`input/click.rs:20`); arrival after navigation is capped at 1.5 s (`crates/browseros-core/src/timeouts.rs:37`) | 30 s default timeout with retries; `goto` waits for `load` |
| Query API | None: no CSS, text, role, or XPath query exists in core (*note*: core-map) | Every `getBy*` and `locator()` is a query |

**Frames.** Since 1.63, `frameLocator()` without a selector searches every frame and throws if elements match in several (https://playwright.dev/docs/api/class-page#page-frame-locator). `page.frameLocator('#pay').getByRole('button', { name: 'Pay' })` becomes `#pay >> internal:control=enter-frame >> internal:role=button[name="Pay"i]`. In Playwright, a frame hop that matches a non-iframe throws `<iframe> was expected` (*note*). Rust resolves `#pay` strictly in the parent frame, finds the child frame ID, then that frame's isolated world. A same-process iframe uses the page session with another execution context. An out-of-process iframe has its own CDP session. Points measured in a child frame must be translated into page coordinates before input. Existing pieces: frame registry and frame-target resolution (`crates/browseros-core/src/frames.rs:21`, `frames.rs:93`) and frame-scoped evaluation (`page_js.rs:131`). Gap: out-of-process frame sessions enable only DOM and Accessibility (`frames.rs:373-375`), so their console, dialog, and navigation events are never seen. Frame-scoped events and the frame mirror need more domains enabled on those sessions.

**Events without callbacks into JS.** **Recommendation:** the prelude pulls events with an op (`await op('events.next', { sub })`) and dispatches them to listeners. No engine-specific callback machinery is needed. The run ends when the script's own promise settles, not when listeners go idle; teardown cancels the pending pull. An exception inside a listener is logged and traced; whether it fails the run is **unresolved**.

**Timers.** `setTimeout`, `clearTimeout`, and `setInterval` are written in the prelude on top of one sleep op. Today's shim returns `0` and has a no-op `clearTimeout` (`run.rs:106-110`); the new runtime needs real cancellation.

**Same operation on both engines.** Both adapters below expose one function, `__op(name, argsJson)`, that calls the same `neo_ops::dispatch`. The prelude, the click op, and the CDP traffic are identical; only the binding differs.

### QuickJS embedding (rquickjs)

**Verified (*repo*: `rquickjs` 0.12.1 with features `std`, `futures`, `parallel` at `Cargo.toml:65`; its source is in the local Cargo registry):**

- A new `AsyncRuntime` plus `AsyncContext::full` per call gives fresh state (`run.rs:806`, `run.rs:816`).
- Limits: `set_memory_limit` and `set_max_stack_size` (`run.rs:807-808`). An interrupt handler stops JavaScript on cancel or deadline (`run.rs:811-815`).
- An async Rust closure wrapped as `Func::from(Async(...))` becomes a JS function returning a Promise (`run.rs:939-951`, `run.rs:974`).
- Rust awaits the script's Promise with `promise.into_future()` (`run.rs:866`). Only that promise is awaited, so pending listeners do not keep the call alive.
- JS errors arrive as `CaughtError` (`run.rs:1885-1893`). Rust errors become JS exceptions via `Exception::throw_message` (`run.rs:947`).
- Bytes become a `Uint8Array` through `TypedArray::<u8>::new(ctx, Vec<u8>)` (`rquickjs-core-0.12.1/src/value/typed_array.rs:97`). Any value can be thrown with `Ctx::throw` (`rquickjs-core-0.12.1/src/context/ctx.rs:271`).
- A run that hits its deadline returns a failure with the logs captured so far (`run.rs:769-773`).

- The bundled engine is quickjs-ng 0.15.1, not Bellard's QuickJS (`rquickjs-sys-0.12.1/quickjs/quickjs.h:1410-1412`).

**From the rquickjs research note (*note*: engine-rquickjs, with its own citations):**

- Latest `rquickjs` is 0.14.0 (2026-09-18). 0.13.0 made reading `TypedArray`/`ArrayBuffer` bytes `unsafe` (the old API was unsound) and fixed a deadlock when a context is dropped under `parallel`.
- **Fresh means a new runtime, not just a new context.** Class prototypes and native-function prototypes are cached per runtime, so a new context on a reused runtime is not fully fresh. Limits, interrupt handler, job queue, and future scheduler are per runtime too.
- **Concurrency.** Each runtime has one lock; contexts on one runtime take turns. Parallel calls need one runtime each, with `parallel` so futures can move between threads.
- **Interrupts.** The handler fires only while JavaScript runs, so a script waiting on a CDP call needs the outer Tokio deadline as well (today's code has both: `run.rs:769-773`, `run.rs:811-815`). An interrupt throws an error JavaScript cannot catch; `catch`/`finally` blocks are skipped.
- **Limits.** Out of memory is a catchable JS error. The real default stack limit is 1 MiB; values over 16 MiB silently disable the check.
- **Compatibility.** test262.fyi shows quickjs-ng passing 98.2% of language tests, for a newer quickjs-ng than the bundled one. No `Intl`, no debugger, no source maps.
- **Risks.** A leaked JS object makes runtime teardown abort the whole process, because the engine's C assertions are on by default; concurrent calls would die with it. `parallel` is labelled experimental, and upstream CI tests it only on Linux x86_64. Some rquickjs errors print to stdout with `println!`, which would corrupt a stdio protocol.
- No published binary size; a proxy is Alpine's quickjs-ng shared library, about 670–706 KiB.

```rust
// QuickJS adapter sketch (untested). Names follow rquickjs 0.12.1 as used in run.rs.
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Function, Object, Promise,
               TypedArray, Value as JsValue, function::{Async, Func}};

async fn run_on_quickjs(code: String, scope: RunScope) -> Result<Envelope, RunFailure> {
    let rt = AsyncRuntime::new().map_err(RunFailure::engine)?;        // fresh per call
    rt.set_memory_limit(64 * 1024 * 1024).await;
    rt.set_max_stack_size(512 * 1024).await;
    let stop = scope.stop_flag();                                     // cancel || deadline
    rt.set_interrupt_handler(Some(Box::new(move || stop.is_set()))).await;
    let cx = AsyncContext::full(&rt).await.map_err(RunFailure::engine)?;
    let out = cx.async_with(async |ctx| {
        install_op(&ctx, scope.clone()).map_err(RunFailure::engine)?;
        ctx.eval::<(), _>(PRELUDE_JS).catch(&ctx).map_err(|e| RunFailure::engine_js(&ctx, e))?;
        let make: Function = ctx.globals().get("__makeRun").catch(&ctx).map_err(|e| RunFailure::engine_js(&ctx, e))?;
        let user: Function = make.call((code,)).catch(&ctx).map_err(|e| RunFailure::syntax(&ctx, e))?;
        let done: Promise = user.call(()).catch(&ctx).map_err(|e| RunFailure::thrown(&ctx, e))?;
        let value: JsValue = done.into_future().await.catch(&ctx).map_err(|e| RunFailure::thrown(&ctx, e))?;
        Envelope::from_js(&ctx, value)                                // encoding: unresolved
    }).await;
    rt.set_interrupt_handler(None).await;
    out
}

fn install_op<'js>(ctx: &Ctx<'js>, scope: RunScope) -> rquickjs::Result<()> {
    let op = move |ctx: Ctx<'js>, name: String, args: String| {
        let scope = scope.clone();
        async move {
            let args = serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);
            let reply = Object::new(ctx.clone())?;
            match neo_ops::dispatch(&scope, &name, args).await {         // same op on both engines
                Ok(OpValue::Bytes(b)) => { reply.set("ok", true)?;
                    reply.set("value", TypedArray::<u8>::new(ctx.clone(), b.to_vec())?)?; }
                Ok(OpValue::Json(v)) => { reply.set("ok", true)?; reply.set("value", json_to_js(&ctx, v)?)?; }
                Ok(OpValue::Undefined) => { reply.set("ok", true)?; }
                Err(e) => { reply.set("ok", false)?; reply.set("error", json_to_js(&ctx, e.to_json())?)?; }
            }
            Ok::<_, rquickjs::Error>(reply)
        }
    };
    ctx.globals().set("__op", Func::from(Async(op)))   // json_to_js: reuse run.rs:1819
}
```

### Deno Core / V8 embedding

**Verified (*docs*, plus *note*: engine-deno-core).** Checked against `deno_core` 0.413.0 (2026-10-21), which links `v8` 150.4.0 (V8 15.0.245.2).

- **Where it lives.** The standalone repository is archived; its README says it "has been merged into denoland/deno". The source is now https://github.com/denoland/deno/tree/v2.9.8/libs/core . Releases are frequent 0.x versions (0.405 to 0.413 between June and October 2026), and the README calls the documentation "thin".
- **Ops.** An async Rust function marked `#[op2]` becomes a Promise in JavaScript; the older `#[op2(async)]` form was removed in January 2026. Async ops receive `Rc<RefCell<OpState>>`. Errors return as `JsErrorBox`, or as a type deriving `deno_error::JsError` with `#[class]` and `#[property]` fields such as `code`.
- **Extensions.** `extension!(name, ops = [..], options = {..}, state = |state, options| ..)` generates `name::init(..)` for `RuntimeOptions.extensions`. Scripts reach ops as `Deno.core.ops.<name>`.
- **Run to completion.** `execute_script` returns the script's value; `resolve(promise)` turns it into a Rust future; `with_event_loop_promise(future, PollEventLoopOptions)` drives the event loop until that future finishes (https://docs.rs/deno_core/latest/deno_core/struct.JsRuntime.html).
- **Fresh state.** A new `JsRuntime` means a new isolate and a new context. Multiple realms per runtime were removed in 2023, and a reused runtime keeps its globals. `RuntimeOptions.startup_snapshot` loads a snapshot built ahead of time (https://docs.rs/deno_core/latest/deno_core/struct.RuntimeOptions.html). Start-up cost, with or without a snapshot, is not documented.
- **Threads.** `JsRuntime` is `!Send` and `!Sync` (https://docs.rs/deno_core/latest/deno_core/struct.JsRuntime.html). It is thread-affine: created, polled, and dropped on one thread. That does not require one OS thread per isolate; V8 lets an embedder use several isolates in parallel on several threads, each entered by at most one thread at a time. Two constraints shape the layout. deno_core needs a current-thread Tokio runtime and must be created inside it. rusty_v8 asserts that isolates on one thread are dropped in reverse order of creation. Deno itself gives each worker its own thread, and the research note infers that one live runtime per worker thread is the safest layout; that is an inference, not a documented rule. The threads-to-runtimes layout is feasibility check F4.
- **Memory.** `RuntimeOptions.create_params` accepts `v8::CreateParams::heap_limits(initial, max)` (https://docs.rs/v8/latest/v8/struct.CreateParams.html). This bounds the V8 JavaScript heap, not total process memory. Near the limit V8 calls the near-heap-limit callback; if the callback does not raise the limit, V8 aborts the process. So the callback must terminate the script and grant headroom.
- **Stopping scripts.** `IsolateHandle::terminate_execution()` is thread-safe and callable from any thread (https://docs.rs/v8/latest/v8/struct.IsolateHandle.html). deno_core then reports `Uncaught Error: execution terminated`. A script parked on a hung op also needs a Rust-side timeout; dropping the runtime drops pending op futures.
- **Debugging.** `RuntimeOptions.inspector` starts an inspector (https://docs.rs/deno_core/latest/deno_core/struct.RuntimeOptions.html). `LocalInspectorSession` speaks the DevTools protocol in-process, and the separate `deno_inspector_server` crate serves remote DevTools. JS errors reach Rust with name, message, stack, and frames.
- **Packaging.** By default rusty_v8 downloads a prebuilt static V8 library at build time; `V8_FROM_SOURCE` builds V8 instead (https://github.com/denoland/rusty_v8#binary-build). The archive deno_core uses is 37.6–39.2 MB gzipped per target. Prebuilts exist for macOS, Linux, and Windows on x64 and arm64. No size is published for a minimal embedder's final binary.
- **Not included.** No `setTimeout`, `fetch`, `URL`, `TextEncoder`, `AbortController`, or `structuredClone`. The built-in `console` writes to stdout, so the prelude must replace it. V8's explicit resource management (`await using`) is switched on.
- **Not shortcuts.** `rustyscript` 0.12.3 wraps deno_core 0.355, 60 releases behind. deno_core's experimental `quickjs` feature (through the `v8x` release candidate) is too new to rely on.

**Process-wide failure modes to design around (*note*):** heap exhaustion without the callback, the isolate drop-order assertion, a runtime created outside Tokio, V8 initialised off the main thread on hosts with memory protection keys, and worker stacks too small for V8.

```rust
// V8 adapter sketch (untested). Shapes follow deno_core 0.413.0 as described in the engine-deno-core note.
use deno_core::{extension, op2, v8, JsRuntime, OpState, PollEventLoopOptions, RuntimeOptions};
use deno_error::JsErrorBox;                      // pin deno_error to deno_core's exact version
use std::{cell::RefCell, rc::Rc};

#[op2]
#[string]                                        // return-attribute placement on async fn: unverified
async fn op_neo(state: Rc<RefCell<OpState>>, #[string] name: String, #[string] args: String)
    -> Result<String, JsErrorBox>
{
    let scope = state.borrow().borrow::<RunScope>().clone();     // release the borrow before awaiting
    let args = serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);
    Ok(tagged_json(neo_ops::dispatch(&scope, &name, args).await)) // same op as QuickJS: {ok, value|error}
}                                                // bytes need a second op returning a buffer

extension!(neo_ext, ops = [op_neo], options = { scope: RunScope },
           state = |state, options| { state.put(options.scope); });

// Runs on a worker thread that owns a current-thread Tokio runtime; created inside it.
async fn run_on_v8(code: String, scope: RunScope) -> Result<Envelope, RunFailure> {
    let mut js = JsRuntime::try_new(RuntimeOptions {
        extensions: vec![neo_ext::init(scope.clone())],
        create_params: Some(v8::Isolate::create_params().heap_limits(0, 64 * 1024 * 1024)),
        ..Default::default()
    })?;
    let oom = js.v8_isolate().thread_safe_handle();
    js.add_near_heap_limit_callback(move |current, _initial| { oom.terminate_execution(); current * 2 });
    let stop = js.v8_isolate().thread_safe_handle();
    scope.on_stop(move || { stop.terminate_execution(); });        // deadline or cancel, from any thread
    js.execute_script("prelude.js", PRELUDE_JS)?;  // __op = (n, a) => Deno.core.ops.op_neo(n, a).then(JSON.parse)
    let promise = js.execute_script("neo-run.js", wrap_async(code))?;
    let settled = js.resolve(promise);
    let value = tokio::time::timeout_at(scope.deadline(),           // a hung op parks the loop
        js.with_event_loop_promise(settled, PollEventLoopOptions::default())).await??;
    Envelope::from_v8(&mut js, value)  // encoding unresolved; dropping js drops pending op futures
}
```

### Boa (brief)

Boa is a JavaScript engine written in Rust. Checked against `boa_engine` 0.22.0 (2026-08-28) (*note*: engine-boa):

- Its documentation still calls it *"an open-source, experimental ECMAScript Engine"*. https://docs.rs/boa_engine/latest/boa_engine/
- It passes 95.61% of test262 at v0.22. https://boajs.dev/conformance
- Async Rust maps to Promises through `NativeFunction::from_async_fn`; the event loop is the `JobExecutor` trait. https://docs.rs/boa_engine/0.22.0/boa_engine/native_function/struct.NativeFunction.html , https://docs.rs/boa_engine/0.22.0/boa_engine/job/trait.JobExecutor.html
- It has no memory limit and no way to stop a running script from outside. `RuntimeLimits` covers loop count per frame, recursion, and stack slots only. https://docs.rs/boa_engine/0.22.0/boa_engine/vm/struct.RuntimeLimits.html
- `Context` is `!Send`. https://docs.rs/boa_engine/0.22.0/boa_engine/context/struct.Context.html
- On Boa's own nightly V8-v7 benchmark (median of 30 runs), Boa scores 223, QuickJS 1,229, and V8 with its JIT disabled 2,561.5. https://boajs.dev/benchmarks , raw data https://github.com/boa-dev/data/blob/main/bench/results/score.json

**Verdict:** not a candidate today. Untrusted, concurrent calls need a hard interrupt and a memory cap. Revisit if Boa adds both, or if each call moves into its own OS process.

**Separate alternative, not an embedded engine:** run each call in an external Node, Bun, or Deno process. That gives OS-level memory and time limits and full compatibility, at the cost of process start-up, an IPC protocol for every op, and shipping a second runtime. It is listed for completeness and not evaluated further.

### Engine comparison and recommendation

| Question | QuickJS via `rquickjs` | V8 via `deno_core` |
|---|---|---|
| Async Rust ↔ Promise | `Func::from(Async(..))`; `Promise::into_future` (*repo*) | `#[op2] async fn`; `resolve` + `with_event_loop_promise` (*docs*) |
| Fresh state per call | New runtime + context per call, in production today (*repo*); a reused runtime is not fully fresh (*note*) | New `JsRuntime` (new isolate + context) per call; realms removed; startup snapshots supported (*docs*, *note*) |
| Creation cost | Not documented for quickjs-ng (*note*); unmeasured | Not documented (*note*); unmeasured |
| Threads | One runtime per call, each with its own lock; `parallel` lets futures move between threads but is experimental (*note*) | Thread-affine (`!Send`, `!Sync`); current-thread Tokio; isolates on one thread drop in reverse creation order (*docs*, *note*) |
| Memory limit | `set_memory_limit` per runtime; out of memory is a catchable JS error (*repo*, *note*) | `heap_limits` bounds the V8 heap only; the near-limit callback must terminate and raise the limit, or V8 aborts the process (*docs*, *note*) |
| Stop runaway JS | Interrupt handler; fires only while JS runs (*repo*, *note*) | Thread-safe `terminate_execution` (*docs*) |
| Stop a pending op | Race the deadline and drop the future (*repo*: `run.rs:587-596`) | Rust timeout, then drop the runtime; pending op futures are dropped (*note*) |
| Process-wide crash modes | A leaked JS object aborts at teardown (*note*) | Heap exhaustion without the callback, drop-order assertion, runtime outside Tokio, wrong init thread, small stacks (*note*) |
| Build and packaging | C engine compiled with the crate; already builds in this workspace (*repo*). Size proxy: quickjs-ng shared library ≈ 0.7 MB (*note*) | Prebuilt static V8 downloaded at build time, 37.6–39.2 MB gzipped per target; from source takes over 30 minutes (*docs*, *note*). Final binary size unmeasured |
| CPU speed | Interpreter, no JIT. Boa's published nightly benchmark puts QuickJS at about half of V8 running with its JIT disabled (*note*). Our workload: unmeasured | JIT; expected to be faster on CPU-heavy scripts. Unmeasured here |
| Debugging | Stack traces only; no debugger, no source maps (*note*) | Inspector, in-process DevTools session, remote DevTools server crate (*note*) |
| Language coverage | quickjs-ng: 98.2% of test262 language tests (newer build); no `Intl` (*note*) | Full current ES, including `await using` (*note*) |
| API stability | 0.x; repo on 0.12.1, latest 0.14.0 (*note*) | 0.x; nine releases June–October 2026; documentation thin (*note*) |
| Team experience | Running in production today | None in this repository |

**Facts versus estimates.** Rows marked *repo*, *docs*, or *note* are checked; *note* rows rest on the research notes' citations. Nothing in this document was benchmarked. These stay unmeasured until F3 and F4: per-call creation cost on either engine, memory per concurrent call, the final binary-size difference, and how much engine speed matters for our scripts. That CDP waits dominate script time is an estimate.

**Recommendation (not a decision):** start on QuickJS behind the engine-agnostic seam, and run a time-boxed V8 adapter spike against the same conformance suite. QuickJS already delivers fresh state per call, memory limits, and interrupts in production; it fits the existing Tokio model; and its build is simple. V8 offers a JIT, an inspector, and full language coverage, but brings thread-affine runtimes, a large prebuilt download, frequent breaking releases, and its own process-wide crash modes. Two QuickJS risks must be closed first: process-wide aborts from a leaked JS object, and the experimental `parallel` feature on macOS and Windows (F3). Switch to V8 if the spike shows any of these: those risks cannot be contained; real agent scripts hit QuickJS language gaps; CPU-heavy scripts are common and too slow; or interactive debugging becomes a product need. Because the seam is thin, a switch would replace one adapter crate, not the API.

### Return values and errors

**Method returns are per method.** Each Playwright method keeps its documented return type. The op layer has three value kinds plus tagged handles.

| Example | Playwright type (*note*) | Op value | JS value in the script |
|---|---|---|---|
| `locator.click()` | `Promise<void>` | `Undefined` | `undefined` |
| `locator.count()` | `Promise<number>` | `Json(number)` | `number` |
| `locator.textContent()` | `Promise<null \| string>` | `Json(null or string)` | `null` or `string` |
| `locator.boundingBox()` | `Promise<null \| {x,y,width,height}>` | `Json(object or null)` | plain object or `null` |
| `page.getByRole()` | `Locator` (sync) | none | `Locator` |
| `locator.all()` | `Promise<Array<Locator>>` | `count` op, then JS | `Locator[]` |
| `page.url()` | `string` (sync) | sync mirror read | `string` |
| `page.goto()` | `Promise<null \| Response>`; `null` for `about:blank` or a same-URL hash change; HTTP 4xx/5xx do not throw | tagged handle or `null` | `Response` instance or `null` |
| `page.screenshot()` | `Promise<Buffer>` | `Bytes` | `Buffer` shim over `Uint8Array` |
| `page.evaluate(fn)` | `Promise<Serializable>`; a non-serializable result becomes `undefined`; `-0`, `NaN`, `±Infinity` survive (docs). Playwright's code also keeps `BigInt`, `Date`, `RegExp`, `URL`, `Error`, typed arrays, and cycles; `Map`/`Set` become `{}` (*note*) | typed JSON | value rebuilt by Playwright's serialization rules |

**The envelope is a separate, open decision.** Today's run converts the return value with lossy JSON rules (`run.rs:122-142`), drops `undefined` (`run.rs:1858-1860`), and returns `{ ok, value, logs, error }` (`run.rs:496-514`). Screenshots reach the model as image blocks while the script sees only `{ page, format, bytes }` (`crates/browseros-mcp/src/tools/screenshot.rs:126-181`). Whatever format is chosen must be able to say:

- returned a value, returned nothing, or threw;
- how non-JSON values (`undefined`, `NaN`, `BigInt`, `Date`, bytes, class instances) are encoded;
- logs, with levels and caps;
- the error's name, message, script position, code, origin, failing op, and call log;
- artifacts (screenshots, PDFs, downloads) as references, not inline bytes;
- a trace ID for the REST endpoints;
- which pages the call opened or touched, so the next stateless call can continue.

**Errors cross the seam as data.** Rust returns `OpError`; the prelude builds a JS `Error` (`name` = `TimeoutError` or `Error`) with `code`, `origin`, and `callLog`, plus a hidden tag naming the op. If that same object escapes the script, the envelope can say a BrowserOS op raised it. If the script wraps or replaces it, the script owns the failure; the trace still records the op error. This keeps the BrowserOS/customer boundary auditable even when scripts catch errors.

**Proposed origins:**

| Origin | Meaning | Examples |
|---|---|---|
| `script` | The customer's code | `SyntaxError`, `TypeError`, user `throw`, invalid op arguments |
| `page` | The website or page state | locator timeout, strict-mode violation, navigation failure, page script threw |
| `limits` | The call's budget | run deadline, memory limit, output cap |
| `policy` | Refused by BrowserOS rules | page out of scope, gated feature |
| `browseros` | BrowserOS or Chromium failed | CDP disconnect, protocol error, internal bug |

Today, rich errors exist in Rust (`error.rs:37-108`, with stable target codes at `error.rs:110-155`), but the run bridge flattens them to strings (`run.rs:594`, `run.rs:947`). Nothing in claw-server marks a failure as BrowserOS's or the caller's; only telemetry classifies error text (`apps/claw-server/src/analytics/error_allowlist.rs:190`; *note*: claw-server-map). The new op layer keeps the class and code, and must sort the roughly 87 untyped `CoreError::Message` sites into origins (*note*: core-map). CDP errors already separate protocol errors, timeouts, and lost connections (`crates/browseros-cdp/src/error.rs`), which map to `page` or `browseros`.

### Future consumers: Jev-like operations and skills

- **Jev-like operations** become Rust ops in the same registry, exposed under `browseros.*` with `.d.ts`. They inherit tracing, limits, errors, and policy.
- **Skills** stay JavaScript that calls the same API. Because state is fresh per call, a skill is loaded into each call that needs it, as saved helpers are today (`run.rs:776-796`).
- **Rust callers.** Ops take plain data, so server-side Rust can call them without JavaScript.
- Not designed here: skill storage, versioning, contracts, or discovery.

## Appendix

### A. Source references

**Repository (read for this document):** `crates/browseros-mcp/src/tools/run.rs`, `crates/browseros-mcp/src/framework.rs`, `crates/browseros-mcp/src/run_policy.rs`, `crates/browseros-mcp/src/tools/screenshot.rs`, `crates/browseros-core/src/{error.rs,types.rs,target.rs,navigation.rs,input/*,frames.rs,page_js.rs,observer.rs,snapshot/refs.rs,page_signals.rs,file_transfers/mod.rs,screenshot/mod.rs,pages.rs,settle.rs,viewport/mod.rs}`, `crates/browseros-cdp/src/{client.rs,events.rs}`, `crates/browseros-cdp/protocol/{protocol.json,surface.json}`, `apps/claw-server/src/analytics/error_allowlist.rs`, `Cargo.toml`.

**Research notes (raw findings, same machine; each lists its own sources):**

- `/Users/felarof01/.llm/2026-11-04/browseros_ext/playwright-bindings-design/research/pw-page-frame-context.md` — Page, Frame, FrameLocator, Browser, BrowserContext, BrowserType: 242 members and 40 events, from the live 1.64 docs.
- `/Users/felarof01/.llm/2026-11-04/browseros_ext/playwright-bindings-design/research/pw-network-misc-testrunner.md` — network, event, and misc classes (149 members), the library/runner boundary, and the assertion list.
- `/Users/felarof01/.llm/2026-11-04/browseros_ext/playwright-bindings-design/research/pw-locator-input-semantics.md` — Locator, ElementHandle, JSHandle, Keyboard, Mouse, Touchscreen; actionability, strictness, role and text matching, serialization, and the injected script, with GitHub links pinned to commit `f0eadb9783f97ea495cb8bd55860c5d5b99fe1c1`.
- `/Users/felarof01/.llm/2026-11-04/browseros_ext/playwright-bindings-design/research/engine-rquickjs.md`
- `/Users/felarof01/.llm/2026-11-04/browseros_ext/playwright-bindings-design/research/engine-deno-core.md` — deno_core 0.413.0 and rusty_v8 150.4.0, checked against the `v2.9.8` source and docs.rs.
- `/Users/felarof01/.llm/2026-11-04/browseros_ext/playwright-bindings-design/research/engine-boa.md`
- `/Users/felarof01/.llm/2026-11-04/browseros_ext/playwright-bindings-design/research/core-map.md` — browseros-core capability map (~190 rows).
- `/Users/felarof01/.llm/2026-11-04/browseros_ext/playwright-bindings-design/research/cdp-map.md`
- `/Users/felarof01/.llm/2026-11-04/browseros_ext/playwright-bindings-design/research/claw-server-map.md`

**Playwright 1.64 (docs read by the research notes):**

- https://playwright.dev/docs/library (library vs runner; default timeouts under "key differences")
- https://unpkg.com/playwright-core@1.64.1/types/types.d.ts (published library types)
- https://playwright.dev/docs/api/class-page#page-get-by-ref , https://playwright.dev/docs/api/class-page#page-aria-snapshot , https://playwright.dev/docs/api/class-page#page-event-dialog , https://playwright.dev/docs/api/class-page#page-frame-locator

- https://playwright.dev/docs/api/class-page
- https://playwright.dev/docs/api/class-locator
- https://playwright.dev/docs/api/class-frame
- https://playwright.dev/docs/api/class-framelocator
- https://playwright.dev/docs/api/class-browsercontext
- https://playwright.dev/docs/api/class-browser
- https://playwright.dev/docs/api/class-keyboard , https://playwright.dev/docs/api/class-mouse , https://playwright.dev/docs/api/class-touchscreen
- https://playwright.dev/docs/api/class-request , https://playwright.dev/docs/api/class-response , https://playwright.dev/docs/api/class-route
- https://playwright.dev/docs/api/class-apirequestcontext , https://playwright.dev/docs/api/class-apiresponse
- https://playwright.dev/docs/api/class-download , https://playwright.dev/docs/api/class-filechooser , https://playwright.dev/docs/api/class-dialog , https://playwright.dev/docs/api/class-consolemessage
- https://playwright.dev/docs/actionability , https://playwright.dev/docs/locators , https://playwright.dev/docs/evaluating , https://playwright.dev/docs/frames
- https://playwright.dev/docs/test-assertions (runner-only)
- https://playwright.dev/docs/release-notes
- https://github.com/microsoft/playwright (injected script source, licence)

**Engines:**

- https://docs.rs/rquickjs/latest/rquickjs/ , https://github.com/DelSkayn/rquickjs
- https://docs.rs/deno_core/latest/deno_core/struct.JsRuntime.html (`!Send`, `!Sync`; `resolve`, `with_event_loop_promise`)
- https://docs.rs/deno_core/latest/deno_core/struct.RuntimeOptions.html (`startup_snapshot`, `create_params`, `inspector`)
- https://docs.rs/v8/latest/v8/struct.IsolateHandle.html (thread-safe `terminate_execution`)
- https://docs.rs/v8/latest/v8/struct.CreateParams.html (`heap_limits`: V8 heap, not process memory)
- https://github.com/denoland/rusty_v8#binary-build (prebuilt static libraries downloaded by default)
- https://github.com/denoland/deno/tree/v2.9.8/libs/core (current deno_core source)
- https://docs.rs/deno_core/latest/src/deno_core/runtime/jsruntime.rs.html , https://docs.rs/deno_core/latest/src/deno_core/runtime/ops.rs.html
- https://docs.rs/boa_engine/latest/boa_engine/ , https://boajs.dev/conformance , https://boajs.dev/benchmarks

**CDP:** https://chromedevtools.github.io/devtools-protocol/

### B. Proposed method inventory

Scope label: **recommendation, not approved.** Return types follow the three Playwright 1.64 research notes, which were built from the live docs; re-check against the pinned 1.64.1 types when implementation starts (F1). Full member lists, including everything not proposed here, are in those notes (Appendix A). Binding: **build** = synchronous, no Rust call; **op** = async op; **sync** = synchronous mirror read; **js** = composed in the prelude; **field** = data that arrived with the event. Reuse: **R** existing logic fits, **A** existing logic needs changes, **G** no existing logic. This rates logic and patterns only; it is not a decision to depend on the old crates. Core paths are under `crates/browseros-core/src/`.

#### Page

| Member | Returns | Binding → Rust responsibility | Reuse |
|---|---|---|---|
| `goto(url, {waitUntil, timeout, referer, signal})` | `Promise<null \| Response>` | op: URL policy, then `Page.navigate`; wait for `load` (default), `domcontentloaded`, or `commit` (`networkidle` is discouraged); throw on SSL error, invalid URL, unreachable server, or failed main resource, not on HTTP 4xx/5xx; return the main document's Response | A `navigation.rs:84` (returns status and download flag, waits for "settled") |
| `reload(opts)` | `Promise<null \| Response>` | op: reload + lifecycle wait | A `navigation.rs:134` (returns `()`) |
| `goBack(opts)`, `goForward(opts)` | `Promise<null \| Response>` | op: history entry; `null` when none | A `navigation.rs:157`, `navigation.rs:162` (return `bool`) |
| `url()` | `string` | sync | G mirror |
| `title()` | `Promise<string>` | op: `document.title` in isolated world | A `page_js.rs:155` |
| `content()` | `Promise<string>` | op: serialize document with doctype | A `page_js.rs:155` |
| `setContent(html, opts)` | `Promise<void>` | op: replace document + lifecycle wait | G |
| `close(opts)` | `Promise<void>` | op: close tab; ownership policy | R `target.rs:1351` |
| `isClosed()` | `boolean` | sync | G mirror |
| `opener()` | `Promise<null \| Page>` | op: target opener | G |
| `bringToFront()` | `Promise<void>` | op: activate tab. **Unresolved:** may it take the user's focus? | A BrowserOS tab commands (*note*: cdp-map) |
| `locator(sel, {has, hasNot, hasText, hasNotText})` | `Locator` | build | G |
| `getByRole/getByText/getByLabel/getByPlaceholder/getByAltText/getByTitle/getByTestId` | `Locator` | build | G |
| `frameLocator(selector?)` | `FrameLocator` | build; no selector = any frame (1.63) | G |
| `getByRef(ref)` | `Locator` | build; ref resolved against the frame's latest AI-mode ARIA snapshot | G (state kept page-side or in Rust) |
| `ariaSnapshot({mode, depth, boxes, timeout})` | `Promise<string>` | op: vendored engine; `mode: 'ai'` adds `[ref=eN]` | G (today's snapshot is another format: `observer.rs:196`) |
| `ariaSnapshotJSON(opts)` | `Promise<Serializable>` | op: same tree as JSON (1.63) | G |
| `mainFrame()`, `frames()`, `frame(nameOrUrl)` | `Frame`, `Array<Frame>`, `null \| Frame` | sync | A `frames.rs:21` |
| `waitForLoadState(state, opts)` | `Promise<void>` | op: main-frame lifecycle events | A `settle.rs:120` (a different "settled" notion) |
| `waitForURL(url, opts)` | `Promise<void>` | op: string/glob/RegExp match on the mirror; predicate polled in js | G |
| `waitForFunction(fn, arg, opts)` | `Promise<JSHandle>` | op: poll `Runtime.callFunctionOn` in main world. **Unresolved:** return a value until handles exist | G |
| `waitForTimeout(ms)` | `Promise<void>` | op: sleep | R `run.rs:959-962` |
| `waitForEvent(event, opts)` | `Promise<payload>` | op: `events.wait` | G |
| `waitForRequest/waitForResponse(urlOrPredicate, opts)` | `Promise<Request>` / `Promise<Response>` | op: `events.wait` with URL matcher; predicate filtered in js | G (no Network observation in core) |
| `on/once/off/removeListener(event, fn)` | `Page` | js + op `events.subscribe/next` | G |
| `evaluate(fn, arg)` | `Promise<Serializable>` | op: main world, `awaitPromise`, Playwright serialization | A `page_js.rs:112`, `page_js.rs:188` |
| `$eval(sel, fn, arg)`, `$$eval(sel, fn, arg)` | `Promise<Serializable>` | js: `locator(sel).evaluate/evaluateAll` (non-strict) | G |
| `click/dblclick/fill/press/hover/check/uncheck/selectOption/setInputFiles/focus/textContent/innerText/getAttribute/isVisible(selector, …)` | as the Locator member | js: non-strict locator wrapper (discouraged) | G |
| `screenshot(opts)` | `Promise<Buffer>` | op: `Page.captureScreenshot` (clip, fullPage, type, quality) → Bytes; `path` is policy-gated | A `screenshot/mod.rs:129`, `target.rs:1152` |
| `pdf(opts)` | `Promise<Buffer>` | op: `Page.printToPDF` → Bytes | A `crates/browseros-mcp/src/tools/pdf.rs:59`, policy `target/cdp_policy.rs:183` |
| `keyboard`, `mouse`, `touchscreen`, `request` | objects | build | — |
| `context()` | `BrowserContext` | build | — |
| `viewportSize()` | `null \| {width, height}` | sync | A `viewport/mod.rs:170` |
| `dragAndDrop(source, target, opts)` | `Promise<void>` | op: as `locator.dragTo` | A `input/mouse.rs:165` |
| `setDefaultTimeout(ms)`, `setDefaultNavigationTimeout(ms)` | `void` | js: this call only | — |

#### Locator

| Member | Returns | Binding → Rust responsibility | Reuse |
|---|---|---|---|
| `locator`, `getBy*`, `filter`, `and`, `or`, `nth`, `first`, `last`, `frameLocator`, `contentFrame`, `page` | `Locator` / `FrameLocator` / `Page` | build | G |
| `click(opts)`, `dblclick(opts)` | `Promise<void>` | op: strict resolve → visible, stable, enabled, receives events → scroll → point → mouse move/press/release; `force`, `trial`, `position`, `modifiers`, `button`, `clickCount`, `delay` | A `input/mod.rs:201`, `input/mouse.rs:50`, `input/geometry.rs:154`, `input/geometry.rs:194`, `input/geometry.rs:203` |
| `hover(opts)` | `Promise<void>` | op: as click without the enabled check; mouse move | A `input/mod.rs:300`, `input/mouse.rs:131` |
| `tap(opts)` | `Promise<void>` | op: `Input.dispatchTouchEvent` | G |
| `fill(value, opts)` | `Promise<void>` | op: visible, enabled, editable → focus → select all → insert text; typed inputs (date, colour, range) set directly | A `input/fill.rs:35` |
| `clear(opts)` | `Promise<void>` | op: fill with `""` | A `input/keyboard.rs:137` |
| `pressSequentially(text, {delay})` | `Promise<void>` | op: focus + key events per character | A `input/keyboard.rs:85` |
| `press(key, opts)` | `Promise<void>` | op: focus + key combination | A `input/keyboard.rs:182`, `input/keyboard.rs:36` |
| `check`, `uncheck`, `setChecked(checked)` | `Promise<void>` | op: click when the state differs, then verify it changed | A `input/mod.rs:401`, `input/mod.rs:405` |
| `selectOption(values, opts)` | `Promise<Array<string>>` | op: select by value, label, or index; fire input and change | A `input/mod.rs:374` |
| `selectText(opts)` | `Promise<void>` | op: page-side selection | G |
| `setInputFiles(files, opts)` | `Promise<void>` | op: `DOM.setFileInputFiles`; paths policy-checked; buffers written to a temp file | A `file_transfers/mod.rs:191`, `input/mod.rs:471` |
| `focus(opts)`, `blur(opts)` | `Promise<void>` | op | A `input/geometry.rs:217` |
| `scrollIntoViewIfNeeded(opts)` | `Promise<void>` | op: wait stable, then scroll | A `input/geometry.rs:194` |
| `dragTo(target, opts)` | `Promise<void>` | op: resolve both; mouse down, move, up | A `input/mouse.rs:165` |
| `dispatchEvent(type, init)` | `Promise<void>` | op: page-side event, no actionability | G |
| `count()` | `Promise<number>` | op: no strictness, no wait | G |
| `all()` | `Promise<Array<Locator>>` | js: `count` then `nth` | — |
| `textContent`, `getAttribute(name)` | `Promise<null \| string>` | op: strict, waits for attached | G |
| `innerText`, `innerHTML`, `inputValue` | `Promise<string>` | op: strict, waits for attached | A `input/geometry.rs:270` (value) |
| `allInnerTexts`, `allTextContents` | `Promise<Array<string>>` | op: all matches | G |
| `isVisible`, `isHidden` | `Promise<boolean>` | op: immediate, no wait | G |
| `isEnabled`, `isDisabled`, `isEditable`, `isChecked` | `Promise<boolean>` | op: strict, waits for attached | G |
| `boundingBox(opts)` | `Promise<null \| {x, y, width, height}>` | op: box model | A `input/geometry.rs:154` |
| `waitFor({state, timeout})` | `Promise<void>` | op: attached, detached, visible, hidden | A `crates/browseros-mcp/src/tools/wait.rs` |
| `evaluate(fn, arg)` | `Promise<Serializable>` | op: element passed to `Runtime.callFunctionOn` in main world | A `page_js.rs:188` |
| `evaluateAll(fn, arg)` | `Promise<Serializable>` | op: all matches as an array | G |
| `screenshot(opts)` | `Promise<Buffer>` | op: element clip → Bytes | A `screenshot/mod.rs:129` |
| `ariaSnapshot(opts)` | `Promise<string>` | op: vendored engine's ARIA snapshot | G (today's snapshot is another format: `observer.rs:196`) |

#### Frame and FrameLocator

| Member | Returns | Binding → Rust responsibility | Reuse |
|---|---|---|---|
| `frame.goto`, `waitForLoadState`, `waitForURL`, `waitForFunction`, `waitForTimeout` | as on Page | op scoped to the frame | A `frames.rs:93` |
| `frame.locator`, `getBy*`, `frameLocator` | `Locator` / `FrameLocator` | build | G |
| `frame.evaluate(fn, arg)` | `Promise<Serializable>` | op in the frame's main world | A `page_js.rs:131` |
| `frame.title()`, `content()`, `setContent()` | `Promise<string>` / `Promise<void>` | op in the frame | A `page_js.rs:155` |
| `frame.url()`, `name()`, `parentFrame()`, `childFrames()`, `isDetached()`, `page()` | `string`, `string`, `null \| Frame`, `Array<Frame>`, `boolean`, `Page` | sync | A `frames.rs:21` |
| `frameLocator.locator`, `getBy*`, `frameLocator`, `owner()` | `Locator` / `FrameLocator` | build | G |

#### Keyboard, Mouse, Touchscreen

| Member | Returns | Binding → Rust responsibility | Reuse |
|---|---|---|---|
| `keyboard.down(key)`, `up(key)`, `press(key, {delay})` | `Promise<void>` | op: `Input.dispatchKeyEvent` with key tables and modifiers | A `input/keyboard.rs:36`, `input/keyboard.rs:73`, `input/keyboard.rs:182` |
| `keyboard.type(text, {delay})` | `Promise<void>` | op: key events per character | A `input/keyboard.rs:85` |
| `keyboard.insertText(text)` | `Promise<void>` | op: `Input.insertText` | R `input/keyboard.rs:130` |
| `mouse.move(x, y, {steps})`, `down(opts)`, `up(opts)` | `Promise<void>` | op: `Input.dispatchMouseEvent`; tracks pressed buttons | A `input/mouse.rs:28`, `input/mouse.rs:131` |
| `mouse.click(x, y, opts)`, `dblclick(x, y, opts)` | `Promise<void>` | op | A `input/mouse.rs:50` |
| `mouse.wheel(dx, dy)` | `Promise<void>` | op | A `input/mouse.rs:141` |
| `touchscreen.tap(x, y)` | `Promise<void>` | op: `Input.dispatchTouchEvent` | G |

#### BrowserContext and Browser

| Member | Returns | Binding → Rust responsibility | Reuse |
|---|---|---|---|
| `context.pages()` | `Array<Page>` | sync: admitted pages from the mirror | A `target.rs:975` |
| `context.newPage()` | `Promise<Page>` | op: background tab; host claims ownership | R `target.rs:1206`, `framework.rs:248` |
| `context.browser()` | `null \| Browser` | build | — |
| `context.waitForEvent('page')`, `on('page')` | `Promise<Page>` / `BrowserContext` | op: target events | A `crates/browseros-core/src/target/events.rs` |
| `context.request` | `APIRequestContext` | build | G |
| `context.setDefaultTimeout(ms)` | `void` | js: this call only | — |
| `browser.contexts()`, `version()`, `isConnected()` | `Array<BrowserContext>`, `string`, `boolean` | sync | G |

#### Event and network objects

| Member | Returns | Binding → Rust responsibility | Reuse |
|---|---|---|---|
| `request.url/method/headers/postData/resourceType/isNavigationRequest/frame/redirectedFrom/redirectedTo/failure/timing` | sync values | field: `Network.requestWillBeSent` and related events | G |
| `request.allHeaders()`, `response()`, `sizes()`, `postDataBuffer()` | `Promise<…>` / `null \| Buffer` | op | G |
| `response.url/status/statusText/ok/headers/request/frame` | sync values | field: `Network.responseReceived` | G |
| `response.body()`, `text()`, `json()`, `allHeaders()`, `finished()` | `Promise<Buffer>`, `Promise<string>`, `Promise<Serializable>`, … | op: `Network.getResponseBody` while Chromium still holds it | G |
| `dialog.type/message/defaultValue/page` | sync values | field: `Page.javascriptDialogOpening` | A `page_signals.rs:45`, `page_signals.rs:402` |
| `dialog.accept(promptText)`, `dismiss()` | `Promise<void>` | op: `Page.handleJavaScriptDialog` | A `page_signals.rs:234`, `page_signals.rs:250` |
| `download.url/suggestedFilename/page` | sync values | field: `Browser.downloadWillBegin` | A `target/events.rs:62` |
| `download.path()`, `failure()`, `cancel()`, `delete()` | `Promise<string>`, `Promise<null \| string>`, `Promise<void>` | op: managed download folder | A `file_transfers/mod.rs:66` |
| `download.saveAs(path)` | `Promise<void>` | op: policy-checked copy | G |
| `fileChooser.isMultiple/page` | sync values | field: intercepted chooser | A `file_transfers/mod.rs:16` |
| `fileChooser.setFiles(files)` | `Promise<void>` | op | A `file_transfers/mod.rs:139`, `file_transfers/mod.rs:191` |
| `consoleMessage.type/text/location/page` | sync values | field: `Runtime.consoleAPICalled` | A `page_signals.rs:100`, `page_signals.rs:404` |
| `webError.error()`, `page()` | `Error`, `null \| Page` | field: `Runtime.exceptionThrown` | A `page_signals.rs:405` |
| `request.get/post/put/patch/delete/head/fetch(url, opts)` | `Promise<APIResponse>` | op: Rust HTTP client with the context's cookies. **Unresolved:** write `Set-Cookie` back to the profile | G |
| `apiResponse.status/statusText/ok/headers/headersArray/url` | sync values | field | G |
| `apiResponse.body()`, `text()`, `json()`, `dispose()` | `Promise<Buffer>`, `Promise<string>`, `Promise<Serializable>`, `Promise<void>` | op | G |
| `apiRequestContext.cookies()`, `addCookies()`, `clearCookies()` (1.64) | `Promise<Array<Cookie>>`, `Promise<void>`, `Promise<void>` | op: `Storage.getCookies`/`setCookies` for the context jar; policy-gated | G |

#### Not proposed for V1 (newer classes)

`Disposable` (undo handle returned by `route`, `addInitScript`, `exposeFunction`, `exposeBinding` since 1.59), `Screencast` (REST covers it), `WebStorage` (`page.localStorage`/`sessionStorage`, policy), `Credentials` (virtual passkeys, policy), `Debugger` (runner tooling), and `WebMCP` (1.64; worth a later look, because it exposes page-declared tools to agents). Sources: the class pages listed in the pw-network-misc-testrunner note.

### C. Existing code reference map

"Reuse" and "Adapt" below mean reusing logic and patterns in the new SDK codebase. They are not an approved decision to depend on `browseros-core`, `browseros-cdp`, `browseros-mcp`, or `claw-server`; the parent asks for a new codebase. Import, copy, or rewrite is **unresolved**.

| Capability | Existing code (*repo*) | Use |
|---|---|---|
| Page admission and sessions | `target.rs:1014` (`page`), `target.rs:1026` (`page_session`), `framework.rs:600-612` | Reuse |
| Page list, create, close | `target.rs:975`, `target.rs:1206`, `target.rs:1351`, `pages.rs:122`, `pages.rs:435`, `pages.rs:616` | Reuse |
| Navigation | `navigation.rs:84`, `navigation.rs:134`, `navigation.rs:157`, `navigation.rs:162`; arrival wait capped at 1.5 s (`timeouts.rs:37`, used from `navigation.rs:286`) | Adapt: Playwright load states, timeouts, and Response objects |
| Readiness and settle | `settle.rs:120`, `settle.rs:203` | Adapt: BrowserOS "settled" is not Playwright's `waitUntil` |
| Ref-based input | `input/mod.rs:201` (`click`), `input/mod.rs:216` (`click_backend_node`), `input/mod.rs:300`, `input/mod.rs:374`, `input/mod.rs:401`, `input/mod.rs:405`, `input/mod.rs:471`; fixed 1.5 s click budget (`input/click.rs:20`); fallback press (`input/click.rs:121`) | Adapt: drive from a resolved element, use the op's timeout, drop the fallback from Playwright names |
| Low-level mouse and keys | `input/mouse.rs:28`, `input/mouse.rs:50`, `input/mouse.rs:131`, `input/mouse.rs:141`, `input/mouse.rs:165`; `input/keyboard.rs:15`, `input/keyboard.rs:36`, `input/keyboard.rs:73`, `input/keyboard.rs:85`, `input/keyboard.rs:130`, `input/keyboard.rs:137`, `input/keyboard.rs:182` | Reuse |
| Geometry, scroll, blockers | `input/geometry.rs:154`, `input/geometry.rs:194`, `input/geometry.rs:203`, `input/geometry.rs:217`, `input/geometry.rs:270` | Reuse; add Playwright stability and hit-target rules |
| Fill | `input/fill.rs:35` | Adapt |
| Frames | `frames.rs:21` (`FrameRegistry`), `frames.rs:57`, `frames.rs:93`; child sessions auto-attached with only DOM and Accessibility enabled (`frames.rs:77-86`, `frames.rs:373-375`) | Adapt: frame hops for `enter-frame`; enable Runtime and Page on child sessions |
| Page JS, isolated world | `page_js.rs:53` (off by default), `page_js.rs:95` (`isolated`), `page_js.rs:103` (`isolated_only`), `page_js.rs:112` (`page_world`), `page_js.rs:131` (`in_frame`), `page_js.rs:188` (`call`), `page_js.rs:347` (`Page.createIsolatedWorld`) | Reuse for the selector engine; adapt serialization (JSON only today, so `NaN`, `±Infinity`, `-0`, `BigInt` arrive as `null`: *note*) |
| Snapshot refs | `observer.rs:196`, `observer.rs:256` (`resolve_ref`), `observer.rs:656` (stale ref re-found by role, name, position), `observer.rs:776` (`Accessibility.getFullAXTree`), `snapshot/refs.rs:12`, `snapshot/refs.rs:355`; one observer per page shared by all callers (`session.rs:125`) | Keep as `browseros.*`; not a locator |
| Dialogs, console, page errors | `page_signals.rs:45`, `page_signals.rs:100`, `page_signals.rs:234`, `page_signals.rs:250`, `page_signals.rs:272`, `page_signals.rs:402-405` | Adapt into call-scoped events |
| Downloads and uploads | `target/events.rs:62`, `file_transfers/mod.rs:16`, `file_transfers/mod.rs:66`, `file_transfers/mod.rs:139`, `file_transfers/mod.rs:170`, `file_transfers/mod.rs:191` | Reuse |
| Screenshots and PDF | `screenshot/mod.rs:129`, `target.rs:1152`, `crates/browseros-mcp/src/tools/pdf.rs:59` | Adapt: return bytes to the script |
| Viewport | `viewport/mod.rs:170`, `viewport/mod.rs:196`, `target.rs:425` | Reuse behind policy |
| Raw CDP and policy | `target.rs:1460`, `target.rs:1505`, `target/cdp_policy.rs` | Reuse for `browseros.cdp` |
| Errors | `error.rs:37-108`, `error.rs:110-155`, `crates/browseros-mcp/src/framework.rs:470-482` | Adapt into `OpError` |
| Trace and audit hook | `framework.rs:209-289`, `framework.rs:291-314`, `run.rs:1006-1070` | Reuse the pattern |
| QuickJS runtime | `run.rs:798-911`, `run.rs:913-986`, `run.rs:1819-1843` | Reuse the pattern |
| Network observation, request interception | none found in core (searched `requestWillBeSent`, `"Fetch.`) | Gap |
| Selector engine, strictness, Playwright actionability | none (today's API reports them absent: `run.rs:201-235`) | Gap |
| `run` request path in claw-server | `apps/claw-server/src/api/mcp/service.rs:1151` (`call_tool`) → `apps/claw-server/src/api/mcp/dispatch.rs:344` → `run.rs:551` | Reference for the new binary's request path |
| Session handle | `apps/claw-server/src/api/mcp/session_handle.rs:31-32` | Candidate session reference |
| Audit and tool traces | `apps/claw-server/src/db/audit_log.rs:30`, `apps/claw-server/src/api/mcp/observers/audit.rs:124-131` | Reuse the pattern for op traces |
| Screencast capture | `apps/claw-server/src/services/capture/tab_capture.rs:292` (`Page.startScreencast`) | REST side, unchanged |
| Hosted REST router | `apps/claw-server/src/app.rs:311-313` | REST side, unchanged |
| URL scheme guard | `apps/claw-server/src/api/mcp/guards/navigate_scheme.rs:7`, `navigate_scheme.rs:12` (`navigate` tool only) | Move into the `page.goto` op |
| Absent-member telemetry | `apps/claw-server/src/analytics/error_allowlist.rs:101-132`, `error_allowlist.rs:190` | Input for subset priorities |

### D. Feasibility checks (small, before any decision)

| # | Check | Settles |
|---|---|---|
| F1 | Re-check every Appendix B row against the pinned `playwright-core@1.64.1` types before implementation. | Inventory accuracy |
| F2 | Build the injected script from tag `v1.64.1` (commit `cfe490d941f992655948eee8fd4fd94e1c4234bf`), not `main`; load it into a BrowserOS isolated world; resolve `getByRole`, `getByText`, and `frameLocator` on 20 real pages; compare with Playwright. Confirm licence notice duties (Apache-2.0, plus a CC0 tokenizer). | Locator engine option A |
| F3 | QuickJS: time to create runtime, context, and prelude (p50/p99); memory per idle call; 100 concurrent calls with `parallel` on macOS arm64, Linux, Windows; drop with pending futures; leak-triggered aborts (assertions on vs off); interrupt latency in a tight loop and in a microtask chain; upgrade from 0.12.1 to 0.14.0. | QuickJS limits, concurrency, blast radius |
| F4 | V8: runtime creation time with and without a startup snapshot (undocumented); a threads-to-runtimes layout that respects the reverse-drop rule; memory per call; final binary-size change; builds on macOS arm64, Linux x64, Windows x64; termination plus Rust timeout plus drop while an op is pending; near-heap-limit handling. | V8 viability |
| F5 | CDP: lossless per-session event routing on a separate client; `Fetch.requestPaused` under load. | Network events and `route` |
| F6 | Sync mirror correctness for `url()` and `frames()` under rapid navigation and out-of-process iframes. | Synchronous getters |
| F7 | Out-of-process iframe input: translate child-frame points into page coordinates. | `frameLocator` clicks |
| F8 | `evaluate` serialization: port Playwright's serializer versus CDP `serializationOptions`. | `evaluate` fidelity |
| F9 | Try two or three envelope formats against real agent transcripts. | Envelope (open) |
| F10 | Read the absent-member counts from analytics (`error_allowlist.rs:101`) to order the subset. | V1 priorities |
