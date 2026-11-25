//! The BrowserOS half of the APC wire contract: the messages a patched BrowserOS Chromium nests
//! at tag 1000 of upstream `AnnotatedPageContent`, `ContentAttributes`, `FrameData` and
//! `RedactedFrameMetadata`.
//!
//! Chromium sends facts here (tags, raw attributes, relation targets, states, redaction lengths)
//! and Rust computes roles, names and hidden-ness from them, so snapshot and `getByRole` read the
//! same facts. The field numbers are final (`apc-contract-design` draft 2, sections 4.1 to 4.4);
//! each Chromium PR only starts writing fields that are already declared, and raises
//! [`BrowserOsPageContentData::contract_version`] by one. A browser at version N leaves later
//! fields unset, and a stock browser sends none of these messages.
//!
//! The source of truth is BrowserOS-internal's patch to
//! `components/optimization_guide/proto/features/common_quality_data.proto`. Field declaration
//! order follows field numbers, so prost re-encodes a decoded message byte for byte.

use prost::{Enumeration, Message};

/// Protobuf field tag of the `browseros` submessage on every upstream message that carries one.
/// Upstream tops out at 30, so 1000 keeps BrowserOS clear of future upstream fields.
pub const BROWSEROS_EXTENSION_TAG: u32 = 1000;

/// Raw open-enum value of [`RedactedFrameReason::BrowserOsTimeout`], for comparing against
/// [`RedactedFrameMetadata::reason`] storage.
///
/// [`RedactedFrameReason::BrowserOsTimeout`]: super::RedactedFrameReason::BrowserOsTimeout
/// [`RedactedFrameMetadata::reason`]: super::RedactedFrameMetadata::reason
pub const REASON_BROWSEROS_TIMEOUT: i32 = super::RedactedFrameReason::BrowserOsTimeout as i32;

/// Page-level contract metadata (`AnnotatedPageContent.browseros`, proto `BrowserOSPageContentData`).
///
/// Its presence is what tells a patched browser from stock Chromium. Stock APC sends `cc-number`
/// and one-time-code values in clear, so the engine must refuse to run without a
/// `contract_version` high enough for the facts it reads.
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct BrowserOsPageContentData {
    /// Version of the BrowserOS contract the browser writes; each Chromium PR raises it by one.
    /// Version 1 is the privacy gate (redacted values and `redacted_value_length`).
    #[prost(uint32, optional, tag = "1")]
    pub contract_version: Option<u32>,
    /// The browser's subframe capture deadline in milliseconds, so a partial snapshot can state
    /// the real deadline. Version 1 reports stock Chromium's 10 s; version 2 lowers it.
    #[prost(uint32, optional, tag = "2")]
    pub subframe_timeout_ms: Option<u32>,
}

