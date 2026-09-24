# Architecture review: the script execution path of BrowserOS neo

Scan date: 2026-09-23. Scope named by the owner: `run`/`evaluate` in
`browseros-mcp`, the host pipeline in `claw-server-rust/src/api/mcp`, the page
and session modules in `browseros-core` and `claw-server-rust/src/services`,
the audit store, and the cockpit's audit display. HTML twin:
`architecture-review.html` beside this file.

Vocabulary: **module, interface, implementation, depth, seam, adapter,
leverage, locality** from ft-codebase-design; **dispatch, script dispatch,
child audit row, primitive, page, ownership (a label), tab group, session,
ledger, cockpit** from CONTEXT.md.

Process notes:

- DECIDED: scan only the named scope; no hot-spot inference from git history
  (the owner named the direction). The history was still read: `run.rs`,
  `framework.rs` and `dispatch.rs` each carry 11-13 commits, `script_hook.rs`
  4, `helper_runtime.rs` 1.
- DECIDED: steps 1 and 2 of ft-improve-codebase-architecture only. No
  grilling, no CONTEXT.md or ADR edits, no repo changes.
- DECIDED: the deep module proposed below is named `ScriptDispatch` because
  CONTEXT.md already calls the top-level row "the script dispatch" and its
  primitives "child audit rows". The port the engine calls is `ScriptHost`.
  The MCP tool name for the Playwright engine is the mediator's call.

---

## The answer in two lines

**Top recommendation:** deepen the per-action host bookkeeping into one
module, `ScriptDispatch`, so audit rows, page claims, tab grouping, ownership
notices, screenshots, liveness and helper preloading are written once, and
both the granular-tool pipeline and every script engine call it.

**The seam for a second engine:** a `ScriptHost` port owned by the host and
handed to the engine through `ToolCtx`, keyed on the fork's `tabId` (not the
per-process `PageId`), with the action choreography (`action_started` /
`action_finished` / `page_created`) inside the host, not inside the engine's
bridge. Engines (`QuickJsEngine`, `PlaywrightEngine`, a `FakeEngine` for
tests) are adapters of a `ScriptEngine` port.

---

## How a script runs today (verified)

```
agent ──MCP──▶ ClawMcpService::call_tool            service.rs:737
                 │  resolves Session, ToolIdentity{ownership_key=ConvoId}
                 ▼
             dispatch_tool_call(ToolCall)            dispatch.rs:243
                 GUARDS ─▶ execute_with_cancellation ─▶ EFFECTS ─▶ OBSERVERS
                              │
                              │ tool ∈ ARBITRARY_SCRIPT_TOOLS ["run","evaluate"]
                              │   → ToolCtx.inner_call_hook = ScriptInnerCallHook(call)
                              │   → ToolCtx.preloaded_helpers = helper_runtime::preload_helpers
                              ▼
             browseros_mcp::execute_tool(run)        framework.rs:296
                 ▼
             run.rs: QuickJS runtime, BOOTSTRAP_JS defines `browser` SDK
                 every SDK call → __browserosCall → BrowserBridge::call   run.rs:599
                    hook.authorize(page) → dispatch(method) → [on_page_created] → hook.record(...)
                    dispatch: pages.*/observe.*/input.*/nav.*/cdp → browseros-core
                              tool:* → execute_tool(read|grep|wait|screenshot|evaluate|…)
                 ▼
             ScriptInnerCallHook (script_hook.rs)
                 authorize      → notes foreign pages into call.foreign_pages
                 record         → session.touch, tab_activity.record_tool,
                                  audit_log.record_tool_dispatch (direct, awaited),
                                  persist_screenshot (direct, awaited)
                 on_page_created→ ownership_claims::record_new_page (claim + ledger + activity)
                                  tokio::spawn(tab_groups::run_tab_group_work)
                 annotate_pages → tabs_list_view::annotate_pages_with_ownership
                 resolve_host / save / list / read_helper → services::helpers
                 ▼
             EFFECTS: page_ownership_notice reads call.foreign_pages; helper_runtime::discovery
             OBSERVERS: audit (parent row via audit_worker, created_at = started_at_ms),
                        run_failure (tool == "run", raw_args.code)
```

Facts that shape everything below (file:line in the surveys):

