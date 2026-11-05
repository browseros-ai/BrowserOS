//! Neo's cross-language runtime contracts, translated from Playwright's protocol YAML dialect.
//!
//! The call traits in this crate are sealed; the types that implement them are generated from
//! `spec/` by the translator in `codegen/`.

mod call;

pub use call::{
    CallDecodeError, Command, ElementCall, InjectedCall, MethodFlag, MethodGroup, MethodMeta,
    RemoteElement, ValueCall,
};
