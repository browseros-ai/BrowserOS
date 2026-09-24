# VERDICT

**Node: WORKS WITH CAVEATS.** Real, unmodified `playwright-core` 1.63.0 attached to an isolated BrowserOS neo and passed the requested ordinary browser operations under Node 26.4.0, including locators, a same-origin iframe, screenshot and PDF. Existing tabs and fork tab/window IDs are reachable. A VM/Proxy demonstration emitted method-level audit events before returning to script code. **The full product promise does not work automatically:** stock `context.newPage()` steals tab selection; `expect` is absent from core; an asynchronous page event does not wait for claiming/grouping; and the VM plus these host objects is escapable. The runtime dependency works, but focus-safe creation, durable host acknowledgments, attribution, a complete audit interface, and OS isolation remain integration work.

**Bun-compiled: WORKS WITH CAVEATS.** A **64.10 MB** macOS ARM64 executable passed the same fixture with the original `node_modules` directory unavailable, a different working directory, and no Node/Bun executable on `PATH`. However, **unmodified Bun 1.3.6 does not attach successfully**: Playwright's bundled `ws` handshake times out. The successful binary substitutes Bun's `ws` implementation, rewrites two dynamic metadata imports to static imports during bundling, and externalizes unused BiDi imports. Those are explicit, version-sensitive adaptations to the real package. The unchanged script passes with the transport adaptation; this is not proof of all Playwright features or other platforms. Bun's VM/proxy experiments pass separately under the adapted Bun interpreter; the compiled executable was exercised on the browser-operation matrix, not the VM/audit matrix.

Measured on **2026-09-23**, Darwin ARM64, BrowserOS neo **151.0.8162.137**. Complete experiment commands, outputs, unsuccessful attempts, and timings are in [COMMANDS.log](COMMANDS.log). Source and documentation checks are in [SOURCES.md](SOURCES.md). Scratch code and binaries are retained in [code/](code/). No monorepo files were changed or committed; no Chromium build or browser download was run.

## Fidelity table

“Compiled” below means the adapted binary. Unmodified `bun run runner.mjs` failed at attach after 30 seconds, so its downstream operations did not execute.

| Form tried | Node | Compiled | Evidence / caveat |
|---|---|---|---|
| `chromium.connectOverCDP(httpEndpoint)` | Works | Works with adapter | Real local BrowserOS CDP, default profile context |
| `browser.contexts()[0]` | Works | Works | One default context; also exposes tabs created outside Playwright |
| `context.newPage()` | Caveat | Works headless; headed caveat applies | Node headed runs selected the newly created tab, 6/6 trials |
| `page.goto(url)` | Works | Works | Locally served HTTP fixture |
| `page.getByRole('button', {name}).click()` | Works | Works | Accessible button name, trusted input path |
| `page.getByRole(...).first().click()` through Proxy | Works | Not compiled-tested | Adapted Bun interpreter also passed; one `click` event |
| `page.getByLabel(...).fill()` | Works | Works | Labelled input contained the expected value |
| `page.keyboard.press('Enter')` | Works | Works | Fixture displayed `Submitted Ada Lovelace` |
| `page.getByText(...).waitFor()` | Works | Works | Text appears after a 50 ms timer |
| Standard `expect(locator).toBeVisible()` | Missing dependency | Missing dependency | Core exports no `expect`; `playwright/test` and `@playwright/test` are not installed |
| Injected lightweight `expect(locator).toBeVisible()` | Caveat | Caveat | Implemented with `locator.waitFor({state:'visible'})`; not full Playwright Test assertion semantics |
| `page.frameLocator('iframe').getByLabel(...).fill()` | Works | Works | Same-origin iframe, value read back as `inside` |
| `frameLocator(...).getByRole(...).click()` | Works | Works | Also passed through the VM Proxy |
| `page.evaluate(() => ({...}))` | Works | Works | Object serialization, title, input, submitted text, iframe value |
| `page.screenshot()` | Works | Works | 17,157-byte PNG; valid PNG signature |
| `page.pdf()` | Works | Works | 30,634-byte PDF, `%PDF-1.4`; headless only tested |
| `context.pages()` | Works | Works | Includes a raw-CDP-created external tab and startup tab |
| `context.newCDPSession(page)` + `Target.getTargetInfo` | Works | Works | `targetId`, numeric `tabId` and `windowId` present |
| Existing-page lookup by `targetId` | Works | Works | Clicked the fixture in the tab created before attach |
| `context.on('page')`, popup, `waitForEvent('page')` | Caveat | Not compiled-tested | Node/adapted Bun emit events; host claim must be awaited explicitly |
| `page.on('console')` / `page.on('pageerror')` | Works | Not compiled-tested | Forwarded text/error with target ID in Node/adapted Bun |
| `page.bringToFront()` / `window.focus()` on background tab | Caveat by design | Not headed-tested | Fork suppressed selection changes in Node headed probes |
| VM execution and sync timeout | Works with security caveat | Not compiled-tested | Node and adapted Bun interpreter tested; host-object escape succeeds |
| Second client `Runtime.enable` + `Runtime.evaluate` | Works | Not compiled-tested | Node Playwright plus a separate raw Bun process; bindings continued working |

