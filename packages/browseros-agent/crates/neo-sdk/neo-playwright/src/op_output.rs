//! Op results as the JSON the prelude reads.

use serde::Serialize;
use serde_json::Value;

use crate::op_scope::OpScope;
use crate::{ErrorCode, PlaywrightError};

/// `{"$neo":"bytes","base64":"..."}`: binary results (screenshots, PDFs), which the bootstrap
/// turns into a `Uint8Array`.
#[derive(Debug, Serialize)]
pub(crate) struct Bytes {
    #[serde(rename = "$neo")]
    brand: BytesBrand,
    base64: String,
}

/// The only brand binary results carry.
#[derive(Debug, Serialize)]
enum BytesBrand {
    #[serde(rename = "bytes")]
    Bytes,
}

impl Bytes {
    /// Bytes CDP already returned as base64.
    pub(crate) fn from_base64(base64: String) -> Self {
        Self {
            brand: BytesBrand::Bytes,
            base64,
        }
    }
}

/// `value` as the op's JSON result. The crate's result types always serialize; a failure would
/// be a Neo bug, so it is `neo / internal` rather than a panic.
pub(crate) fn to_output<T: Serialize>(
    scope: &OpScope<'_>,
    value: &T,
) -> Result<Value, PlaywrightError> {
    serde_json::to_value(value).map_err(|error| {
        scope.fail(
            ErrorCode::Internal,
            &format!("the result does not serialize: {error}"),
        )
    })
}