- `PageId(u32)` is a process-local counter minted by `PageManager`
  (`browseros-core/src/pages.rs:55,113,342`). It survives target swaps by
  re-matching targetId then tabId. Only the caller of `new_page` learns a new
  PageId; core has no page-created event (`PageManagerHooks` has
  `on_session_attached` and `on_page_detached` only).
- Ownership lives in two stores that disagree: in-memory `PageOwnership` is
  `HashMap<PageId, ConvoId>` (`services/sessions/tab_ownership.rs:62`); the
  durable `SessionTabLedger` is `session_tabs(session_id, tab_id, …)` keyed on
  the fork's Chrome tabId (`db/session_tabs.rs`). Popups inherit ownership
  only in the ledger (`services/browser/tab_registry.rs:318-332`), so a popup
  is owned for screenshots, recordings and cleanup but reads as "user" in
  `tabs list`.
- The host already observes page creation from the browser:
  `TabRegistry` subscribes to `Target.*` events (`tab_registry.rs:212,305`)
  to keep the ledger current. The MCP dispatch path never uses it.
- Replay recording is engine-agnostic: the extension content script records
  every tab, and "agent-owned" is decided after the fact from ledger claim
  windows (`recordings/store.rs:338-348`). The only trigger that matters is
  the claim written by `record_new_page`.
- Screenshots are engine-agnostic (host-side CDP capture of the session's
  most recently active owned tab), but there are two write paths: the parent
  row's preview goes through the audit worker (coalesced, ≤2 in flight,
  `services/audit.rs:323-332`); each child row's screenshot is captured
  inline and awaited inside the script (`script_hook.rs:160-174`), with a
  task recompute per primitive.
- Audit rows have two write paths too: parent via `audit_worker.submit`
  (bounded queue, backpressure), children via `audit_log.record_tool_dispatch`
  directly. Children land in the database before their parent exists; the
  parent is sorted first only because `created_at` is stamped with
  `started_at_ms` (`observers/audit.rs:146`).
- The cockpit never reads `parentDispatchId` (zero hits in `claw-app`). Rows
  render flat, ordered by `created_at, id`. Two lists key on tool names:
  `NOTABLE_TOOLS` (`Timeline.tsx:39`) and `TOOL_TO_VERB`
  (`replay.data.ts:301-318`). `run`'s parent row has no page, so it shows only
  in the Session tab; its children show in per-page tabs with no parent.
- A second CDP client on the same browser sees none of: PageId numbering,
  `@eN` refs, `page_signals` (dialogs, console), OOPIF session maps. It does
  see targetId and, through `Target.TargetInfo`, the fork's tabId/windowId.
  It would race core on dialogs (core auto-accepts alerts; Playwright
  auto-dismisses) and interleave input with no lock.

---

## Deletion test, as asked

**`script_hook.rs` (`ScriptInnerCallHook`).** Delete it and a `run` script
still executes, but child audit rows, per-step screenshots, tab activity,
session liveness, page claims, tab grouping, the tri-bucket in
`browser.pages.list()`, the foreign-page notice and helpers all vanish. That
complexity would have to reappear, and it cannot reappear in `run.rs`
(`browseros-mcp` cannot reach the host). So it earns its keep. But look at
*what* it is: a second copy of the sequencing the EFFECTS/OBSERVERS pipeline
already performs for a granular tool, written against a different interface
(`InnerCallRecord` instead of `ToolCall + ToolResult`). Four helper functions
were carved out of the effects precisely so this hook could call them
(`record_new_page`, `run_tab_group_work`, `annotate_pages_with_ownership`,
`persist_screenshot`). That is a deep module trying to be born. Verdict:
**necessary today, shallow, and the second adapter of an interface nobody
has named.** It is the deepening target.

**`helper_runtime.rs`.** Delete it and `preload_helpers` reappears in its one
caller (`dispatch.rs:474`), `discovery` is one effect, and
`helper_info_json` moves into `script_hook.rs`. Nothing concentrates. Its
five public functions are as wide as its 120 non-test lines. The one policy
it holds ("load the helpers for the hosts of the tabs I own") is the only
thing worth keeping. Verdict: **shallow glue; absorb into the deep module.**
It also hides a constraint: `HelperSource` is JavaScript against the bespoke
`browser` SDK. A Playwright engine cannot load it. Helper selection is
engine-neutral; helper content is engine-specific.

