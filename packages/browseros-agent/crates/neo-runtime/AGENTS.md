# neo-runtime

Runs one script in a fresh QuickJS realm from a starting BrowserOS tab. Its context can hand out more pages; `RunOutcome` records the calls and page identities. Every Playwright getter is a live CDP call; nothing is cached. Read `../NEO_STYLE.md` before changing anything here.

- Depends on `browseros-cdp`, `neo-runtime-spec` and ordinary crates only; `browseros-core` is never a dependency.
- `src/tabs.rs` owns the context scope, first-seen page order, and one lazy session per handed-out tab. `src/tab_calls.rs` handles page/context lifecycle. Finish shares one bounded drain across creation settlement, tasks, and closing-state reads; late unkept creations close in the background and may be absent from the outcome. Normal tabs survive the run.
- `src/lib.rs` draws the call path; `src/bridge.rs` is the only door from JS into Rust, and `src/dispatch.rs` the only path a browser call takes.
- A browser call runs on its own tokio task and keeps its state in the run's trace (`src/progress.rs`), never in the realm, so a call the script never awaits, or one the run ends under, still has its record.
- `tests/fixtures/replies.json` pins the reply envelope for neo-sdk's tests; rewrite it with `NEO_RUNTIME_WRITE_FIXTURES=1 cargo test -p neo-runtime --lib dispatch` after a deliberate change.
- Runs need tokio's multi-threaded runtime: a synchronous getter blocks its worker with `block_in_place` while it waits for CDP.
- `src/realm.rs` embeds `../neo-sdk/dist/sdk.js`; change the SDK in `neo-sdk`, rebuild it, and commit the bundle.

- `src/reads.rs` is `Frame.textContent`, `Frame.innerText` and `Frame.getAttribute` (Playwright's `_waitForFunctionOnSelector` over loop A's backoff) and `Frame.ariaSnapshot` without a selector (polls `body,frameset`), over their own seam traits, tested with a scripted tab under paused time; `src/aria_yaml.rs` renders the snapshot as Playwright's YAML, pinned byte for byte by `../neo-runtime-injected/tests/aria-yaml-goldens.json`.
- `src/states.rs` waits for visibility's selector context, then reads once (missing/evaluation disconnect → false); `isChecked` polls for a match through `reads.rs`. Both are reads without input delivery.
- `src/lists.rs` serves Locator collection snapshots (`count`, `allTextContents`): `describe` rejects an invalid selector with Playwright's message, then one non-strict glue read, without waiting, target marks or call-log lines; scripted-tab tests pin values and page steps.
- `src/waits.rs` owns selector, URL and lifecycle waits over one loop and the input-free `WaitTab` seam; navigation waits use buffered commits and lifecycle counts, DOM waits use loop A's backoff, and `Progress` owns every deadline and cancellation. `waitForTimeout` is one timer from the call's start, with `prepare` raced against it only to arm close detection. `waitForSelector` replies `{}` for every state until Neo serves `ElementHandle`; an assertion on `ElementHandleChannel` stops the build when it does.
- `src/click.rs` is `Frame.click`'s policy (Playwright's two retry loops, every call-log line) over the `Tab` seam; `src/page.rs` implements the seam over CDP, and `src/world.rs` calls the injected glue. Click policy is tested against a scripted `FakeTab` under paused time (what a click calls, in which order, and what it delivers; never log wording or retry counts); the mechanism against `tests/support/fake_browser.rs` (`tests/click_lifecycle.rs`).
- A tab has one mouse and one hit-target slot, shared by every call and run on it. Input to a tab takes a turn (`Tab::input`, `src/page.rs` `TabInputs`, per `Runtime` and tab) from the interceptor's setup until its stop has answered, mouse writes included; never wait for an element while holding one.

- Every constant, and every value, step or message ported from Playwright, carries a provenance line (`Playwright:`, `Deviation:`, `Neo:`, `Platform:` or `Unexplained:`); `cargo test -p neo-runtime --test provenance` checks each line against Playwright vendored under `vendor/playwright/`, and regenerates `../NEO_DEVIATIONS.md` with `NEO_RUNTIME_WRITE_FIXTURES=1` (`../NEO_STYLE.md`, Provenance).

Checks, from `packages/browseros-agent`: `scripts/neo-check.sh`. The end-to-end tests need a browser and skip without one:

```sh
export NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo"
cargo test -p neo-runtime --test tabs_e2e --test sync_getters --test click_e2e --test navigation_e2e \
  --test reads_e2e --test states_e2e --test acceptance_e2e --test waits_e2e -- --nocapture
```

Try a script by hand with `cargo run -p neo-runtime --example neo_run -- --help`.

Live website scenarios are in `integration-tests/`, registered in Cargo.toml. They are ignored
by default; their module docs give opt-in commands. They use temporary profiles, including for
Amazon's anonymous cart write, and keep one Rust `Runtime` across submitted scripts.
