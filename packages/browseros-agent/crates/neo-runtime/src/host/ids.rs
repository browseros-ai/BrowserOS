//! The identities the host side speaks, parsed once where they enter. Hosts address sessions by
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

    /// A fresh 8-hex-digit key, the shape claw-server mints for stateless sessions
    /// (`apps/claw-server/src/api/mcp/session_handle.rs:31-33`). Uniqueness against live and
    /// stored sessions is the caller's check.
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

/// A BrowserOS tab id: `Browser.TabInfo.tabId`, and BrowserOS's patched
/// `Target.TargetInfo.tabId` (probe 6 in the neo-host-events design). Always positive, so a
/// frame can never carry `X-Tab-Id: 0`, which the main host rejects. Chromium never reuses one
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
    #[must_use]
    pub fn from_millis(ms: u64) -> Option<Self> {
        NonZeroU64::new(ms).map(Self)
    }

    /// `None` before the epoch or at exactly 0, which no real clock reads.
    pub(crate) fn from_system_time(time: SystemTime) -> Option<Self> {
        let ms = time.duration_since(UNIX_EPOCH).ok()?.as_millis();
        Self::from_millis(u64::try_from(ms).ok()?)
    }

    #[expect(dead_code, reason = "scaffold: used by the run path's host events")]
    pub(crate) fn now() -> Option<Self> {
        Self::from_system_time(SystemTime::now())
    }

    /// `Page.screencastFrame.metadata.timestamp`, epoch seconds with a microsecond fraction
    /// (probe 8), as milliseconds, truncated as Playwright converts it
    /// (`packages/playwright-core/src/server/chromium/crPage.ts:997` at 26137face). `None` when
    /// absent, not finite or not positive: the caller drops such a frame rather than stamping
    /// it with arrival time, which cannot be correlated with a browser action.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "scaffold: used by host capture")
    )]
    pub(crate) fn from_frame_seconds(seconds: f64) -> Option<Self> {
        if !seconds.is_finite() || seconds <= 0.0 {
            return None;
        }
        let ms = (seconds * 1000.0).trunc();
        // Far past any wall clock (year ~586 million) and below `u64::MAX`, so the cast is exact.
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
