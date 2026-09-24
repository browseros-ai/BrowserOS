# VERDICT: NOT CLOSABLE OVERNIGHT

**The requested Rust exposure gap is small and now prototyped: 378 added lines, 15 public methods, zero existing implementation changes beyond one module declaration. Background creation and single-target attachment both work.** Label-based fill, role-based Enter/check/hover, text waits, selection, reads, matcher probes, and composite locator JSON all ran against isolated BrowserOS neo without Python. The verdict applies to a **complete Playwright-shaped JS facade**, which remains **L**: much of Python's strictness, option normalization, retries, events, request/response objects, routing callbacks, and error behavior still belongs to the shim. A deliberately bounded JS facade is **M**, not an engine rewrite. The measured results remove the initial dependency blocker for option A; they do not establish complete compatibility.

## Added public functions and delegation

All additions live in [fork/src/native_locator_spike.rs](fork/src/native_locator_spike.rs), a new private module that adds public methods to the existing public types. It deliberately remains synchronous and uses the existing owned runtime. Existing APIs are unchanged.

In this table, every method has an explicit optional `&CancelToken`; page methods also accept a timeout. Exact complete signatures, including every argument and return type, are listed in [public-signatures.md](evidence2/public-signatures.md) and appended below.

| Added method | Existing private implementation it delegates to | Important retained limit |
|---|---|---|
| `RustwrightBrowser::new_page_with_background` | `create_target_cancellation_safe` → `attach_existing_page` | Default context; passes explicit `background` to Target.createTarget; keeps target-guard cleanup until delivery |
| `RustwrightBrowser::attach_to_target` | Target.getTargetInfo for one ID → `attach_existing_page` | Checks page type; shares one deadline; no Target.getTargets/pages sweep |
| `RustwrightPage::fill_locator_json` | `native_fill_body` → `page_fill_actionable_with_script_async` | Strict, non-forced, index 0; retains native fill guard cancellation cleanup |
| `press_locator_json` | `press_locator_for_native_input` | Native focus/key pipeline; shim's pre-action strictness wait is not added |
| `type_locator_json` | `type_locator_for_native_input` | Per-character native input; delay supported; slow in this probe |
| `hover_locator_json` | `page_pointer_actionable_async(..., Hover, ...)` | Strict basic hover; no modifiers/position/force/trial options |
| `set_checked_locator_json` | `page_set_checked_actionable_async` | `true` = check, `false` = uncheck; native click, idempotency, checked-state verification |
| `select_options_locator_json` | `locator_select_apply_script` → `evaluate_locator_dispatch_for_page` | One apply attempt; JSON `ok/selected/missing` envelope, not complete auto-wait policy |
| `inner_text_locator_json` | `evaluate_locator_json_cancelable` → `evaluate_locator_for_page` | Immediate read; missing-element/strictness waiting still belongs to higher-level Locator |
| `text_content_locator_json` | Same, with textContent body | Same immediate-read limit |
| `is_visible_locator_json` | Same, with `!!el && visible(el)` | Immediate state, not an assertion |
| `wait_for_locator_json` | New strict convenience wrapper → `page_wait_for_selector_async` | Core navigation retry/terminal-state handling; no host polling |
| `wait_for_selector_json` | `page_wait_for_selector_async` | Explicit strict/non-strict; supports attached, detached, visible, hidden |
| `probe_locator_state_json` | `locator_probe_state_body` → `evaluate_locator_action_for_page` | Returns tagged wire JSON: decode with existing `decode_wire_value`; options can scroll/probe stability |
| `assert_locator_json` | `locator_assertion_duration` → `assert_locator_for_page` | Returns `{passed,actual,log}`; host must throw when passed=false, even if Rust returned Ok |

No generic public transport interface or new async runtime was added. The host still must call these through `spawn_blocking` and carry cancellation explicitly.

### How Python reaches the same engine

The exact private Rust signatures are captured in [private-signatures.md](evidence2/private-signatures.md). Its line references point to the fork; they are one line later than the pinned original because of the new module declaration.

