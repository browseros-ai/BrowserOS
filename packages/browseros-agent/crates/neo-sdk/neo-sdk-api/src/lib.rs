//! Wire types of the claw-sdk API (`/v1` on claw-sdk's host listener), generated from
//! `crates/neo-sdk/neo-sdk-api/openapi.yaml`.
//!
//! claw-sdk's service routes return only these types, which is what keeps the served JSON
//! identical to the published spec. Regenerate with `bun run codegen:neo-sdk-api`; never
//! edit `src/generated` by hand.

#[allow(clippy::all, unused_imports)]
#[path = "generated/mod.rs"]
pub mod models;

pub use models::*;
