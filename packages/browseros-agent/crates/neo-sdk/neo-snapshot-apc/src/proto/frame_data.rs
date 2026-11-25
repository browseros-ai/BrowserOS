use prost::{Enumeration, Message, Oneof};

/// Frame-level metadata shared by the main frame ([`AnnotatedPageContent::main_frame_data`]) and
/// embedded iframes ([`IframeDataContent::FrameData`]).
///
/// [`AnnotatedPageContent::main_frame_data`]: super::AnnotatedPageContent::main_frame_data
#[derive(Clone, PartialEq, Eq, Message)]
pub struct FrameData {
    /// Security origin of the frame's active document.
    #[prost(message, optional, tag = "1")]
    pub security_origin: Option<SecurityOrigin>,
    /// Selection and focus state scoped to this frame.
    #[prost(message, optional, tag = "2")]
    pub frame_interaction_info: Option<FrameInteractionInfo>,
    /// Browser-lifetime-unique token identifying this document across APC calls.
    #[prost(message, optional, tag = "3")]
    pub document_identifier: Option<DocumentIdentifier>,
    /// Current URL of the frame.
    #[prost(string, optional, tag = "4")]
    pub url: Option<String>,
    /// Title of the frame's document.
    #[prost(string, optional, tag = "5")]
    pub title: Option<String>,
    /// Paywall classification when paid-content detection ran on this frame.
    #[prost(message, optional, tag = "6")]
    pub paid_content_metadata: Option<PaidContentMetadata>,
    /// Media session and transcript data when this frame controls the page's media session.
    #[prost(message, optional, tag = "8")]
    pub media_data: Option<MediaData>,
    /// Page-declared script tools (`WebMCP` / document tools) exposed by this document.
    #[prost(message, repeated, tag = "9")]
    pub script_tools: Vec<ScriptTool>,
    /// Results from previously executed script tools in this frame.
    #[prost(message, repeated, tag = "11")]
    pub script_tool_results: Vec<ScriptToolResult>,
    /// Positive used line height in CSS pixels for scaling-proof geometry heuristics.
    #[prost(uint32, optional, tag = "12")]
    pub default_line_height_px: Option<u32>,
    // Field 1000 (`BROWSEROS_EXTENSION_TAG`): reserved for BrowserOS per-frame extension
    // submessage per `apc-contract-design`.
}

/// Payload for an embedded iframe node ([`ContentData::Iframe`](super::ContentData::Iframe)).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct IframeData {
    /// Either full frame metadata for an included iframe or redaction metadata when excluded.
    #[prost(oneof = "IframeDataContent", tags = "3, 4")]
    pub data: Option<IframeDataContent>,
}

/// Oneof payload carried by [`IframeData::data`].
#[derive(Clone, PartialEq, Eq, Oneof)]
pub enum IframeDataContent {
    /// Metadata for an embedded iframe whose content was included in the snapshot.
    #[prost(message, boxed, tag = "3")]
    FrameData(Box<FrameData>),
    /// Metadata explaining why this iframe's content was omitted from the snapshot.
    #[prost(message, tag = "4")]
    RedactedFrameMetadata(RedactedFrameMetadata),
}

/// Explanation for why an embedded iframe's content was redacted from the snapshot.
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct RedactedFrameMetadata {
    /// Reason the iframe was redacted (open [`RedactedFrameReason`] enum; preserves future values
    /// such as [`REASON_BROWSEROS_TIMEOUT`](super::REASON_BROWSEROS_TIMEOUT)).
    #[prost(enumeration = "RedactedFrameReason", optional, tag = "1")]
    pub reason: Option<i32>,
}

/// Reason an embedded iframe was redacted in [`RedactedFrameMetadata::reason`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum RedactedFrameReason {
    /// Redaction reason unspecified.
    Unspecified = 0,
    /// Redacted because the iframe is not on the same `eTLD+1` site as the top-level frame.
    CrossSite = 1,
    /// Redacted because the iframe is not on the same origin as the top-level frame.
    CrossOrigin = 2,
    // Value 1000 (`REASON_BROWSEROS_TIMEOUT`): reserved for BrowserOS frame-capture timeout per
    // `apc-contract-design`. Open enum `Option<i32>` storage preserves `1000` without failing
    // decode.
}

/// Selection and focus state scoped to a single frame.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct FrameInteractionInfo {
    /// Current text selection inside the frame, if any.
    #[prost(message, optional, tag = "1")]
    pub selection: Option<Selection>,
    /// Content node ID currently holding DOM focus within this frame.
    #[prost(int32, optional, tag = "2")]
    pub focused_node_id: Option<i32>,
    /// Content node ID currently holding screen-reader accessibility focus within this frame.
    #[prost(int32, optional, tag = "3")]
    pub accessibility_focused_node_id: Option<i32>,
}

