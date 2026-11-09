//! Neo: The identities the host side speaks, parsed once where they enter. Hosts address sessions by
//! [`SessionKey`] and see BrowserOS tab ids ([`TabId`]); scripts and capture speak CDP target ids
//! (`PageId`). The two tab identities meet only in the host's tab directory.

use std::fmt;
use std::num::{NonZeroI64, NonZeroU64};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ulid::Ulid;

/// A host session: the label a host put on its runs (`_meta["com.browseros.neo/session"]`), or
/// a key the host side minted for a run with no label or as the successor of an ended session.
/// Opaque everywhere: the runtime copies it onto events, and `/v1/sessions/{sessionId}` is this
/// string. Never derive a key from another (`label~2`): partners treat handles as opaque.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionKey(Arc<str>);

impl SessionKey {
    #[must_use]
    pub fn new(key: impl Into<Arc<str>>) -> Self {
        Self(key.into())
    }

    /// Neo: claw-server's `candidate` (`apps/claw-server/src/api/mcp/session_handle.rs`), retaining an opaque eight-hex-digit host session handle; uniqueness against live and stored sessions is the caller's check.
    pub(crate) fn mint() -> Self {
        // The low 32 random bits of a ULID; its timestamp bits would make keys sortable.
        let random = Ulid::new().random() & u128::from(u32::MAX);
        Self::new(format!("{random:08x}"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Platform: BrowserOS's `Browser.TabInfo.tabId` and `Target.TargetInfo.tabId` are session-scoped tab identities (`browseros-cdp/protocol/protocol.json`, confirmed by `designs/neo-host-events/grounding/probes.md`, probe 6).
/// A BrowserOS tab id. Always positive, so a frame can never carry `X-Tab-Id: 0`, which the main host rejects. Chromium never reuses one
/// within a browser session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(NonZeroI64);

impl TabId {
    /// `None` for zero and negatives: what CDP reports for a target that is not a tab.
    #[must_use]
    pub fn parse(raw: i64) -> Option<Self> {
        if raw > 0 {
            NonZeroI64::new(raw).map(Self)
        } else {
            None
        }
    }

    #[must_use]
    pub fn get(self) -> i64 {
        self.0.get()
    }
}

/// Unix epoch milliseconds, always positive: `X-Frame-At` and every host-facing time. Runs and
/// calls are stamped from the runtime's wall clock, frames from Chromium's frame-swap time; both
/// read the same machine's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EpochMs(NonZeroU64);

impl EpochMs {
    /// Neo: the earliest representable host timestamp is 1 ms after the epoch; zero must never reach `X-Frame-At`.
    pub(crate) const MIN: Self = Self(NonZeroU64::MIN);

    #[must_use]
    pub fn from_millis(ms: u64) -> Option<Self> {
        NonZeroU64::new(ms).map(Self)
    }

    /// `None` before the epoch or at exactly 0, which no real clock reads.
    pub(crate) fn from_system_time(time: SystemTime) -> Option<Self> {
        let ms = time.duration_since(UNIX_EPOCH).ok()?.as_millis();
        Self::from_millis(u64::try_from(ms).ok()?)
    }

    pub(crate) fn now() -> Option<Self> {
        Self::from_system_time(SystemTime::now())
    }

    /// Playwright: `server/chromium/crPage.ts` `FrameSession._onScreencastFrame`: `frameSwapWallTime: payload.metadata.timestamp ? payload.metadata.timestamp * 1000 : Date.now(),`
    /// Platform: CDP frame timestamps are epoch seconds (`browseros-cdp/protocol/protocol.json`, `Page.ScreencastFrameMetadata.timestamp` referencing `Network.TimeSinceEpoch`; measured in `designs/neo-host-events/grounding/probes.md`, probe 8).
    /// Deviation: `server/chromium/crPage.ts` `FrameSession._onScreencastFrame`: `frameSwapWallTime: payload.metadata.timestamp ? payload.metadata.timestamp * 1000 : Date.now(),` Neo requires positive integer milliseconds for `X-Frame-At`, truncates fractional milliseconds and drops invalid timestamps instead of substituting arrival time, so hosts can correlate frames with browser actions.
    pub(crate) fn from_frame_seconds(seconds: f64) -> Option<Self> {
        if !seconds.is_finite() || seconds <= 0.0 {
            return None;
        }
        let ms = (seconds * 1000.0).trunc();
        // Neo: reject milliseconds at 1.8e19, below u64::MAX and far past any wall clock (year ~586 million), so the conversion stays in range.
        if ms >= 1.8e19 {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "checked finite, positive and below u64::MAX above"
        )]
        let ms = ms as u64;
        Self::from_millis(ms)
    }

    #[must_use]
    pub fn get(self) -> u64 {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_ids_are_positive() {
        assert_eq!(
            TabId::parse(1734).map(TabId::get),
            Some(1734),
            "a real tab id"
        );
        assert_eq!(TabId::parse(0), None, "zero is not a tab");
        assert_eq!(TabId::parse(-1), None, "negatives are not tabs");
    }

    #[test]
    fn frame_seconds_become_truncated_millis() {
        let at = EpochMs::from_frame_seconds(1_794_002_916.593_477).map(EpochMs::get);
        assert_eq!(at, Some(1_794_002_916_593), "probe 8's sample");
        assert_eq!(EpochMs::from_frame_seconds(0.0), None, "absent timestamp");
        assert_eq!(
            EpochMs::from_frame_seconds(-3.0),
            None,
            "negative timestamp"
        );
        assert_eq!(EpochMs::from_frame_seconds(f64::NAN), None, "not a number");
    }

    #[test]
    fn minted_keys_are_eight_hex_digits() {
        let key = SessionKey::mint();
        assert_eq!(key.as_str().len(), 8, "{key}");
        assert!(key.as_str().chars().all(|c| c.is_ascii_hexdigit()), "{key}");
    }
}
