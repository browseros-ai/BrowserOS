//! SeaORM entities for the three tables this server owns in `browseros.sqlite`. Column names
//! and types are the old Drizzle schema's (migration 0014), unchanged, because the TypeScript
//! server may still open the same file after a fallback. Private to `store`: the secret columns
//! are read nowhere else.

pub(in crate::store) mod conversations;
#[allow(
    dead_code,
    reason = "read by `store::tokens`, which chunk `settings` writes"
)]
pub(in crate::store) mod oauth_tokens;
#[allow(
    dead_code,
    reason = "read by `store::providers` and `store::agents`, which chunk `settings` writes"
)]
pub(in crate::store) mod providers;
