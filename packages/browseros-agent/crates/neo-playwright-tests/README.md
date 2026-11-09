# Neo Playwright tests

Run Playwright's own tests against real Playwright and Neo. Each upstream case is a named Cargo test. Failures and missing methods are expected; the report tells feature owners what to build next.

From `packages/browseros-agent`, prepare the pinned upstream source once. This builds a private copy and does not modify the source checkout or download a browser.

```sh
node crates/neo-playwright-tests/js/prepare.mjs /path/to/playwright
export CARGO_BUILD_BUILD_DIR="$PWD/target/build-private"
export CARGO_TARGET_DIR="$PWD/target/private"
export NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo"
cargo test -p neo-playwright-tests --locked --features playwright
```

The source checkout must contain commit `26137face0500cacddf90c767dc1364da7262559` (Playwright 1.66.0-next). Preparation uses `git archive`, `npm ci`, and the upstream build. Node 26 and Bun are needed for the repository's checks. **Node runs the upstream suite.** Bun 1.3.6 was tried first, but its `http.Server.closeAllConnections()` closes the listening socket used by upstream `TestServer.reset()`. Node 26 preserves the listener. Changing the upstream server would change the test infrastructure being measured.

List or filter the cases with normal Cargo arguments.

```sh
cargo test -p neo-playwright-tests --locked --features playwright -- --list
cargo test -p neo-playwright-tests --locked --features playwright -- pa_007
cargo test -p neo-playwright-tests --locked --features playwright -- --test-threads=2
```

Listing requires the prepared source but launches no browser. A real run fails clearly if the browser or prepared source is missing. Default `cargo test -p neo-playwright-tests --locked` runs the offline harness tests; the expected-red upstream target requires `--features playwright`.

```text
Original upstream test body, fixtures and expect        [Node]
       |                              |
       v                              v
Real Playwright                 Native object proxy
       |                              |
Headless BrowserOS A        Independent I/O thread
                                      |
                            One neo-runtime run         [QuickJS]
                                      |
                            Actual neo-quickjs-injected objects
                                      |
                            Existing native Dispatcher
                                      |
                            Headless BrowserOS B

Independent read-only observers retain DOM and delivered events.
```

The two engines run sequentially on fresh processes and profiles, using the same leased upstream server origins and assets. Cargo runs four independent cases concurrently by default. Every case owns its processes and output files; only the Cargo parent writes the aggregate. No real Playwright object attaches to Neo's browser. Missing browser methods and browser-aware assertions have no fallback helpers. Unsupported transport values remain explicit evidence gaps.

Final effects are captured at the fixture boundary, before disconnecting the reference client resets its CDP viewport. The native fixture reaches that boundary after its single run and cleanup drain. A bounded file handshake keeps the independent observer in charge; a missing acknowledgement or lost snapshot stays an infrastructure error, with no later snapshot substituted.

The corpus includes 45 complete page suites, two complete tab suites and the single `HE-012` bringToFront case. `corpus.json` lists all 136 page suites, each inclusion/exclusion reason and every source digest. The only spec edit is the owner's explicit headless skip waiver for HE-012. Its test body stays unchanged. Other changes are the browser fixture seam and nonzero server port allocation. The overlay records all before/after digests.

Each unique run directory under `target/neo-playwright-tests/runs/` contains `scoreboard.json`, `missing-methods.json`, collection metadata and exclusive per-case evidence. Set `NEO_TESTS_OUTPUT` to choose a different output parent and `NEO_TESTS_PLAYWRIGHT_SOURCE` to use another prepared directory. CI can retain those JSON files; filtered runs carry their catalog/selection counts and never overwrite a complete report.

A pass requires a raw reference pass, a raw Neo pass, every measured return and exact public error agreeing, complete effects evidence and proof of the owned native target. Discarded values count, including `goto` returning `Response` versus `null`. Matching expected failures cannot pass. Missing APIs are `Unsupported` Cargo completions; earlier mismatches remain in their evidence. The missing-method ranking deduplicates full generated case identities, records later requirements reached by the complete reference, and labels partial reference paths. Counts overlap across methods.

Real mismatches, invalid references and harness failures fail Cargo. An empty or entirely unsupported selection also exits unsuccessfully because it establishes no compatibility pass rate. Skips remain distinct. Observation loss, a worker death or untransportable data cannot become a pass. Raw public errors and upstream assertions are retained without message stripping.
