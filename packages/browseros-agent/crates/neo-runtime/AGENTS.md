# neo-runtime

Runs one agent script in a fresh QuickJS realm against one BrowserOS tab, and reports a `RunOutcome`. Every Playwright getter is a live CDP call; nothing is cached. Read `../NEO_STYLE.md` before changing anything here.

- Depends on `browseros-cdp` and ordinary crates only; `browseros-core` is never a dependency.
- `src/lib.rs` draws the call path; `src/bridge.rs` is the only door from JS into Rust.
- Runs need tokio's multi-threaded runtime: a synchronous getter blocks its worker with `block_in_place` while it waits for CDP.
- `src/realm.rs` embeds `../neo-sdk/dist/sdk.js`; change the SDK in `neo-sdk`, rebuild it, and commit the bundle.

Checks, from `packages/browseros-agent`: `scripts/neo-check.sh`. The end-to-end test needs a browser and skips without one:

```sh
NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo" \
  cargo test -p neo-runtime --test sync_getters -- --nocapture
```

Try a script by hand with `cargo run -p neo-runtime --example neo_run -- --help`.
