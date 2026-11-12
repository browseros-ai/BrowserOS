# Playwright corpus on Claw v1

One runner uses Playwright Test's workers and pinned upstream specs; legacy `expectLoop`
passes remain a separate verdict. No tests are translated into SDK calls.

## Running

From `packages/browseros-agent`:

```sh
bun run test:neo-spec                          # every spec on Neo (claw engine, 4 workers)
bun run test:neo-spec page-goto page-fill      # some specs, by file name without .spec.ts
bun run test:neo-spec page-goto --reference    # the same on real Playwright
bun run test:neo-spec page-fill --grep 'should fill range input'
bun run test:neo-spec --ci                     # the CI gate: the whole corpus against its expected list
```

It needs Node 22 or newer (taken from PATH, then `/opt/homebrew/opt/node@26/bin`; the corpus
runs on 26), BrowserOS neo installed, `git`, and `cargo` for the claw add-on. The first run
prepares the pinned Playwright source in `~/.cache/browseros/neo-spec/playwright-1.64.1`, which
takes a few minutes. Each run ends with the in-scope score, the all-tests score, the results
folder and the command that reruns one failing test.

A name is a spec file without `.spec.ts`, or `page/<name>` when two folders share it; full paths
work too. Other flags go to `node/run.mjs` unchanged: `--grep`, `--workers 1` to `32` (each
worker owns one browser; keep the default 4 on a machine that runs other builds), `--skip-build`,
`--hosted-mode`, `--no-curation`. `--list` lists the selected tests and needs no browser.

Overrides:

- `NEO_E2E_BROWSER`: the browser executable. Default: `/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo`
  on macOS, `/usr/bin/browserclaw` on Linux.
- `NEO_TESTS_PLAYWRIGHT_SOURCE`: a source prepared by `js/prepare.mjs`. Default: the cache above.
- `NEO_SPEC_CACHE_DIR`: the cache folder that holds prepared sources, one per pinned release.
  Default: `~/.cache/browseros/neo-spec` (under `$XDG_CACHE_HOME` when set).
- `NEO_NODE_OUTPUT_DIR`: where result folders go. Default: `.llm/neo-node-results` at the
  repository root.
- `CLAW_NODE_ADDON`: an add-on build to load instead of building one (with `--skip-build`).

