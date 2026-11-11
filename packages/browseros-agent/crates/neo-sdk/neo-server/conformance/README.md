# Playwright conformance

One source pin, one upstream suite, two real runtimes. The selected suite is Playwright v1.64.1's
32 core page specs (633 registrations), plus seven explicit screenshot checks. The engine's
`../neo-playwright/third_party/playwright/VERSION` pins the commit. No source checkout is moved,
and no assertions, setContent calls or page values are dropped or normalized.

```
read-only Playwright commit -> upstream transform -> program bytes (one file per test)
                                                   |                 |
                       native Playwright / BrowserOS                 Neo MCP run / QuickJS
                                                   |                 |
                                                   +---- judge ------+
```

The native BrowserOS oracle is not hosted. Neo uses the same browser binary in hosted mode,
through the built server, its dispatcher, takeover rules and trace. A program's upstream test
body, hooks and closure state stay in one module. Only Playwright Test infrastructure—test
registration, fixtures, hook execution, budgets and soft assertions—is supplied by `src/runtime.js`.
The runtime's own pages, locators, callbacks, handles and matchers remain under test.

Run commands from this server package, after the workspace's `bun install --frozen-lockfile --ignore-scripts`:

```sh
export PLAYWRIGHT_SRC=/path/to/read-only/playwright-clone
bun run conformance inventory
bun run conformance inventory --check
bun run test:conformance
bun run conformance show 'tests/page/page-fill.spec.ts › should fill textarea @smoke'

# Build the actual server and compile-only helper with the repo-pinned Rust toolchain,
# using task-private CARGO_TARGET_DIR and CARGO_BUILD_BUILD_DIR.
cargo +1.98.0 build --locked -p neo-server --example conformance_compile
cargo +1.98.0 build --locked -p neo-server --bin browseros-claw-sdk
export NEO_BINARY=/absolute/private-target/debug/browseros-claw-sdk
export BROWSEROS_APP=/absolute/path/to/browserclaw
bun run conformance compile
bun run conformance selftest --ports 19561
bun run conformance run --ports 19561
bun run conformance run --spec page-fill --shard 1/2 --repeat 2
bun run conformance judge /absolute/path/to/a/stored/run
```

The CLI deliberately uses Node (pinned to 24.22.2 in Docker). The upstream loader must not run
inside Bun: host Function#toString and transform behavior differ. Bun owns installation and the
focused unit tests. For registration parity under Bun tests, set `PLAYWRIGHT_SRC` and, when needed,
`CONFORMANCE_NODE` to the real Node executable. That check launches the real pinned Playwright Test
loader and compares spec, title path and source line: 2,552 registrations match its 2,619 tests;
the extra 67 are the Node-only upstream page-screenshot spec, recorded as a load exclusion.

Linux is authoritative. Docker builds with Rust 1.98.0, Bun 1.4.2, Node 24.22.2 and an explicit
multi-architecture Neo image digest. The server bundled in that browser image is disabled.
From the repository root:

```sh
export SOURCE_SHA=$(git rev-parse HEAD)
export PLAYWRIGHT_SRC=/absolute/path/to/playwright
export CONFORMANCE_OUTPUT=/absolute/private/results
export CONFORMANCE_DATA=/absolute/private/data
mkdir -p "$CONFORMANCE_OUTPUT" "$CONFORMANCE_DATA"
docker compose --project-name <run-prefix> -f packages/browseros-agent/crates/neo-sdk/neo-server/conformance/compose.yaml build
docker compose --project-name <run-prefix> -f packages/browseros-agent/crates/neo-sdk/neo-server/conformance/compose.yaml run --rm conformance
```

No host ports are published. Each run owns its profiles, short Unix socket, bearer token, children
and four consecutive internal ports. Both browsers and the fixture server are stopped on completion
or interruption; only the run's temporary directories are removed. Native tests execute in individual
Node processes, so a timed-out async body cannot continue into the next test. A page-count threshold
restarts a leaking rig and records the event. Private result directories retain every program,
per-side record, trace, calibration, screenshot and report.

`run` first compiles every generated program with the production QuickJS engine, then calibrates both
runners. The calibrations intentionally include a wrong native expectation, a Neo refusal, a deliberate
assertion difference, an assertion-count difference, a one-millisecond run timeout and a closed MCP port. They
must classify as intended before any upstream tests run. The runtime-global check exercises production
URL/URLSearchParams/atob/btoa; there are no harness polyfills or pending-global exceptions.

A browser-level CDP client on each side sizes new pages and popups to Playwright's default
1280×720 at DPR 1 before releasing them. The viewport calibration checks both browsers before
and after cross-site navigation. A test may still change its own viewport where the runtime
supports it. Sizing and connection failures are harness errors. Each
`viewport-rig-<side>.json` records the page attachments for every browser launched in the run.

A verdict means upstream's own assertions held on both sides, with equal assertion counts and verified
program hashes/provenance. `unsupported`, `deviation`, `oracle-fail`, `harness-unavailable`, `skipped` and
`harness-error` are distinct outcomes. Missing results, malformed records, different program hashes,
invalid traces and transport failures never count as passes. Unsupported calls are non-gating only
when present in the production refusal table, checked by `loader.test.mjs`. An unexpected runtime gap
is a failing result to report to its owning product task.

Known upstream-only dependencies are listed by test ID in `harness-unavailable.json`. These
registrations stay in the inventory with the fixture or TestInfo requirement and count as
`harness-unavailable`. Remaining native failures keep their oracle verdicts.

Only diagnostic text is tolerated. The loader exposes marked `.message` and `toThrow(text/regex)`
assertions in the inventory. The runner captures the original throw or rejection once. The judge checks exception identity
against the native record and gives traced refusals their unsupported verdict, including refusals
caught by an assertion. Page text, input values, URLs, titles,
accessibility snapshots, error classes and screenshot dimensions remain strict. Pixel comparisons
allow channel noise of at most 2/255 on no more than 0.1% of pixels; images and measurements are retained.

Deviation and oracle-exception entries require a test id, exact status/code/message pattern, reason
and decision. Stale, changed and unknown entries fail. Repeat runs retain all attempts and mark changing
classifications flaky; a passing later attempt cannot hide an earlier failure. A filtered run validates
only its selected entries. Default suite budget is 120 minutes; each test has its own runtime timeout,
with hook/cleanup allowance and a larger outer MCP/native-worker budget. Interrupted or incomplete
runs still publish failure evidence. Stored judging reads the requested repeat count and
terminal state from `run-state.json`, requires one record per side for each planned attempt,
and preserves terminal failures.

Neo-specific expectations are separate in `tests/behavior/` and can be run with `bun run conformance behavior`
or the `behavior_browseros` Rust integration target (set `CLAW_SDK_E2E_BROWSER`). They use real MCP run
with explicit values and have no native comparison. `tier-triage.md` accounts for the 52 former tier-1
cases; source-specific integration regressions remain product tests.

Single-side diagnostics are available with `--side native` or `--side neo`; the report remains
unpaired and cannot pass conformance. `--keep` retains a copy of the run's private state after
process shutdown for investigation. A repeated run id is rejected to prevent stale records
from being mistaken for fresh results. The root project test suite includes the pure harness checks.
