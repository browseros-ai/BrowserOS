use prost::{Enumeration, Message};

/// Geometry of a content node in the page coordinate system.
///
/// Populated for all nodes in `AnnotatedPageContentMode::ActionableElements`, and in default mode
/// only for the accessibility-focused node. When `visible_bounding_box` is empty on an offscreen
/// node inside an iframe, `outer_bounding_box` may be inaccurate upstream (`crbug.com/384996050`).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct Geometry {
    /// Bounding box mapped to the viewport ignoring ancestor and viewport clips.
    ///
    /// An empty box implies the node is not hit-testable even after scrolling onscreen; a
    /// negative `y` origin indicates content starting above the viewport.
    #[prost(message, optional, tag = "1")]
    pub outer_bounding_box: Option<BoundingRect>,
    /// Bounding box relative to the viewport including all ancestor clipping.
    ///
    /// Represents the element bounds currently visible and hit-testable in the DOM.
    #[prost(message, optional, tag = "2")]
    pub visible_bounding_box: Option<BoundingRect>,
    /// Deprecated upstream (`crbug.com/490195093`): whether the node has fixed or sticky
    /// positioning. Prefer [`Self::css_position`].
    #[prost(bool, optional, tag = "3")]
    pub is_fixed_or_sticky_position: Option<bool>,
    /// Hit-testable visible fragments relative to the viewport origin for nodes split across
    /// multiple lines, columns, or pages.
    #[prost(message, repeated, tag = "6")]
    pub fragment_visible_bounding_boxes: Vec<BoundingRect>,
    /// Computed CSS `position` property for the node (open [`CssPosition`] enum).
    #[prost(enumeration = "CssPosition", optional, tag = "8")]
    pub css_position: Option<i32>,
}

/// Axis-aligned integer bounding rectangle in viewport or screenshot coordinates.
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct BoundingRect {
    /// Left coordinate of the rectangle.
    #[prost(int32, optional, tag = "1")]
    pub x: Option<i32>,
    /// Top coordinate of the rectangle.
    #[prost(int32, optional, tag = "2")]
    pub y: Option<i32>,
    /// Width of the rectangle.
    #[prost(int32, optional, tag = "3")]
    pub width: Option<i32>,
    /// Height of the rectangle.
    #[prost(int32, optional, tag = "4")]
    pub height: Option<i32>,
    /// Whether this box uses the screenshot coordinate space defined by
    /// [`ScreenshotInfo::screenshot_size`](super::ScreenshotInfo::screenshot_size).
    #[prost(bool, optional, tag = "5")]
    pub is_screenshot_relative: Option<bool>,
}

/// Two-dimensional integer extent used for scroll layouts and captured screenshot dimensions.
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct BoundingSize {
    /// Horizontal extent in pixels.
    #[prost(int32, optional, tag = "1")]
    pub width: Option<i32>,
    /// Vertical extent in pixels.
    #[prost(int32, optional, tag = "2")]
    pub height: Option<i32>,
}

/// Computed CSS `position` property values carried on [`Geometry::css_position`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum CssPosition {
    /// Default `static` positioning.
    StaticDefault = 0,
    /// CSS `position: relative`.
    Relative = 1,
    /// CSS `position: absolute`.
    Absolute = 2,
    /// CSS `position: fixed`.
    Fixed = 3,
    /// CSS `position: sticky`.
    Sticky = 4,
}
