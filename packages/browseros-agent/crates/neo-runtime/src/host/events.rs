//! Neo: What the run path tells the host side, in the order it learns it. The run path produces these
//! at the one door every run and browser call already passes (`progress.rs` `Trace`,
//! `runtime.rs` `Runtime::run`, `tabs.rs` `NewPage::keep`). The host side consumes them in one
//! FIFO, the tap, so the order below is the order the ledger sees.
//!
//! Exactly once each: `RunStarted` and `RunEnded` per run (also a run whose tab was already
//! closed, and a run whose future was dropped), `CallStarted` and `CallEnded` per browser call
//! (a call the run's end cut off ends `Unfinished`), `PageCreated` per committed creation.
//! `PassReleased` comes from the host side itself, when the run tool drops a run's pass, and
//! lands behind every event that run emitted: consuming it proves the run is over for the
//! host side (the completion protocol in the neo-host-events design, §6.6).

use crate::host::ids::{EpochMs, SessionKey, TabId};
use crate::run::{CallResult, PageId, RunId, RunStatus};

/// One fact for the host side. Never params, call logs or results' payloads: hosts read those from
/// the trace store, and the host side needs only who touched which tab, when, and how it ended.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HostEvent {
    RunStarted {
        session: SessionKey,
        run: RunId,
        /// `RunRequest.page`, the tab the run starts on.
        page: PageId,
        at: EpochMs,
    },
    CallStarted {
        session: SessionKey,
        run: RunId,
        call_id: u64,
        /// The page the dispatcher resolved the call's guid to; `None` for context calls.
        page: Option<PageId>,
        /// Playwright: `server/trace/recorder/tracing.ts` `Tracing._shouldCaptureAtPhase`: `if (!metainfo?.snapshot)`
        /// Neo: the host uses the generated snapshot flag to request an after-call JPEG.
        snapshot: bool,
        at: EpochMs,
    },
    CallEnded {
        session: SessionKey,
        run: RunId,
        call_id: u64,
        /// As on `CallStarted`, except a context-level `newPage` carries the page it created.
        page: Option<PageId>,
        result: CallResult,
    },
    PageCreated {
        session: SessionKey,
        run: RunId,
        page: PageId,
        /// Platform: BrowserOS's create-tab reply includes `Browser.TabInfo.tabId` (`browseros-cdp/protocol/protocol.json`), allowing attribution before discovery reports the target.
        tab: Option<TabId>,
        /// The `newPage` call that created it.
        call_id: Option<u64>,
    },
    RunEnded {
        session: SessionKey,
        run: RunId,
        status: RunStatus,
        at: EpochMs,
    },
    PassReleased {
        session: SessionKey,
    },
}

impl HostEvent {
    /// Lifecycle events have a reserved band in the tap and are never refused while the server's
    /// admission limit holds; per-call events are refused first when the tap is full.
    pub(crate) fn is_lifecycle(&self) -> bool {
        !matches!(self, Self::CallStarted { .. } | Self::CallEnded { .. })
    }

    pub(crate) fn session(&self) -> &SessionKey {
        match self {
            Self::RunStarted { session, .. }
            | Self::CallStarted { session, .. }
            | Self::CallEnded { session, .. }
            | Self::PageCreated { session, .. }
            | Self::RunEnded { session, .. }
            | Self::PassReleased { session } => session,
        }
    }
}