This matrix is the fidelity claim. Upstream explicitly describes CDP as lower fidelity than Playwright's own protocol, and warns about externally supplied launch arguments. It also documents `noDefaults` for avoiding profile overrides. [Playwright CDP documentation](https://playwright.dev/docs/api/class-browsertype#browser-type-connect-over-cdp).

## Numbers

These are individual successful fixture runs on an already-running, warm browser, not distribution benchmarks. Node was first; subsequent runs encountered three extra tabs left by the intentionally unsuccessful Bun attach attempts. The standalone compiled run had five existing pages at attach. The wait deliberately includes a fixture delay; screenshot/PDF costs reflect a tiny page. The final cleanup removed all isolated browser processes.

| Measurement | Node | Bun interpreter with adapter | Compiled, dependencies unavailable |
|---|---:|---:|---:|
| Attach | 58.18 ms | 42.58 ms | 33.53 ms |
| `context.newPage` | 22.91 ms | 19.28 ms | 21.15 ms |
| `page.goto` | 41.20 ms | 30.36 ms | 28.13 ms |
| Role locator click | 28.53 ms | 28.02 ms | 29.07 ms |
| Label locator fill | 3.80 ms | 3.36 ms | 3.30 ms |
| Keyboard Enter | 1.81 ms | 1.19 ms | 1.29 ms |
| Text wait | 73.97 ms | 77.17 ms | 77.15 ms |
| Lightweight visibility assertion | 1.47 ms | 1.13 ms | 1.13 ms |
| Frame label fill | 15.72 ms | 16.29 ms | 16.95 ms |
| Frame role click | 29.98 ms | 29.10 ms | 29.15 ms |
| Evaluate | 1.12 ms | 1.27 ms | 1.19 ms |
| Screenshot | 46.40 ms | 37.76 ms | 30.70 ms |
| PDF | 51.02 ms | 50.51 ms | 49.84 ms |
| `context.pages` | 0.07 ms | 0.09 ms | 0.07 ms |
| RSS at connected | 140.3 MB | 128.5 MB | 117.6 MB |
| RSS at end of workload | 153.2 MB | 137.3 MB | 124.9 MB |

RSS is `process.memoryUsage().rss`, decimal MB, for the sidecar only. Browser memory is excluded. The transcript also records `resourceUsage().maxRSS`, but Node and Bun returned different units, so those values are deliberately not compared.

- `playwright-core`: **1.63.0**, installed by `bun add playwright-core`; `du -sh node_modules/playwright-core` returned **13M**. The lockfile pins the resolution.
- Successful compiled binary: **64,100,736 bytes = 64.10 MB = 61.13 MiB**, Mach-O ARM64, ad-hoc linker signature, no Developer ID signature.
- Retained binary SHA-256: `dd4acf33f8a03a9c7784acc10ce36ef2627d39a85de12c13229778a6b2c0ac05`.
- External parent measurement, process launch to receipt of `connected`, three fresh compiled processes: **164.92 / 160.91 / 156.36 ms**; median **160.91 ms**. Browser and filesystem caches were warm; this is process cold start, not cold boot or first-run Gatekeeper cost. Attach alone in those runs was 34.15 / 34.17 / 33.68 ms.
- A Node connect-only run reported 193.15 ms process uptime at `connected`, including 48.33 ms attach. This is a different timer and is not presented as a controlled speed comparison.
- Installed Homebrew Node occupies **93M**, including a **75M** `libnode.147.dylib`, and links other Homebrew libraries. Its 68,320-byte launcher is not a standalone Node distribution. A shippable Node package was not built or sized here.