| Python operation | Python/PyO3 path | Private Rust implementation / responsibility |
|---|---|---|
| Locator.fill | `_fill_impl` → `PyPage::locator_fill_actionable` | `page_fill_actionable_async` → `page_fill_actionable_with_script_async`; native fill preparation, input, cleanup |
| Locator.press | `_wait_for_single` → `PyPage::press_key_native` | `press_locator_for_native_input`; Python supplies strictness/remaining deadline |
| Locator.type / press_sequentially | `_wait_for_single` → `PyPage::type_text_native` | `type_locator_for_native_input` |
| Locator.hover | `_hover_impl` → state/actionability wait + `_dispatch_locator_pointer_action` → PyPage method of that name | `resolve_locator_point` plus native pointer dispatch/settlement inside the PyO3 method; the patch instead reuses the already-existing native `page_pointer_actionable_async(Hover)` |
| Locator.check/uncheck | `_check_impl/_uncheck_impl` → target-state probe, pointer click, checked-state recheck | `locator_probe_state_body`, `locator_check_state_body`, locator evaluation and pointer dispatch; patch uses existing native `page_set_checked_actionable_async` equivalent for basic options |
| Locator.select_option | `_select_option_impl` → readiness wait + retry loop → `_select_apply` → `PyPage::locator_select_apply` | `locator_select_apply_script` → `evaluate_locator_dispatch_for_page`; Python retains the repeated missing-option/readiness policy |
| Locator.inner_text/text_content | Shim fast paths, `_ensure_single` + `wait_for(attached)` → PyPage inner_text/text_content | `evaluate_locator_for_page` with read body; low-level getters do not by themselves reproduce every Locator wait |
| Locator.is_visible | Shim fast paths or `_ensure_single` → `PyPage::is_visible` | Locator resolution + `visible(el)`, not a retrying assertion |
| Locator.wait_for / Page.wait_for_selector | `PyPage::wait_for_selector` | `page_wait_for_selector_async` → `run_locator_wait_retry` → `evaluate_wait_for_selector_attempt` |
| Actionability/state probe | `Locator::_target_state` → `PyPage::locator_probe_state` | `locator_probe_state_body(options_json)` → `evaluate_locator_action_for_page` |
| expect(locator) | `_native_locator_poll` → `PyPage::assert_locator` → `assert_locator_sync` | `assert_locator_for_page`; core has 23 matcher kinds, Python builds expected/negated/strict fields and formats/raises failure |

This distinction matters: hover/check have a substantial Python option path, and select has a real Python retry loop. Merely making similarly named Rust functions public does not port those behaviors.

## New coverage and numbers

Browser: installed `Chrome/151.0.8162.137`, fresh profiles and free loopback ports. Rust/cargo 1.96.1, edition 2024 probe; all calls from a Tokio blocking worker. Fork build with `--no-default-features`: **34.43 s wall** using the existing artifact cache and fork lockfile. Probe build: **12.45 s wall**; subsequent harness-only builds approximately 1.1–1.4 s. Dependency tree contains no PyO3. These are observed warm-machine builds, not cold-machine benchmarks.

| Operation actually exercised | Outcome | Final headless wall ms |
|---|---|---:|
| Connect | works | 7.912 |
| Attach exactly chosen existing target | works; other page remains unattached | 3.149 |
| Fill adopted target by label | works | 22.197 |
| Create page with background=true | works | 21.448 |
| Goto + load | works | 56.331 |
| getByLabel("Your name").fill("Ada") | works; value asserted | 59.165 |
| getByRole("textbox",{name:"Your name"}).press("Enter") | works; delayed submission asserted | 69.683 |
| getByText("Submitted Ada").waitFor(visible) | works through native wait | 142.675 |
| innerText | “Submitted Ada” | 2.885 |
| getByRole("checkbox").check | works | 20.170 |
| Repeated check | no duplicate change; event count stays 1 | 21.780 |
| uncheck | checked=false | 23.627 |
| selectOption by value / label / index | correct blue / red / blue | 6.511 / 6.453 / 4.885 |
| Hover by role | mouseenter handler observed | 19.446 |
| Type “ Lovelace” by label | correct “Ada Lovelace”; **slow** | **5,832.314** |
| textContent / isVisible | correct text / true | 3.975 / 4.379 |
| State probe | count=1, attached/visible=true, checked=false | 6.090 |
| expect visible | passed=true, actual=true | 5.676 |
| Expected failing hidden assertion (80 ms budget) | Ok envelope with passed=false | 81.516 |
| Missing select option | immediate `{"ok":false,"missing":["value=missing"]}` | 11.053 |
| locator(...).locator(...) descendant spec | correct scoped button | 45.490 |
| filter({hasText:"Text action"}) spec | correct button | 447.342 |
| nth(1) / first() / last() specs | item1 / item0 / item1 | 318.336 / 19.393 / 401.175 |
| Frame locator + label fill | same-origin frame input = “Inside” | 321.526 |
| Wait detached | successful false return for absent element | 6.861 |
| Page wait with strict=false and two matches | works | 8.484 |
| Missing-locator 80 ms timeout | expected timeout error | 107.308 |
| In-flight wait cancellation, token fired after 60 ms | expected Cancelled error | 69.580 |
| Fill after cancellation | works | 206.482 |
| PNG screenshot | valid and visually inspected | 35.472 |

