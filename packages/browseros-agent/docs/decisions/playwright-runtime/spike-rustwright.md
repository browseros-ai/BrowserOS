# VERDICT: WORKS WITH CAVEATS

Rustwright runs in-process from Rust without Python or a Node driver, attaches to BrowserOS neo's real default context, performs trusted locator clicks/fill/Enter/evaluate/PNG capture, adopts existing tabs, and coexists with independent CDP sessions. **It does not satisfy the full requirement unchanged:** native `Browser::new_page()` switches the active tab; Playwright `>>` chains and `:has-text()` fail; semantic locator specs are publicly usable for **click only**, so `getByLabel(...).fill(...)` and native locator `wait_for` are not exposed. A measured alternative—host CDP `Target.createTarget({background:true})`, then `Browser::pages()` and matching `Page::target_id()`—allows background actions without switching tabs. All facade/core calls must run off Tokio executor threads: `spawn_blocking` worked, while a direct call from `#[tokio::main]` panicked. This proves a usable dependency with integration gaps, not full Playwright compatibility.

## Coverage table

Results concern the pinned checkout `dfb481b6ab641b013c6d35fb12ed223efc752f38`, rustwright/core 0.3.0, on installed BrowserOS neo `Chrome/151.0.8162.137`. “Works” means the retained local fixture exercised that form, not universal semantic parity. `Core::spec_click` below abbreviates the exact public symbol `rustwright_core::RustwrightPage::click_locator_json_with_cancel(locator_json, index, timeout_ms, strict, cancel)`. No changes were made to Rustwright.

| Playwright selector/action form | Result | Rust symbol and actual evidence |
|---|---|---|
| `chromium.connectOverCDP(httpEndpoint)` | works | `rustwright::chromium().connect_over_cdp(ConnectOptions::new(endpoint))`; `is_owned=false`, `is_connected=true` |
| `browser.newPage()` in default context | works, focus caveat | `Browser::new_page()`; same default context as sentinel and other raw-created tabs |
| `page.goto(url)` | works | `Page::goto`, `GotoOptions::wait_until("load")` |
| `getByRole("button", {name, exact:true}).click()` | works via core spec; named facade builder not exposed | `Core::spec_click` with `{"kind":"role","role":"button","name":"Reveal result","exact":true}`; button text is “Reveal”, so accessible name matching is demonstrated |
| `getByText("Text action", {exact:true}).click()` | works via core spec | `Core::spec_click`, `{"kind":"text","text":"Text action","exact":true}`; fixture click count increments |
| `getByLabel("Your name").click()` | works via core spec | `Core::spec_click`, `{"kind":"label","value":"Your name","exact":true}`; active element becomes `#name` |
| `getByLabel(...).fill("Ada")` | **not exposed** | Core exposes only selector-string `RustwrightPage::fill[_with_cancel]`, not spec fill; facade has no locator builder |
| `getByTestId("test-button").click()` | works via core spec | `Core::spec_click`, `{"kind":"test_id","value":"test-button","attribute":"data-testid"}`; correct handler observed |
| CSS `#role`, `css=#role` | works | `Page::click` |
| XPath `xpath=//button[@id='role']`, bare `//button[@id='role']` | works | `Page::click` |
| `text=Text action`, `text="Text action"`, `text=/Text action/` | works | `Page::click`; plain, quoted exact, regex all hit expected button |
| `role=button[name="Reveal result"]` selector string | **broken** | `Page::click` treats it as CSS; DOM selector SyntaxError |
| `css=#scope >> text=Scoped action` | **broken** | `Page::click`; `#scope >> text=Scoped action` passed to `querySelectorAll` |
| `css=#scope >> css=button` | **broken** | Same invalid CSS path |
| `button:has-text("Text action")` | **broken** | `Page::click`; invalid DOM CSS selector |
| `locator(".item").nth(1).click()` | works via core spec; named builder not exposed | `Core::spec_click`, `{"kind":"nth","base":{"kind":"css","selector":".item"},"index":1}`; `last="item1"` |
| `css=.item >> nth=1` | **broken** | `Page::click`; invalid DOM CSS selector |
| `frameLocator("#frame").locator("#frame-button").click()` | works via native convenience method | `Page::click_in_frame("#frame","#frame-button",...)`; same-origin frame handler increments parent counter |
| `frameLocator("#frame").getByRole("button",{name:"Frame action"}).click()` | works via core spec | `Core::spec_click`, frame wrapper with inner role spec; frame counter increments |
| `click_in_frame("#frame","text=Frame action",...)` | **broken** | Native method hardcodes inner `kind:"css"`; text prefix becomes invalid CSS |
| `page.fill("#name","Ada")` | works | `Page::fill`; value verified via object evaluation and screenshot |
| `locator("#name").press("Enter")` | works via page method | `Page::press_key(Some("#name"),"Enter")`; delayed “Submitted Ada” appears |
| `locator("text=Submitted Ada").waitFor()` / `wait_for_selector` | **not exposed** | Probe uses explicit bounded polling with `Page::is_visible`, not a claimed native wait primitive |
| Wait for navigation load | works | `Page::goto(..., GotoOptions::wait_until("load"))`; separate `Page::wait_for_load_state` exists but was not independently exercised |
| Evaluate expression returning object | works | `Page::evaluate` returns `serde_json::Value`: `{"clicks":1,"name":"Ada","result":"Revealed","submitted":"Submitted Ada"}` |
| Evaluate async Promise | works | `Page::evaluate` waits on a 300 ms Promise during coexistence test |
| Screenshot PNG | works | `Page::screenshot(ScreenshotOptions::default().full_page(true).path(...))`; valid 26,178-byte PNG, 516×475, visually inspected |
| `browser.contexts()[0].pages()` | **not exposed literally** | `Browser::pages()` is the working default-context equivalent; contexts builder missing |
| Attach a specified existing target | works by enumerate + match; direct method not exposed | `Browser::pages()?.into_iter().find(|p|p.target_id()==target_id)`; navigated, filled, evaluated raw-created tab |
| Disconnect attached browser without closing tabs | works | `Browser::close()`, `RustwrightBrowser::close()`; raw CDP still sees all four default-context pages afterward |