**The bridge in `run.rs` (`BrowserBridge::call` + `dispatch`).** Delete
`call` and the JS↔JSON marshalling, page targeting and the
authorize→dispatch→page_created→record choreography reappear in every SDK
method. Delete `dispatch` and 35 primitives lose their arg parsing. Both earn
their keep against the QuickJS runtime. But the choreography does not belong
there: it is host policy (what to record, when to claim, how to time, how to
estimate tokens) living inside one engine's bridge, in a crate that "cannot
reach the host". A second engine that never routes through
`BrowserBridge::dispatch` (rustwright in-process, or a sidecar) gets none of
it unless it copies `call` line by line. Verdict: **the runtime half is
deep; the choreography half is misplaced, and the method table is a shallow
interface duplicated four times** (BOOTSTRAP_JS, the `dispatch` match, the
tool `DESCRIPTION`, and the published skill).

---

## Shallow or deep: today's modules on this path

| Module | Verdict | Why |
|---|---|---|
| `browseros-core` snapshot/refs/observer/input | **Deep** | ~4k lines of AX capture, OOPIF routing, stable ref minting, stale-node recovery behind ~5 pub fns. |
| `browseros-core::BrowserSession` as an interface | **Wide and porous** | Pub sub-managers, raw `cdp()`/`cdp_events()`, `ProtocolSession`, pub free fns. The host bypasses it for discovery (`tab_registry.rs`) and downloads. Callers must know: refs are shared last-writer-wins per page; PageIds appear lazily; `goto` ignores `errorText` and returns Ok after its timeout; `click` returning `Ok(None)` means the untrusted JS fallback ran. |
| `run.rs` QuickJS runtime (limits, interrupt, logs, marshalling) | **Deep** | Small interface (`code`, `timeout` → `RunOutcome`), lots hidden. |
| `run.rs` `BrowserBridge::dispatch` method table | **Shallow** | Interface (35 SDK methods) equals implementation (35 match arms), listed four times. |
| `framework.rs::InnerCallHook` | **Right idea, wrong placement** | The port exists (two adapters already: `ScriptInnerCallHook` in the host, `MockHook` in tests), but choreography is on the engine side, pages are `u32`, and helper persistence rides the same trait. |
| `script_hook.rs` | **Shallow; earns its keep only as the second copy** | See deletion test. |
| `helper_runtime.rs` | **Shallow glue** | See deletion test. |
| `dispatch.rs` pipeline (guards→execute→effects→observers) | **Deep as a mechanism, leaky in its inputs** | `ToolFlags` derived from `tool == "tabs" && action`, `ARBITRARY_SCRIPT_TOOLS` by name; effects take the whole `ToolCall`. |
| `effects/tab_groups.rs` | **Deep** | Locks, retries, exists-checks, title sync, colour lock, timeout behind `apply`/`run_tab_group_work`/`collapse`/`close`. Over-wide parameter: takes `ToolCall` when it needs identity, browser, ownership, default group. |
| `effects/{ownership_claims, tab_activity, tabs_list_view, page_ownership_notice, session_naming}` | **Small adapters at a real seam** | Fine individually; together with `script_hook.rs` they show the duplication. |
| `observers/audit.rs` | **Two paths in one file** | `build_event`+worker for parents; `persist_screenshot` inline for children. |
| `observers/run_failure.rs` | **Thin adapter over a deep reporter** | Fine. The smell is `REPORTED_TOOL = "run"`. |
| `services/run_failures.rs` | **Deep** | One method `report(...)` hides consent, sink choice, daily budget, fingerprinting, classification. |
| `services/browser/tab_registry.rs` | **Deep** | `Target.*` subscription, lagged rebuild, tombstones, popup inheritance, claim release behind `observe/resolve/tab_for_target`. |
| `services/sessions/tab_ownership.rs` | **Shallow** | ~22 pub methods over ~280 lines, two concerns (page→owner map; per-agent tab-group state machine) under one lock. |
| `services/sessions/session.rs` | **Shallow** | ~25 pub methods, mostly getters; the dispatch admission/stop linearization is the one deep part. |
| `services/helpers.rs` | **Deep behind a wide interface** | Store, provenance, parse/format, analysis; but raw and structured write paths are both public. |
| `db/audit_log.rs` | **Deep** | SQL + task projection behind record/list/mark. |
| `services/screenshots.rs` | **Shallow** | Path join + fs; `path_for` leaks to HTTP handlers. |
| `services/session_efficiency.rs` | **Deep enough** | Reads audit rows only. Counts child rows as dispatches (baseline inflates by N×3000 per script). |
| `claw-app` `Timeline.tsx` | **Shallow renderer** | Reads toolName, argsJson (raw), resultMeta.isError, duration, screenshot, url. Ignores parent linkage. |

