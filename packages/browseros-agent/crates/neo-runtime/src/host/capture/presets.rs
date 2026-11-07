//! The quality presets behind a live view's `?quality=`, copied verbatim from claw-server
//! (`apps/claw-server/src/services/capture/presets.rs:1-58`, origin/main). Every capture reads
//! its numbers from this table. The hosted contract publishes them (`openapi.yaml`'s `Quality`
//! description), so changing a value here is a contract text change too.
//!
//! The frame rates are ceilings. Chromium sends a screencast frame only when the page changes,
//! and the screencast's rate is enforced by when the capture acknowledges frames, which keeps
//! the browser's UI-thread work proportional to it (claw-server measured 6–9% of the UI thread
//! per tab at the display rate, 1–3% paced to these values).

/// How detailed a live view is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Quality {
    Low,
    High,
}

impl Quality {
    /// Parses the `quality` query value.
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "low" => Some(Self::Low),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Preset {
    /// Frames are scaled to fit inside this box, keeping the tab's aspect ratio. A cap is
    /// required: without one Chromium captures the full device-pixel viewport, and on a wide
    /// display it produced no frames at all.
    pub(crate) max_width: u32,
    pub(crate) max_height: u32,
    pub(crate) screencast_fps: u32,
    /// The encoder runs at this rate itself, so video needs no pacing on the host side.
    pub(crate) video_fps: u32,
    pub(crate) jpeg_quality: u32,
}

pub(crate) const fn preset(quality: Quality) -> Preset {
    match quality {
        Quality::Low => Preset {
            max_width: 640,
            max_height: 400,
            screencast_fps: 4,
            video_fps: 10,
            jpeg_quality: 50,
        },
        Quality::High => Preset {
            max_width: 1280,
            max_height: 800,
            screencast_fps: 10,
            video_fps: 15,
            jpeg_quality: 70,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Quality, preset};

    /// The hosted contract, whose `Quality` parameter publishes the presets' numbers.
    const OPENAPI: &str = include_str!("../../../../../contracts/hosted-api/openapi.yaml");

    #[test]
    fn quality_parses_only_the_published_presets() {
        assert_eq!(Quality::parse("low"), Some(Quality::Low), "low");
        assert_eq!(Quality::parse("high"), Some(Quality::High), "high");
        assert_eq!(Quality::parse("medium"), None, "not a preset");
    }

    #[test]
    fn the_presets_match_the_published_contract_text() {
        // The YAML description is folded, so compare with its line breaks collapsed.
        let published = OPENAPI.split_whitespace().collect::<Vec<_>>().join(" ");
        for (name, quality) in [("low", Quality::Low), ("high", Quality::High)] {
            let preset = preset(quality);
            let text = format!(
                "`{name}` is {}×{} at up to {} frames per second ({} for `/video`)",
                preset.max_width, preset.max_height, preset.screencast_fps, preset.video_fps
            );
            assert!(published.contains(&text), "openapi.yaml lost {text:?}");
        }
    }
}