The semantic JSON entry point is **not a general locator object interface**. Click can accept the richer specs; fill, press, hover, reads, and waits do not automatically gain them. Compile-fail evidence confirms missing `Browser::contexts`, `Browser::attach_to_target`, `Page::get_by_role`, `Page::wait_for_selector`, and `RustwrightPage::fill_locator_json`: [missing-surface.log](evidence/missing-surface.log). Complete inspected public signatures: [core-public-surface.txt](evidence/core-public-surface.txt).

## Numbers

Apple Silicon Mach-O arm64; cargo/rustc 1.96.1; spike edition 2024 and rust-version 1.94. Rust 1.94 itself was not installed/tested. Cargo resolved dependencies compatible with the declared 1.94 requirement.

| Build measurement | Result |
|---|---:|
| Initial release dependency build, new scratch target, crates index/download included | 21.14 s wall |
| Complete action probe built in a separate fresh `target-cold`, registry already cached | **21.72 s wall**, 119.69 s user, 9.15 s system |
| Incremental build adding the action probe | 1.56 s wall |
| Follow-up adding headed background-adoption measurement | 1.75 s wall |
| Empty binary, release with `strip="symbols"` | 340,896 bytes |
| Full final probe binary | 6,867,600 bytes |
| Delta versus empty binary | **6,526,704 bytes (6.224 MiB)** |
| `cargo tree \| wc -l` | **296** |
| Unique package/version entries in host tree, including root | 121 |
| Cargo metadata packages including target-specific dependencies/root | 163 |
| PyO3 entries in dependency tree/features/metadata | **0** |

The clean-target complete probe binary was 6,867,568 bytes, 32 bytes smaller than the final headed follow-up. These are standalone executable deltas including the Tokio/serde harness and selected Rustwright paths, **not a measured marginal increase to claw-server-rust**, which already carries shared dependencies. No LTO was specified by the scratch project. `otool -L` shows only macOS system libraries (`libiconv`, `libSystem`), no Python or Node runtime. Core generated 190 warnings, primarily unused code in the Python-disabled build; compilation succeeded.

