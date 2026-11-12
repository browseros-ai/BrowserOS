//! A value an op answers in Playwright's in-page serialized format.

use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

/// One value the page answered, in Playwright's in-page serialized format
/// (`utilityScriptSerializers.ts`), already held to that format's depth and shape
/// (`page_value.rs`); the prelude revives it with the vendored `parseEvaluationResultValue`.
/// Rust passes it through untouched, so TypeScript names the vendored declaration instead of a
/// derived shape. `tests/data/in-page-values.json` holds the two sides together.
// A newtype serializes as the value it wraps, so the answer's bytes are the page's own.
#[derive(Debug, Serialize, TS)]
#[ts(
    type = "import('../../../../../neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers').SerializedValue"
)]
pub(crate) struct SerializedValue(pub(crate) Value);
