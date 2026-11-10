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

Both initial blank pages also receive Playwright's default focus emulation before either engine attaches, so reference setup cannot contribute an unmatched focus event. Events from the original bodies and native cleanup remain measured without filtering.

The corpus includes all 136 complete page suites, two complete library tab suites and the single `HE-012` bringToFront case from the third library suite: 139 suites and 2,667 generated cases (previously 48 suites and 948 cases). `corpus.json` assigns every suite a feature family, inclusion reason and source digest. Missing APIs are measured as unsupported rather than excluding their suites. Preparation verifies every digest; collection checks that every included suite contributes cases. The only spec edit is the owner's explicit headless skip waiver for HE-012. Its test body stays unchanged. Other changes are the browser fixture seam and nonzero server port allocation. The overlay records all before/after digests.

Each unique run directory under `target/neo-playwright-tests/runs/` contains `scoreboard.json`, `missing-methods.json`, collection metadata and exclusive per-case evidence. Set `NEO_TESTS_OUTPUT` to choose a different output parent and `NEO_TESTS_PLAYWRIGHT_SOURCE` to use another prepared directory. CI can retain those JSON files; filtered runs carry their catalog/selection counts and never overwrite a complete report.

A `Pass` requires raw reference and Neo passes, matching measured return values and thrown error **names**, matching complete effects, and proof of the owned native target. Error message differences become `wordingNotes`, never behavioral failures. Terminal ANSI color codes are stripped from comparison copies of journal values, effects and assertion diagnostics; raw evidence stays unchanged, including UTF-16. Discarded values count, including `goto` returning `Response` versus `null`. Matching expected failures cannot pass. A failed upstream wording assertion still fails: the pinned reporter retains rendered text but not the received value needed to prove it tested a message Neo actually threw.

Reports separate `Pass`, `Fail` (behavior differences), `Unsupported` (missing APIs), `BridgeLimit` (transport limitations), `ReferenceFailure`, `HarnessError` and `Skip`. A valid reference with missing APIs is unsupported; otherwise any Neo `bridge limit: ` limitation or transport error is bridge-limited, even when differences were measured. Worker death, runner exit mismatch and lost observations remain harness errors. Other fixture limitations remain harness errors. Earlier mismatches remain in evidence.

`wordingCount` counts cases with wording notes overall, by feature and by suite. `bridgeLimitWithEarlierDifferenceCount` counts bridge-limited cases with a paired behavioral difference before the first unfinished or untransportable Neo call. Unmatched reference tails alone do not increment it; limits without a provable call boundary retain differences without claiming they happened earlier. Bridge limits and unsupported cases are ignored Cargo completions and excluded from pass/(pass+fail). Real differences, invalid references and harness errors fail Cargo. A selection without a fully comparable case exits unsuccessfully.

The missing-method ranking deduplicates full generated case identities, records later requirements reached by the complete reference, and labels partial reference paths. Counts overlap across methods.
