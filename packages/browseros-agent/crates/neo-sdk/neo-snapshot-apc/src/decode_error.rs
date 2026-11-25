/// Failure decoding a CDP `Page.getAnnotatedPageContent` reply or payload into an
/// [`AnnotatedPageContent`](crate::AnnotatedPageContent) tree.
///
/// Each variant retains its underlying library error so callers can distinguish transport framing
/// corruption (JSON envelope or base64 encoding) from protobuf wire-format errors without losing
/// diagnostic context.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The `content` field was not valid standard base64.
    #[error("invalid base64 in AnnotatedPageContent payload: {0}")]
    Base64(#[from] base64::DecodeError),
    /// The decoded bytes did not conform to the `AnnotatedPageContent` protobuf wire format.
    #[error("invalid protobuf in AnnotatedPageContent payload: {0}")]
    Protobuf(#[from] prost::DecodeError),
    /// The `ContentNode` tree exceeded [`MAX_CONTENT_NODE_DEPTH`](crate::MAX_CONTENT_NODE_DEPTH).
    #[error("AnnotatedPageContent tree exceeded maximum nesting depth of {limit}")]
    NestingLimitExceeded {
        /// Maximum permitted `ContentNode` nesting depth.
        limit: usize,
    },
    /// The CDP JSON reply did not match the expected `{"content": "<base64>"}` envelope.
    #[error("invalid Page.getAnnotatedPageContent reply JSON: {0}")]
    ReplyJson(#[from] serde_json::Error),
}
