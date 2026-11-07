# Neo crates: boundaries and style

Applies to every `crates/neo-*` crate and to `crates/neo-quickjs-injected`. Run `scripts/neo-check.sh` (from `packages/browseros-agent`) before you commit; it is the gate for everything below that a tool can check.

## Crate map

| Crate | Language | Role |
| --- | --- | --- |
| `neo-runtime-spec` | Rust, generated TS | Neo's cross-language contracts, translated from Playwright's protocol |
| `neo-runtime` | Rust | Runs an agent's Playwright-style script in a QuickJS realm against one BrowserOS tab |
| `neo-trace-store` | Rust | Persists runtime operation traces off the run path in its own SeaORM/SQLite file; provides typed reads and committed session heads |
| `neo-quickjs-injected` | TypeScript | The Playwright-shaped objects a script sees; `neo-runtime` embeds its `dist/sdk.js` |
| `neo-playwright-injected` | TypeScript (page) | Playwright's injected script, vendored unchanged, behind Neo's typed glue table; `neo-runtime` embeds its `dist/*.js` and calls them over CDP in an isolated world |

Renamed in November 2026: `neo-quickjs-injected` was `neo-sdk`, and `neo-playwright-injected` was `neo-runtime-injected`.

Dependencies point one way: `neo-quickjs-injected` and `neo-playwright-injected` are embedded by `neo-runtime`, which talks to the browser through `browseros-cdp` and may use `neo-runtime-spec`. `neo-quickjs-injected` and `neo-playwright-injected` never import each other; both take types from `neo-runtime-spec`'s TypeScript. Rust neo crates depend on those plus ordinary crates (tokio, serde, rquickjs, thiserror, tracing).

## Page objects

A page object that Rust refers to across page calls (an element, an armed hit-target interceptor) is a handle: `neo_runtime_spec::Handle<K>`, the CDP `objectId` of an object that stays in the page, typed by its kind. Each kind is declared in `spec/injected.yml` (the built-in `Element`, or a `type: handle` definition), and the translator lets a handle cross only as a top-level page call parameter or result property. Never pass a raw `objectId` string between page calls, and never keep element state in the page for Neo to look up later: a handle lives in the CDP object group of the session that read it, and releasing that group is what frees it.

`browseros-core` is never a dependency of a neo crate. Read it for patterns and copy what you need; Neo is a rebuild, and depending on it would pull the old runtime's state model back in.

## Rust

Google Rust style, enforced by the `[lints]` tables, `rustfmt.toml` and `clippy.toml` every Rust neo crate copies from `neo-runtime-spec`. The `[lints]` tables must stay identical across crates (`neo-check.sh` compares them); add crate-specific `clippy.toml` entries below the copied ones.

Our exceptions to the guide:

- Its `[chrome]` rules do not apply: async and tokio are fine, and crates use the normal Cargo layout (`src/`, `tests/`, `examples/`).
- Panics are avoided entirely, not only for recoverable errors: a broken invariant is a `Result` too. `unwrap` and `expect` are denied outside tests. Never call `catch_unwind` or reach inside a poisoned mutex.
- Silence a lint with `#[expect(lint, reason = "…")]`; `#[allow]` is linted.

Stable rustfmt skips three of the configured rules, so apply them by hand (or format once with a nightly `cargo +nightly fmt -p <crate>` and confirm stable `cargo fmt --check` agrees):

- One module per `use` statement: `use std::sync::{Arc, Mutex};`, never `use std::{fs, io::Write};`.
- Lowercase hex literals: `0xff`, not `0xFF`.
- Macro matchers formatted like code.

Two naming rules the lints do not catch: call free functions through their module (`time::sleep`, `task::block_in_place`), and import types to use them unqualified (`Duration`, not `std::time::Duration`).

## TypeScript (`neo-quickjs-injected`)