`spec.ts` only finds Node, the browser and the source; `node/run.mjs` does the rest and can be
run directly with the variables above set, e.g.
`node crates/neo-sdk/neo-playwright-tests/node/run.mjs --engine claw tests/page/page-goto.spec.ts`.
`claw` is the default engine; `--reference` aliases `--engine reference`. `neo` retains
v2's fixture implementation but fails immediately: `Neo v2's SDK is not on this branch`.
Claw builds `node/claw-addon` with `cargo build --release --locked`, then installs
`claw_node.node`. `--skip-build` reuses it.

## CI

`.github/workflows/neo-spec.yml` runs `bun run test:neo-spec --ci --workers 16` on WarpBuild's
32-vCPU Linux runner for every pull request and push to `main` that touches `crates/`, the
workspace manifest or this command, with the browser `neo-server/docker/browser.lock` pins
(headless, the SDK image's fonts). `--ci` is the same run plus a verdict (`ci.ts`, rules in
`node/expected.mjs`):

- `node/expected/<platform>.tsv` (`linux-x64` in CI) lists the in-scope tests that do not pass
  there, each `fail` or `skip`. The runner writes it; nobody edits it by hand.
- A test whose full-run result differs from its expectation (pass, or its listed outcome) is
  rerun alone, up to twice, stopping at the first rerun that matches. Matching once makes it
  **flaky**: reported, not failing the job, and it keeps its expectation. Never matching settles
  it at what its reruns agree on: an expected pass that does not pass is a **regression**; a
  listed test that passed every rerun is **newly passing**, and the job fails until the pull
  request takes it off the list, so the count only ratchets up; a listed `fail` that now skips
  (or the reverse) has a **changed outcome**.
- A skip in the full run is decided by the spec or the harness, not by timing, so it is not
  rerun: an expected pass that skipped is a regression straight away.
- A listed test that left the corpus is **stale**, and a rerun that produced no result for its
  test is **inconclusive**; both fail the job. A run that reported an error (Playwright's, a
  stop signal, browsers that survived cleanup) fails it too.
- The job summary has the totals, every regression, newly passing, changed and flaky test by name.
  The results folder (with `verdict.json`, `summary.md` and failure screenshots) is the
  `neo-spec-report` artifact; the list this run supports is `neo-spec-expected`.

**Updating the list**, when your pull request fixes or (deliberately) changes a test: take the
list your pull request's CI run wrote, with the GitHub CLI signed in,

```sh
bun run test:neo-spec --expected-from <run URL or ID>
```

then commit `node/expected/linux-x64.tsv`. It prints what it removed, added and changed; an
added test accepts a regression, so the pull request says why. Two pull requests that each fix
a test change different lines and merge cleanly. On a Linux machine,
`bun run test:neo-spec --ci --write-expected` writes the list from a local run instead.

## The corpus

**Pin.** The specs are Playwright `v1.64.1`
(`cfe490d941f992655948eee8fd4fd94e1c4234bf`), the release whose engine Neo vendors
(`../neo-playwright/third_party/playwright/VERSION`). Tests and engine come from the same
release, so no test asks for behavior newer than the engine. `corpus.json` holds the pin and
each suite's SHA-256; `node/curation.json` and `node/case-ids.json` name the same commit, and a
run stops if any of them disagrees with the prepared source.

**Preparing the source.** `bun run test:neo-spec` does this on its first run. `js/prepare.mjs`
exports the pinned commit from a Playwright clone with `git archive` (the clone's checkout is not
touched), runs `npm ci` and `npm run build` in the copy, and stamps it with the revision and the
hashes of every test, fixture and build product. Every run verifies that stamp. By hand:

```sh
git clone https://github.com/microsoft/playwright ~/code/playwright   # or: git fetch origin tag v1.64.1
NEO_TESTS_PLAYWRIGHT_SOURCE=~/.cache/browseros/neo-spec/playwright-1.64.1 \
  node crates/neo-sdk/neo-playwright-tests/js/prepare.mjs ~/code/playwright
