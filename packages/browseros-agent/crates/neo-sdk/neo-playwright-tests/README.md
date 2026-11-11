# Playwright corpus on Claw v1

One runner uses Playwright Test's workers and pinned upstream specs. The unchanged
v2 run-B curation selects 2,439 tests; `expectLoop` passes remain a separate verdict.
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
- `browser` and `context` are v1's real objects. `playwright.errors.TimeoutError`
  is v1's real agent parameter class, placed where upstream checks expect it.
  No other Playwright namespace members are supplied; those count as lacks.
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
- `expectLoop` wraps real v1 Page/Locator assertions through the same public-read
  polling table as v2. Other assertions retain Playwright's expect. Unsupported
  checks skip; passing loop checks are counted separately. `--no-expect-loop`
  and `--no-curation` reproduce the unadapted selection.
- The implicit `data-testid` selector attribute already matches v1. Explicit
  overrides reach its absent `playwright.selectors` namespace.
- Curation, `_setupArtifacts`, `_browserOptions`, `toImplInWorkerScope` and
  `startRemoteServer` retain v2's seams and explicit internals refusals.

Fidelity costs: Node/V8 replaces QuickJS; one SDK run and profile span a worker;
`context.pages()` includes the launch tab; trace screenshots have no host camera;
v1's Buffer is not Node's Buffer. These are harness differences, not SDK features.

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
