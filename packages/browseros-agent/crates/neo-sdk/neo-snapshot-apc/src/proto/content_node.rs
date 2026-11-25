use std::mem;

use prost::{Enumeration, Message, Oneof};

use super::ax_role::AxRole;
use super::browseros::BrowserOsContentAttributes;
use super::form_control_data::{FormControlData, FormInfo};
use super::frame_data::IframeData;
use super::geometry::Geometry;
use super::interaction_info::InteractionInfo;
use super::text_info::{
    AnchorData, CanvasData, ImageInfo, MathData, SvgRootData, TableData, TableRowData, TextInfo,
    VideoData,
};

/// Single node in Chromium's sparse semantic DOM tree (`AnnotatedPageContent.root_node`).
///
/// Roughly 1:1 with semantically meaningful DOM nodes across the main document and embedded
/// iframes, excluding pure layout/styling wrappers. Text and image nodes are leaf nodes.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct ContentNode {
    /// Ordered child content nodes; empty for leaf text and image nodes.
    #[prost(message, repeated, tag = "1")]
    pub children_nodes: Vec<ContentNode>,
    /// Semantic type, payload, DOM node ID, geometry, ARIA role, and interaction info for this
    /// node.
    #[prost(message, optional, tag = "2")]
    pub content_attributes: Option<ContentAttributes>,
}

impl Drop for ContentNode {
    /// Tears down arbitrarily deep `ContentNode` subtrees iteratively so dropping a deeply nested
    /// page cannot overflow the call stack.
    fn drop(&mut self) {
        let mut stack = mem::take(&mut self.children_nodes);
        while let Some(mut node) = stack.pop() {
            stack.extend(mem::take(&mut node.children_nodes));
        }
    }
}

/// Attributes and type-specific payload attached to a [`ContentNode`].
#[derive(Clone, PartialEq, Eq, Message)]
pub struct ContentAttributes {
    /// Renderer-assigned DOM node ID within the owning document's lifetime; unset for synthetic
    /// layout nodes such as CSS pseudo-elements.
    #[prost(int32, optional, tag = "2")]
    pub common_ancestor_dom_node_id: Option<i32>,
    /// Structural category of this node (open [`ContentAttributeType`] enum), indicating which
    /// [`ContentData`] variant is populated.
    #[prost(enumeration = "ContentAttributeType", optional, tag = "3")]
    pub attribute_type: Option<i32>,
    /// Viewport and clipped layout geometry for [`Self::common_ancestor_dom_node_id`].
    #[prost(message, optional, tag = "4")]
    pub geometry: Option<Geometry>,
    /// Type-specific payload matching [`Self::attribute_type`].
    #[prost(
        oneof = "ContentData",
        tags = "8, 9, 10, 11, 13, 14, 19, 20, 23, 24, 27, 30"
    )]
    pub content_data: Option<ContentData>,
    /// Semantic landmark and content-visibility annotations (open [`AnnotatedRole`] enum values).
    #[prost(enumeration = "AnnotatedRole", repeated, tag = "15")]
    pub annotated_roles: Vec<i32>,
    /// Accessible description from non-rendered sources such as `aria-label` or `aria-labelledby`.
    #[prost(string, optional, tag = "16")]
    pub label: Option<String>,
    /// Scrollability, focusability, clickability, and disabled hints for this node.
    #[prost(message, optional, tag = "21")]
    pub interaction_info: Option<InteractionInfo>,
    /// Target input DOM node ID when this node is a `<label for="...">` element.
    #[prost(int32, optional, tag = "25")]
    pub label_for_dom_node_id: Option<i32>,
    /// Accessibility role derived from the element's ARIA role (open [`AxRole`] enum).
    #[prost(enumeration = "AxRole", optional, tag = "26")]
    pub aria_role: Option<i32>,
    /// Whether Chromium classified this content node as ad-related.
    #[prost(bool, optional, tag = "28")]
    pub is_ad_related: Option<bool>,
    /// Sensitive-field redaction classification for this node (open [`RedactionDecision`] enum).
    #[prost(enumeration = "RedactionDecision", optional, tag = "29")]
    pub redaction_decision: Option<i32>,
    /// BrowserOS per-node facts, boxed because most nodes carry none.
    #[prost(message, optional, boxed, tag = "1000")]
    pub browseros: Option<Box<BrowserOsContentAttributes>>,
}