Single-run timings below include operation completion, not just command enqueue. They are a local smoke measurement, not a statistical benchmark. The fixture deliberately delays submission by 200 ms; the wait's remaining delay is therefore intentional.

| Operation | Wall ms |
|---|---:|
| Native CDP attach | 7.660 |
| Core CDP attach (second Rustwright connection) | 2.702 |
| Default-context initial page adoption | 7.054 |
| Create page | 24.096 |
| Navigate + load | 53.080 |
| Role + accessible-name click | 16.378 |
| Fill input | 17.840 |
| Press Enter | 57.020 |
| Bounded polling until submitted text visible | 155.179 |
| Evaluate object | 1.768 |
| PNG screenshot | 24.546 |
| Exact text spec click | 11.765 |
| Label spec click | 19.390 |
| Test-ID spec click | 46.835 |
| Nth spec click | 14.796 |
| Frame role-spec click | 19.865 |
| Native frame CSS click | 23.203 |
| Adopt later raw-created target through pages inventory | 69.808 |
| Existing-target fill | 52.408 |
| Five clicks concurrent with raw CDP | 6.300–17.186 each |
| Native evaluate after raw Runtime.disable/detach | 0.425 |

All action durations, including failing selectors, are in [measurements.md](evidence/measurements.md); underlying records are in [headless.jsonl](evidence/headless.jsonl). Headless Rust workflow completed in 1,227.704 ms. Headed follow-up workflow completed in 272.357 ms. See [build-initial.log](evidence/build-initial.log), [build-cold-full.log](evidence/build-cold-full.log), [cargo-tree.txt](evidence/cargo-tree.txt), [cargo-features.txt](evidence/cargo-features.txt), and [artifact-sha256.txt](evidence/artifact-sha256.txt).

## Focus result

**Native new_page changes the selected tab.** Two headed isolated runs reproduced this. In the final run, `Browser.getActiveWindow` and `Browser.getWindows` agreed throughout:

| Stage | windowId | activeTabId | Tabs |
|---|---:|---:|---:|
| Before Rustwright attach | 1817352785 | 1817352787 | 2 |
| After attach and pages inventory | 1817352785 | 1817352787 | 2 |
| After native new_page | 1817352785 | **1817352788** | 3 |
| After native goto/click | 1817352785 | 1817352788 | 3 |
| After raw background create → native adoption → goto/click | 1817352785 | **1817352788** | 4 |
| After disconnect | 1817352785 | 1817352788 | 4 |

The background-created tab was 1817352789 / target `DAE3FE74C67D4E0FA0075FCF77C5C3CE`, remained `isActive:false`, and was successfully clicked. So host-created background tabs are a **measured viable route**. The native facade has no background option on new_page.