### Bun failure and successful packaging changes

Exact unsuccessful commands are retained:

```sh
bun run runner.mjs
bun build --compile runner.mjs --outfile pw-sidecar-naive
bun build --compile runner.mjs --external chromium-bidi --outfile pw-sidecar-external
./pw-sidecar-external
```

The first and fourth timed out during WebSocket connection at about 30,010 and 30,005 ms. A five-second debug run logged HTTP `101 WebSocket Protocol Handshake` as an unexpected response after cancellation. The native raw Bun WebSocket had already succeeded, narrowing the observed failure to Playwright's bundled `ws` path rather than CDP reachability. The naive build failed resolving optional `chromium-bidi/lib/cjs/...` modules. Externalizing BiDi made a binary, but did not fix the transport.

The successful interpreter command is `bun run --preload ./bun-preload.cjs runner.mjs`. The preload replaces only the `ws` export of Playwright's utility module with Bun's built-in compatibility module. The successful [build.mjs](code/build.mjs) additionally performs checked, version-specific substitutions in the bundler's in-memory input:

1. `ws = require('./utilsBundle').ws` → `ws = require('ws')`.
2. Dynamic package metadata lookup → static `require('../package.json')`.
3. Dynamic browser registry metadata lookup → static `require('../browsers.json')`.

It marks `ws` and unused `chromium-bidi` external. Bun supplies `ws`; BiDi is not available in this CDP-only artifact. Installed package files remain unchanged. Bun warns that `upgrade` and `unexpected-response` events are not implemented. The successful test used loopback HTTP discovery and a direct local websocket, not redirects/authentication edge cases.

