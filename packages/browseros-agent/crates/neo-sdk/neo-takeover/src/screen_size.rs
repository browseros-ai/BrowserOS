//! Neo's one screen: its size, and the Xvnc server that shows it.

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::str::FromStr;

/// Dimensions of Neo's physical framebuffer, or the logical size carried by a [`crate::Viewer`].
/// The real screen has DPR 1; mobile preview scaling can draw one logical pixel across two
/// framebuffer pixels. Any size Xvnc can report is representable; host requests are bounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScreenSize {
    /// Width.
    pub width: u16,
    /// Height.
    pub height: u16,
}

/// Text that is not a screen size such as `1600x1000`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a screen size such as 1600x1000")]
pub struct NotAScreenSize;

/// `1600x1000`, the way the image's `NEO_SCREEN` spells it: two whole numbers above zero.
impl FromStr for ScreenSize {
    type Err = NotAScreenSize;

    fn from_str(text: &str) -> Result<Self, NotAScreenSize> {
        let (width, height) = text.split_once('x').ok_or(NotAScreenSize)?;
        let side = |text: &str| {
            text.parse::<u16>()
                .ok()
                .filter(|side| *side > 0 && text.bytes().all(|byte| byte.is_ascii_digit()))
                .ok_or(NotAScreenSize)
        };
        Ok(Self {
            width: side(width)?,
            height: side(height)?,
        })
    }
}

impl fmt::Display for ScreenSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// Neo's screen: where Xvnc listens, and the size it rests at when no fit holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xvnc {
    /// Xvnc's RFB address. Tests point it at a fake.
    pub addr: SocketAddr,
    /// The size between fits: the image's `NEO_SCREEN`, which the app reads at start.
    pub home: ScreenSize,
}

impl Xvnc {
    /// The image's Xvnc: `-rfbport 5900 -localhost -SecurityTypes None -AlwaysShared`.
    pub const NEO_ADDR: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5900));
}

#[cfg(test)]
mod tests {
    use super::{NotAScreenSize, ScreenSize};

    #[test]
    fn neo_screen_reads_as_width_by_height() {
        assert_eq!(
            "1600x1000".parse(),
            Ok(ScreenSize {
                width: 1600,
                height: 1000
            })
        );
        assert_eq!(
            ScreenSize {
                width: 390,
                height: 844
            }
            .to_string(),
            "390x844"
        );
        for text in [
            "",
            "1600",
            "1600x",
            "x1000",
            "0x1000",
            "1600x0",
            "1600X1000",
            "+1600x1000",
            "1600x1000x24",
            "70000x1000",
            " 1600x1000",
        ] {
            assert_eq!(text.parse::<ScreenSize>(), Err(NotAScreenSize), "{text:?}");
        }
    }
}