---

## Candidates

### 1. Deepen the per-action host bookkeeping into `ScriptDispatch` — **Strong**

Dependency category: in-process (ownership, activity, foreign pages) +
local-substitutable (SQLite audit, tempdir already used in tests) + ports &
adapters (CDP through `CdpConnection`, already in place).

**Files.** `api/mcp/script_hook.rs`, `api/mcp/dispatch.rs`
(`execute_with_cancellation`, EFFECTS, OBSERVERS), `api/mcp/effects/
{ownership_claims,tab_activity,tabs_list_view,page_ownership_notice}.rs`,
`api/mcp/effects/tab_groups.rs::run_tab_group_work`,
`api/mcp/observers/audit.rs`, `api/mcp/helper_runtime.rs`,
`crates/browseros-mcp/src/framework.rs::InnerCallHook`,
`crates/browseros-mcp/src/tools/run.rs::BrowserBridge::call`.

**Problem.** "What the host does around one browser action" is implemented
twice against two interfaces, and the choreography sits inside the QuickJS
bridge. A second engine is a third copy.

**Solution.** One host module, `ScriptDispatch`, owns an in-flight script
dispatch and accounts each action once: note foreign pages, write the child
row through the audit worker (with the parent-first ordering invariant),
queue the coalesced screenshot, record tab activity, touch liveness, claim
and group new pages, annotate page lists, and hand out the helper catalog.
Engines call it through a `ScriptHost` port; the granular-tool effects
become callers of the same functions (a granular tool is a one-action
script).

**Wins.**
- locality: choreography in one module, not two plus a bridge
- leverage: one interface, N engines, one fake for tests
- audit ordering invariant enforced once
- child screenshots go through the worker (coalesced, off the script path)
- delete `script_hook.rs`, `helper_runtime.rs`, four carved-out helper fns
- `InnerCallHook`'s two adapters become `ScriptHost`'s three

### 2. Key page identity on the fork's `tabId` at the host seam — **Strong**

Dependency category: in-process (the maps) + local-substitutable (ledger in
SQLite).

**Files.** `services/sessions/tab_ownership.rs`, `db/session_tabs.rs`,
`services/browser/tab_registry.rs`, `services/cockpit` tab activity
(`page_id` + `target_id` incarnation), `framework.rs` (`page: Option<u32>`),
`script_hook.rs`, `services/browser/connection.rs:125-139`.

**Problem.** Three page keys are in play. `PageId` (per-process counter)
keys the in-memory ownership map and the hook interface; `tabId` (fork,
stable across CDP clients) keys the ledger, replay, screenshots and cleanup;
`targetId` keys audit rows and activity incarnations. They already disagree
(popups). A second CDP client can produce `targetId`/`tabId`, never `PageId`.

**Solution.** `PageOwnership` keyed by `TabId`; the action seam accepts a
`PageRef { Page | Tab | Target }` and the host resolves it through
`PageManager`/`TabRegistry`; `PageId` stays the agent-facing handle that the
host mints (a Playwright engine's new page gets a `PageId` on
`page_created`, so audit rows and cockpit tabs stay comparable across
engines). Split the tab-group state machine out of `PageOwnership`.

**Wins.**
- one ownership answer for list, notice, screenshots, replay, cleanup
- popups classify correctly in `tabs list`
- a sidecar can report actions by `tabId`
- `browser/connection.rs` stops depending on `sessions`

### 3. Declare tool semantics on `ToolMetadata`, stop name-sniffing — **Strong**

Dependency category: in-process.

**Files.** `dispatch.rs` (`ARBITRARY_SCRIPT_TOOLS`, `ToolFlags` from
`tool == "tabs"`), `observers/run_failure.rs` (`REPORTED_TOOL`),
`helper_runtime.rs::discovery` gate, `distill.rs`,
`crates/browseros-mcp/src/tools/mod.rs::metadata_for_tool` (name list),
`claw-app/components/audit/Timeline.tsx::NOTABLE_TOOLS`,
`claw-app/screens/replay/replay.data.ts::TOOL_TO_VERB`.

