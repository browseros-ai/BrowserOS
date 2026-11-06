//! Neo's cross-language runtime contracts, translated from Playwright's protocol YAML dialect.
//!
//! The call traits in this crate are sealed; the types that implement them are generated from
//! `spec/` by the translator in `codegen/`.

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
    CallDecodeError, Command, ElementCall, InjectedCall, MethodFlag, MethodGroup, MethodMeta,
    RemoteElement, ValueCall,
};
pub use spec_hash::SPEC_SHA256;

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
