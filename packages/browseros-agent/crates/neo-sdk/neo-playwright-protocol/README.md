# neo-playwright-protocol

Rust and TypeScript types for the two calls that cross a language boundary in Neo's browser
runtime, generated from one source:

- `serve`: the Playwright protocol commands `neo-runtime` serves to the agent's script.
- `injected`: the page calls `neo-runtime` makes into `neo-playwright-injected`'s glue table.

The source is Playwright's protocol spec (`spec/playwright/`, a byte-for-byte copy) plus two Neo
files in the same YAML dialect. `spec/serve.yml` selects the commands Neo serves, and
`spec/injected.yml` declares the page calls. A mismatch on either side of a call fails `rustc`
or `tsc`.

The crate docs in `src/lib.rs` explain how to serve another command, how to add a page call,
and what consumers need to know when decoding.

## Commands

Run these from `packages/browseros-agent`.

```sh
# Regenerate src/{serve,injected,spec_hash}.rs and ts/*.ts after a spec change.
NEO_RUNTIME_SPEC_CODEGEN=1 cargo run -p neo-playwright-protocol --example codegen

# Write nothing, and exit 1 if a committed file is stale. `cargo test` runs the same check.
NEO_RUNTIME_SPEC_CODEGEN=1 cargo run -p neo-playwright-protocol --example codegen -- --check

# Upgrade Playwright. This copies its spec, re-pins spec/playwright/UPSTREAM, and regenerates.
crates/neo-sdk/neo-playwright-protocol/sync-playwright.sh <playwright-checkout>
```

The bun package `@browseros/neo-playwright-protocol` wraps the first two as `codegen` and
`codegen:check`, and its `typecheck` script runs `tsc` over `ts/` and over the negative cases in
`tests/ts/`. The real glue table lives in `neo-playwright-injected`, whose `typecheck` checks it
against `ts/injected.ts`.

## Layout

| Path | Contents |
| --- | --- |
| `spec/playwright/` | Playwright's spec at the commit in `UPSTREAM`, with its `LICENSE` and `NOTICE` (Apache-2.0). Never edit. |
| `spec/serve.yml`, `spec/injected.yml` | Neo's selection and page calls. |
| `codegen/` | The translator, the crate's `codegen` example. `main.rs` maps its stages. |
| `src/lib.rs`, `src/call.rs` | Hand-written: the crate docs, the sealed call traits, and the freshness check. |
| `src/serve.rs`, `src/injected.rs`, `src/spec_hash.rs`, `ts/` | Generated and committed. Never edit. |
| `build.rs` | Hashes `spec/` so that a spec change without regenerating fails the build. |
| `tests/` | Wire behavior, the seal, and the TypeScript contract. |
