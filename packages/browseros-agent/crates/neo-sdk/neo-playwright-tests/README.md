# Playwright corpus on Claw v1

One runner uses Playwright Test's workers and pinned upstream specs. The unchanged
v2 run-B curation selects 2,439 tests; legacy `expectLoop` passes remain a separate verdict.
No tests are translated into SDK calls.

```sh
export PATH=/opt/homebrew/opt/node@26/bin:$PATH
export NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo"
export NEO_TESTS_PLAYWRIGHT_SOURCE=/absolute/path/to/prepared/pw-source
export CARGO_TARGET_DIR=/Volumes/extssd/cargo-targets/claw-runner
export CARGO_BUILD_BUILD_DIR=/Volumes/extssd/cargo-targets/claw-runner-build
export NEO_NODE_OUTPUT_DIR="$PWD/.llm/claw-node-results"

node packages/browseros-agent/crates/neo-sdk/neo-playwright-tests/node/run.mjs --engine reference
node packages/browseros-agent/crates/neo-sdk/neo-playwright-tests/node/run.mjs --engine claw
node packages/browseros-agent/crates/neo-sdk/neo-playwright-tests/node/run.mjs --engine neo
```

`claw` is the default; `--reference` aliases `--engine reference`. `neo` retains
v2's fixture implementation but fails immediately: `Neo v2's SDK is not on this branch`.
Claw builds `node/claw-addon` with `cargo build --release --locked`, then installs
`claw_node.node`. `--skip-build` reuses it; `CLAW_NODE_ADDON` can name an existing build.

Append suite paths, `--grep`, or `--workers 1` through `4` for subsets. For example:

```sh
node packages/browseros-agent/crates/neo-sdk/neo-playwright-tests/node/run.mjs --engine claw --workers 2 tests/page/page-goto.spec.ts tests/page/page-click.spec.ts tests/library/page-close.spec.ts
```

Every engine launches its own headless BrowserOS with a private profile and leased
server ports. Guardians clean and verify only their own process groups, even after
a worker crashes. Normal mode uses `--window-size=1520,801`: BrowserOS 0.52.1.1's chrome
leaves a 1280×720 viewport. Other releases size their chrome differently (0.52.2 takes
34 px less), so the harness measures the launch tab and resizes the window by the
difference. It then verifies both the launch tab and a temporary native
`Browser.createTab` at device scale 1. A mismatch fails setup. `run.json` records flags, environment, version, viewport probes, the
three v1 script hashes, and the add-on build/hash. Profiles are deleted after use.

`--hosted-mode` sets `BROWSERCLAW_HOSTED_MODE=1` for either engine and uses a
1280×841 window for the same viewport. This is a secondary comparison: hosted mode
opens popups as tabs, but suppresses CDP console events that vanilla Playwright's
`setContent` needs. Default runs leave this variable unset, including when inherited.

## Fixtures and test-only shims

- Reference uses vanilla `chromium.connectOverCDP` and upstream contexts.
- `__claw` opens one real v1 run per worker with a 24-hour deadline. Playwright
  owns test deadlines. `finish()` writes `worker-*/outcome.json`.
- `browser`, `context` and `playwright` are v1's real objects; `playwright` is the
  run's own parameter (`selectors`, `errors.TimeoutError`). Its other upstream members
  (browser types, `devices`, `request`) are absent and count as lacks.
- `_contextFactory` lends the worker's one context on its first call without
  explicit options. Only verified implicit Playwright defaults are omitted.
  Explicit options—even default-valued ones—and second calls reach the real
  `browser.newContext` refusal. `claw-context` annotations explain each attempt.
- Upstream still creates the fixture `page` with `context.newPage()`. The harness
  diffs CDP targets around that fixture, then installs upstream `window.builtins`
  on its target. Pages opened by test code receive no builtins shim.
- Per-test timeouts are reapplied. Teardown closes the test's pages, removes
  context listeners, clears cookies and server-origin storage, IndexedDB,
  HTTP cache, Cache Storage and service workers over a private CDP connection.
  Worker shutdown keeps that connection open until test teardown finishes.
  Closing tabs destroys their sessionStorage; retained server-origin tabs have
  sessionStorage cleared through DOMStorage. This substitutes for fresh-context isolation;
  it adds no SDK methods.
- Claw Page/Locator assertions use the active run's real `expect` parameter. Neo
  owns retries, matcher options, cancellation, refusals and assertion errors;
  the harness passes them through unchanged. Playwright retains ordinary value
  assertions and its statics, including extended value matchers. Configured expect
  imports retain the run binding and the generated config's assertion timeout.
  `neoExpect` annotations record this path; these passes count as product passes.
  The legacy v2 adapter still uses `expectLoop` and reports its passes separately.
  The historical `--no-expect-loop` flag disables either assertion import adapter.
- Claw ignores only curation skips whose reason is an uncovered `expectLoop`
  matcher. The manifest, corpus, spec bodies and existing diagnostic patches
  remain unchanged. Product matcher refusals fail rather than skip, so unsupported
  assertions remain visible as product gaps. `run.json.assertions` records which
  assertion engine ran; `--no-curation` disables all curation as before.
- Each test sets its `testIdAttribute` through v1's `playwright.selectors`, so a
  test that changes the attribute does not leave it for the next test in the worker's run.
- Curation, `_setupArtifacts`, `_browserOptions`, `toImplInWorkerScope` and
  `startRemoteServer` retain v2's seams and explicit internals refusals.

The Node host registers its native `Buffer` parameter and byte-result reviver through the
bootstrap's provider/brand hooks. Screenshot bytes therefore have Node Buffer identity for
upstream image libraries and assertions. The production bundle stays byte-identical; its
QuickJS Buffer implementation has separate run-realm tests.

Fidelity costs: Node/V8 replaces QuickJS; one SDK run and profile span a worker;
`context.pages()` includes the launch tab; trace screenshots have no host camera.
These are harness differences, not SDK features.

The 71 previously skipped tests need the following matcher surface. Options and
expected values are forwarded to Neo, including negative assertion chains.

| Matcher | Tests | Arguments and options used |
|---|---:|---|
| `toMatchAriaSnapshot` | 36 | YAML strings/templates, regular expressions in snapshots, `timeout`, `signal` |
| `toHaveAccessibleErrorMessage` | 12 | string/RegExp, `ignoreCase` |
| `toBe` | 6 | object identity |
| `toHaveScreenshot` | 4 | snapshot filename |
| `toContainClass` | 3 | string/string array; invalid RegExp inputs |
| `toBeInViewport` | 3 | `ratio` (0.1 through 1) |
| `toHaveAccessibleName` | 2 | string/RegExp, `ignoreCase` |
| `toHaveRole` | 2 | role string; invalid RegExp input; `timeout` |
| `toHaveAccessibleDescription` | 1 | string/RegExp, `ignoreCase` |
| `toBeOK` | 1 | invalid Page receiver |
| `toBeTruthy` | 1 | Page receiver |

The neighboring assertion cases also exercise `signal`, `.rejects`, nested JS
properties, shadow DOM text and select values through labels. Missing product
`matcherResult` fields are not filled in by the harness.

## Tests and reports

```sh
node --test packages/browseros-agent/crates/neo-sdk/neo-playwright-tests/node/*.test.mjs packages/browseros-agent/crates/neo-sdk/neo-playwright-tests/node/report/*.test.mjs
node packages/browseros-agent/crates/neo-sdk/neo-playwright-tests/node/report/report.mjs --neo <claw-run-dir> --reference <reference-run-dir> --bridge <cases.jsonl> --out .llm/report.md --json .llm/verdicts.jsonl
```

Real-browser tests require `NEO_E2E_BROWSER`; the Claw fixture test also requires its
built add-on. V2-only tests skip when its SDK is absent. Run real-browser test files
sequentially to keep the four-browser limit.

The report selects refusal wordings from `run.json.engine`, labels Claw reports
“v1”, and retains the stable verdict IDs (`neo-lacks`, `neo-differs`, etc.). Only
an unexpected reference failure establishes `invalid-here`. Missing APIs are
SDK gaps; add-on loading and fixture setup failures are harness errors.

The runner prints **All tests** and **In-scope tests** after execution and writes
`scores.json`. Both count Playwright expected/flaky outcomes (including expectLoop
passes). A skipped test ran nothing, so it stays out of the rate: each score reports
`skipped` beside it, and `skippedByHarness` for the skips this harness made (curation's
internals list, expectLoop's uncovered checks) rather than the spec itself. Incomplete
cases stay in the denominator. An empty denominator prints `n/a`. The offline report
prints the same scores alongside its unchanged verdicts.

`node/scope.json` is reviewed reporting policy, separate from curation. Entries name
a corpus-relative spec `file`, optionally narrowed with `ids`; omitting `ids` covers
that whole spec. Each has one reason: `network`, `handles`, `drift`, or
`decided-deviation`. Network/handle entries identify explicit refusal evidence;
ordinary failures stay in scope. The initial evidence is the lead's parity gap
list and the corresponding per-test errors from the full run (mixed event groups
also contain supported events, so the entire group is not excluded).

Scope never skips, deletes, patches or reclassifies a test. Each score subtracts
the matched cases and their passing count; exclusions are counted per reason.
Every passing excluded test is listed for review. Filtered runs count only their
selected cases. `run.json.scope.manifestSha256` records a copied `scope.json` next
to the results; offline reports verify its hash. Legacy runs use the current
repository policy with an explicit provenance note.