Final headless workflow: **8,467.250 ms**, success. Full records: [headless.jsonl](evidence2/headless.jsonl), [measurements.md](evidence2/measurements.md), [fixture PNG](evidence2/fixture-2.png).

Two earlier runs are intentionally preserved. Typing timed out with a 2 s budget, then succeeded in 5.381 s with an 8 s budget and again in 5.832 s. Its latency cause was not established, so this report does not call it a solved performance path. The second run's visibility assertion used an incomplete matcher without `expected:true`; after matching Python's payload, it passed. These were harness/coverage findings, not additional changes to the 378-line core patch. See [attempt 1](evidence2/headless-attempt1.jsonl) and [attempt 2](evidence2/headless-attempt2.jsonl).

### Background creation and single-target attachment

**Both pass.** Headed activeTabId was **1331773596** before creation, after `new_page_with_background(true)`, and after goto/fill/Enter/wait. The new target was `86731DE6FB516A65A5E83AF23BFE672D`. Headed page creation took **17.747 ms**; the complete headed workflow took **348.744 ms**.

Headless target `9030538D7166B18A8FC02F8397B9625A` changed to `attached:true`. The unrelated page `4DC5116ED7C11C713771F68828184896` stayed `attached:false` immediately after attach and through the complete workflow. The headed run independently checked the same invariant. The probe never calls `Browser::pages()`; inventory comes from the independent raw client. Connections may still attach service-worker targets as upstream already does; the invariant measured here is other **page tabs**.

Evidence includes complete `Target.getTargets`, `Browser.getActiveWindow`, and `Browser.getWindows` responses: [headless](evidence2/headless.jsonl), [headed](evidence2/headed.jsonl).

## Patch size and exact fork

```text
 src/lib.rs                  |   1 +
 src/native_locator_spike.rs | 377 ++++++++++++++++++++++++++++++++++++++++++++
 2 files changed, 378 insertions(+)
```

- Fork: `/Users/shadowfax/llm/code/browseros-project/grove-ref/main-1/playwright-runtime/spikes/rustwright/fork/`
- Branch: `spike/public-locator-surface`
- Base: `dfb481b6ab641b013c6d35fb12ed223efc752f38`
- **Local fork commit: `73dd99247607198934a006230761710a223d66ad`**
- [Exact patch](evidence2/rustwright-surface.patch), [portable Git bundle](fork-spike.bundle), [hashes](evidence2/artifact-sha256.txt)
- Original Rustwright checkout and monorepo unchanged. No push or monorepo commit.

The 378 lines include comments and rustfmt-expanded signatures. This count is the Rust dependency patch only; the retained probe/fixture/coordinator are separate under `code2/`.

## What a complete JS facade still lacks

Sizes are engineering estimates, not measured elapsed time: **S** = localized exposure or JSON construction; **M** = several coordinated implementation/validation tasks; **L** = broad behavioral compatibility with lifecycle, concurrency, and failure handling. The full facade is **L** even though this particular native patch is **S**.