The bundle runs in QuickJS, not Node or a browser: ES2023 only, no DOM or Node APIs, and evaluating it defines no globals. Getters return plain values, never promises, as in Playwright. `src/runtime-bridge.ts` is the only file that touches `__neo_runtime`. Commit `dist/sdk.js` with every `src/` change (`bun run build`).

## Provenance

Neo matches Playwright: where Neo serves a method, it uses Playwright's names, values and error wording. Every value, step and message that comes from outside Neo says where it comes from, on one line that `cargo test -p neo-runtime --test provenance` checks against Playwright's own source (vendored under `neo-runtime/vendor/playwright/` at the pinned commit), in CI too.

- Give every `const` and `static`, and every top-level TypeScript constant with a literal value, one provenance line in its doc comment. Pick the first that fits:
  - `Playwright: \`<path>\` \`<symbol>\`: \`<quote>\`` when it does what Playwright does. `<path>` starts at `server/` or `client/` (playwright-core's `src/`), at a `packages/` directory (`isomorphic/yaml.ts`, `injected/src/injectedScript.ts`), or at `/` for the repository root. `<symbol>` is `Class.member`, `function`, `CONSTANT`, or `outer/inner` for a closure. `<quote>` is Playwright's code copied exactly from that symbol, on one line.
  - `Deviation: \`<path>\` \`<symbol>\`[: \`<quote>\`] <why>` when a script, or a host reading the outcome, could tell it apart from Playwright: a cap, a refusal, a skipped step, other wording. Cite what Playwright does and say why Neo differs.
  - `Neo: <why>` when no script can see it and Playwright has nothing like it.
  - `Platform: <fact>` for a fact of Chromium, CDP, BrowserOS, JavaScript, QuickJS, tokio or serde, with its source.
  - `Unexplained: <what is unknown>` for existing code whose reason nobody recorded. It is a question for the owner.
- A `Playwright:` constant holds Playwright's value exactly. Compute a value that differs from it in a separate `Neo:` or `Deviation:` constant.
- Start each file's header with its provenance: `Neo:` for Neo's own module, one `Playwright:` line for a port of one Playwright file or class, several for a port of several. In a port of several, tag every free function, `impl` and `trait` block (in TypeScript, every top-level function and table member). A tag on a block covers its members.
- Inside a port, everything is Playwright's unless a `Deviation:` line says otherwise at that spot. Tag each step Playwright runs and Neo skips or does differently with `// Deviation: …` at the branch.
- Cite a symbol and a quote, never a line number. Name a Playwright source file in a comment only inside one of these lines.
- `crates/NEO_DEVIATIONS.md` lists every `Deviation:` and `Unexplained:` line and is generated: rerun `NEO_RUNTIME_WRITE_FIXTURES=1 cargo test -p neo-runtime --test provenance`, commit it with the change, and on a rebase conflict regenerate it rather than merge it.
- When the gate asks for a Playwright file that is not vendored yet, vendor it from a checkout at the pin: `NEO_PLAYWRIGHT=<checkout> cargo test -p neo-runtime --test provenance -- --ignored sync`. Never edit `vendor/`.

Every line written anywhere is checked: its citation must resolve, its quote must be in the cited symbol, a `Playwright:` constant's value must equal its quote's, and its reason must not be empty. Coverage (every item carries a line, no Playwright path outside a line, Playwright wording in call logs and errors) is enforced for the files listed in `neo-runtime/tests/provenance-enforced.txt`, which grows one file at a time until it holds every neo source. Until a file is listed, tag what you add to it anyway.

## Comments

Explain what a reader cannot reconstruct from one file:

- Every new module gets a `//!` (or file-level `/** */`) docstring saying its role in the run; central types get a short doc comment.
- Comment the seams: thread and async handoffs, the realm boundary between JS and Rust, a blocking wait, cancellation and deadlines, cleanup, and permission checks.
- Say what breaks if an invariant is removed.

Keep straightforward code bare: no line-by-line narration, PR narration, stale TODOs or commented-out code.
