use prost::{Enumeration, Message};

use super::frame_data::SecurityOrigin;

/// Leaf text run and presentation summary carried by
/// [`ContentData::Text`](super::ContentData::Text).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct TextInfo {
    /// Rendered text string for this leaf node.
    #[prost(string, optional, tag = "1")]
    pub text_content: Option<String>,
    /// Coarse size, emphasis, and RGBA color styling for the text run.
    #[prost(message, optional, tag = "3")]
    pub text_style: Option<TextStyle>,
}

/// Coarse presentation style attached to a [`TextInfo`] node.
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct TextStyle {
    /// Relative text size bucket (open [`TextSize`] enum; defaults to [`TextSize::MDefault`]).
    #[prost(enumeration = "TextSize", optional, tag = "1")]
    pub text_size: Option<i32>,
    /// Whether inline emphasis semantics (bold, italic, underline, superscript, and so on) apply.
    #[prost(bool, optional, tag = "2")]
    pub has_emphasis: Option<bool>,
    /// Text color packed in RGBA format.
    #[prost(uint32, optional, tag = "3")]
    pub color: Option<u32>,
}

/// Coarse relative text size bucket carried on [`TextStyle::text_size`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum TextSize {
    /// Medium (default) body text size.
    MDefault = 0,
    /// Extra-small text size.
    Xs = 1,
    /// Small text size.
    S = 2,
    /// Large text size.
    L = 3,
    /// Extra-large text size.
    Xl = 4,
}

/// Image caption and provenance carried by [`ContentData::Image`](super::ContentData::Image).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct ImageInfo {
    /// Caption or alternative text associated with the image (wire tag 2; tags 1 and 3 reserved).
    #[prost(string, optional, tag = "2")]
    pub image_caption: Option<String>,
    /// Security origin from which the image resource was loaded.
    #[prost(message, optional, tag = "4")]
    pub security_origin: Option<SecurityOrigin>,
    /// Original `src` URL of the image prior to any redirect.
    #[prost(string, optional, tag = "5")]
    pub url: Option<String>,
}

/// Text extracted from an `<svg>` subtree carried by
/// [`ContentData::SvgRoot`](super::ContentData::SvgRoot).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct SvgRootData {
    /// Concatenated inner text inside the SVG subtree.
    #[prost(string, optional, tag = "1")]
    pub inner_text: Option<String>,
}

/// Layout dimensions of a `<canvas>` element carried by
/// [`ContentData::Canvas`](super::ContentData::Canvas).
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct CanvasData {
    /// Layout width of the canvas element.
    #[prost(int32, optional, tag = "1")]
    pub layout_width: Option<i32>,
    /// Layout height of the canvas element.
    #[prost(int32, optional, tag = "2")]
    pub layout_height: Option<i32>,
}

/// Source URL and origin of a `<video>` element carried by
/// [`ContentData::Video`](super::ContentData::Video).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct VideoData {
    /// URL of the video resource.
    #[prost(string, optional, tag = "1")]
    pub url: Option<String>,
    /// Security origin from which the video resource is served.
    #[prost(message, optional, tag = "2")]
    pub security_origin: Option<SecurityOrigin>,
}

/// Destination URL and link relationship tokens carried by
/// [`ContentData::Anchor`](super::ContentData::Anchor).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct AnchorData {
    /// Destination URL of the hyperlink.
    #[prost(string, optional, tag = "1")]
    pub url: Option<String>,
    /// Link relationships between the current document and [`Self::url`] (open [`AnchorRel`] enum
    /// values).
    #[prost(enumeration = "AnchorRel", repeated, tag = "2")]
    pub rel: Vec<i32>,
}

/// HTML hyperlink `rel` token carried on [`AnchorData::rel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum AnchorRel {
    /// Unrecognized or unspecified link relation.
    Unknown = 0,
    /// `rel="noreferrer"`.
    NoReferrer = 1,
    /// `rel="noopener"`.
    NoOpener = 2,
    /// `rel="opener"`.
    Opener = 3,
    /// `rel="privacy-policy"`.
    PrivacyPolicy = 4,
    /// `rel="terms-of-service"`.
    TermsOfService = 5,
}

/// Table caption metadata carried by [`ContentData::Table`](super::ContentData::Table).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct TableData {
    /// Table name extracted from an immediate child `<caption>` element (tags 2..=4 reserved).
    #[prost(string, optional, tag = "1")]
    pub table_name: Option<String>,
}

/// Table row section classification carried by
/// [`ContentData::TableRow`](super::ContentData::TableRow).
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct TableRowData {
    /// Row section classification (wire field `type` at tag 1; open [`TableRowType`] enum).
    #[prost(enumeration = "TableRowType", optional, tag = "1")]
    pub row_type: Option<i32>,
}

/// Table row section carried on [`TableRowData::row_type`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum TableRowType {
    /// Row section unspecified or unrecognized.
    Unknown = 0,
    /// Header row (`<thead>`).
    Header = 1,
    /// Body row (`<tbody>`).
    Body = 2,
    /// Footer row (`<tfoot>`).
    Footer = 3,
}

/// Re-serialized `MathML` formula carried by [`ContentData::Math`](super::ContentData::Math).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct MathData {
    /// Stripped, XML-escaped `<math>` subtree containing only known `MathML` tags and safe
    /// attributes; unset if markup exceeds 65,536 bytes or 64 levels of nesting.
    #[prost(string, optional, tag = "1")]
    pub stripped_mathml: Option<String>,
    /// Whether computed style lays out the formula as block rather than inline.
    #[prost(bool, optional, tag = "2")]
    pub is_block: Option<bool>,
}