Code explains the creation gap: [core create_page_async](https://github.com/Skyvern-AI/rustwright/blob/dfb481b6ab641b013c6d35fb12ed223efc752f38/src/lib.rs#L49672) sends only `{"url":"about:blank"}` to `Target.createTarget`, then attaches the debugger after target creation. No `background:true` is supplied. The local fork's separate `Browser.createTab` defaults to background; that does not change Rustwright's request. This diagnosis is consistent with the measurement; no Chromium code was edited.

This measures selected tabs/window metadata in the isolated app, not cross-application macOS activation or every possible popup/dialog. Fresh profile defaults were used; the preference was not overridden. Evidence: [headed.jsonl](evidence/headed.jsonl), [first headed run](evidence/headed-original.jsonl).

## Coexistence result

**Works in the exercised case.** Native facade connection, independent core connection, and Bun's plain WebSocket CDP connection were simultaneously attached. The raw client issued `Target.getTargets`, `Target.attachToTarget(flatten:true)`, `Runtime.enable`, and repeated `Runtime.evaluate` on the same Rust-driven page. Five Rust clicks and a pending 300 ms evaluated Promise ran while **18 raw evaluations** incremented `window.rawTicks`; there were **zero raw errors**. The observed click was trusted (`lastTrusted:true`).

The raw client then disabled Runtime and detached its own session. Rust evaluation and clicking still worked, including after the independent core connection disconnected. The only raw detached event recorded was for the explicit `Target.detachFromTarget` request and the same raw session ID. No unexpected detach or Runtime ownership conflict was observed. This was a bounded smoke probe using plain CDP, not the full browseros-cdp crate or a load/network-interception stress test.

Existing-page result: raw `Target.createTarget` returned `45F65A4E5DD47BD3089121288DFEFB23`. Native inventory found that target, filled it with “Existing tab”, and returned the value through evaluate. Final inventory included all **four** default-context pages, including the initial blank page and the pre-attach sentinel that represented a user's tab. The owner's actual browser was never enumerated.

`browser.contexts()[0].pages()` does not compile in the Rust facade; `Browser::pages()` is its actual surface. [list_pages_raw](https://github.com/Skyvern-AI/rustwright/blob/dfb481b6ab641b013c6d35fb12ed223efc752f38/src/lib.rs#L57647) asks for browser contexts and targets, excludes non-default contexts, and **attaches to every matching page**. Thus “every tab” means default-context pages, not incognito contexts or browser UI/service-worker targets. Direct target-ID adoption is private; the public workaround attaches the inventory before filtering it. Incognito exclusion is source-inspected, not separately exercised here.

## Async shape result

**Synchronous facade and synchronous public core.** Both positive workflows ran inside `#[tokio::main]` → `tokio::task::spawn_blocking`; handle construction, actions, close, and destruction stayed inside that closure. They completed without deadlock/panic. Bun was only a fixture/process/CDP measurement harness; it was not a driver subprocess used by Rustwright.

A separate negative probe invoked `chromium().connect_over_cdp(...)` directly on the Tokio main thread: **exit 101**, with:

> Cannot start a runtime from within a runtime.

Exact stderr: [direct-tokio.jsonl](evidence/direct-tokio.jsonl). The isolated browser was closed afterward.

Relevant pinned source symbols:

- [connect_browser_over_cdp_cancelable, core line 45625](https://github.com/Skyvern-AI/rustwright/blob/dfb481b6ab641b013c6d35fb12ed223efc752f38/src/lib.rs#L45625): creates a separate Tokio multi-thread runtime with **two worker threads**, then `runtime.block_on`.
- [BrowserInner::block_on_raw, line 29236](https://github.com/Skyvern-AI/rustwright/blob/dfb481b6ab641b013c6d35fb12ed223efc752f38/src/lib.rs#L29236): directly calls its owned runtime's `block_on`.
- [RustwrightPage::click_locator_json_with_cancel, line 47255](https://github.com/Skyvern-AI/rustwright/blob/dfb481b6ab641b013c6d35fb12ed223efc752f38/src/lib.rs#L47255): synchronous public method invoking the private async action pipeline.
- [create_page_async, line 49672](https://github.com/Skyvern-AI/rustwright/blob/dfb481b6ab641b013c6d35fb12ed223efc752f38/src/lib.rs#L49672), `attach_existing_page` and `page_pointer_actionable_async`: private, not directly callable from the server.
- [Browser::pages_with_cancel, native line 254](https://github.com/Skyvern-AI/rustwright/blob/dfb481b6ab641b013c6d35fb12ed223efc752f38/rust-native/src/lib.rs#L254): a named OS worker thread plus `std::sync::mpsc::sync_channel` and blocking `recv_timeout` for the cancellable path. Ordinary page calls mostly use runtime blocking, not mpsc.
- [OwnedRuntime::drop, line 29156](https://github.com/Skyvern-AI/rustwright/blob/dfb481b6ab641b013c6d35fb12ed223efc752f38/src/lib.rs#L29156): transfers runtime destruction to an OS thread when dropped inside a Tokio context. This does not make direct action calls safe.
- There are **no public async functions** in the compiled core public surface. Internal async code and Python's async facade are not a public native Rust async interface.

## Cold-machine recipe

Prerequisites: Apple Silicon Mac with command line developer tools, Rust 1.96.1, Python 3, Bun (tested installed 1.3.6), and the tested BrowserOS neo application installed at `/Applications/BrowserOS neo.app`. Browser distribution and code-signing installation are external prerequisites; this spike does not download an arbitrary substitute Chromium. A fresh target directory and fresh browser profile are created; no live-browser connection is required.

The retained [source archive](source.tar.gz) contains the Cargo manifest/lockfile, Rust probe, compile-fail example, fixture, Bun coordinator, command recorder, cleanup checker, and summarizer. Copy that archive to the clean machine, then run:

```sh
rustup toolchain install 1.96.1
mkdir -p "$HOME/code/oss"
git clone https://github.com/Skyvern-AI/rustwright.git "$HOME/code/oss/rustwright"
git -C "$HOME/code/oss/rustwright" checkout dfb481b6ab641b013c6d35fb12ed223efc752f38

export SPIKE_ROOT="$HOME/rustwright-spike"
export RUSTWRIGHT_SOURCE="$HOME/code/oss/rustwright"
mkdir -p "$SPIKE_ROOT/evidence"
tar -xzf source.tar.gz -C "$SPIKE_ROOT"

python3 - <<'PY'
import os, pathlib
p = pathlib.Path(os.environ["SPIKE_ROOT"]) / "code/Cargo.toml"
s = p.read_text().replace("/Users/shadowfax/code/oss/rustwright",
                          os.environ["RUSTWRIGHT_SOURCE"])
p.write_text(s)
PY

cd "$SPIKE_ROOT/code"
rustup override set 1.96.1
python3 ../record.py '/usr/bin/time -p cargo build --release --no-default-features --locked'
python3 ../record.py 'cargo tree | wc -l'
python3 ../record.py 'cargo tree -e features'
python3 ../record.py 'stat -f "%N %z bytes" target/release/empty target/release/rustwright-spike'
python3 ../record.py 'bun run.ts headless'
python3 ../record.py 'bun run.ts headed'
python3 ../record.py 'ulimit -c 0; bun run.ts direct-tokio'
# Intentional compiler failure documents the absent surface:
python3 ../record.py 'cargo check --example missing_native_surface --locked'
python3 ../record.py 'cargo fmt --check'
python3 ../record.py 'python3 summarize.py'
python3 ../record.py 'python3 verify_cleanup.py'
```

On this machine, the ready-to-run directory is:
`/Users/shadowfax/llm/code/browseros-project/grove-ref/main-1/playwright-runtime/spikes/rustwright/code/`.

`run.ts` binds the fixture to a free loopback port, allocates a free CDP port (refuses 9108), creates a fresh profile, and launches:

```text
/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo
  --user-data-dir=<new spike evidence/profile-MODE-*>
  --remote-debugging-port=<free port>
  --headless=new                 # omitted for "headed"
  --no-first-run --no-default-browser-check about:blank
```

The exact resolved argv, HTTP URL, CDP endpoint, PIDs, every raw CDP request/response, Rust timings/results, and cleanup record are in each run's JSONL. Command invocations, outputs, and shell exit codes are retained in [commands.log](evidence/commands.log); full build output is preserved separately. Expected failed selector actions are logged and the probe continues. Other workflow failures return nonzero; the coordinator still closes its own browser and fixture in `finally`.

## Risks observed

- **Focus is a real failing requirement for native page creation.** No background parameter is exposed. The measured host background-create/adopt route avoids this; simply linking the crate and using new_page does not.
- **The Rust interface is narrower than the Python shim.** `selector_to_locator_json` ([line 49569](https://github.com/Skyvern-AI/rustwright/blob/dfb481b6ab641b013c6d35fb12ed223efc752f38/src/lib.rs#L49569)) recognizes leading text/css/xpath or otherwise uses CSS. It does not parse Playwright chains/pseudo-classes; richer click specs do not supply missing fill/wait methods.
- **No tab-scoped public attach entry point.** `Browser::pages()` adopts all matching default-context pages. That incurs protocol work and marks unrelated pages as debugger-attached; the BrowserOS focus policy itself consults that marker. Session ownership labels, audit records, tab grouping, and screenshots per action remain the host's responsibility.
- **Blocking and cancellation need care.** Dropping/aborting a Tokio blocking task does not cancel already-running synchronous work. Public `CancelToken`-accepting methods exist, but this probe did not prove the host's 30 s deadline/cancellation behavior. The cancellable pages inventory can leave its worker running until its own timeout.
- **Broad connection behavior.** Core connect initializes service-worker auto-attachment (`start_service_worker_stealth_auto_attach_cancelable`). Shared-target smoke coexistence passed; routing/network interception, concurrent navigation, session races, shutdown storms, and load were not tested.
- **Large alpha implementation.** Local `src/lib.rs` has **63,976 lines**. It contains mutex `unwrap()` and invariant `expect()` calls in live paths, e.g. attached-page registry locking, OwnedRuntime dereference, and newly-created target guard. The simple grep count (1,592 matching lines) includes tests and must not be interpreted as 1,592 production panic sites. The direct Tokio panic is a demonstrated failure, not merely a code concern.
- **No full selector/iframe conformance claim.** Frame tests cover one same-origin iframe; OOPIF, nested frame, shadow DOM, detachment, navigation races, accessibility edge cases, and strict-mode collisions remain unproven here. Fixture handlers and returned state were checked, rather than treating a success return as sufficient.
- **License: MIT**, recorded in the local LICENSE/Cargo.toml. This spike found no special license restriction beyond the license terms. Upstream labels the project alpha/Chromium-only and distinguishes surface coverage from behavioral parity in its [README](https://github.com/Skyvern-AI/rustwright) and [parity document](https://github.com/Skyvern-AI/rustwright/blob/main/docs/PARITY.md). Decisions above are grounded in the pinned local checkout and measured behavior, not moving main-branch claims.

## Decisions and assumptions

DECIDED: Keep all probe code, builds, profiles, screenshots, and report outside the monorepo in the requested spike directory. No monorepo files changed, no commit, no Chromium build, no owner browser access.

DECIDED: Use the native facade for ordinary operations and a second core connection solely to test the public locator-spec click entry point. This demonstrates both public surfaces without modifying upstream.

DECIDED: Use Bun only as an independent fixture/process/CDP harness. The Rust browser-control path has no JavaScript driver subprocess.

DECIDED: Classify the dependency as WORKS WITH CAVEATS because in-process attachment and actions work and a measured background-adoption path exists; explicitly reject unchanged native new_page as satisfying the focus invariant.

DECIDED: Treat native locator wait_for and spec-based fill as not exposed. A hand-written is_visible poll or CSS selector rewrite is a fallback, not proof that those Playwright forms exist.

ASSUMPTION: A raw-created pre-attach sentinel represents an existing user/session tab for default-context enumeration. It was actually enumerated and preserved. The real owner's tabs were never touched.

ASSUMPTION: Fresh profile defaults represent the installed app's default behavior. The supplied context mentions an older Chromium base; measured installed version is 151.0.8162.137. The focus conclusion is therefore tied to this installed build and the retained request/response evidence.

DECIDED: Do not kill another spike's processes merely to force “only owner remains.” All four browsers launched here (44725, 48212, 52090, 60680) exited, all recorded runner PIDs and loopback listeners are gone, and no process retains this spike's profile path. The final inventory also contained another worker's Playwright browser PID 31624 on port 60895 under `spikes/playwright/`, alongside owner PID 11851; both were left untouched. Thus the brief's literal “only 11851 remains” condition was not true in this concurrent workspace, while this spike's cleanup is verified. Evidence: [cleanup-summary.json](evidence/cleanup-summary.json), [full pgrep output](evidence/processes-final.txt).

DECIDED: Retain the initial recording-wrapper error (`status` is a read-only zsh variable); the underlying build succeeded and its complete time/output is preserved. Subsequent wrappers use `probe_exit`. Likewise, incorrect exploratory glob/path reads and the intentional compiler/runtime negative probes remain in the journal instead of being hidden.

Current documentation lookup followed the requested two-step Context7 workflow (`npx ctx7 library`, then `npx ctx7 docs /skyvern-ai/rustwright`); source and executable results take precedence where general documentation describes broader Python support.
