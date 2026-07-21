use serde::Serialize;
use std::fmt;

const TAB_GROUP_COLORS: [TabGroupColor; 9] = [
    TabGroupColor::Grey,
    TabGroupColor::Blue,
    TabGroupColor::Red,
    TabGroupColor::Yellow,
    TabGroupColor::Green,
    TabGroupColor::Pink,
    TabGroupColor::Purple,
    TabGroupColor::Cyan,
    TabGroupColor::Orange,
];

/// Chrome tab-group colour names accepted by the BrowserOS tab_groups tool.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TabGroupColor {
    Grey,
    Blue,
    Red,
    Yellow,
    Green,
    Pink,
    Purple,
    Cyan,
    Orange,
}

impl TabGroupColor {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grey => "grey",
            Self::Blue => "blue",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Pink => "pink",
            Self::Purple => "purple",
            Self::Cyan => "cyan",
            Self::Orange => "orange",
        }
    }

    /// Hex twin of the browser tab-group colour (TS TAB_GROUP_HEX) so the
    /// cockpit card border matches the tab strip.
    #[must_use]
    pub fn hex(self) -> &'static str {
        match self {
            Self::Grey => "#6B7280",
            Self::Blue => "#2F6FE0",
            Self::Red => "#DC2626",
            Self::Yellow => "#F59E0B",
            Self::Green => "#10A37F",
            Self::Pink => "#DB2777",
            Self::Purple => "#7A5AF8",
            Self::Cyan => "#0EA5E9",
            Self::Orange => "#F26B2A",
        }
    }
}

/// Hex colour for an agent slug; stable across processes and the TS server.
#[must_use]
pub fn hex_for_slug(slug: &str) -> &'static str {
    color_for_slug(slug).hex()
}

impl fmt::Display for TabGroupColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Selects the deterministic tab-group colour shared with the TS Claw server.
#[must_use]
pub fn color_for_slug(slug: &str) -> TabGroupColor {
    let idx = usize::try_from(fnv1a(slug) % u32::try_from(TAB_GROUP_COLORS.len()).unwrap_or(1))
        .unwrap_or(0);
    TAB_GROUP_COLORS
        .get(idx)
        .copied()
        .unwrap_or(TabGroupColor::Grey)
}

fn fnv1a(input: &str) -> u32 {
    let mut hash = 0x811c9dc5_u32;
    for byte in input.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_add(
            (hash << 1)
                .wrapping_add(hash << 4)
                .wrapping_add(hash << 7)
                .wrapping_add(hash << 8)
                .wrapping_add(hash << 24),
        );
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::{TabGroupColor, color_for_slug};

    #[test]
    fn color_for_slug_matches_tab_group_palette() {
        assert_eq!(color_for_slug("codex"), TabGroupColor::Purple);
        assert_eq!(color_for_slug("finance-ops"), TabGroupColor::Grey);
    }

    #[test]
    fn hex_for_slug_matches_ts_tab_group_hex() {
        assert_eq!(super::hex_for_slug("codex"), "#7A5AF8");
        assert_eq!(super::hex_for_slug("finance-ops"), "#6B7280");
    }
}
