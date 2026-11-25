use prost::{Enumeration, Message};

use super::geometry::{BoundingRect, BoundingSize};

/// Interaction signals attached to a [`ContentAttributes`](super::ContentAttributes) node.
///
/// [`Self::scroller_info`] is populated in default APC mode; all other fields are populated in
/// `AnnotatedPageContentMode::ActionableElements`.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct InteractionInfo {
    /// Deprecated upstream (`crbug.com/490195093`): whether the content node is selectable.
    #[prost(bool, optional, tag = "3")]
    pub is_selectable: Option<bool>,
    /// Deprecated upstream (`crbug.com/490195093`): whether text in this node can be edited.
    #[prost(bool, optional, tag = "4")]
    pub is_editable: Option<bool>,
    /// Deprecated upstream (`crbug.com/490195093`): whether CSS `resize` allows horizontal resize.
    #[prost(bool, optional, tag = "5")]
    pub can_resize_horizontal: Option<bool>,
    /// Deprecated upstream (`crbug.com/490195093`): whether CSS `resize` allows vertical resize.
    #[prost(bool, optional, tag = "6")]
    pub can_resize_vertical: Option<bool>,
    /// Whether the node can receive keyboard focus, even if excluded from sequential tab order.
    #[prost(bool, optional, tag = "7")]
    pub is_focusable: Option<bool>,
    /// Deprecated upstream (`crbug.com/490195093`): whether the content node is draggable.
    #[prost(bool, optional, tag = "9")]
    pub is_draggable: Option<bool>,
    /// Deprecated upstream (`crbug.com/490195093`): whether the node is clickable.
    #[prost(bool, optional, tag = "10")]
    pub is_clickable: Option<bool>,
    /// Scroll container dimensions and user-scrollability flags when this node scrolls.
    #[prost(message, optional, tag = "12")]
    pub scroller_info: Option<ScrollerInfo>,
    /// Paint z-order relative to sibling nodes in the same document; set only for nodes in the
    /// viewport. Higher values paint in front of lower values.
    #[prost(int32, optional, tag = "13")]
    pub document_scoped_z_order: Option<i32>,
    /// Deprecated upstream (`crbug.com/490195093`): legacy clickability reasons. Prefer
    /// [`Self::clickability_reasons`].
    #[prost(enumeration = "ClickabilityReason", repeated, tag = "14")]
    pub debug_clickability_reasons: Vec<i32>,
    /// Signals indicating why the node is actionable via click or hover (open
    /// [`ClickabilityReason`] enum values).
    #[prost(enumeration = "ClickabilityReason", repeated, tag = "15")]
    pub clickability_reasons: Vec<i32>,
    /// Whether the node is in a strong disabled state (such as native `<button disabled>`) where
    /// click actuation is rejected.
    #[prost(bool, optional, tag = "16")]
    pub is_disabled: Option<bool>,
    /// Strong and advisory hints explaining why interaction is disabled (open
    /// [`InteractionDisabledReason`] enum values).
    #[prost(enumeration = "InteractionDisabledReason", repeated, tag = "17")]
    pub interaction_disabled_reasons: Vec<i32>,
    /// Whether the node is focusable with a non-negative computed tab index in the tab cycle.
    #[prost(bool, optional, tag = "18")]
    pub is_tabbable: Option<bool>,
    /// Whether the element declares `aria-activedescendant` to identify its active descendant.
    #[prost(bool, optional, tag = "19")]
    pub has_aria_activedescendant: Option<bool>,
    /// DOM node IDs of secondary-action targets inside a container widget (such as a close button
    /// on a tab or secondary button in a split button).
    #[prost(int32, repeated, tag = "20")]
    pub aria_action_target_node_ids: Vec<i32>,
}

/// Physical-coordinate layout and viewport metrics for a scrollable node, excluding non-overlay
/// scrollbars.
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct ScrollerInfo {
    /// Complete scrollable layout dimensions reachable via scroll operations.
    #[prost(message, optional, tag = "1")]
    pub scrolling_bounds: Option<BoundingSize>,
    /// Currently visible subset relative to the origin of [`Self::scrolling_bounds`].
    #[prost(message, optional, tag = "2")]
    pub visible_area: Option<BoundingRect>,
    /// Whether the user can scroll this container horizontally.
    #[prost(bool, optional, tag = "3")]
    pub user_scrollable_horizontal: Option<bool>,
    /// Whether the user can scroll this container vertically.
    #[prost(bool, optional, tag = "4")]
    pub user_scrollable_vertical: Option<bool>,
}

/// Signal indicating why Chromium considers a content node clickable or hover-actionable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum ClickabilityReason {
    /// The node is a native form control.
    ClickableControl = 0,
    /// The node has a DOM `click` event handler.
    ClickHandler = 1,
    /// Deprecated upstream (`crbug.com/490195093`): generic mouse event handlers. Replaced by
    /// [`Self::MouseClick`] and [`Self::MouseHover`].
    MouseEvents = 2,
    /// The node has keyboard event handlers (`keydown`, `keypress`, or `keyup`).
    KeyEvents = 3,
    /// The node's text content can be edited.
    Editable = 4,
    /// The node uses `cursor: pointer` to signal interactivity.
    CursorPointer = 5,
    /// The node carries a clickable ARIA role.
    AriaRole = 6,
    /// The node declares `aria-haspopup`, indicating activation opens a popup.
    AriaHasPopup = 7,
    /// The node declares `aria-expanded="true"`.
    AriaExpandedTrue = 8,
    /// The node declares `aria-expanded="false"`.
    AriaExpandedFalse = 9,
    /// The node specifies a valid `tabIndex` attribute.
    TabIndex = 10,
    /// The node declares `autocomplete` or `aria-autocomplete`.
    Autocomplete = 11,
    /// The node has mouse click-phase handlers (`mouseup` or `mousedown`).
    MouseClick = 12,
    /// Deprecated upstream (`crbug.com/576774954`): hover listeners on Chrome `<= 157`. Replaced
    /// by [`Self::MouseEnter`] and [`Self::MouseOver`].
    MouseHover = 13,
    /// A CSS `:hover` pseudo-class rule matches this node.
    HoverPseudoClass = 14,
    /// The node declares `aria-checked` or `aria-pressed`.
    AriaToggle = 15,
    /// The node declares `aria-selected`.
    AriaSelectable = 16,
    /// The node has an explicit non-bubbling `mouseenter` listener.
    MouseEnter = 17,
    /// The node has an explicit bubbling `mouseover` listener.
    MouseOver = 18,
}

/// Strong or advisory hint explaining why interaction with a content node is disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum InteractionDisabledReason {
    /// Reason not specified.
    Unspecified = 0,
    /// Disabled by the HTML `disabled` attribute.
    Disabled = 1,
    /// Disabled by `aria-disabled` on the node or an ancestor.
    AriaDisabled = 2,
    /// Styled with `cursor: not-allowed`.
    CursorNotAllowed = 3,
    /// Hidden from the accessibility tree via `aria-hidden="true"` on the node or an ancestor.
    AriaHidden = 4,
    /// Assigned an ARIA role of `presentation` or `none`.
    AriaRolePresentational = 5,
    /// Styled with `pointer-events: none` or marked `inert` on the node or an ancestor.
    PointerEventsNone = 6,
    /// Has no visible pixels and cannot be scrolled into view due to clipping or off-bounds layout.
    VisuallyUnreachable = 7,
}
