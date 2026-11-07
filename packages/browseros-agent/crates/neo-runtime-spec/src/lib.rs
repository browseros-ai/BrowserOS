//! Neo's cross-language runtime contracts, translated from Playwright's protocol YAML dialect.
//!
//! Two calls cross a language boundary in Neo's browser runtime. This crate types both from one
//! source, Playwright's own protocol spec plus two Neo files, so a mismatch on either side fails
//! a compiler:
//!
//! ```text
//!  agent script (QuickJS)            neo-runtime (Rust)                    page (V8)
//!  Playwright client ──serve──▶ FrameCall::decode, handler ──injected──▶ glue table
//!  {method, params} JSON        typed params and result      CDP Runtime.callFunctionOn
//! ```
//!
//! - [`serve`] holds the Playwright commands Neo serves (`spec/serve.yml`), with params and
//!   result types taken unchanged from Playwright's spec (`spec/playwright/`). The call bridge
//!   decodes a call with [`serve::FrameCall::decode`], matches on it, and answers with
//!   [`Command::reply`], which accepts only that command's result type.
//! - [`injected`] holds the page calls `neo-runtime` makes into `neo-runtime-injected`'s glue
//!   table (`spec/injected.yml`). Each call's params type implements [`InjectedCall`], and its
//!   result type implements [`PageResult`]. A call crosses plain JSON plus handles: a
//!   [`Handle<K>`](Handle) names a page object that stays in the page, by its CDP object id,
//!   typed by kind ([`Element`], or a marker generated from a `type: handle` definition). The
//!   page does no serialization work, and the decode here is the only validation.
//!
//! The same types exist in TypeScript under `ts/`, in the bun package
//! `@browseros/neo-runtime-spec` (exports `./serve`, `./injected` and `./spec-hash`).
//! `neo-runtime-injected`'s glue table is written as
//! `export const glue = { ... } satisfies InjectedApi<InjectedScript, Handles>`, where `Handles`
//! binds each named handle kind to its Playwright page type, so it fails `tsc` when it drifts
//! from `spec/injected.yml`.
//!
//! # Regenerate after a spec change
//!
//! The generated files (`src/serve.rs`, `src/injected.rs`, `src/spec_hash.rs` and `ts/*.ts`)
//! are committed. Never edit them. After you change `spec/`, run this from
//! `packages/browseros-agent`:
//!
//! ```text
//! NEO_RUNTIME_SPEC_CODEGEN=1 cargo run -p neo-runtime-spec --example codegen
//! ```
//!
//! A spec change without that run does not compile. `build.rs` hashes `spec/`, and this crate
//! const-asserts that the hash equals [`SPEC_SHA256`], the hash the generated code came from.
//! The translator is an example of this crate, so Cargo builds this library before it runs.
//! `NEO_RUNTIME_SPEC_CODEGEN=1` builds the library empty, so the stale code cannot block the
//! run that replaces it. `cargo test` fails while a committed file differs from what the spec
//! generates, and adding `-- --check` to the command above reports the same without writing.
//!
//! # Serve another Playwright command
//!
//! 1. Add the command's name under its interface in `spec/serve.yml`, for example `dblclick`
//!    under `Frame`. The translator rejects a name that Playwright's spec lacks and suggests the
//!    closest one.
//! 2. Regenerate. `FrameCall` gains a variant, so `neo-runtime`'s exhaustive `match` fails to
//!    compile until the command has a handler.
//!
//! # Add a page call
//!
//! 1. Add a command under `Injected.commands` in `spec/injected.yml`, with `internal: true`,
//!    `parameters` and `returns`. The file's header lists the rules the translator enforces.
//! 2. Regenerate. `ts/injected.ts` gains the call, so `neo-runtime-injected` fails `tsc` until
//!    its glue table implements it, and its `dist/` must be rebuilt because the spec hash
//!    changed.
//!
//! A handle may be a top-level parameter, which crosses as a `Runtime.callFunctionOn` argument
//! after the params ([`InjectedCall::handles`]), or a top-level property of the result, alone or
//! as one member of a union. A result that holds one crosses through `Runtime.getProperties`
//! ([`PageResult::from_properties`]), so its other properties must be primitives. The translator
//! rejects a handle anywhere else, where it would have to cross as JSON.
//!
//! # Notes for consumers
//!
//! - `serde_json`'s default recursion limit fails on aria trees deeper than about 63 levels.
//!   Decode page results with a raised limit, or cap `depth`.
//! - `int` fields are `i64`. Decoding relies on CDP writing an integral double as a JSON
//!   integer, `-0` included, because `serde_json` rejects a literal `-0` for an integer.
//! - `Frame.goto` answers `response` as a reference to a `Response` object, or `None` where
//!   Playwright's client reads `null`. A `Request` initializer's `serviceWorker` stays `None`:
//!   `Worker` is not served, so its reference type has no values.
//! - An `Option` field decodes a JSON `null` as absent, which Playwright's validator would
//!   reject. Encoding never writes `null`.

// Cargo links this library into the translator (the `codegen` example), which has to run
// exactly when the freshness check below fails. Regenerating sets `NEO_RUNTIME_SPEC_CODEGEN`,
// which build.rs turns into this cfg, and the library then builds empty.
#![cfg(not(neo_runtime_spec_codegen))]

mod call;
#[rustfmt::skip]
pub mod injected;
#[rustfmt::skip]
pub mod serve;
#[rustfmt::skip]
mod spec_hash;

pub use call::{
    CallDecodeError, Command, Element, Handle, HandleId, HandleKind, InjectedCall, MethodFlag,
    MethodGroup, MethodMeta, PageResult, Properties, ResultDecodeError,
};
pub use spec_hash::SPEC_SHA256;

/// Whether `hash` is the spec hash this crate was generated from. `const`, so `neo-runtime` can
/// assert at compile time that the page bundle it embeds (`neo-runtime-injected`'s
/// `dist/spec-sha256`) was built from the same spec as these types.
#[must_use]
pub const fn is_current(hash: &str) -> bool {
    str_eq(hash, SPEC_SHA256)
}

// The generated modules must come from the spec as it is now: build.rs hashes `spec/` on every
// build, and a different hash means a spec file changed after the last translator run.
const _: () = assert!(
    str_eq(SPEC_SHA256, env!("NEO_RUNTIME_SPEC_SHA256")),
    "spec/ changed since the code in src/ was generated: in packages/browseros-agent, run \
     `NEO_RUNTIME_SPEC_CODEGEN=1 cargo run -p neo-runtime-spec --example codegen`",
);

/// `a == b` for strings, in a const context.
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut index = 0;
    while index < a.len() {
        if a[index] != b[index] {
            return false;
        }
        index += 1;
    }
    true
}
