# VERDICT: WORKS WITH CAVEATS

**Playwright 1.63.0's extracted injected engine runs standalone in BrowserOS neo isolated worlds, driven only by raw CDP. Frame hopping and all five requested `expect` matcher types worked.** No Playwright client/server module was loaded by either runtime harness. The 320,783-byte bundle includes the utility code it needs; it only needs a local `module` export object and the correct constructor options. This validates the dependency premise of Design C, **not full Playwright behavior without host work**. The experiment found design-critical differences: the old positional constructor is wrong for 1.63.0; `expect` takes three arguments without `progress`; missing-element assertions require host handling; frame traversal requires host routing; and animation-frame stability checks stall on hidden background tabs. An optional focus-emulation probe unblocked stability without changing the selected tab, but changed page-observable visibility/focus. Failed assertion diagnostics also included a synthetic password value, requiring redaction before production audit/logging.

Tested 2026-09-23 on Darwin ARM64, Node **26.4.0** as a plain-WebSocket harness, BrowserOS neo **151.0.8162.137**, isolated headless PID **25808**, CDP **63994**. The browser and fixture servers are stopped. Owner PID **11851** was untouched. No monorepo changes or commits were made.

Evidence: [command/output transcript](COMMANDS.log), [exact CDP requests/responses/events](CDP.jsonl), [main results](RESULTS.json), [background stability results](STABILITY.json), [pinned source notes](SOURCES.md), [scratch code](code/).

## Bundle provenance

| Field | Verified value |
|---|---|
| npm package | `playwright-core@1.63.0`, installed during the previous spike |
| Exact origin | `/Users/shadowfax/llm/code/browseros-project/grove-ref/main-1/playwright-runtime/spikes/playwright/code/node_modules/playwright-core/lib/coreBundle.js` |
| Origin marker | `// packages/playwright-core/src/generated/injectedScriptSource.ts`, assignment to `source4` at line 19837 |
| Origin file SHA-256 | `549070af3acabb3efcc4f55bfe6210f9f7c2fcf633cf7eaa59bfe60719969171` |
| Extracted file | [injected-script-1.63.0.js](code/injected-script-1.63.0.js) |
| UTF-8 bytes | **320,783** (320.783 kB; 313.265 KiB) |
| Extracted SHA-256 | **`94103308b4f5791976b53543f5812be61ffb988574f7a51f412f87ab0ad60a85`** |
| License | Apache-2.0; upstream [LICENSE](code/LICENSE) and [NOTICE](code/NOTICE) copied unchanged |
| Modifications | None to the extracted engine; only decoded its generated JavaScript string literal |
| Extraction tool | [extract.mjs](code/extract.mjs); checks package version and generated-source marker, evaluates only the string literal in Node VM, never imports the package |
| Machine-readable provenance | [provenance.json](code/provenance.json) |

The local source checkout was **1.53.0-next**, commit `37a9535a27e3b1628109576b8febe6fb89608aee`, so it was not used as the runtime asset or signature authority. No older npm fallback was needed. Five official source files pinned to v1.63.0 were downloaded under [upstream-v1.63.0](code/upstream-v1.63.0/) for interpretation only. Context7 resolved the package, but its internal-engine query found no matching documentation; the decoded artifact and pinned upstream source supplied the actual contracts.

Keep the license and applicable NOTICE attribution with a redistributed asset. The retained NOTICE identifies Microsoft and Puppeteer-derived code. Mark downstream changes if modifying the vendored source; this extracted copy is unmodified. The asset's source-byte count is measured; Rust executable growth, compression, and per-frame renderer heap cost are not.

## Coverage and measured latency

These are single local samples. Selector latency includes query, `Runtime.getProperties`, `DOM.describeNode` for every match, and array-handle release. State/matcher rows measure their raw call. Measurements include harness/CDP transcript-writing overhead; they are not performance distributions.

### Selectors and frame traversal

