//! Request extractors that reproduce the TS (Hono + zod) input quirks: `ZodJson<T>` (non-JSON
//! content-type → `{}`; validation failure → 400 ZodError body; malformed JSON → 500 chat
//! envelope) and `ZodUuid` (the exact zod-4 uuid regex, stricter than `Uuid::parse_str` in some
//! ways and looser in others). Implemented by P3; S0 ships the types.

use serde::Serialize;
use uuid::Uuid;

/// A JSON body validated the zod way. Implements `FromRequest` once P3 lands.
#[derive(Debug, Clone)]
pub struct ZodJson<T>(pub T);

/// A path uuid accepted by zod 4's regex (versions 1-8, RFC variant, nil and max).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZodUuid(pub Uuid);

/// One entry of a zod error's issue array, rendered into the 400 body's `message`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ZodIssue {
    pub code: String,
    pub path: Vec<serde_json::Value>,
    pub message: String,
}