**Problem.** Whether a tool is a script, which arg holds the script, and
whether it creates/closes/lists pages are decided by string match in seven
places across two languages. `evaluate` is treated as a script tool (gets a
hook and helper preloading it never uses). A second script tool means seven
edits.

**Solution.** `ToolMetadata { accepts_page_arg, kind: ToolKind }` with
`ToolKind::{Script { code_arg }, PageNew, PageClose, PageList, Page, Other}`
declared by each `ToolDef`. Host code reads the kind. The dispatch row can
later carry the kind so the cockpit stops keeping its own lists (contract
change: follow-up, not tonight).

**Wins.**
- second script tool is one `ToolDef`
- run-failure, discovery, warn-log follow automatically
- `evaluate` stops paying script costs
- cockpit lists have a source of truth

### 4. Collapse the SDK method table in `run.rs` — **Worth exploring**

Dependency category: in-process.

**Files.** `run.rs` (`BOOTSTRAP_JS`, `DESCRIPTION`, `BrowserBridge::dispatch`,
`target_page`, `tool_def`, `build_tool_args`), `api/mcp/prompt.rs`,
`skills/browseros-neo/SKILL.md`.

**Problem.** The bespoke SDK's ~35 methods are enumerated in the JS shim, the
Rust match, the tool description and the published skill. `target_page`
re-derives page-first-ness by string prefix. Interface as wide as
implementation, four times.

**Solution.** One Rust table `SdkMethod { name, page_first, arity, handler }`
that generates the JS shim and the method list of the description; the
bridge becomes a lookup.

**YAGNI caveat.** If the Playwright engine becomes primary and `run` is
frozen, skip this. It pays only while `run` keeps changing.

**Wins.**
- one list, four consumers
- adding a primitive is one row
- `target_page` disappears

### 5. Helpers carry an engine dialect — **Worth exploring**

Dependency category: local-substitutable (filesystem; tests already use
`tempdir`).

**Files.** `services/helpers.rs` (`HelperMeta`, layout
`<host>/<name>.md`), `helper_runtime.rs`, `framework.rs::HelperSource`,
`run.rs::load_preloaded_helpers`, `script_hook.rs` save/list/read.

**Problem.** Helpers are JS against the bespoke SDK. `preload_helpers` would
hand a Playwright engine unusable source, and `saveHelper` from a Playwright
script would poison the QuickJS catalog. Helper persistence also rides the
action seam (`InnerCallHook::save_helper`), which has nothing to do with
actions.

**Solution.** `HelperDialect { BrowserSdk, Playwright }` in `HelperMeta` and
the path; a `HelperCatalog` preloaded per (owned hosts × dialect); a
`HelperStore` port separate from `ScriptHost`; `helpers.rs` narrows to
save/list/read (raw write and parse/format become internal seams).

**Wins.**
- engine loads only its dialect
- persistence leaves the action seam
- `helpers.rs` interface halves

### 6. Cockpit derives structure from rows, not tool names — **Worth exploring**

Dependency category: in-process (TypeScript).

**Files.** `components/audit/Timeline.tsx`, `screens/replay/replay.data.ts`,
`screens/task-detail/task-detail.helpers.ts`, `modules/api/audit.hooks.ts`,
`api/http/sessions.rs` (row mapping), `db/audit_log.rs::result_meta`.

**Problem.** Rows render flat; the only nesting is the ordering trick. The
parent `run` row lands only in the Session tab while its children land in
per-page tabs with no parent. Scripts display as escaped JSON cut at 4 KB.
`result_meta` stores no error text, so a failed child cannot say why.
"Notable" and the replay verb are name lists.

**Solution.** Group by `parentDispatchId` client-side; render a script row's
code as a code block (shiki is already a dependency); mark notable as "has
children or kind == Script"; store bounded error text in `result_meta`.
Nothing here blocks the second engine: rows written through `ScriptDispatch`
already display identically.

**Wins.**
- both engines readable the same way
- children keep their parent in page tabs
- no per-tool lists in the UI

---

## Also noted (not cards)

- `session_efficiency.rs` sums child rows' output estimates and counts them
  as dispatches, so a script of N primitives adds (N+1)×3000 baseline tokens.
  Deliberate per `script_hook.rs:131-133`, but it inflates "savings" and will
  double under a second engine.
- `browseros-core` leaks host vocabulary: tab-group policy inside
  `PageManager::new_page` (`pages.rs:321-339`), window choice in
  `Browser::new_page`, MCP wording in the dialog line
  (`page_signals.rs:46`), LLM-directed error prose. Not on tonight's path.