```

It refuses to overwrite an existing directory; delete the old copy first to rebuild it.

**Selection.** `corpus.json` lists 139 suites: all 136 `tests/page` specs, plus three
`tests/library` specs that exercise pages: `page-close`, `browsercontext-pages`, and from
`headful` only HE-012 (`Page.bringToFront should work`), which runs headless by owner policy;
the file's other cases need a headed browser. Collection finds 2,649 tests. `node/curation.json`
leaves out these, each entry with its reason:

| Reason | Tests | Meaning |
|---|---:|---|
| `error formatting only` | 86 | The test checks only Playwright's message text, stack or call log. |
| `upstream-skipped` | 22 | The pinned spec skips or fixmes it on Chromium, so it never runs there. |
| `invalid-here` | 5 | It fails on reference Playwright against BrowserOS. |

Every other test runs. Curation also patches assertions on error wording (keeping that the call
fails, and its error class) and skips tests that need Playwright internals; `run.json.curation`
counts each. Tests are never dropped to raise a score: a deletion whose reason stops holding at
a new pin is removed.

**Case IDs.** Scope, curation and reports name tests by ID (`FI-005`). Upstream added IDs to its
test titles after 1.64.1 (microsoft/playwright#43485), so 1.64.1's titles carry none, and `node/case-ids.json`
assigns them by spec file and title path:

- A test whose file and title match a test in upstream's registry at `26137face` (the previous
  pin, whose IDs the earlier results use) keeps that ID.
- A test with no match gets the next `9NN` number of its file's code, from 901. Upstream numbers
  each code from 001 and is still below 600, so the two do not collide. At this pin: NE-901
  (split later upstream into NE-090 and NE-143) and FI-901 to FI-907 (`page.fill` value
  validation, removed upstream by microsoft/playwright#43400 before IDs existed).
- An ID whose test is not at the pin is retired, never reused.

Every run checks the map against the collected tests and stops on any test without an ID or
any ID without a test. Playwright's own output and `--grep` see titles only, so `--grep FI-005`
selects nothing: pass the spec file and `--grep` the title. Reports print the ID before the
title.

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
  The test runner owns three things on a Neo receiver, each with its own annotation
  description:
  - `toHaveScreenshot` (`playwright-visual/neo-capture`): Playwright Test owns baseline
    paths, tolerances, attachments and the pinned stability/comparison loop. A visual
    receiver supplies only real Neo `screenshot` and `evaluate` calls, including for
    locators and explicit masks.
  - matchers a spec defines with `expect.extend` (`playwright-custom/neo-calls`): user
    code that Playwright runs on the Neo receiver itself, so its page reads are Neo calls
    (`page-emulate-media`'s `toMatchMedia`). Only the names passed to `extend` leave Neo;
    a built-in matcher Neo lacks still fails in Neo.
  - soft assertions (`expect.soft`, `configure({ soft: true })`, still `product`): Neo's
    hard matcher runs, and a failure is recorded on the test through its
    `_failWithError`, as Playwright's soft step records one, so the test fails at its end.
  Standalone Neo `expect` keeps its clear refusals of `soft` and `toHaveScreenshot`: a
  run has no test to record on and no baseline store.
  The legacy v2 adapter still uses `expectLoop` and reports its passes separately.
  The historical `--no-expect-loop` flag disables either assertion import adapter.
- Claw ignores only curation skips whose reason is an uncovered `expectLoop`
  matcher. The manifest, corpus, spec bodies and existing diagnostic patches
  remain unchanged. Product matcher refusals fail rather than skip, so unsupported
  assertions remain visible as product gaps. `run.json.assertions` records which
  assertion engine ran; `--no-curation` disables all curation as before.
- Each test sets its `testIdAttribute` through v1's `playwright.selectors`, so a
  test that changes the attribute does not leave it for the next test in the worker's run.
- `_setupArtifacts`, `_browserOptions`, `toImplInWorkerScope` and `startRemoteServer`
  keep explicit internals refusals.

The Node host registers its native `Buffer` parameter and byte-result reviver through the
bootstrap's provider/brand hooks. Screenshot bytes therefore have Node Buffer identity for
upstream image libraries and assertions. The production bundle stays byte-identical; its
QuickJS Buffer implementation has separate run-realm tests.

Fidelity costs: Node/V8 replaces QuickJS; one SDK run and profile span a worker;
`context.pages()` includes the launch tab; trace screenshots have no host camera.
These are harness differences, not SDK features.

The 71 previously skipped tests need the following matcher surface. Options and
expected values are forwarded to Neo, including negative assertion chains, except
for the runner-owned visual matcher described above.

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
that whole spec. Each has one of the reasons the manifest declares: `network`, `handles`
or `decided-deviation`. (`drift`, for specs newer than the engine, ended when the corpus
moved to the engine's own release; runs recorded before that keep it in their copied
manifest.) Network/handle entries identify explicit refusal evidence;
ordinary failures stay in scope. The initial evidence is the lead's parity gap
list and the corresponding per-test errors from the full run (mixed event groups
also contain supported events, so the entire group is not excluded).

Scope never skips, deletes, patches or reclassifies a test. Each score subtracts
the matched cases and their passing count; exclusions are counted per reason.
Every passing excluded test is listed for review. Filtered runs count only their
selected cases. `run.json.scope.manifestSha256` records a copied `scope.json` next
to the results; offline reports verify its hash. Legacy runs use the current
repository policy with an explicit provenance note.
