use prost::{Enumeration, Message};

use super::content_node::ContentNode;
use super::frame_data::{DocumentIdentifier, FrameData, SecurityOrigin};
use super::geometry::{BoundingRect, BoundingSize};

/// Top-level Chromium `AnnotatedPageContent` message returned by `Page.getAnnotatedPageContent`.
///
/// Carries the sparse semantic [`ContentNode`] tree across the main frame and embedded iframes,
/// along with main-frame metadata, page-level focus/mouse state, viewport dimensions, and any
/// open internal popup widget.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct AnnotatedPageContent {
    /// Schema version of this snapshot (open [`AnnotatedPageContentVersion`] enum).
    #[prost(enumeration = "AnnotatedPageContentVersion", optional, tag = "1")]
    pub version: Option<i32>,
    /// Root node of the semantic content tree, including embedded iframe subtrees.
    #[prost(message, optional, tag = "2")]
    pub root_node: Option<ContentNode>,
    /// Frame metadata for the main frame of the page.
    #[prost(message, optional, tag = "3")]
    pub main_frame_data: Option<FrameData>,
    /// Page-scoped interaction state such as mouse position and focused frame/node.
    #[prost(message, optional, tag = "4")]
    pub page_interaction_info: Option<PageInteractionInfo>,
    /// Viewport rectangle in device-independent captured pixels for coordinate-based actions.
    #[prost(message, optional, tag = "5")]
    pub viewport_geometry: Option<BoundingRect>,
    /// Extraction mode that produced this snapshot (open [`AnnotatedPageContentMode`] enum).
    #[prost(enumeration = "AnnotatedPageContentMode", optional, tag = "6")]
    pub mode: Option<i32>,
    /// Browsing-session-unique identifier of the tab from which the page content was extracted.
    #[prost(int32, optional, tag = "7")]
    pub tab_id: Option<i32>,
    /// Details for an open internal popup widget (such as `<select>` or `<input type="color">`),
    /// which paints above the main frame's document.
    #[prost(message, optional, tag = "8")]
    pub popup_window: Option<PopupWindow>,
    /// Profile-level metadata (such as available autofill categories) not scoped to the page.
    #[prost(message, optional, tag = "9")]
    pub profile_information: Option<ProfileInformation>,
    /// Deprecated upstream (`crbug.com/484011242`): Gemini in Chrome screenshot capture metadata.
    #[prost(message, optional, tag = "10")]
    pub gemini_in_chrome_page_metadata: Option<GeminiInChromePageMetadata>,
    // Field 1000 (`BROWSEROS_EXTENSION_TAG`): reserved for BrowserOS page-level extension
    // submessage per `apc-contract-design`.
}

/// Wire format version reported on [`AnnotatedPageContent::version`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum AnnotatedPageContentVersion {
    /// Version unspecified or unrecognized.
    Unknown = 0,
    /// Schema version `1.0`.
    Version10 = 1,
    /// Deprecated upstream (`crbug.com/490195093`): legacy actionable-elements version `1.0`.
    /// Prefer [`AnnotatedPageContent::mode`].
    OnlyActionableElements10 = 2,
}

impl AnnotatedPageContentVersion {
    /// Highest valid schema version, mirroring Chromium's `allow_alias` value
    /// `ANNOTATED_PAGE_CONTENT_VERSION_MAX_VALID = 2`.
    pub const MAX_VALID: Self = Self::OnlyActionableElements10;
}

/// Extraction mode configured when generating an [`AnnotatedPageContent`] snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum AnnotatedPageContentMode {
    /// Default extraction mode containing semantic content and scroller info.
    Default = 0,
    /// Superset of default mode adding geometry, ARIA roles, labels, and interaction signals for
    /// actionable elements.
    ActionableElements = 1,
}