- Two races seen by reading only: `new_page` vs a concurrent `list()` can
  mint two PageIds for one tab (`pages.rs:341-350`); two concurrent
  snapshots can mint the same `eN` for different nodes (`refs.rs:335`,
  `observer.rs:154`). A Playwright engine that snapshots for the agent would
  need to serialize per page as `run` scripts implicitly do.
- `bounded_args_json` (`db/audit_log.rs:976-978`) truncates to 4 KiB and
  redacts nothing, so values a script fills into forms land in `args_json`.
  A Playwright engine's `fill`/`type` actions would land the same way; the
  `ScriptDispatch` write path is the one place to add redaction.
- `run_failures.rs:178-183` docstring says "with no DSN this refuses
  everything"; `from_env` actually falls back to a file sink. Stale comment.
- `BrowserMcpService` (`crates/browseros-mcp/src/service.rs`) is a second
  host of `execute_tool` with no production caller. A hypothetical seam.

---

## Seam for a second engine

### Where it goes

Between **the engine** (anything that runs a script and performs browser
actions) and **the host's script dispatch** (which accounts each action).
Today this seam exists as `InnerCallHook`, and by the skill's rule it is
already real (two adapters: `ScriptInnerCallHook`, `MockHook`). It is placed
wrong in three ways:

1. The choreography (authorize → run → page_created → record, with timing
   and the token estimate) is on the engine side, in `BrowserBridge::call`.
   Every engine must reproduce it.
2. Pages are `u32` PageIds, which only this process's `PageManager` mints. A
   second CDP client has `targetId`/`tabId`.
3. Helper persistence rides the same trait.

Move it: the host owns a concrete deep `ScriptDispatch`; engines get it
through `ToolCtx` as a `ScriptHost` port and only *report* actions; the host
does the rest.

Where each concern is written once:

| Concern | Written once in | Engine's only obligation |
|---|---|---|
| child audit rows, parent-first ordering | `ScriptDispatch::action_finished` via the audit worker | bracket each action with `action_started`/`action_finished` |
| page claiming + tab grouping | `ScriptDispatch::page_created` (`record_new_page` + `run_tab_group_work`) | report page creation with any `PageRef`; the host also hears `Target.targetCreated` through `TabRegistry` and reconciles a late report (idempotent) |
| ownership notice | `action_started` notes foreign pages; `page_ownership_notice` reads them | none |
| screenshots | preview queued with the child row (worker, coalesced) | none |
| replay-recorder | ledger claim from `page_created` | none |
| run-failure telemetry | observer keyed on `ToolKind::Script { code_arg }` | none |
| helper preloading | `HelperCatalog` on the dispatch, filtered by engine dialect | load its dialect |
| cancellation + 30 s cap | `RunControl { cancel, deadline }` from the host | honour both (QuickJS: interrupt handler; sidecar: abort message, then kill) |

### Designed twice, briefly

- **A. Keep `InnerCallHook` where it is; add a second `impl` per engine.**
  Rejected: the choreography is copied into each engine; the `u32` page key
  blocks a sidecar outright.
- **B. Observe the browser, not the engine.** The host subscribes to CDP
  events on its own socket and reconstructs actions. Zero engine cooperation,
  works for any client. Rejected as the primary seam: CDP has no "an action
  happened" event for input dispatched by another client, so audit parity
  (a row per click/fill with args) fails. Kept as the safety net for page
  creation, which `TabRegistry` already does for the ledger.
- **C. Engine reports, host choreographs (recommended).** Needs a chokepoint
  in each engine: QuickJS has `__browserosCall`; an in-process rustwright
  adapter wraps `RustwrightPage` methods (we write that shim anyway); a Node
  Playwright sidecar can tap the client↔server protocol channel where every
  action is one message.

### Dependency categories, named

- **The browser over CDP:** *remote but owned (ports & adapters)*. The port
  is `CdpConnection` (`browseros-core/src/connection.rs`); the production
  adapter is `browseros_cdp::CdpClient`; in-memory adapters already exist in
  tests (`OnePageConnection`, `PageListConnection`, `GroupDispatchRecorder`).
  The fork itself doubles as a *local-substitutable* stand-in for the
  conformance suite (`bun contracts/claw-mcp/tests/run.ts --smoke`). It is
  not category 4: we own the fork and its `Browser.*` domain.
