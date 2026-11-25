use std::borrow::Cow;

use serde::Deserialize;

use crate::decode_error::DecodeError;
use crate::proto::AnnotatedPageContent;

/// Raw `Page.getAnnotatedPageContent` CDP JSON result shape.
///
/// Borrows directly from the JSON input buffer whenever `content` contains no escape sequences,
/// avoiding a second allocation of the base64 payload before decoding.
#[derive(Deserialize)]
struct ReplyEnvelope<'a> {
    #[serde(borrow)]
    content: Cow<'a, str>,
}

/// Decodes a standard-base64 `AnnotatedPageContent` payload (such as the `content` field from
/// CDP's `Page.getAnnotatedPageContent`) into a typed [`AnnotatedPageContent`] tree.
pub fn decode(base64_content: &str) -> Result<AnnotatedPageContent, DecodeError> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    let bytes = STANDARD.decode(base64_content)?;
    decode_protobuf(&bytes)
}

/// Decodes raw `AnnotatedPageContent` protobuf wire bytes into a typed [`AnnotatedPageContent`]
/// tree.
pub fn decode_protobuf(bytes: &[u8]) -> Result<AnnotatedPageContent, DecodeError> {
    use prost::Message as _;

    let content = AnnotatedPageContent::decode(bytes)?;
    Ok(content)
}

/// Decodes a raw `Page.getAnnotatedPageContent` CDP reply JSON string (`{"content": "<base64>"}`)
/// directly into a typed [`AnnotatedPageContent`] tree.
///
/// Deserializes straight into a typed borrowing envelope rather than an intermediate JSON DOM so
/// unescaped base64 payloads avoid copying.
pub fn decode_reply_json(reply_json: &str) -> Result<AnnotatedPageContent, DecodeError> {
    let envelope: ReplyEnvelope<'_> = serde_json::from_str(reply_json)?;
    decode(&envelope.content)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::error::Error;

    use super::ReplyEnvelope;

    #[test]
    fn reply_envelope_borrows_unescaped_content_zero_copy() -> Result<(), Box<dyn Error>> {
        let json = r#"{"content":"CgA="}"#;
        let envelope: ReplyEnvelope<'_> = serde_json::from_str(json)?;
        assert!(
            matches!(envelope.content, Cow::Borrowed("CgA=")),
            "unescaped base64 JSON strings should borrow directly from the input slice"
        );
        Ok(())
    }

    #[test]
    fn reply_envelope_owns_escaped_content() -> Result<(), Box<dyn Error>> {
        let json = r#"{"content":"\u0043gA="}"#;
        let envelope: ReplyEnvelope<'_> = serde_json::from_str(json)?;
        assert!(
            matches!(envelope.content, Cow::Owned(ref value) if value == "CgA="),
            "escaped JSON strings must unescape into an owned Cow buffer"
        );
        Ok(())
    }
}