[standalone.py](code/standalone.py) copied the binary to another directory, temporarily renamed the original `node_modules`, set `PATH=/usr/bin:/bin`, ran the complete fixture, timed three connect-only processes, and restored the dependency directory in `finally`. This demonstrates independence from that package tree for the exercised operations. It does not prove that recorder, trace viewer, WebP codec, browser launch, or other lazy disk assets are embedded. [Pinned Bun executable documentation](https://github.com/oven-sh/bun/blob/bun-v1.3.6/docs/bundler/executables.mdx).

## Page identity

The script created a tab using a separate raw CDP websocket **before Playwright attached**, then found it in `browser.contexts()[0].pages()` by matching `Target.getTargetInfo`. It clicked that page successfully. Example from Node:

```json
{"targetId":"1263D25C5D03AABDAE0220BF50023B86","type":"page","title":"Playwright spike fixture","url":"http://127.0.0.1:60907/external","attached":true,"canAccessOpener":false,"browserContextId":"FA9F753E83AF37993C0AA1568EDB828C","tabId":530396293,"windowId":530396291}
```

The bridge uses public methods:

```js
const session = await context.newCDPSession(page);
const { targetInfo } = await session.send('Target.getTargetInfo');
await session.detach();
```

`targetId` is sufficient to find already-owned tabs; there is no need to create an incognito context with `browser.newPage()`. The fork's numeric IDs are available for existing ownership/group machinery. Enumerating and inspecting all pages is an O(number of pages) fallback; a host should retain a target-to-page registry and handle close/reconnect lifetimes. `context.pages()` is browser-wide, not a session ownership filter. Observing a page never by itself proves that the current agent created it.

## Audit feasibility

[audit.mjs](code/audit.mjs) executes an ordinary async script in `node:vm` with wrapped `browser`, `context`, and returned `page` objects. [audit-proxy.mjs](code/audit-proxy.mjs) preserves the real receiver, recursively wraps locator/frame/keyboard objects and page arrays, unwraps known objects passed back, and emits one completion event per method before returning its result. The same flow passed under Node and adapted Bun. Screenshot/PDF also passed through the final Node VM run.

Selected **actual emitted lines** from the final Node run:

```json
{"ev":"call","runtime":"node","id":1,"obj":"browsercontext","method":"newPage","args":[],"targetId":"D16A10B79BDBD07C165A5AB9B93A230F","url":"about:blank","ms":62.99,"ok":true}
{"ev":"call","runtime":"node","id":5,"obj":"locator","method":"click","args":[],"selector":"browsercontext.newPage().getByRole(\"button\",{\"name\":\"Show message\"}).first()","targetId":"D16A10B79BDBD07C165A5AB9B93A230F","url":"http://127.0.0.1:61952/","ms":30.31,"ok":true}
{"ev":"call","runtime":"node","id":15,"obj":"page","method":"screenshot","args":[],"targetId":"D16A10B79BDBD07C165A5AB9B93A230F","url":"http://127.0.0.1:61952/","ms":23.46,"ok":true}
{"ev":"console","runtime":"node","targetId":"D16A10B79BDBD07C165A5AB9B93A230F","type":"log","text":"forwarded console"}
{"ev":"pageerror","runtime":"node","targetId":"D16A10B79BDBD07C165A5AB9B93A230F","name":"Error","message":"forwarded page error"}
```

The locator chain emits `getByRole`, `first`, and exactly one `click`, with a readable selector description. A failed click emitted `ok:false`, `TimeoutError`, and the Playwright call log before the script caught it. Concurrent operations finish out of order: a pending `waitForEvent` can have a lower call ID than a click that completes first. Keep correlation IDs, not row arrival order, as the nesting/ordering source.

**Claim ordering is a real seam.** The raw event listener starts a simulated 40 ms host claim. In the unwrapped case, output order was `page_hook` → `unwrapped_newPage_returned` → `claim_complete`. In the wrapped case it was `page_hook` → `claim_complete` → `call newPage` → script continuation. The popup path also waits before returning the popup from `waitForEvent`. The 40 ms work is deliberately simulated, not a Rust database write or actual tab-group operation. The hook is immediate notification when Playwright exposes the page; it is not an atomic browser-level “claim before any page code runs” guarantee.

**This proves feasibility, not exhaustive or tamper-proof audit parity.** The prototype's recognized object list is finite; it needed an adjustment for bundled class names `_Page` / `_BrowserContext`. Production needs complete coverage of events, callbacks, routes, workers, handles, page lists, and borrowed methods, with identity preserved. `evaluate` callbacks must not be wrapped like host event callbacks because their source is serialized into the browser. The prototype blocks some private properties but reflection/host-function escapes remain. Console/pageerror forwarding is demonstrated; early popup messages can precede listener registration. Every `context.on('page')` notification must be attributed before claiming, because user/other-agent pages also appear there.

JSONL was written before promise resolution, but no Rust reader acknowledged persistence. Per-step host screenshots, audit DB insertion, parent dispatch linkage, redaction, bounded arguments, cancellation records, and tab grouping were **not implemented**. The measured screenshot operation shows the primitive is available; it does not demonstrate atomic capture for each audited action. Killing a worker can interrupt an action before its completion line; durable audit needs host-side start/pending state plus a terminated outcome.

## Focus

Headed isolated browser PID **69820**, CDP **61666**, fresh profile, default preferences. Queries used a raw browser websocket discovered from `/json/version`; no Playwright CDP session was used for `Browser.getActiveWindow` / `Browser.getWindows`.

| Operation | Observed selected tab |
|---|---|
| Before attach / after attach | Baseline `1009812222` stayed selected |
| Plain `context.newPage()`, trials 1–3 | Changed to `1009812223`, `1009812225`, `1009812227` |
| Same, `noDefaults:true`, trials 1–3 | Changed to `1009812231`, `1009812233`, `1009812235` |
| Raw `Target.createTarget({url:'about:blank', background:true})` | Baseline `1009812222` stayed selected |
| `bringToFront()` on that background page | Baseline stayed selected |
| `page.evaluate(() => window.focus())` on that background page | Baseline stayed selected |

`isActive` stayed true for the isolated window. Thus this measures tab selection within a foreground window, not whether an inactive app would be raised. The owner's browser was never queried or driven. The initial isolated app launch itself can activate its window; the protected product behavior being tested starts after launch/attach.

Playwright's installed `coreBundle.js:38598` sends `Target.createTarget` without `background`. **Inference from source plus the control:** a host adapter can preserve ordinary `context.newPage()` syntax while creating the tab with `background:true`, then resolving the corresponding Playwright page by target ID. Restoring selection after creation would still allow a visible switch and is not equivalent. This spike proves the raw creation and lookup primitives, not a production override or all popup/new-window paths. `noDefaults:true` does not fix creation focus.

The headed browser was closed immediately after these probes. No Chromium change is required for the tested background-creation primitive.

## Coexistence

[coexist.mjs](code/coexist.mjs) held a Node Playwright connection while a **separate Bun process**, [coexist-client.mjs](code/coexist-client.mjs), opened its own native websocket, called `Target.getTargets`, attached to the same page with `flatten:true`, enabled Runtime, and ran ten evaluations. Playwright filled an input, clicked, waited, and invoked an exposed host binding concurrently:

```json
{"ev":"playwright_during_raw_session","value":{"sum":42,"rawCount":7,"name":"concurrent"}}
```

No misbehavior was observed for those operations. The raw client then closed the page while a Playwright locator click was pending. Public error shape:

```json
{"name":"Error","message":"locator.click: Target page, context or browser has been closed\nCall log:\n\u001b[2m  - waiting for getByText('this will never exist')\u001b[22m\n","constructor":"TargetClosedError2"}
```

`page.isClosed()` became true; a subsequent `page.evaluate` failed with the same closed-target message. A fresh page still evaluated `6 * 7` to 42. Do not depend on the bundled constructor name `TargetClosedError2` as a stable public error contract. The second client exited with code 0 in the final run. An earlier harness run left stdin open; it was terminated, fixed to close stdin, and rerun successfully. This result does not establish safety for competing navigation, Fetch interception, debugger pauses, auto-attach changes, or binding name collisions.

## Sandbox and limits

Both Node and adapted Bun rejected direct `require('fs')` and `process` references in the VM. `codeGeneration:{strings:false,wasm:false}` rejected context-local `constructor.constructor` probes. A direct `page.constructor` property was rejected by the Proxy. **However, the requested stronger “cannot escape” claim is false with these host objects:**

```json
{"label":"host_function_escape","source":"page.goto.constructor('return typeof process')()","value":"object"}
{"label":"host_function_require_escape","source":"page.goto.constructor('return typeof process.getBuiltinModule(\"fs\").readFileSync')()","value":"function"}
```

These probes returned only types; no filesystem contents, environment variables, or credentials were read. `Object.getPrototypeOf(page).constructor` was also reachable. Removing globals and adding a few Proxy traps is not sufficient. Node explicitly disclaims VM as a security mechanism. [Pinned Node VM documentation](https://github.com/nodejs/node/blob/v26.4.0/doc/api/vm.md).

`while(true){}` under a 100 ms VM timeout threw `ERR_SCRIPT_EXECUTION_TIMEOUT` in **100.86 ms Node / 101.08 ms Bun**, and the same browser page still evaluated to 42. For an async continuation, [runaway.mjs](code/runaway.mjs) created a page, set a DOM marker, logged entry into an infinite loop after `await Promise.resolve()`, and was supervised by a separate Node host:

- Node child: external SIGKILL at **501.09 ms**, exit signal `SIGKILL`; its same-process 200 ms timer did not run.
- Adapted Bun child: external SIGKILL at **501.67 ms**; loop-entry marker and the same-process timer both appeared, but the awaited script produced no completion. This differs from Node's scheduling; no identical async timeout semantics are claimed.
- Both retained the created browser tab and `document.body.dataset.persisted === 'yes'`. The host reattached by target ID, inspected it, and explicitly closed it. Process death does not roll back navigation, DOM effects, downloads, or page-side activity.

For production, keep the 30-second wall-clock limit in Rust and kill a dedicated execution process when it expires or is cancelled. A separate process gives the host control over lifecycle and crashes; **a process running as the same OS user is not filesystem/network confinement**. Hostile code needs OS-enforced access/resource restrictions and an explicit capability allowlist. Memory/CPU/filesystem/network limits were not implemented or stress-tested here. Merely allowing only certain globals or Node permissions is not a demonstrated hostile-code sandbox. [Node timeout caveats](https://nodejs.org/api/vm.html#timeout-interactions-with-asynchronous-tasks-and-promises), [Node permissions scope](https://nodejs.org/api/permissions.html).

## Cold-machine recipe

Copy the retained `code/` source directory to an ARM64 Mac with **BrowserOS neo 151.0.8162.137** installed at `/Applications/BrowserOS neo.app`, plus Python 3 (the experiment used 3.14.3). The browser app is supplied separately; this recipe never downloads a Playwright browser. To provision the two JS runtimes locally, run the following from the copied directory. Both official archive URLs returned HTTP 200 in the recorded HEAD checks; installation/extraction on a pristine Mac was not rerun during this spike.

```sh
cd /path/to/copied/code
mkdir -p toolchains
curl -fL https://nodejs.org/dist/v26.4.0/node-v26.4.0-darwin-arm64.tar.gz \
  -o toolchains/node.tar.gz
tar -xzf toolchains/node.tar.gz -C toolchains
curl -fL https://github.com/oven-sh/bun/releases/download/bun-v1.3.6/bun-darwin-aarch64.zip \
  -o toolchains/bun.zip
unzip -q toolchains/bun.zip -d toolchains
export PATH="$PWD/toolchains/node-v26.4.0-darwin-arm64/bin:$PWD/toolchains/bun-darwin-aarch64:$PATH"
```

The full official Node archive is **57,137,448 compressed bytes** according to its response header; that includes more than a minimal production runtime and is not directly comparable to the uncompressed Bun binary. The commands below reproduce the experiments; profile directories and ports are allocated afresh.

```sh
cd /path/to/copied/code
node --version                           # v26.4.0
bun --version                            # 1.3.6
python3 --version
bun install --frozen-lockfile            # retained lock resolves core 1.63.0
# Original dependency discovery command was: bun add playwright-core

python3 record.py python3 browser.py headless
python3 record.py node runner.mjs
python3 record.py bun run runner.mjs      # expected 30-second attach failure
python3 record.py bun build --compile runner.mjs --outfile pw-sidecar-naive
# Expected missing optional chromium-bidi imports above.
python3 record.py bun run --preload ./bun-preload.cjs runner.mjs
python3 record.py bun run build.mjs
python3 record.py python3 standalone.py

python3 record.py node audit.mjs
python3 record.py bun run --preload ./bun-preload.cjs audit.mjs
python3 record.py python3 browser.py headed
python3 record.py node focus.mjs
python3 record.py env NO_DEFAULTS=1 node focus.mjs
python3 record.py python3 browser.py headed stop

python3 record.py node coexist.mjs
python3 record.py node sandbox.mjs
python3 record.py bun run --preload ./bun-preload.cjs sandbox.mjs
python3 record.py node watchdog.mjs
python3 record.py env RUNAWAY_RUNTIME=bun node watchdog.mjs
python3 record.py python3 browser.py headless stop
pgrep -fl 'BrowserOS neo'
```

The browser launcher uses exactly this shape, plus safe isolation flags copied from the project's real-browser harness:

```sh
'/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo' \
  --user-data-dir=<fresh-directory> --remote-debugging-port=<free-port> \
  --headless=new --no-first-run --no-default-browser-check \
  --use-mock-keychain --disable-browseros-extensions --disable-browseros-server \
  about:blank
```

Drop `--headless=new` for the headed run. `browser.py` persists PID/endpoint/profile in `headless.json` or `headed.json` and stops only its recorded process group. It refuses port 9108. The fixture HTTP listener belongs to each runner and closes in `finally`. `cleanup-check.py` has this machine's expected owner PID (11851); adjust that expectation on another machine instead of killing unfamiliar processes.

### What would ship

| Choice | Package contents and platform story |
|---|---|
| Node sidecar | A supported standalone Node runtime per OS/architecture, runner code, pinned core package and needed assets (~13M here), plus a chosen assertion implementation. Existing BrowserOS is the browser. Homebrew's local Node layout is not a redistributable package. Runtime size and platform signing remain unmeasured work. |
| Bun compiled | Rust host plus one adapted executable per target. This macOS ARM64 proof is 64.10 MB before distribution compression/signing. Include license notices and any additional lazy assets required by the allowed feature set. Native browser launch/BiDi are outside this artifact's claim. |
| macOS | Bun 1.3.6 documents ARM64 and x64 targets. Build/sign/notarize each distributed sidecar with the application; the measured binary has only an ad-hoc signature. |
| Linux | Bun 1.3.6 documents x64/ARM64 and musl variants. Select the right libc/CPU target; compatibility and size are unmeasured here. |
| Windows | Bun 1.3.6 documents x64 compilation. Native Windows ARM64 is not supported by this pinned compiler; emulation is not native support. Windows execution, signing, and process-tree cancellation are untested. |

Targets and signing details are verified against [Bun 1.3.6 documentation](https://github.com/oven-sh/bun/blob/bun-v1.3.6/docs/bundler/executables.mdx), not newer Bun documentation that lists additional targets. Production macOS distribution also needs the normal [Apple notarization workflow](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).

## Risks

1. **Focus is a shipping blocker for stock creation.** A host-controlled background creation path and coverage for popups/new windows are required to meet the product invariant.
2. **Audit is observable, not complete or secure yet.** A reflected method, unwrapped object, callback, or host-object escape can bypass this prototype. Untrusted stdout cannot be treated as authenticated host audit. Complete object coverage and a trustworthy host persistence protocol are required.
3. **Page events are not ownership attribution.** Discovery is browser-wide; the host must associate creation/popup causes with the right conversation, label pre-existing ownership, and await claim/group work where appropriate. No actual Rust ownership/group DB integration was attempted.
4. **Bun adaptation is version-sensitive.** Stock 1.3.6 fails the transport path; the successful build uses internal source substitutions and an incomplete `ws` compatibility interface. Pin versions and keep clean-directory/runtime checks.
5. **VM is not a hostile-code sandbox.** The escape is demonstrated. OS confinement, resource limits, capability restrictions, and an external watchdog remain necessary; browser access is itself a powerful capability.
6. **Full Playwright fidelity is not established.** No cross-origin OOPIF, downloads/uploads, auth/proxy/TLS edge cases, trace assets, network routing conflicts, browser launch, or full assertion library was tested. CDP browser-version drift remains an upstream caveat.
7. **Sidecar termination preserves browser effects.** Track in-flight calls and owned targets in the host; apply existing session cleanup/group behavior after crash or cancellation.
8. **One-machine measurements.** These are tiny-fixture, warm-browser samples. No multitenant load, Rust IPC, DB, per-step screenshot overhead, cross-platform runtime validation, or distribution package validation is included.

## Assumptions, decisions, and cleanup

ASSUMPTION: The fixture matrix is the bounded dependency proof requested, not a claim of unrestricted Playwright Test compatibility.

DECIDED: Install latest once (resolved 1.63.0), retain the lockfile, and never run `playwright install`.

DECIDED: Follow the project's isolated test harness flags, disabling BrowserOS extensions/server in the scratch profiles so no duplicate sidecar or owner profile is involved. The extension-enabled production combination was not tested.

DECIDED: Keep ordinary page operations in the real package. Treat the transport/bundler substitutions as explicit Bun compatibility costs rather than hiding the failed stock runs.

DECIDED: Use a deliberately limited visibility helper; do not add Playwright Test or claim its entire matcher contract.

DECIDED: Simulate asynchronous claim latency only to prove ordering. No claim/group database, permissions model, or production host change was made.

DECIDED: Record the VM escape honestly instead of asserting the brief's desired isolation result. Escape probes return types only.

DECIDED: Use 100 ms VM and 500 ms external watchdog experiments to demonstrate the mechanism without spending 30 seconds per runaway. Production's 30-second cap remains a host requirement.

DECIDED: Preserve scratch code, artifacts, binaries, profiles, worktree, branch, and pane for the owner. Stop only processes this spike started.

**Cleanup verified at 20:55:24 PDT:** headless PID 31624 and headed PID 69820 process groups had **zero remaining members**, both endpoints were closed, and no spike worker processes remained. The required `pgrep -fl 'BrowserOS neo'` output is saved verbatim in [CLEANUP-pgrep.log](CLEANUP-pgrep.log). Its only root browser process was **11851**, the owner's pre-existing browser. There were 64 total matching processes including that browser's existing helpers; “only PID 11851 remains” means only that root instance, not that Chromium has no helper processes. Git status in the assigned monorepo worktree was clean.
