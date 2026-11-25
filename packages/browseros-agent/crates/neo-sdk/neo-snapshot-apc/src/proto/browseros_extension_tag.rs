//! Reserved field-tag and enum-value constants for BrowserOS extensions to Chromium's
//! `AnnotatedPageContent` schema.
//!
//! Per `apc-contract-design`, BrowserOS never occupies upstream Chromium low tag numbers in
//! `common_quality_data.proto`. Instead, BrowserOS extension payloads and sentinel enum values
//! begin at tag `1000` on [`AnnotatedPageContent`](super::AnnotatedPageContent),
//! [`ContentAttributes`](super::ContentAttributes), [`FrameData`](super::FrameData), and
//! [`RedactedFrameReason`](super::RedactedFrameReason). Until those extension messages are
//! defined, the decoder skips unknown field `1000` cleanly and preserves unknown enum value
//! `1000` via Edition 2023 open-enum `i32` storage.

/// Protobuf field tag reserved for BrowserOS extension submessages on `AnnotatedPageContent`,
/// `ContentAttributes`, and `FrameData` (per `apc-contract-design`).
pub const BROWSEROS_EXTENSION_TAG: u32 = 1000;

/// Open-enum value reserved for a BrowserOS frame-capture timeout in `RedactedFrameReason`
/// (per `apc-contract-design`).
pub const REASON_BROWSEROS_TIMEOUT: i32 = 1000;
