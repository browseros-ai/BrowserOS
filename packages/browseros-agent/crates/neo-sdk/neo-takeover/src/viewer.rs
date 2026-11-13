//! The host's logical viewer request and the effective geometry shared by active fits.
//! Screen dimensions stay physical; preview scale changes how the logical phone is drawn.

use crate::limits::{MAX_SIDE, MIN_SIDE, MOBILE_MAX_WIDTH};
use crate::{PartError, ScreenSize};

/// Physical framebuffer pixels per logical viewer pixel. The page keeps its mobile DPR 2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewerScale {
    /// The existing preview, one framebuffer pixel per logical pixel.
    #[default]
    One,
    /// A phone preview drawn at 200 percent without changing its logical device metrics.
    Two,
}

impl ViewerScale {
    /// The integer multiplier used for framebuffer dimensions and keyboard boxes.
    #[must_use]
    pub const fn factor(self) -> u16 {
        match self {
            Self::One => 1,
            Self::Two => 2,
        }
    }
}

/// A validated request in logical pixels. Omitted scale is resolved by the shared surface,
/// which retains its current scale for a refit of the same tab and uses 1 for a new fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewer {
    size: ScreenSize,
    scale: Option<ViewerScale>,
}

impl Viewer {
    /// Checks request bounds before a host route takes a lock or changes the browser.
    ///
    /// # Errors
    /// A logical side outside 100 to 4096, desktop scale 2, or a scaled side over 4096.
    pub fn new(size: ScreenSize, scale: Option<ViewerScale>) -> Result<Self, PartError> {
        validate(size, scale.unwrap_or_default())?;
        Ok(Self { size, scale })
    }

    pub(crate) fn resolve(self, inherited: ViewerScale) -> Result<ViewerGeometry, PartError> {
        let scale = self.scale.unwrap_or(inherited);
        // An omitted scale may have become 2 since admission. Validate again under the
        // surface lock before any browser work, so a desktop refit cannot inherit phone zoom.
        validate(self.size, scale)?;
        Ok(ViewerGeometry {
            size: self.size,
            scale,
        })
    }
}

/// The effective logical dimensions and scale of the one layout all active fits share.
/// A fit records this snapshot alongside its physical screen for polling and stale-read checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerGeometry {
    /// The logical device dimensions, before preview scaling.
    pub size: ScreenSize,
    /// The preview's physical pixels per logical pixel.
    pub scale: ViewerScale,
}

fn validate(size: ScreenSize, scale: ViewerScale) -> Result<(), PartError> {
    if size.width < MIN_SIDE || size.height < MIN_SIDE {
        return Err(PartError::UnusableViewer(size));
    }
    if size.width > MAX_SIDE || size.height > MAX_SIDE {
        return Err(PartError::InvalidViewer(
            "`width` and `height` must each be from 100 to 4096",
        ));
    }
    if scale == ViewerScale::Two && size.width > MOBILE_MAX_WIDTH {
        return Err(PartError::InvalidViewer(
            "`scale` 2 requires a logical width of at most 600; use 1 for a desktop",
        ));
    }
    if [size.width, size.height].into_iter().any(|side| {
        side.checked_mul(scale.factor())
            .is_none_or(|physical| physical > MAX_SIDE)
    }) {
        return Err(PartError::InvalidViewer(
            "scaled `width` and `height` must each be at most 4096",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Viewer, ViewerScale};
    use crate::ScreenSize;

    #[test]
    fn requests_validate_logical_mode_and_physical_bounds() {
        for (width, height, scale, valid) in [
            (366, 700, None, true),
            (366, 700, Some(ViewerScale::Two), true),
            (600, 2048, Some(ViewerScale::Two), true),
            (601, 700, Some(ViewerScale::Two), false),
            (366, 2049, Some(ViewerScale::Two), false),
            (366, u16::MAX, Some(ViewerScale::Two), false),
            (4096, 4096, Some(ViewerScale::One), true),
            (4097, 700, None, false),
            (99, 700, None, false),
        ] {
            assert_eq!(
                Viewer::new(ScreenSize { width, height }, scale).is_ok(),
                valid,
                "a request must fit the logical device and bounded framebuffer"
            );
        }
    }
}
