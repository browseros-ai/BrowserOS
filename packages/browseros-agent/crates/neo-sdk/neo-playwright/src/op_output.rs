//! Op results as the JSON the prelude reads.

use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

use crate::op_scope::OpScope;
use crate::{ErrorCode, PlaywrightError};

/// `{"$neo":"bytes","base64":"..."}`: binary results (screenshots, PDFs), which the bootstrap
/// turns into a `Uint8Array`.
// The bootstrap revives this brand to a Uint8Array before the prelude sees it.
#[derive(Debug, Serialize, TS)]
#[ts(type = "Uint8Array")]
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
///
/// Every typed route row's answer passes through here (`Run::op`), and this step is what puts
/// its keys in sorted order on the wire, at every level: `to_value` builds `serde_json::Map`s,
/// which are `BTreeMap`s while `serde_json`'s `preserve_order` feature is off, and whoever writes
/// the `Value` out walks them in that order. Several reply types declare their fields out of
/// alphabetical order (`NavigationResult`, `NavigationReply`, `LoadStateReached`, `Response`,
/// `FrameInfo`, `PartialFrame`, `AriaNode`) and answer the sorted keys the `json!` maps they
/// replaced did only because of it. Serializing a result straight to text, or any crate in the
/// build enabling `preserve_order`, would send those replies in declaration order with nothing
/// else noticing; `wire::route`'s `tests_that_an_answer_leaves_with_sorted_keys` fails first.
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
