//! What a screen of a given size gets (decision 20): zuzu's `viewportFor`, with its desktop
//! changed to no emulation at all. Pure, and a function of the screen's size alone, so every
//! active fit agrees on the mode and the window.
//!
//! | Screen | Window | Viewport |
//! |---|---|---|
//! | a side under 100 | unchanged | unchanged: no layout fits it |
//! | 600 wide or less | fullscreen | mobile at the screen's size (DPR 2, touch), at most 4096 tall |
//! | wider | maximized | the default, with no emulation |

use crate::ScreenSize;
use crate::browser_tabs::WindowState;
use crate::limits::{MAX_SIDE, MIN_SIDE, MOBILE_MAX_WIDTH};
use browseros_core::viewport::{Size, ViewportMode, ViewportRequest};

/// How a fit shows its tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitMode {
    /// Mobile emulation at the screen's size (DPR 2, touch) in a fullscreen window: a phone.
    Mobile,
    /// The tab's default viewport, with no emulation, in a maximized window: a desktop.
    Desktop,
}

/// How every fitted tab is shown on a screen of one size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Layout {
    /// The screen's size.
    pub(crate) screen: ScreenSize,
    pub(crate) shape: Shape,
}

/// The two ways a fitted tab is shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Shape {
    /// This mobile viewport, in a fullscreen window, so the page starts at the screen's
    /// top-left under no browser chrome.
    Mobile(ViewportRequest),
    /// The default viewport in a maximized window, so the page is exactly the window's content
    /// area. Desktop emulation at the screen's size would clip about 87 px: the content area
    /// is the screen less the browser's toolbar.
    Desktop,
}

/// The layout for a screen of `screen`'s size; `None` when a side is under 100 px, which Xvnc
/// can report and no page fits.
pub(crate) fn layout_for(screen: ScreenSize) -> Option<Layout> {
    if screen.width < MIN_SIDE || screen.height < MIN_SIDE {
        return None;
    }
    let shape = if screen.width <= MOBILE_MAX_WIDTH {
        let size = Size {
            width: screen.width.into(),
            height: screen.height.min(MAX_SIDE).into(),
        };
        let request = ViewportRequest::new(ViewportMode::Mobile, None)
            .and_then(|request| request.with_size(size))
            .ok()?;
        Shape::Mobile(request)
    } else {
        Shape::Desktop
    };
    Some(Layout { screen, shape })
}

impl Shape {
    pub(crate) fn mode(self) -> FitMode {
        match self {
            Self::Mobile(_) => FitMode::Mobile,
            Self::Desktop => FitMode::Desktop,
        }
    }

    pub(crate) fn window(self) -> WindowState {
        match self {
            Self::Mobile(_) => WindowState::Fullscreen,
            Self::Desktop => WindowState::Maximized,
        }
    }

    /// The tab's viewport request; `None` is the default mode, as core reports it.
    pub(crate) fn viewport(self) -> Option<ViewportRequest> {
        match self {
            Self::Mobile(request) => Some(request),
            Self::Desktop => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FitMode, Shape, layout_for};
    use crate::ScreenSize;
    use crate::browser_tabs::WindowState;
    use browseros_core::viewport::{Size, ViewportMode};

    fn screen(width: u16, height: u16) -> ScreenSize {
        ScreenSize { width, height }
    }

    /// The rule as text: `mobile WxH`, `desktop`, or `none`.
    fn rule(width: u16, height: u16) -> String {
        match layout_for(screen(width, height)).map(|layout| layout.shape) {
            None => "none".to_string(),
            Some(Shape::Desktop) => "desktop".to_string(),
            Some(Shape::Mobile(request)) => match request.size() {
                Some(Size { width, height }) => format!("mobile {width}x{height}"),
                None => "mobile without a size".to_string(),
            },
        }
    }

    #[test]
    fn a_phone_gets_mobile_at_its_size_and_anything_wider_gets_the_desktop() {
        for (width, height, want) in [
            (366, 700, "mobile 366x700"),
            (390, 844, "mobile 390x844"),
            (600, 960, "mobile 600x960"),
            (601, 960, "desktop"),
            (1440, 900, "desktop"),
            (5120, 2880, "desktop"),
            (390, 5000, "mobile 390x4096"),
            (100, 100, "mobile 100x100"),
            (80, 700, "none"),
            (390, 99, "none"),
            (0, 0, "none"),
        ] {
            assert_eq!(rule(width, height), want, "a {width}x{height} screen");
        }
    }

    #[test]
    fn mobile_is_fullscreen_emulation_and_desktop_is_a_maximized_default_viewport()
    -> Result<(), &'static str> {
        let phone = layout_for(screen(390, 844)).ok_or("a phone has a layout")?;
        let desktop = layout_for(screen(1280, 800)).ok_or("a desktop has a layout")?;
        assert_eq!(phone.screen, screen(390, 844));
        assert_eq!(phone.shape.mode(), FitMode::Mobile);
        assert_eq!(phone.shape.window(), WindowState::Fullscreen);
        let request = phone.shape.viewport();
        assert_eq!(
            request.map(|request| request.mode()),
            Some(ViewportMode::Mobile)
        );
        assert_eq!(request.and_then(|request| request.zoom()), Some(100.0));
        assert_eq!(desktop.shape.mode(), FitMode::Desktop);
        assert_eq!(desktop.shape.window(), WindowState::Maximized);
        assert_eq!(desktop.shape.viewport(), None, "no emulation at all");
        Ok(())
    }
}
