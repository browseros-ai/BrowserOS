//! Resolves logical viewer geometry into the shared screen, window and tab viewport.
//! Mobile selection uses logical width; preview zoom changes the physical drawing while the
//! page retains the same logical mobile device and DPR 2. Every active fit shares one layout.
//!
//! | Logical viewer | Window | Viewport |
//! |---|---|---|
//! | a side under 100 | unchanged | unchanged: no layout fits it |
//! | 600 wide or less | fullscreen | mobile at the logical size (DPR 2, touch), at most 4096 tall |
//! | wider | maximized | the default, with no emulation |

use crate::browser_tabs::WindowState;
use crate::limits::{MAX_SIDE, MIN_SIDE, MOBILE_MAX_WIDTH};
use crate::{ScreenSize, ViewerGeometry, ViewerScale};
use browseros_core::viewport::{Size, ViewportMode, ViewportRequest};

/// How a fit shows its tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitMode {
    /// Mobile emulation at the logical viewer size (DPR 2, touch) in a fullscreen window.
    Mobile,
    /// The tab's default viewport, with no emulation, in a maximized window: a desktop.
    Desktop,
}

/// How every fitted tab is shown on a screen of one size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Layout {
    /// The physical framebuffer, which may have an unused edge pixel after an odd resize.
    pub(crate) screen: ScreenSize,
    pub(crate) viewer: ViewerGeometry,
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
    layout(
        screen,
        ViewerGeometry {
            size: screen,
            scale: ViewerScale::One,
        },
    )
}

/// A validated host request has exact integer scaling and bounded physical dimensions.
pub(crate) fn requested_layout(viewer: ViewerGeometry) -> Option<Layout> {
    let screen = ScreenSize {
        width: viewer.size.width.checked_mul(viewer.scale.factor())?,
        height: viewer.size.height.checked_mul(viewer.scale.factor())?,
    };
    layout(screen, viewer)
}

/// An external resize describes physical pixels. Keep the active phone scale and floor an odd
/// side instead of cropping the preview. Crossing the logical phone breakpoint ends scale 2;
/// desktop previews have no zoom, so later resizes follow the ordinary 1x rule.
pub(crate) fn observed_layout(screen: ScreenSize, scale: ViewerScale) -> Option<Layout> {
    let size = ScreenSize {
        width: screen.width / scale.factor(),
        height: screen.height / scale.factor(),
    };
    if scale == ViewerScale::One {
        return layout_for(screen);
    }
    if size.width < MIN_SIDE || size.height < MIN_SIDE {
        return None;
    }
    if size.width > MOBILE_MAX_WIDTH {
        return layout_for(screen);
    }
    if size.height > MAX_SIDE {
        return None;
    }
    layout(screen, ViewerGeometry { size, scale })
}

fn layout(screen: ScreenSize, viewer: ViewerGeometry) -> Option<Layout> {
    if viewer.size.width < MIN_SIDE || viewer.size.height < MIN_SIDE {
        return None;
    }
    let shape = if viewer.size.width <= MOBILE_MAX_WIDTH {
        let size = Size {
            width: viewer.size.width.into(),
            height: viewer.size.height.min(MAX_SIDE).into(),
        };
        let zoom = f64::from(100 * viewer.scale.factor());
        let request = ViewportRequest::new(ViewportMode::Mobile, Some(zoom))
            .and_then(|request| request.with_size(size))
            .ok()?;
        Shape::Mobile(request)
    } else {
        Shape::Desktop
    };
    Some(Layout {
        screen,
        viewer,
        shape,
    })
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
    use super::{FitMode, Shape, layout_for, observed_layout, requested_layout};
    use crate::browser_tabs::WindowState;
    use crate::{ScreenSize, ViewerGeometry, ViewerScale};
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

    #[test]
    fn two_x_selects_mobile_from_logical_width_and_only_zooms_the_preview()
    -> Result<(), &'static str> {
        let viewer = ViewerGeometry {
            size: screen(366, 700),
            scale: ViewerScale::Two,
        };
        let phone = requested_layout(viewer).ok_or("a scaled phone has a layout")?;
        assert_eq!(phone.screen, screen(732, 1400));
        assert_eq!(phone.viewer, viewer);
        assert_eq!(phone.shape.window(), WindowState::Fullscreen);
        let request = phone.shape.viewport().ok_or("a phone has emulation")?;
        assert_eq!(request.mode(), ViewportMode::Mobile);
        assert_eq!(
            request.size(),
            Some(Size {
                width: 366,
                height: 700
            })
        );
        assert_eq!(request.zoom(), Some(200.0));
        Ok(())
    }

    #[test]
    fn two_x_observations_keep_real_pixels_and_end_scaling_at_the_desktop_breakpoint()
    -> Result<(), &'static str> {
        let odd = observed_layout(screen(733, 1401), ViewerScale::Two)
            .ok_or("an odd screen still fits a phone")?;
        assert_eq!(odd.screen, screen(733, 1401));
        assert_eq!(odd.viewer.size, screen(366, 700));
        assert_eq!(odd.viewer.scale, ViewerScale::Two);
        let desktop = observed_layout(screen(1202, 1400), ViewerScale::Two)
            .ok_or("a wide external screen becomes a desktop")?;
        assert_eq!(desktop.shape.mode(), FitMode::Desktop);
        assert_eq!(desktop.viewer.size, screen(1202, 1400));
        assert_eq!(desktop.viewer.scale, ViewerScale::One);
        assert_eq!(observed_layout(screen(199, 1400), ViewerScale::Two), None);
        assert_eq!(observed_layout(screen(1202, 199), ViewerScale::Two), None);
        assert_eq!(observed_layout(screen(732, 8194), ViewerScale::Two), None);
        Ok(())
    }
}