/// Top-level interaction state unique across the entire page, such as mouse coordinates and
/// active focus.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct PageInteractionInfo {
    /// Content node ID currently focused in the main frame.
    #[prost(int32, optional, tag = "1")]
    pub focused_node_id: Option<i32>,
    /// Content node ID currently focused by a screen reader in the main frame.
    #[prost(int32, optional, tag = "2")]
    pub accessibility_focused_node_id: Option<i32>,
    /// Last known mouse pointer position in the page coordinate system.
    #[prost(message, optional, tag = "3")]
    pub mouse_position: Option<Point>,
    /// Document identifier of the frame (main frame or iframe) that currently holds focus.
    #[prost(message, optional, tag = "4")]
    pub focused_frame: Option<DocumentIdentifier>,
    /// Document identifier of the frame that currently holds accessibility focus; an empty token
    /// means no frame has accessibility focus.
    #[prost(message, optional, tag = "5")]
    pub accessibility_focused_frame: Option<DocumentIdentifier>,
}

/// Two-dimensional integer point in page coordinates.
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct Point {
    /// Horizontal coordinate.
    #[prost(int32, optional, tag = "1")]
    pub x: Option<i32>,
    /// Vertical coordinate.
    #[prost(int32, optional, tag = "2")]
    pub y: Option<i32>,
}

/// Internal browser popup widget currently open on top of the page (for example, a native
/// `<select>` dropdown or color picker).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct PopupWindow {
    /// Root content node of the popup widget's document.
    #[prost(message, optional, tag = "1")]
    pub root_node: Option<ContentNode>,
    /// Document identifier of the frame that opened this popup window.
    #[prost(message, optional, tag = "3")]
    pub opener_document_id: Option<DocumentIdentifier>,
    /// DOM node ID of the element inside [`Self::opener_document_id`] that opened this popup.
    #[prost(int32, optional, tag = "4")]
    pub opener_common_ancestor_dom_node_id: Option<i32>,
    /// Popup bounds relative to the viewport, clipped only by the viewport itself.
    #[prost(message, optional, tag = "5")]
    pub visible_bounding_box: Option<BoundingRect>,
}

/// Browser profile metadata carried alongside an [`AnnotatedPageContent`] snapshot.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct ProfileInformation {
    /// Coarse availability of saved autofill data on the client profile.
    #[prost(message, optional, tag = "1")]
    pub autofill_information: Option<AutofillInformation>,
}

/// Coarse summary of autofill data categories stored in the user's profile.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct AutofillInformation {
    /// Categories of autofill data present on file (open [`FillableData`] enum values).
    #[prost(enumeration = "FillableData", repeated, tag = "1")]
    pub fillable_data: Vec<i32>,
}

/// Coarse category of client-side autofill data reported in
/// [`AutofillInformation::fillable_data`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum FillableData {
    /// Category not specified.
    Unspecified = 0,
    /// The profile has at least one saved address.
    Address = 1,
    /// The profile has at least one saved credit card (tag 6 aligns with Chromium's
    /// `FormFillingRequest.RequestedData`).
    CreditCard = 6,
}

/// Deprecated upstream page metadata attached by Gemini in Chrome captures.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct GeminiInChromePageMetadata {
    /// Deprecated upstream: metadata describing an associated page screenshot.
    #[prost(message, optional, tag = "1")]
    pub screenshot_info: Option<ScreenshotInfo>,
}

/// Metadata describing an out-of-band screenshot associated with a page snapshot.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct ScreenshotInfo {
    /// Whether the captured screenshot contains an active text selection region.
    #[prost(bool, optional, tag = "1")]
    pub has_selection_region_in_screenshot: Option<bool>,
    /// Positions and origins of embedded iframes visible in the screenshot.
    #[prost(message, repeated, tag = "2")]
    pub iframe_info: Vec<IframeInfo>,
    /// Original pixel dimensions of the captured screenshot.
    #[prost(message, optional, tag = "3")]
    pub screenshot_size: Option<BoundingSize>,
}

/// Position and origin metadata for an iframe appearing inside [`ScreenshotInfo::iframe_info`].
#[derive(Clone, PartialEq, Eq, Message)]
pub struct IframeInfo {
    /// Security origin of the iframe document.
    #[prost(message, optional, tag = "1")]
    pub security_origin: Option<SecurityOrigin>,
    /// URL of the iframe document.
    #[prost(string, optional, tag = "2")]
    pub url: Option<String>,
    /// Iframe bounding box in screenshot coordinates, including all ancestor clipping.
    #[prost(message, optional, tag = "3")]
    pub bounding_box: Option<BoundingRect>,
}