- **A hypothetical sidecar process:** *remote but owned (ports & adapters)*.
  The port is `ScriptEngine`; the production adapter is the sidecar transport
  (stdio JSON lines or a Unix socket, plus process lifecycle); the test
  adapter is an in-memory `FakeEngine` that emits a scripted action sequence.
  Playwright/Node *inside* the sidecar is category 4 (true external), but that
  stays inside the adapter and never reaches the host's seam.
- **rustwright in-process:** an in-process library, still an adapter of
  `ScriptEngine`. Its browser dependency is the same category-3 port, on its
  own socket, so from the browser's view it is a second client.

### Proposed Rust signatures (not built; names are proposals)

```rust
// crates/browseros-mcp/src/framework.rs — engine-neutral port types.
// Replaces InnerCallHook, InnerCallRecord, HelperSource and
// ToolCtx::{inner_call_hook, preloaded_helpers}.

/// How an engine names a page. The host resolves any variant to a PageId
/// through PageManager (Page/Target) or TabRegistry (Tab). Sidecars use Tab.
pub enum PageRef {
    Page(PageId),
    Tab(TabId),
    Target(TargetId),
}

pub struct ActionStart<'a> {
    /// Engine-neutral method name, e.g. "input.click", "page.goto", "locator.fill".
    pub method: &'a str,
    pub page: Option<PageRef>,
    /// JSON array of arguments; the host bounds it on write (4 KiB today).
    pub args: &'a Value,
    pub from_helper: bool,
}

pub struct ActionEnd<'a> {
    pub outcome: Result<&'a Value, &'a str>,
    pub output_token_estimate: i64,
}

/// Opaque. Minted by `action_started`, consumed by `action_finished`. Carries
/// the start instant, the resolved PageId and the bounded args, so an engine
/// cannot mis-time or mis-attribute a row.
pub struct ActionToken { /* private */ }

/// The port an engine calls. Implemented once by the host (`ScriptDispatch`)
/// and once by a test fake. Never refuses: ownership is a label.
pub trait ScriptHost: Send + Sync {
    fn action_started<'a>(&'a self, start: ActionStart<'a>) -> BoxFuture<'a, ActionToken>;
    fn action_finished<'a>(&'a self, token: ActionToken, end: ActionEnd<'a>) -> BoxFuture<'a, ()>;
    /// A page the script opened (newPage, popup). Idempotent: the host also
    /// hears Target.targetCreated and reconciles. Returns the agent-facing id.
    fn page_created<'a>(&'a self, page: PageRef) -> BoxFuture<'a, PageId>;
    /// Tri-bucket ownership for a page list, same as `tabs list`.
    fn annotate_pages<'a>(&'a self, pages: &'a [Value]) -> BoxFuture<'a, Vec<Value>>;
    /// Helpers for the caller's owned-tab hosts, already filtered to `dialect`.
    fn helpers(&self) -> &HelperCatalog;
    fn helper_store(&self) -> &dyn HelperStore;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HelperDialect { BrowserSdk, Playwright }

pub struct HelperCatalog {
    pub dialect: HelperDialect,
    pub sources: Vec<HelperSource>, // { name, source }
}

/// Persistence only; nothing about actions.
pub trait HelperStore: Send + Sync {
    fn save<'a>(&'a self, host: &'a str, name: &'a str, dialect: HelperDialect, source: &'a str)
        -> BoxFuture<'a, Result<(), String>>;
    fn list<'a>(&'a self, host: &'a str, dialect: HelperDialect) -> BoxFuture<'a, Vec<Value>>;
    fn read<'a>(&'a self, host: &'a str, name: &'a str) -> BoxFuture<'a, Option<String>>;
    fn host_of<'a>(&'a self, page: PageRef) -> BoxFuture<'a, Option<String>>;
}

/// What the host hands every engine. The deadline is the 30 s cap; both
/// tokens fold session, client and operator cancellation (dispatch.rs today).
pub struct RunControl {
    pub cancel: CancellationToken,
    pub deadline: Instant,
}

pub struct ScriptRequest {
    pub code: String,
    pub timeout: Duration, // already clamped by the tool
}

/// Today's `RunOutcome`, shared, so every engine yields the same MCP result
/// shape and the cockpit renders both without change.
pub struct ScriptOutcome {
    pub ok: bool,
    pub value: Option<Value>,
    pub logs: Vec<String>,
    pub error: Option<String>,
}

pub enum EngineError {
    Syntax(String),
    Cancelled,
    Engine(String),
    /// Sidecar missing, failed to start, or crashed mid-run.
    Unavailable(String),
}

/// The seam. `QuickJsEngine` (in browseros-mcp) is adapter one;
/// `PlaywrightEngine` (in claw-server-rust or a new crate, because it needs
/// the sidecar lifecycle) is adapter two; `FakeEngine` (tests) is three.
pub trait ScriptEngine: Send + Sync {
    fn dialect(&self) -> HelperDialect;
    fn run<'a>(
        &'a self,
        request: ScriptRequest,
        host: Arc<dyn ScriptHost>,
        session: Arc<BrowserSession>, // the host's CDP client; a sidecar may ignore it
        control: RunControl,
    ) -> BoxFuture<'a, Result<ScriptOutcome, EngineError>>;
}

// ToolCtx change: one field replaces two.
pub struct ToolCtx {
    // ...existing fields...
    /// Present only while a script tool runs. Engines call it; granular tools ignore it.
    pub script: Option<ScriptContext>,
}
pub struct ScriptContext {
    pub host: Arc<dyn ScriptHost>,
    pub control: RunControl,
}
```

