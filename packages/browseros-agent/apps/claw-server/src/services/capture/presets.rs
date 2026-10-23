//! The quality presets behind `?quality=`. Every capture reads its numbers from this table, so
//! tuning a value here is not an API change: the spec names the presets, not their numbers.
//!
//! The frame rates are ceilings. Chromium sends a screencast frame only when the page changes,
//! and the screencast's rate is enforced by when we acknowledge frames, which keeps the browser's
//! UI-thread work proportional to it (measured: 6–9% of the UI thread per tab at the display
//! rate, 1–3% paced to these values; see `.llm/1022-session-screencast/design.md`).

/// How detailed a live view is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    Low,
    High,
}

impl Quality {
    /// Parses the `quality` query value.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "low" => Some(Self::Low),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Preset {
    /// Frames are scaled to fit inside this box, keeping the tab's aspect ratio. A cap is
    /// required: without one Chromium captures the full device-pixel viewport, and on a wide
    /// display it produced no frames at all.
    pub(super) max_width: u32,
    pub(super) max_height: u32,
    pub(super) screencast_fps: u32,
    /// The encoder runs at this rate itself, so video needs no pacing on our side.
    pub(super) video_fps: u32,
    pub(super) jpeg_quality: u32,
}

pub(super) const fn preset(quality: Quality) -> Preset {
    match quality {
        Quality::Low => Preset {
            max_width: 480,
            max_height: 300,
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
