# neo-runtime

Runs one agent script in a fresh QuickJS realm against one BrowserOS tab, and reports a `RunOutcome` with one record per browser call. Every Playwright getter is a live CDP call; nothing is cached. Read `../NEO_STYLE.md` before changing anything here.

- Depends on `browseros-cdp`, `neo-runtime-spec` and ordinary crates only; `browseros-core` is never a dependency.
- `src/lib.rs` draws the call path; `src/bridge.rs` is the only door from JS into Rust, and `src/dispatch.rs` the only path a browser call takes.
- A browser call runs on its own tokio task and keeps its state in the run's trace (`src/progress.rs`), never in the realm, so a call the script never awaits, or one the run ends under, still has its record.
- `tests/fixtures/replies.json` pins the reply envelope for neo-sdk's tests; rewrite it with `NEO_RUNTIME_WRITE_FIXTURES=1 cargo test -p neo-runtime --lib dispatch` after a deliberate change.
- Runs need tokio's multi-threaded runtime: a synchronous getter blocks its worker with `block_in_place` while it waits for CDP.
- `src/realm.rs` embeds `../neo-sdk/dist/sdk.js`; change the SDK in `neo-sdk`, rebuild it, and commit the bundle.

- `src/reads.rs` is `Frame.textContent` (Playwright's `_waitForFunctionOnSelector` over loop A's backoff) and `Frame.ariaSnapshot` without a selector (polls `body,frameset`), over their own seam traits, tested with a scripted tab under paused time; `src/aria_yaml.rs` renders the snapshot as Playwright's YAML, pinned byte for byte by `../neo-runtime-injected/tests/aria-yaml-goldens.json`.
- `src/click.rs` is `Frame.click`'s policy (Playwright's two retry loops, every call-log line) over the `Tab` seam; `src/page.rs` implements the seam over CDP, and `src/world.rs` calls the injected glue. Click policy is tested against a scripted `FakeTab` under paused time (the goldens' logs byte for byte); the mechanism against `tests/support/fake_browser.rs` (`tests/click_lifecycle.rs`).
- A tab has one mouse and one hit-target slot, shared by every call and run on it. Input to a tab takes a turn (`Tab::input`, `src/page.rs` `TabInputs`, per `Runtime` and tab) from the interceptor's setup until its stop has answered, mouse writes included; never wait for an element while holding one.

Checks, from `packages/browseros-agent`: `scripts/neo-check.sh`. The end-to-end tests need a browser and skip without one:

```sh
NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo" \
  cargo test -p neo-runtime --test sync_getters --test click_e2e --test navigation_e2e \
    --test reads_e2e -- --nocapture
```

Try a script by hand with `cargo run -p neo-runtime --example neo_run -- --help`.