```rust
// apps/claw-server-rust/src/api/mcp/script_dispatch.rs — the deep module.
// Replaces script_hook.rs and helper_runtime.rs; absorbs
// ownership_claims::record_new_page, tab_groups::run_tab_group_work,
// tabs_list_view::annotate_pages_with_ownership, observers::audit::persist_screenshot.

pub struct ScriptDispatch {
    call: ToolCall,                                  // identity, dispatch_id, started_at_ms, state
    foreign_pages: Mutex<BTreeMap<PageId, String>>,  // read by page_ownership_notice
    helpers: HelperCatalog,
}

impl ScriptDispatch {
    /// Preloads the caller's helpers for `dialect`. Called by
    /// execute_with_cancellation for any ToolKind::Script tool.
    pub async fn open(call: &ToolCall, dialect: HelperDialect) -> Arc<Self>;
    pub fn foreign_pages(&self) -> Vec<(PageId, String)>;
}

impl ScriptHost for ScriptDispatch { /* the eight concerns above, once */ }

// Engine selection is the tool handler's business, not the seam's:
// run::definition() builds a QuickJsEngine; the Playwright ToolDef (host-side,
// appended to the catalog by ClawMcpService) builds a PlaywrightEngine from
// AppState. Both call `engine.run(request, ctx.script.host, ctx.session, ctx.script.control)`.
```

### On the 30 s cap and long scripts

The engine seam is dispatch-shaped: one `ToolCall` is one script with one
`RunControl`. That is the right shape for tonight, because cancellation
(session stop, client disconnect, cockpit Stop) and the MCP client timeout
all attach to a dispatch. A job model for longer Playwright scripts would be
a change at the *tool* seam (a tool that returns a handle and a tool that
polls it), not at the engine seam: the job's actions still nest under the
job's dispatch id through the same `ScriptDispatch`, and the cockpit already
sorts children after a parent stamped at start.

### Assumptions to verify

- ASSUMPTION: a Node Playwright sidecar can expose a per-action chokepoint
  by tapping its protocol channel. The playwright spike owns this.
- ASSUMPTION: rustwright's `RustwrightPage` methods can be wrapped to report
  actions; its public locator surface is UNVERIFIED per CONTEXT.md.
- ASSUMPTION: the fork's `tabId` is stable across CDP clients within one
  browser run (`tab_registry.rs:46-47` says so; not measured tonight).
- ASSUMPTION: the dispatches HTTP contract stays unchanged tonight; the
  `kind` field in candidates 3 and 6 is a follow-up that needs codegen.

---

## Top recommendation

**Candidate 1: deepen into `ScriptDispatch`, with candidate 2's `tabId`
key as its page identity.** It is the one refactor that makes the second
engine cheap: after it, `PlaywrightEngine` is an adapter that reports
actions and pages, and everything the owner listed as non-negotiable
(audit parity, tab-group parity, ownership as a label, per-step screenshots,
replay, run-failure telemetry, helpers, cancellation) is inherited rather
than re-implemented. Do candidate 3 alongside it; it is small and removes
the seven name lists the new tool would otherwise have to join.
