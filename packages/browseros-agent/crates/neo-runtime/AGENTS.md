# neo-runtime

Runs one agent script in a fresh QuickJS realm against one BrowserOS tab, and reports a `RunOutcome` with one record per browser call. Every Playwright getter is a live CDP call; nothing is cached. Read `../NEO_STYLE.md` before changing anything here.

- Depends on `browseros-cdp`, `neo-runtime-spec` and ordinary crates only; `browseros-core` is never a dependency.
- `src/lib.rs` draws the call path; `src/bridge.rs` is the only door from JS into Rust, and `src/dispatch.rs` the only path a browser call takes.
- A browser call runs on its own tokio task and keeps its state in the run's trace (`src/progress.rs`), never in the realm, so a call the script never awaits, or one the run ends under, still has its record.
- `tests/fixtures/replies.json` pins the reply envelope for neo-sdk's tests; rewrite it with `NEO_RUNTIME_WRITE_FIXTURES=1 cargo test -p neo-runtime --lib dispatch` after a deliberate change.
- Runs need tokio's multi-threaded runtime: a synchronous getter blocks its worker with `block_in_place` while it waits for CDP.
- `src/realm.rs` embeds `../neo-sdk/dist/sdk.js`; change the SDK in `neo-sdk`, rebuild it, and commit the bundle.

Checks, from `packages/browseros-agent`: `scripts/neo-check.sh`. The end-to-end test needs a browser and skips without one:

```sh
NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo" \
  cargo test -p neo-runtime --test sync_getters -- --nocapture
```

Try a script by hand with `cargo run -p neo-runtime --example neo_run -- --help`.