| Area | Size | What already exists and what remains |
|---|---|---|
| locator().locator(), filter({hasText}), nth, first/last | **S** for builders, **M** including selector parsing/validation | Engine accepts descendant/filtered/nth specs; measured successful clicks. JS can build identical JSON. Same-page/frame restrictions, regex encoding, strictness, escaping, custom engines and raw selector-string parsing still need a faithful shim. |
| Frames | **M** | Frame specs already route actions; role click from probe 1 and label fill from probe 2 work. Need Frame/FrameLocator objects, frame lifecycle/identity, nested and cross-origin validation, detachment and navigation tests. OOPIF is not proven by the same-origin fixture. |
| Dialogs | **M** | Public Rust page event receiver includes Dialog and handles with accept/dismiss. Need JS callbacks/promise scheduling, pre-armed subscriptions, rejection on close, listener disposal, and avoiding a deadlock while an action is waiting for dialog handling. |
| Downloads | **M** | Native events expose guid/url/suggested-name; Python bridge has enable_downloads/download_event_waiter and private wait_for_download_event. Need native exposure for completion/progress and artifact paths, then JS Download methods/saveAs/cancel/failure/lifetime. Notification is not download completion. |
| waitForURL / waitForLoadState | **M / S** | Native wait_for_load_state already exists; waitForURL in Python performs URL/glob/regex/callback matching plus document state. Need one deadline, same-document navigation semantics, callbacks, and terminal errors. |
| waitForResponse | **M** | Private wait_for_network_event tracks cursor, event log, session and request store. Public page event enum does not include Response. Need durable subscribe-before-action waiter, Response/Request objects and body access, JS predicates, cancellation/close handling and queue overflow policy. Snapshot network_records is not an equivalent waiter. |
| expect matchers | **M** | Existing 23-kind locator assertion engine is now callable. JS must build complete matcher payloads, encode regex/expected values, .not/options, inspect passed, format assertion failures, and implement remaining page/API-response/general-value matchers. The malformed-payload probe showed missing fields can become a false assertion rather than validation error. |
| keyboard / mouse objects | **M** | Native locator press/type and private direct keyboard/mouse dispatch exist. Need explicit keyboard.down/up/insertText state, modifiers, mouse coordinates/buttons, sequencing, cancellation cleanup and option parity. Modifier/coordinate state lives in Python today. Slow type measurement needs diagnosis. |
| route | **L** | PyPage implements Fetch enable/disable/continue/fail/fulfill and route_event_waiter; handlers/order/fallback and Request/Response objects are in the shim. Need native event/command surface and reentrant JS callbacks while browser requests are paused, timeout/cleanup, routing precedence, and safe coexistence with the server's other CDP session. |
| Remaining Locator/action semantics | **M** | Patch uses fixed index 0 (nth is encoded in spec), strict basic fill/hover/check, no force/trial/position/modifier options. Press/type/reads still need the shim's strictness and missing-element checks. Select's readiness/retry loop is not in the wrapper. |
| Embedded JS runtime integration | **M** for a bounded subset; **L** for full compatibility | This spike did not implement JS objects, Promise scheduling, callbacks, handles/wire values, event pumping, runtime limits, audit children, session tab ownership/groups, or per-step screenshots. These must survive cancellation and concurrent action/event callbacks. |

The core is not devoid of these capabilities: many are private functions or PyO3-owned glue, so reusing them is possible. But event-driven parts are not all “add pub to one function.” In particular, some PyO3 methods contain orchestration inline; reproducing their shape without exposing transport details needs additional Rust types or adapters.

### Selector composition is already expressible

I executed the pinned Python shim's **pure builder functions only** (31 dependency functions extracted through Python AST; no PyO3 import). Actual outputs are retained in [python-builder-results.json](evidence2/python-builder-results.json), with source in [python-selector-builders.py.txt](evidence2/python-selector-builders.py.txt):

```json
{"selector":"css=#scope >> text=Scoped action",
 "spec":{"kind":"descendant","base":{"kind":"css","selector":"#scope"},
         "inner":{"kind":"text_selector","text":"Scoped action","exact":false}}}
{"selector":"button:has-text(\"Text action\")",
 "spec":{"kind":"css","selector":"button","has_text":"Text action"}}
{"selector":"css=.item >> nth=1",
 "spec":{"kind":"nth","base":{"kind":"css","selector":".item"},"index":1}}
{"selector":"css=.item >> nth=-1",
 "spec":{"kind":"nth","base":{"kind":"css","selector":".item"},"index":-1}}
```