/// Type-specific payload carried by [`ContentAttributes::content_data`].
///
/// [`Self::FormControl`] and [`Self::Iframe`] are boxed to keep `ContentNode` compact on the
/// stack and in `Vec<ContentNode>` allocations.
#[derive(Clone, PartialEq, Eq, Oneof)]
pub enum ContentData {
    /// Table metadata; populated when `attribute_type` is [`ContentAttributeType::Table`].
    #[prost(message, tag = "8")]
    Table(TableData),
    /// Embedded iframe frame metadata or redaction reason; populated when `attribute_type` is
    /// [`ContentAttributeType::Iframe`].
    #[prost(message, boxed, tag = "9")]
    Iframe(Box<IframeData>),
    /// Leaf text content and style; populated when `attribute_type` is
    /// [`ContentAttributeType::Text`].
    #[prost(message, tag = "10")]
    Text(TextInfo),
    /// Image caption, origin, and source URL; populated when `attribute_type` is
    /// [`ContentAttributeType::Image`].
    #[prost(message, tag = "11")]
    Image(ImageInfo),
    /// Table row section classification; populated when `attribute_type` is
    /// [`ContentAttributeType::TableRow`].
    #[prost(message, tag = "13")]
    TableRow(TableRowData),
    /// Hyperlink destination URL and `rel` tokens; populated when `attribute_type` is
    /// [`ContentAttributeType::Anchor`].
    #[prost(message, tag = "14")]
    Anchor(AnchorData),
    /// HTML `<form>` name and resolved action URL; populated when `attribute_type` is
    /// [`ContentAttributeType::Form`].
    #[prost(message, tag = "19")]
    Form(FormInfo),
    /// Interactive form control state, options, and autofill hints; populated when
    /// `attribute_type` is [`ContentAttributeType::FormControl`].
    #[prost(message, boxed, tag = "20")]
    FormControl(Box<FormControlData>),
    /// Inner text extracted from an SVG subtree; populated when `attribute_type` is
    /// [`ContentAttributeType::SvgRoot`].
    #[prost(message, tag = "23")]
    SvgRoot(SvgRootData),
    /// Layout dimensions of a `<canvas>` element; populated when `attribute_type` is
    /// [`ContentAttributeType::Canvas`].
    #[prost(message, tag = "24")]
    Canvas(CanvasData),
    /// Source URL and origin of a `<video>` element; populated when `attribute_type` is
    /// [`ContentAttributeType::Video`].
    #[prost(message, tag = "27")]
    Video(VideoData),
    /// Re-serialized `MathML` formula and layout mode; populated when `attribute_type` is
    /// [`ContentAttributeType::Math`].
    #[prost(message, tag = "30")]
    Math(MathData),
}

/// Structural node kind carried on [`ContentAttributes::attribute_type`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum ContentAttributeType {
    /// Invalid or unrecognized node kind.
    Unknown = 0,
    /// Document root node.
    Root = 1,
    /// Generic semantic container node.
    Container = 2,
    /// Embedded iframe container node.
    Iframe = 3,
    /// Paragraph text block.
    Paragraph = 4,
    /// Section heading.
    Heading = 5,
    /// Ordered list (`<ol>`).
    OrderedList = 6,
    /// Unordered list (`<ul>`).
    UnorderedList = 7,
    /// Form container (`<form>`).
    Form = 8,
    /// Image leaf node.
    Image = 9,
    /// Table container (`<table>`).
    Table = 10,
    /// Table cell (`<td>` or `<th>`).
    TableCell = 11,
    /// Leaf text run.
    Text = 20,
    /// Table row (`<tr>`).
    TableRow = 21,
    /// Anchor hyperlink (`<a>`).
    Anchor = 22,
    /// List item (`<li>`).
    ListItem = 23,
    /// Interactive form control (`<input>`, `<select>`, `<textarea>`, `<button>`, and so on).
    FormControl = 24,
    /// Root SVG element (`<svg>`).
    SvgRoot = 25,
    /// Canvas element (`<canvas>`).
    Canvas = 26,
    /// Video element (`<video>`).
    Video = 27,
    /// Modal dialog container (`<dialog>` opened modally or `aria-modal="true"`).
    DialogModal = 28,
    /// Modeless dialog container.
    DialogModeless = 29,
    /// `MathML` formula container (`<math>`).
    Math = 30,
}

/// Semantic landmark or visibility role carried on [`ContentAttributes::annotated_roles`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum AnnotatedRole {
    /// Role unspecified or unrecognized.
    Unknown = 0,
    /// Header landmark (`<header>` or `role="banner"`).
    Header = 5,
    /// Navigation landmark (`<nav>` or `role="navigation"`).
    Nav = 6,
    /// Search landmark (`<search>` or `role="search"`).
    Search = 7,
    /// Main content landmark (`<main>` or `role="main"`).
    Main = 8,
    /// Article landmark (`<article>`).
    Article = 9,
    /// Section landmark (`<section>`).
    Section = 10,
    /// Complementary aside landmark (`<aside>`).
    Aside = 11,
    /// Footer landmark (`<footer>` or `role="contentinfo"`).
    Footer = 12,
    /// Subtree hidden via `hidden="until-found"` but still searchable in-page.
    ContentHidden = 13,
    /// Subtree declared non-free (`isAccessibleForFree=false`) via paywalled structured data.
    PaidContent = 14,
}

/// Sensitive-content redaction classification carried on [`ContentAttributes::redaction_decision`]
/// (and legacy [`FormControlData::redaction_decision`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum RedactionDecision {
    /// Node does not require redaction.
    NoRedactionNecessary = 0,
    /// Password input that is currently empty and therefore unredacted.
    UnredactedEmptyPassword = 1,
    /// Password input (currently or previously a password field) whose value was redacted.
    RedactedHasBeenPassword = 2,
    /// Payment field that is currently empty and therefore unredacted.
    UnredactedEmptyPaymentField = 3,
    /// Non-empty sensitive payment field recognized by autofill and redacted.
    RedactedIsSensitivePaymentField = 4,
    /// Empty input heuristically classified as password-like (for example via
    /// `-webkit-text-security`) and left unredacted.
    UnredactedEmptyCustomPassword = 5,
    /// Password-like input masked via CSS (such as `-webkit-text-security`) and redacted.
    RedactedCustomPasswordCss = 6,
    /// Password-like input masked in JavaScript (such as bullet characters) and redacted.
    RedactedCustomPasswordJs = 7,
    /// Non-empty OTP field recognized by autofill and redacted.
    RedactedIsOtp = 8,
    /// Empty OTP field recognized by autofill and left unredacted.
    UnredactedEmptyOtpField = 9,
}