/// Per-node facts (`ContentAttributes.browseros`, proto `BrowserOSContentAttributes`).
///
/// Nothing is written when no fact applies, so most text nodes carry no submessage at all. Each
/// field's doc names the contract version that starts writing it; below that version the field
/// is absent, not false.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct BrowserOsContentAttributes {
    /// Lowercase local name of a page-authored element other than `div` and `span`. Absent on an
    /// element means `div` or `span`. Version 3.
    #[prost(string, optional, tag = "1")]
    pub tag: Option<String>,
    /// Raw `role` attribute, unparsed. Version 3.
    #[prost(string, optional, tag = "2")]
    pub role: Option<String>,
    /// Raw `href` of an `a` or `area`, empty kept. Version 3.
    #[prost(string, optional, tag = "3")]
    pub href: Option<String>,
    /// Raw `alt` of an `img`, `area` or image input, empty kept. Version 3.
    #[prost(string, optional, tag = "4")]
    pub alt: Option<String>,
    /// Raw `scope` of a `th`. Version 3.
    #[prost(string, optional, tag = "5")]
    pub scope: Option<String>,
    /// Set on a `select` that renders as a list box rather than a menu list. Version 3.
    #[prost(bool, optional, tag = "6")]
    pub list_box: Option<bool>,
    /// Set on an `input` whose `list` attribute resolves to a datalist. Version 3.
    #[prost(bool, optional, tag = "7")]
    pub has_datalist: Option<bool>,
    /// `aria-hidden="true"` on the node or an ancestor in its document; on text nodes too.
    /// Version 3.
    #[prost(bool, optional, tag = "8")]
    pub aria_hidden: Option<bool>,
    /// A block boundary or `<br>` lies between this node and the previous text in document
    /// order, so names and snapshot text put a space here. Version 3.
    #[prost(bool, optional, tag = "9")]
    pub break_before: Option<bool>,
    /// DOM text of a text node whose rendered (`text-transform`ed) text differs from it.
    /// Version 3.
    #[prost(string, optional, tag = "10")]
    pub dom_text: Option<String>,
    /// Raw `aria-label`, empty kept. Version 4.
    #[prost(string, optional, tag = "11")]
    pub aria_label: Option<String>,
    /// Resolved `aria-labelledby` targets, in attribute order. Version 4.
    #[prost(message, repeated, tag = "12")]
    pub labelledby: Vec<BrowserOsRef>,
    /// Resolved `aria-describedby` targets, in attribute order. Version 4.
    #[prost(message, repeated, tag = "13")]
    pub describedby: Vec<BrowserOsRef>,
    /// Resolved `aria-errormessage` targets, in attribute order. Version 4.
    #[prost(message, repeated, tag = "14")]
    pub errormessage: Vec<BrowserOsRef>,
    /// Raw `title`, set only when not blank. Version 4.
    #[prost(string, optional, tag = "15")]
    pub title: Option<String>,
    /// Raw `aria-description`. Version 4.
    #[prost(string, optional, tag = "16")]
    pub aria_description: Option<String>,
    /// Checked state: a native input's `indeterminate` or checkedness, else `aria-checked`
    /// (open [`BrowserOsTriState`] enum). Version 4.
    #[prost(enumeration = "BrowserOsTriState", optional, tag = "17")]
    pub checked: Option<i32>,
    /// `aria-pressed`, when present (open [`BrowserOsTriState`] enum). Version 4.
    #[prost(enumeration = "BrowserOsTriState", optional, tag = "18")]
    pub pressed: Option<i32>,
    /// `aria-expanded`, when present; unset and false differ (open [`BrowserOsTriState`] enum).
    /// Version 4.
    #[prost(enumeration = "BrowserOsTriState", optional, tag = "19")]
    pub expanded: Option<i32>,
    /// `aria-selected="true"`, or a selected native `<option>` node. Version 4.
    #[prost(bool, optional, tag = "20")]
    pub selected: Option<bool>,
    /// `aria-level`, when it is an integer of at least 1. Version 4.
    #[prost(uint32, optional, tag = "21")]
    pub level: Option<u32>,
    /// Blink's `IsDisabledFormControl()`: a disabled control, or an option disabled itself or
    /// through its group. Also set on a disabled `fieldset`, which Playwright does not treat as
    /// disabled. Version 4.
    #[prost(bool, optional, tag = "22")]
    pub disabled: Option<bool>,
    /// `aria-disabled="true"` on the node or an ancestor. Version 4.
    #[prost(bool, optional, tag = "23")]
    pub aria_disabled: Option<bool>,
    /// Raw `aria-invalid`. Version 4.
    #[prost(string, optional, tag = "24")]
    pub aria_invalid: Option<String>,
    // Numbers 25 and 26 (draft 1's `opacity` and `aria_busy`) stay free so they can return under
    // their old numbers if covers ever move out of the page.
    /// Length in UTF-16 code units of a text control's withheld value, so the snapshot can print
    /// one bullet per character. Set beside every redacted decision, password included.
    /// Version 1.
    #[prost(uint32, optional, tag = "27")]
    pub redacted_value_length: Option<u32>,
    /// The element's own computed `visibility` is not visible; it is in the tree only for its
    /// visible descendants. Element nodes only. Version 3.
    #[prost(bool, optional, tag = "28")]
    pub visibility_hidden: Option<bool>,
    /// The element's computed (not used) `pointer-events` is `none`, so inertness alone never
    /// sets it. Element nodes only. Version 4.
    #[prost(bool, optional, tag = "29")]
    pub pointer_events_none: Option<bool>,
    /// The element's computed style is inert: the `inert` attribute, a modal dialog elsewhere, or
    /// CSS. Element nodes only. Version 4.
    #[prost(bool, optional, tag = "30")]
    pub inert: Option<bool>,
    /// The first visible element inside a closed shadow root the page made, one per root, which
    /// the page probe uses to enter that root. Version 3.
    #[prost(bool, optional, tag = "31")]
    pub closed_root_seed: Option<bool>,
    /// The node is inside a user-agent shadow root and carries no other field: it gets no role,
    /// ref or text, while its page-authored descendants are read as usual. Version 3.
    #[prost(bool, optional, tag = "32")]
    pub ua_shadow: Option<bool>,
    /// A node made for a `display: contents` element; it has no geometry. Version 5.
    #[prost(bool, optional, tag = "33")]
    pub display_contents: Option<bool>,
}

/// Target of an ARIA relation (proto `BrowserOSRef`).
///
/// A rendered target is a node in the same document's tree (the retention rule guarantees it),
/// named by its id so Rust computes its text with its own rules, including masking. A target
/// that cannot be a node carries its text instead.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct BrowserOsRef {
    /// [`ContentAttributes::common_ancestor_dom_node_id`] of the target, in the same document.
    ///
    /// [`ContentAttributes::common_ancestor_dom_node_id`]: super::ContentAttributes::common_ancestor_dom_node_id
    #[prost(int32, optional, tag = "1")]
    pub dom_node_id: Option<i32>,
    /// `textContent` of a target with no layout object or no visible box, with every text
    /// control's content left out.
    #[prost(string, optional, tag = "2")]
    pub unrendered_text: Option<String>,
}

/// ARIA-style tri-state where unset differs from false (proto `BrowserOSTriState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum BrowserOsTriState {
    /// No state given.
    Unset = 0,
    /// Explicitly false, or an ARIA value other than `true` and `mixed`.
    False = 1,
    /// True.
    True = 2,
    /// Mixed: `aria-checked="mixed"` or a native input's `indeterminate`.
    Mixed = 3,
}

/// Per-frame metadata (`FrameData.browseros`, proto `BrowserOSFrameData`).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct BrowserOsFrameData {
    /// The frame's `DevTools` frame token, which is its CDP frame id. Node ids are per renderer,
    /// so this maps each document to the CDP session that can resolve its refs. Version 2.
    #[prost(string, optional, tag = "1")]
    pub devtools_frame_token: Option<String>,
    // Numbers 2 and 3 are held for `dom_tree_version` and `style_version` (patch P10).
}

/// Metadata for an iframe left out of the capture (`RedactedFrameMetadata.browseros`, proto
/// `BrowserOSRedactedFrame`).
///
/// A redacted frame has no `FrameData`, so this is the only place its token can travel.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct BrowserOsRedactedFrame {
    /// The left-out frame's `DevTools` frame token, so a `# partial:` line can name the frame that
    /// missed its deadline. Version 2.
    #[prost(string, optional, tag = "1")]
    pub devtools_frame_token: Option<String>,
}