/// Active text selection range within a single frame.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct Selection {
    /// Content node ID where the selection starts (always before or equal to [`Self::end_node_id`]).
    #[prost(int32, optional, tag = "1")]
    pub start_node_id: Option<i32>,
    /// Character offset of the selection start within [`Self::start_node_id`].
    #[prost(int32, optional, tag = "2")]
    pub start_offset: Option<i32>,
    /// Content node ID where the selection ends.
    #[prost(int32, optional, tag = "3")]
    pub end_node_id: Option<i32>,
    /// Character offset of the selection end within [`Self::end_node_id`].
    #[prost(int32, optional, tag = "4")]
    pub end_offset: Option<i32>,
    /// Plain text covered by the selection.
    #[prost(string, optional, tag = "5")]
    pub selected_text: Option<String>,
}

/// Stable identifier directing actions to a specific loaded document across APC calls.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct DocumentIdentifier {
    /// Serialized document token unique across all documents for the browser's lifetime.
    #[prost(string, optional, tag = "1")]
    pub serialized_token: Option<String>,
}

/// Paid-content detection result for a frame.
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct PaidContentMetadata {
    /// Whether the frame contains paywalled or paid content.
    #[prost(bool, optional, tag = "1")]
    pub contains_paid_content: Option<bool>,
}

/// Media session playback metadata and generated transcripts for a frame.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct MediaData {
    /// Media kind controlled by the session (open [`MediaDataType`] enum).
    #[prost(enumeration = "MediaDataType", optional, tag = "1")]
    pub media_data_type: Option<i32>,
    /// Transcript segments heard by the user, ordered by ascending start timestamp.
    #[prost(message, repeated, tag = "2")]
    pub transcripts: Vec<MediaTranscript>,
    /// Total media duration in milliseconds.
    #[prost(int64, optional, tag = "3")]
    pub duration_milliseconds: Option<i64>,
    /// Current playback position in milliseconds.
    #[prost(int64, optional, tag = "4")]
    pub current_position_milliseconds: Option<i64>,
    /// Whether playback is currently active (`true`) or paused (`false`).
    #[prost(bool, optional, tag = "5")]
    pub is_playing: Option<bool>,
    /// Non-empty media title set by the site or defaulted to the page title.
    #[prost(string, optional, tag = "6")]
    pub title: Option<String>,
    /// Media artist name, if provided.
    #[prost(string, optional, tag = "7")]
    pub artist: Option<String>,
    /// Media album name, if provided.
    #[prost(string, optional, tag = "8")]
    pub album: Option<String>,
}

/// Media kind reported on [`MediaData::media_data_type`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum MediaDataType {
    /// Media kind unknown or unspecified.
    Unknown = 0,
    /// Video media session.
    Video = 1,
    /// Audio-only media session.
    Audio = 2,
}

/// Single timestamped segment of a frame's media transcript.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct MediaTranscript {
    /// Transcribed text for this segment.
    #[prost(string, optional, tag = "1")]
    pub text: Option<String>,
    /// Non-negative offset in milliseconds from the start of the media.
    #[prost(int64, optional, tag = "2")]
    pub start_timestamp_milliseconds: Option<i64>,
}

/// Tool descriptor exposed by a web document in [`FrameData::script_tools`].
#[derive(Clone, PartialEq, Eq, Message)]
pub struct ScriptTool {
    /// Document-unique tool name (wire tag 2; tag 1 is unused in Chromium's schema).
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
    /// Human-readable description of what the tool does.
    #[prost(string, optional, tag = "3")]
    pub description: Option<String>,
    /// JSON Schema string describing the tool's expected input object.
    #[prost(string, optional, tag = "4")]
    pub input_schema: Option<String>,
    /// Behavioral annotations declared on the tool.
    #[prost(message, optional, tag = "5")]
    pub annotations: Option<ScriptToolAnnotations>,
}

/// Behavioral annotations declared on a [`ScriptTool`].
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct ScriptToolAnnotations {
    /// Whether invoking the tool has no side effects.
    #[prost(bool, optional, tag = "1")]
    pub read_only: Option<bool>,
}

/// Output captured from invoking a document's [`ScriptTool`].
#[derive(Clone, PartialEq, Eq, Message)]
pub struct ScriptToolResult {
    /// Index of the corresponding script-tool action in Chromium's `Actions` request.
    #[prost(int32, optional, tag = "1")]
    pub index_of_script_tool_action: Option<i32>,
    /// Result payload returned by the tool when execution succeeded.
    #[prost(string, optional, tag = "2")]
    pub result: Option<String>,
    /// Name of the invoked tool for result correlation.
    #[prost(string, optional, tag = "3")]
    pub tool_name: Option<String>,
    /// Serialized arguments passed when invoking the tool.
    #[prost(string, optional, tag = "4")]
    pub input_arguments: Option<String>,
    /// Descriptor of the executed tool.
    #[prost(message, optional, tag = "5")]
    pub tool: Option<ScriptTool>,
}

/// Web security origin (`scheme`, `host`, `port` or opaque nonce) attributing web content in APC.
///
/// Distinct from a frame's URL because documents such as `about:blank` inherit their opener's
/// origin.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct SecurityOrigin {
    /// Whether this origin is opaque (serialized as a nonce rather than a `scheme://host:port`
    /// tuple).
    #[prost(bool, optional, tag = "1")]
    pub opaque: Option<bool>,
    /// Nonce string for opaque origins, or serialized scheme-host-port string otherwise.
    #[prost(string, optional, tag = "2")]
    pub value: Option<String>,
}