| Selector / path | Result | `backendNodeId` | ms |
|---|---|---|---:|
| `internal:role=button[name="Submit"i]` | 1 button | 6 | 103.034 |
| `internal:text="Hello"i` | 1 heading | 7 | 15.753 |
| `internal:label="Your name"i` | 1 input | 1 | 8.267 |
| `internal:label="Password"i` | 1 password input | 2 | 15.212 |
| `internal:attr=[placeholder="Email"i]` | 1 input | 3 | 5.231 |
| `internal:testid=[data-testid="x"]` | 1 span | 11 | 2.842 |
| `css=.item >> nth=1` | Second list item | 12 | 2.498 |
| `.item >> internal:has-text="two"i` | Second list item | 12 | 1.831 |
| `.item >> internal:has="css=.nested"` | Second list item | 12 | 1.754 |
| `xpath=//h1` | 1 heading | 7 | 1.638 |
| `internal:role=button[name="Dup"i]` | 2 buttons | 14, 15 | 1.751 |
| Same duplicate selector with `querySelector(parsed, document, true)` | Expected strict-mode error with both locator descriptions | — | 5.306 |
| Whole `iframe >> internal:control=enter-frame >> internal:role=button` given to injected engine | **0 matches**: engine does not hop | — | 2.786 |
| Host resolves iframe → child world → remaining role selector | **Works**, child button found | iframe 16; button 18 | 9.631 total |
| Remaining role selector inside child world alone | 1 button | 18 | 2.509 |

