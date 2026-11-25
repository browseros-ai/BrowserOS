//! Typed Rust representation of Chromium's `optimization_guide.proto.AnnotatedPageContent`
//! schema and every message, oneof, and enum reachable from it.
//!
//! All definitions mirror Chromium's
//! `components/optimization_guide/proto/features/common_quality_data.proto` (Protobuf Edition
//! 2023) with exact wire field numbers:
//!
//! ```text
//!  Edition 2023 feature                Rust / prost mapping
//!  ──────────────────────────────────  ───────────────────────────────────────────────────
//!  field_presence = EXPLICIT           Option<T> / Option<i32> for every singular field
//!  repeated_field_encoding = PACKED    Vec<T> with prost `repeated` (decodes packed + unpacked)
//!  enum_type = OPEN                    Option<i32> / Vec<i32> with `enumeration = "..."`,
//!                                      preserving unknown enum values across decode
//! ```
//!
//! A patched BrowserOS Chromium adds its half of the wire contract at tag
//! [`BROWSEROS_EXTENSION_TAG`] (1000): a `browseros` submessage on [`AnnotatedPageContent`],
//! [`ContentAttributes`], [`FrameData`] and [`RedactedFrameMetadata`], plus
//! [`RedactedFrameReason::BrowserOsTimeout`]. The `browseros` module holds those messages. Stock
//! Chromium sends none of them, and the same types decode both.

mod annotated_page_content;
mod ax_role;
mod browseros;
mod content_node;
mod form_control_data;
mod frame_data;
mod geometry;
mod interaction_info;
mod text_info;

pub use annotated_page_content::{
    AnnotatedPageContent, AnnotatedPageContentMode, AnnotatedPageContentVersion,
    AutofillInformation, FillableData, GeminiInChromePageMetadata, IframeInfo, PageInteractionInfo,
    Point, PopupWindow, ProfileInformation, ScreenshotInfo,
};
pub use ax_role::AxRole;
pub use browseros::{
    BROWSEROS_EXTENSION_TAG, BrowserOsContentAttributes, BrowserOsFrameData,
    BrowserOsPageContentData, BrowserOsRedactedFrame, BrowserOsRef, BrowserOsTriState,
    REASON_BROWSEROS_TIMEOUT,
};
pub use content_node::{
    AnnotatedRole, ContentAttributeType, ContentAttributes, ContentData, ContentNode,
    RedactionDecision,
};
pub use form_control_data::{
    CoarseAutofillFieldType, FormControlData, FormControlType, FormInfo, SelectOption,
};
pub use frame_data::{
    DocumentIdentifier, FrameData, FrameInteractionInfo, IframeData, IframeDataContent, MediaData,
    MediaDataType, MediaTranscript, PaidContentMetadata, RedactedFrameMetadata,
    RedactedFrameReason, ScriptTool, ScriptToolAnnotations, ScriptToolResult, SecurityOrigin,
    Selection,
};
pub use geometry::{BoundingRect, BoundingSize, CssPosition, Geometry};
pub use interaction_info::{
    ClickabilityReason, InteractionDisabledReason, InteractionInfo, ScrollerInfo,
};
pub use text_info::{
    AnchorData, AnchorRel, CanvasData, ImageInfo, MathData, SvgRootData, TableData, TableRowData,
    TableRowType, TextInfo, TextSize, TextStyle, VideoData,
};