The relevant source is `_selector_spec` (line 2883), `_within_spec` (2963), `_apply_locator_options` (3063), and `Locator.filter/nth/locator` (22873–22996) in `python/rustwright/sync_api.py`. `_apply_locator_options` builds a filter via `Locator.filter`; filter supports base/has/has_not/has_text/has_not_text/visible. First is nth(0); last is nth(-1).

Browser execution verified descendant-with-inner-role, filtered-has_text, and nth(1/0/-1), including the correct handler for each. The exact Python text-chain and CSS-has_text builder outputs above were source-executed, not separately clicked in the browser. The Rust selector-string parser remains unchanged and would still reject the original strings; a JS facade must send these specs to the spec entry points. It does **not** need a second DOM matching engine for those compositions.

## Prototype limits that affect the estimate

- **Select succeeds at the low-level attempt while high-level selection may still be pending.** A missing value returned Ok JSON with ok=false in 11 ms despite a 1 s budget. Python's outer loop retries; the patch intentionally exposes that same low-level step. A complete facade must wait for readiness/options and interpret that envelope.
- **Assertions need complete matcher objects and result handling.** The correct visibility payload is `{"kind":"visible","strict":true,"negated":false,"expected":true}`. A hidden assertion on a visible element returns Ok with passed=false after its timeout; throwing is a binding responsibility.
- **Probe results use the tagged wire format.** The existing `decode_wire_value` handles object/reference wrappers. Select/assertion envelopes are already plain JSON; conflating these return formats is a binding bug.
- **Basic locator wrappers are not full option parity.** The patch makes the engine reachable; it does not port the 30,491-line Python shim. This count includes more than locators and is context, not a proposal to translate every line.
- **Cancellation is observable but full cleanup is not proven.** A waiting operation returned Cancelled in 69.6 ms and a subsequent fill succeeded. The underlying JS wait may retain its timer/observer until its own deadline after the Rust future is dropped; this spike did not prove page-side cancellation cleanup. Physical-input commit/key-release policy remains upstream.
- **Typing latency is unresolved.** The tested nine-character suffix took over 5 seconds with no requested delay. A 2 s timeout failed. Correct final input was verified with the larger budget; the cause cannot be attributed from this probe.
- **All calls remain synchronous.** No public async surface was added. Cancellation must reach CancelToken; aborting an already-running spawn_blocking task alone is insufficient.
- **The scope is a proof, not a merge-ready dependency release.** Browser assertions, formatting, builds and patch checks passed. There was no comprehensive upstream conformance suite, cross-platform test, event-race stress test, or production-server integration.

## Reproduction and artifacts

On the current machine:

```sh
cd /Users/shadowfax/llm/code/browseros-project/grove-ref/main-1/playwright-runtime/spikes/rustwright/fork
git log -1 --oneline
git diff HEAD^ HEAD --stat
CARGO_TARGET_DIR=../code/target cargo build -p rustwright-core --release --no-default-features --locked

cd ../code2
CARGO_TARGET_DIR=../code/target cargo build --release --no-default-features --locked
bun run.ts headless
bun run.ts headed
python3 inspect_selector_builders.py
python3 summarize.py
python3 verify_cleanup.py
```

The coordinator uses the original recipe: fresh `evidence2/profile-*`, free CDP port, `--headless=new` unless headed, local HTTP fixture, and independent raw WebSocket client. It refuses port 9108 and closes its own browser/server in finally. Code2 preserves the original probe under code/ and its first report.

The portable [follow-up source archive](source-2.tar.gz) contains code2, the patch, signature maps, and command recorder. The [Git bundle](fork-spike.bundle) includes the fork commit/history. A clean machine can fetch that bundle's HEAD into an empty Git repository, check out commit 73dd99247607198934a006230761710a223d66ad, adjust code2/Cargo.toml's two absolute path dependencies to that fork, and run the commands above with BrowserOS neo, Rust 1.96.1, Bun and Python installed. The coordinator expects its executable at ../code/target/release/rustwright-spike2, so retain the shown target-directory setting.

