//! Wire types of the hosted API (`/v1`), generated from `contracts/hosted-api/openapi.yaml`.
//!
//! claw-server's `/v1` handlers return only these types, which is what keeps the served JSON
//! identical to the published spec. Regenerate with `bun run codegen:hosted-api`; never edit
//! `src/generated` by hand.

#[allow(clippy::all, unused_imports)]
#[path = "generated/mod.rs"]
pub mod models;

pub use models::*;