The frame element's `DOM.describeNode` response exposed child frame ID `81D4EE5C0668B4A639E8C548C111E88A`. A second isolated world in that frame, on the same target session, returned the child's button. We did not access `iframe.contentDocument` to perform the hop. This proves **same-origin, same-target frame routing**. OOPIF target/session routing, cross-origin frames, nested-frame click coordinates, and frame-detachment races remain untested. Production must split the parsed selector at frame-control boundaries; a naive string split can misread quoted/nested selectors. [Pinned frame resolver](https://github.com/microsoft/playwright/blob/v1.63.0/packages/playwright-core/src/server/frameSelectors.ts).

### States and actions

| Operation | Actual result | ms |
|---|---|---:|
| Submit `elementState(...,'visible')` | `{matches:true, received:'visible'}` even though page is hidden | 0.521 |
| Submit enabled | `{matches:true, received:'enabled'}` | 0.344 |
| Name editable | `{matches:true, received:'editable'}` | 0.643 |
| Password editable | `{matches:true, received:'editable'}` | 0.287 |
| Checkbox checked | `{matches:true, received:'checked', isRadio:false}` | 0.679 |
| Disabled button enabled | `{matches:false, received:'disabled'}` | 0.242 |
| Hidden element visible | `{matches:false, received:'hidden'}` | 0.650 |
| `elementState(...,'stable')` | Expected error: use `checkElementStates` for stable | 0.430 |
| Background `checkElementStates(...,['visible','enabled','stable'])` | **Host timeout**, animation frames did not arrive | 5000.999 |
| Scroll + box center + `expectHitTarget` + native mouse press/release | Handler fired once, `isTrusted:true`, hit check `done`, scrollY 923 | 8.751 for box/hit/input/readback; prior scroll separate |
| Name `fill` + `Input.insertText` | `needsinput`; replaced `old text` with `Ada Lovelace`; input trusted | 9.306 |
| Password `fill` + `Input.insertText` | `needsinput`; synthetic value read back; input trusted | 6.827 |
| Enter `Input.dispatchKeyEvent` | Fixture saw key `Enter`, `isTrusted:true` | 1.837 |
| `selectOptions(...,[{value:'blue'}])` | `['blue']`; selected value `blue`; generated input/change events **not trusted** | 4.125 |
| Follow-up background stability, focus emulation disabled | Host timeout, RAF probe remained 0 | 752.092 |
| Follow-up stability, focus emulation enabled | `done` | 20.253 |
| Follow-up hit check + trusted click **after successful stability** | Handler fired once, `isTrusted:true` | 11.653 additional |

The initial native-input probe deliberately continued after recording the failed stability probe; it was not proof of the complete actionability sequence. The follow-up [background-stability.mjs](code/background-stability.mjs) explicitly required successful stability before clicking.

**Hidden-tab finding:** `Target.createTarget({background:true})` left `document.visibilityState === 'hidden'`, and a page RAF callback never ran during the bounded checks. Injected “visible” means the element is rendered, not that the document is the selected tab. `Emulation.setFocusEmulationEnabled({enabled:true})` made the document report `visible` and `hasFocus:true`, allowed RAF, and unblocked stability. `Browser.getWindows` showed the same selected tab, **1426443177**, throughout. Disabling emulation restored `visibilityState:'hidden'`; after the trusted click, `hasFocus()` still reported true. Thus this is a page-observable policy, not an invisible isolated-world detail. Only a headless isolated window was tested here; no claim about headed OS focus is made.

`selectOptions` synthesizes DOM input/change events inside Playwright's own helper; their `isTrusted:false` is expected for this implementation. Fill of text/password inputs delegates the actual insertion to CDP and produced trusted input events. These helpers do not by themselves supply all waiting, retry, navigation, cancellation, or interception behavior of Playwright actions. [Pinned injected implementation](https://github.com/microsoft/playwright/blob/v1.63.0/packages/injected/src/injectedScript.ts), [host action orchestration](https://github.com/microsoft/playwright/blob/v1.63.0/packages/playwright-core/src/server/dom.ts).

### Expect matchers

Each matcher was called directly through `Runtime.callFunctionOn` as `this.expect(elements[0], options, elements)`, with `awaitPromise:true`. `received` below is the nested `received.value`; failed non-array matchers can additionally return `received.ariaSnapshot`.

| Expression | Passing case: result / ms | Failing case: result / ms |
|---|---|---|
| `to.be.visible` | Submit: true, `visible` / 0.695 | Hidden element: false, `hidden` / 3.459 |
| `to.have.text` | Heading expected `Hello`: true, `Hello` / 0.569 | Expected `Goodbye`: false, `Hello` / 0.483 |
| `to.have.count` | List expected 3: true, 3 / 0.472 | Expected 4: false, 3 / 0.371 |
| `to.have.value` | Name expected `Ada Lovelace`: true / 0.403 | Expected `Grace`: false, `Ada Lovelace` / 0.593 |
| `to.be.enabled` | Submit: true, `enabled` / 0.372 | Disabled button: false, `disabled` / 0.708 |

Example actual return:

```json
{"matches":false,"received":{"value":"Hello","ariaSnapshot":"- heading \"Hello\" [level=1]"}}
```

**Missing-element behavior is not in this injected entry point.** Passing no element to `to.be.hidden` threw `TypeError: Cannot read properties of undefined (reading 'nodeType')` in 1.134 ms. The real host handles zero matches, negation, array assertions and strictness before/around this call. A Rust adapter must do the same. Engine `matches` is the underlying result; assertion success uses `matches !== isNot`. [Pinned host assertion implementation](https://github.com/microsoft/playwright/blob/v1.63.0/packages/playwright-core/src/server/frames.ts#L1535).

The fixture's delayed element was restarted with a 500 ms timer. A host loop re-resolving every 50 ms found it and got a passing text matcher after **578.196 ms / 12 polls**. That timing belongs to the harness's polling policy, not automatic retry inside `expect`.

**Diagnostic redaction is required.** The failed visibility assertion's body snapshot contained `textbox "Password": spike-synthetic-password`. All values were purpose-made fixture data. The experiment demonstrates that a failure in an unrelated matcher can include password-field contents. Do not blindly persist or forward the engine's `received`, ARIA diagnostics, input arguments, or errors into audit/user logs. Review existing sanitization against this concrete shape.

## Exact bootstrap and CDP contract

Connect a plain websocket to `webSocketDebuggerUrl` from the **isolated** endpoint's `/json/version`. Browser-level commands go on the root connection. Page, Runtime, DOM, Emulation, and Input commands below carry the flattened target `sessionId` in the CDP envelope. `T`, `S`, `F`, `C`, and `E` below name values returned by earlier calls; `C` is an integer, while target/session/frame/object IDs are strings.

```js
// Discovery/attachment; use an existing target for the production server.
Target.attachToTarget({ targetId: T, flatten: true }) // -> {sessionId: S}
Page.enable({}, sessionId=S)
Runtime.enable({}, sessionId=S)
Page.getFrameTree({}, sessionId=S)                  // -> frameTree.frame.id = F
Page.createIsolatedWorld({
  frameId: F,
  worldName: '__browseros_pw',
  grantUniveralAccess: false
}, sessionId=S)                                    // -> {executionContextId: C}
```

`grantUniveralAccess` is the protocol's actual spelling, including the missing “s”; it is not a typo to correct in Rust serialization. The fixture setup additionally used root `Target.createTarget({url:'about:blank',background:true})`, then page-session `Page.navigate({url:fixtureUrl})` and waited for `Page.loadEventFired` before taking the frame tree.

Construct `BOOTSTRAP_SOURCE` from this template with the **unmodified file contents** inserted in place of `BUNDLE`:

```js
(() => {
  const module = {};
  /* BUNDLE: injected-script-1.63.0.js, 320783 UTF-8 bytes */
  return globalThis.__pw = new (module.exports.InjectedScript())(globalThis, {
    isUnderTest: false,
    sdkLanguage: 'javascript',
    frameSeq: 1,
    testIdAttributeName: 'data-testid',
    stableRafCount: 1,
    browserName: 'chromium',
    shouldPrependErrorPrefix: false,
    isUtilityWorld: true,
    customEngines: []
  });
})()
```

Then:

```js
Runtime.evaluate({
  expression: BOOTSTRAP_SOURCE,
  contextId: C,
  returnByValue: false,
  objectGroup: 'injected-spike'
}, sessionId=S)                                    // -> result.objectId = E

Runtime.callFunctionOn({
  objectId: E,
  functionDeclaration: 'function(selector) { return this.querySelectorAll(this.parseSelector(selector), document); }',
  arguments: [{value:'internal:role=button[name="Submit"i]'}],
  returnByValue: false,
  awaitPromise: true,
  objectGroup: 'injected-spike'
}, sessionId=S)                                    // -> result.objectId = ARRAY

Runtime.getProperties({objectId:ARRAY, ownProperties:true}, sessionId=S)
// For every numeric array property, use property.value.objectId = ELEMENT:
DOM.describeNode({objectId:ELEMENT, depth:0}, sessionId=S)
// -> node.backendNodeId; for IFRAME also node.frameId
Runtime.releaseObject({objectId:ARRAY}, sessionId=S)
```

The engine is the receiver for helper calls, and element objects are CDP object arguments from **the same world/session**:

```js
Runtime.callFunctionOn({
  objectId: E,
  functionDeclaration: 'function(el, state) { return this.elementState(el, state); }',
  arguments: [{objectId:ELEMENT}, {value:'enabled'}],
  returnByValue: true,
  awaitPromise: true,
  objectGroup: 'injected-spike'
}, sessionId=S)
```

For frame traversal, first resolve the iframe in its parent world, take `DOM.describeNode(...).node.frameId`, call `Page.createIsolatedWorld` for that frame, evaluate the same bootstrap there, and run the remainder of the selector against the **child engine object**. In this same-origin test, the CDP session stays `S`; an OOPIF needs the child target's session. Do not carry a parent-world object ID into another execution context. Retain backend node identity only for the valid target/document lifetime, and release remote handles/object groups when finished.

For native click, the tested sequence was `DOM.scrollIntoViewIfNeeded({backendNodeId})` → `checkElementStates` → `DOM.getBoxModel({backendNodeId})` → engine `expectHitTarget(point,element)` → `Input.dispatchMouseEvent` twice with `{type:'mousePressed'|'mouseReleased',x,y,button:'left',clickCount:1}`. The center was the mean of the four `model.content` vertices. Full pointer fidelity still needs host retries, scrolling alternatives, event interception, frame-coordinate mapping, and navigation coordination.

For text fill, after visible/enabled/editable checks, engine `fill(element,text)` returned `needsinput`, then `Input.insertText({text})` performed replacement. Empty text is a separate host branch (Playwright uses Delete); that branch was not tested. Enter used `Input.dispatchKeyEvent` with `rawKeyDown` and `keyUp`, `key:'Enter'`, `code:'Enter'`, `windowsVirtualKeyCode:13`, `nativeVirtualKeyCode:13`. Selection used engine `selectOptions(element,[{value:'blue'}])`; the helper supplies its own DOM events.

Check **both** protocol errors and `Runtime.*.exceptionDetails`; a successful CDP reply can carry a thrown page exception. Exact wire values, including every bootstrap expression, are retained in [CDP.jsonl](CDP.jsonl). The copyable implementation is [engine.mjs](code/engine.mjs).

## Constructor, exports, and globals

The exact 1.63.0 generated signatures are:

```js
constructor(window, options)           // arity 2
async expect(element, options, elements) // arity 3; NO progress parameter
fill(node, value)                      // arity 2
selectOptions(node, optionsToSelect)
elementState(node, state)              // stable is not accepted here
async checkElementStates(node, states) // stable supported here
expectHitTarget({x, y}, targetElement)
```

Constructor options require `isUnderTest`, `sdkLanguage`, `frameSeq`, `testIdAttributeName`, `stableRafCount`, `browserName`, and `customEngines`. Custom engines are `{name,source}` entries, not the older `{name,engine}` form. `shouldPrependErrorPrefix` and `isUtilityWorld` are optional flags supplied explicitly here. The measured harness used `frameSeq:1` in every world because no cross-frame ARIA reference identifiers were being tested; a real host should assign frame sequence values consistently. `stableRafCount:1` matches the Chromium delegate's implementation; do not copy Design C's unexplained `20` as a versioned contract. [Pinned constructor/options](https://github.com/microsoft/playwright/blob/v1.63.0/packages/injected/src/injectedScript.ts#L50), [production bootstrap](https://github.com/microsoft/playwright/blob/v1.63.0/packages/playwright-core/src/server/dom.ts#L86).

The generated export `module.exports.InjectedScript` is a **getter function returning the class**. Hence the parentheses in `new (module.exports.InjectedScript())(...)`. Evaluating the bare bundle with no export stub failed with `ReferenceError: module is not defined` in 10.801 ms. Supplying a locally scoped `const module = {}` fixed it.

No separate utility script, Node globals, package import, Playwright binding, or host `progress` stub was required. The generated bundle already defines `UtilityScript` and constructs it internally for builtins. It expects normal browser DOM/window facilities. `globalThis.__pw` was a harness-chosen isolated-world marker for the visibility test; retaining only the returned object handle would also avoid needing that marker. The wrapper keeps the class and module variables out of the world's global namespace.

## Navigation and world invalidation

| Bootstrap | Isolated context | Bundle evaluation | World creation + evaluation |
|---|---:|---:|---:|
| Main document, first valid wrapper | 5 | 10.715 ms | 65.434 ms |
| Same-origin child frame | 6 | 4.176 ms | 5.000 ms |
| Main document after navigation | 9 | **4.235 ms** | **5.151 ms** |

The first valid wrapper followed the deliberate bare-bundle failure, so this is not a completely cold parser measurement. Each world used a full bundle evaluation. The separate stability probe had a 7.611 ms bundle evaluation / 8.503 ms total.

Navigation from `/` to `/?navigated=1` produced **two `Runtime.executionContextsCleared` events** on the engine session. No individual `executionContextDestroyed` event appeared in that recorded slice. The old context ID and old engine object ID both failed with protocol code `-32000`, `Cannot find context with specified id`. Recreating the named world yielded context 9, and the Submit selector resolved again to backend node 55 in 1.756 ms.

Cache worlds by connection epoch, target/session, frame and execution-context lifetime. Clear on both forms of context-destruction notification, and recover from stale-context protocol errors. A stable frame ID across navigation does not preserve the old engine or its element handles. This spike tested one cross-document navigation; reconnect, frame detach, BFCache, process swap, and OOPIF lifecycle behavior remain separate cases.

The harness's request timeout drops host correlation; it does **not** cancel a pending page-side RAF promise. Closing the target at teardown destroys it. Production cancellation must account for work that completes late; do not equate a timed-out CDP wait with browser-side cancellation.

## Coexistence and main-world isolation

A second independent browser websocket attached to the same target and called `Runtime.enable` before engine injection. It represented the existing production session; the actual Rust server was not launched. That client set `window.serverMarker=41` in the main world and later read/incremented it while the engine was active:

```json
{"serverMarker":42,"title":"Injected engine fixture","pw":"undefined","engine":"undefined","ownScript":{"pw":"undefined","engine":"undefined"}}
```

The fixture's **own trusted click handler** also recorded `typeof window.__pw` and `typeof InjectedScript` as `undefined` after injection. Inside the isolated world, `typeof globalThis.__pw` was `object`. After navigation/rebootstrap, the second client's main-world evaluation and the new page script still worked and saw no engine globals. No shared binding or Playwright server auto-attach machinery was required.

This demonstrates two Runtime-enabled sessions plus separate world globals for this fixture. It does not demonstrate every possible concurrent CDP command combination. DOM changes, input events, and the optional focus-emulation setting are observable across worlds; isolated globals do not mean invisible automation.

## Risks and required corrections to Design C

1. **Pinned internals, not a public standalone contract.** Version 1.63.0 already differs from the design's constructor and the brief's four-argument `expect` suggestion. Pin the asset hash, options, matcher payloads and result shapes together. Test upgrades against a real browser; checking that a class name remains in the string is insufficient.
2. **Background actionability is not solved by extraction.** The engine's RAF stability loop stalled on the default background tab. Focus emulation is a demonstrated workaround that preserves tab selection in this headless test, but alters visibility/focus reported to site code. The existing browseros-core/browseros-cdp source search found no `setFocusEmulationEnabled` call. Decide and verify the production policy rather than assuming current primitives already provide it.
3. **Diagnostics can disclose sensitive fields.** A failed assertion included the unrelated synthetic password in `received.ariaSnapshot`. Explicit redaction is needed wherever these diagnostics, values, or input arguments leave the browser. Existing audit reuse does not by itself establish that this shape is sanitized.
4. **Host semantics remain substantial.** Strict/absent matching, negation, retries, deadlines, cancellation, stale handles, hit interception, click points, navigation coordination, event attribution, and frame routing remain outside the helper. Reusing the engine avoids rewriting selector/matcher internals; it does not establish full ordinary Playwright script fidelity.
5. **Frames require target/session awareness.** The same-origin hop passed, but the entire selector produced zero matches when handed straight to the engine. OOPIF and cross-origin behavior must be tested before claiming general frame support.
6. **Lifecycle and cancellation.** Context IDs/object IDs die on navigation; backend-node handles must not survive blindly. Timed-out host requests can leave pending page promises; bounded retry and cleanup policies must account for late results.
7. **License/size.** Preserve Apache-2.0 attribution and applicable NOTICE material alongside the vendored bundle. The measured asset is 320,783 bytes, not the older design's 309,776 bytes. No Rust build, final binary-size delta, compressed artifact, renderer-memory budget, or production distribution review was performed.
8. **Scope of proof.** Only the specified local fixture, one browser version and macOS ARM64 were exercised. No actual QuickJS facade/Rust adapter, audit pipeline, ownership/group integration, screen replay, real website corpus, or extension-enabled production browser was tested. Nothing in this report approves the design's broader compatibility claims.

## Decisions and assumptions

DECIDED: Prefer the exact installed 1.63.0 string over the stale local source checkout or an older npm artifact. Preserve its byte-for-byte decoded content and provenance.

DECIDED: Node is only an extraction/raw-CDP test harness. No Node/Bun runtime or Playwright client/server dependency is implied for production by this experiment.

DECIDED: Use `constructor(window, options)` and `expect(element, options, elements)` from the actual pinned artifact. No fake server `progress` object is needed or supplied.

DECIDED: Run the primary probe with a background tab and no focus emulation, preserving the failure. Run the emulation workaround separately and report its observable changes.

DECIDED: Treat strict-mode, missing-element, stale-context and bare-export failures as explicit probes. Preserve unsuccessful output rather than replacing it with a success-only transcript.

DECIDED: Use only synthetic password data and document the demonstrated diagnostic leak. No owner's browser/session/content is accessed.

ASSUMPTION: A second raw Runtime-enabled session is an adequate dependency-level coexistence probe; it is not a live Rust server integration test.

ASSUMPTION: The measured same-origin frame hop is the requested initial frame proof; cross-origin/OOPIF behavior is an outstanding requirement, not inferred success.

DECIDED: No monorepo files, worktrees, branches or panes are changed or removed. Leave all scratch source, extracted bundle, license files and evidence for the owner.

## Cold-machine recipe

On an ARM64 Mac, install the same **BrowserOS neo 151.0.8162.137** build at `/Applications/BrowserOS neo.app`, Python 3, and Node 26.4.0. Copy this report's `code/` directory, including the extracted bundle. Node is a harness prerequisite only. If Node is absent, the official pinned archive can be provisioned locally:

```sh
cd /path/to/copied/code
mkdir -p toolchain
curl -fL https://nodejs.org/dist/v26.4.0/node-v26.4.0-darwin-arm64.tar.gz \
  -o toolchain/node.tar.gz
tar -xzf toolchain/node.tar.gz -C toolchain
export PATH="$PWD/toolchain/node-v26.4.0-darwin-arm64/bin:$PATH"
node --version  # v26.4.0
python3 --version
shasum -a 256 injected-script-1.63.0.js
# Expected: 94103308b4f5791976b53543f5812be61ffb988574f7a51f412f87ab0ad60a85

python3 record.py python3 browser.py headless
python3 record.py node runner.mjs
python3 record.py node background-stability.mjs
python3 record.py python3 browser.py headless stop
pgrep -fl 'BrowserOS neo'
```

`browser.py` chooses a free port, creates a fresh profile under this scratch directory, records the process group, and launches with `--headless=new --no-first-run --no-default-browser-check --use-mock-keychain --disable-browseros-extensions --disable-browseros-server`. It never uses CDP 9108. The launch binary can be overridden with `BROWSEROS_BINARY`. Both Node scripts own their fixture HTTP servers and websockets and close them in `finally`; the browser is owned by the separate launcher and must be stopped with the final command even after a failed experiment. The original toolchain was already installed; the archive bootstrap above was not rerun on a pristine machine.

To independently re-extract rather than trusting the retained artifact, use a separate directory and the exact version. This step needs Bun; none of the browser experiments does:

```sh
mkdir extraction-package
cd extraction-package
bun add playwright-core@1.63.0
cd ..
python3 record.py node extract.mjs ./extraction-package/node_modules/playwright-core
shasum -a 256 injected-script-1.63.0.js
# No playwright install; no browser download.
```

The original extraction used `node extract.mjs` with its default path to the previous spike's package. `extract.mjs` writes `provenance.json`, the bundle, LICENSE and NOTICE. The CDP harness imports only Node builtins and local raw transport/fixture/engine files; package installation is not part of runtime execution.

**Cleanup verified at 21:15:09 PDT:** isolated browser PID **25808** had no remaining process-group members and CDP **63994** was closed. [CLEANUP-pgrep.log](CLEANUP-pgrep.log) preserves the full `pgrep -fl 'BrowserOS neo'` output: root instance **11851** was the only remaining browser root, with its existing Chromium helpers (64 total matching processes). Both harnesses exited and closed their fixture servers; no sidecar or extra browser remains from this spike. The monorepo worktree's Git status was clean. `cleanup-check.py` contains this machine's expected owner PID; adjust that expectation on a different Mac instead of killing unfamiliar processes.