All shell build/probe/source-inspection commands and their outputs continue in [commands.log](evidence/commands.log). Every raw CDP request/response and Rust action result is in evidence2/*.jsonl. [Private signatures](evidence2/private-signatures.md), [public signatures](evidence2/public-signatures.md), and [dependency tree](evidence2/cargo-tree.txt) are retained.

## Decisions and assumptions

DECIDED: Clone the pinned source into a standalone scratch fork with --no-hardlinks. Do not modify the original checkout or monorepo. Commit only in the fork to give the mediator an exact reusable revision.

DECIDED: Keep the Rust patch additive and narrow: expose existing engine work without moving/refactoring old code. Expose a single boolean set_checked method for both check and uncheck.

DECIDED: Preserve Python's low-level select/assertion result envelopes rather than silently inventing high-level success semantics. Explicitly count the missing facade logic in the remaining work.

DECIDED: Call the full Playwright-complete JS objective NOT CLOSABLE OVERNIGHT, while rating this already-working Rust exposure patch S and a bounded useful JS facade M. These are different scopes.

ASSUMPTION: Size estimates assume reuse of the existing engine and JS runtime chosen by the mediator, not a new CDP engine. They include behavior/cleanup verification; code that merely compiles is not considered complete.

ASSUMPTION: The independent initial fixture target represents a pre-owned tab; the other initial page represents an unrelated tab. Attached flags and selected-tab IDs were checked from raw CDP. No owner tabs were queried.

DECIDED: Preserve failed attempts and the probe correction for missing assertion payload fields. No engine fix was needed to turn that malformed assertion into a valid one.

DECIDED: Shut down every process launched by this follow-up. Browser PIDs 41307, 66374, 70042, and 78169 have exited; latest coordinator PIDs/listeners are gone, and no process retains this follow-up's profile path. Final pgrep main-browser inventory contains only owner PID 11851 (its helper processes also remain). See [cleanup-summary.json](evidence2/cleanup-summary.json) and [full process inventory](evidence2/processes-final.txt). Other workers' processes were never killed.

## Exact added public signatures

The following is generated directly from the committed fork source.

Line 11:
```rust
pub fn new_page_with_background( &self, background: bool, cancel: Option<&CancelToken>, ) -> RwResult<RustwrightPage>;
```

Line 40:
```rust
pub fn attach_to_target( &self, target_id: &str, timeout: Duration, cancel: Option<&CancelToken>, ) -> RwResult<RustwrightPage>;
```

Line 79:
```rust
pub fn fill_locator_json( &self, locator_json: &str, value: &str, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<()>;
```

Line 117:
```rust
pub fn press_locator_json( &self, locator_json: &str, key: &str, delay: Option<Duration>, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<()>;
```

Line 141:
```rust
pub fn type_locator_json( &self, locator_json: &str, text: &str, delay: Option<Duration>, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<()>;
```

Line 165:
```rust
pub fn hover_locator_json( &self, locator_json: &str, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<()>;
```

Line 187:
```rust
pub fn set_checked_locator_json( &self, locator_json: &str, checked: bool, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<()>;
```

Line 210:
```rust
pub fn select_options_locator_json( &self, locator_json: &str, values_json: &str, labels_json: &str, indexes_json: &str, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<String>;
```

Line 236:
```rust
pub fn inner_text_locator_json( &self, locator_json: &str, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<Option<String>>;
```

Line 255:
```rust
pub fn text_content_locator_json( &self, locator_json: &str, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<Option<String>>;
```

Line 274:
```rust
pub fn is_visible_locator_json( &self, locator_json: &str, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<bool>;
```

Line 293:
```rust
pub fn wait_for_locator_json( &self, locator_json: &str, state: &str, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<bool>;
```

Line 305:
```rust
pub fn wait_for_selector_json( &self, locator_json: &str, state: &str, strict: bool, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<bool>;
```

Line 332:
```rust
pub fn probe_locator_state_json( &self, locator_json: &str, options_json: &str, timeout_ms: Option<f64>, cancel: Option<&CancelToken>, ) -> RwResult<String>;
```

Line 355:
```rust
pub fn assert_locator_json( &self, locator_json: &str, matcher_json: &str, timeout_ms: f64, polling_interval_ms: f64, cancel: Option<&CancelToken>, ) -> RwResult<String>;
```
