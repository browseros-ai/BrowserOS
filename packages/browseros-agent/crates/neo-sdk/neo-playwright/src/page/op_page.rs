//! One op's page: the tab it admitted, the tab's state, the endpoints its worlds live on, and how
//! long the op goes on trying a frame whose world's install missed its budget.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::{BrowserSession, FrameId, PageId, ProtocolSession, TabId, TargetId};
use futures_util::FutureExt;
use futures_util::future::Either;
use tokio::time::{self, Instant};

use crate::op_scope::{AdmittedPage, OpScope};
use crate::page::PageState;
use crate::probe::ProbeFailure;
use crate::target::PageRef;
use crate::world::{Endpoint, FrameWorlds, Installer, Place, SealedWorld, SessionKind};
use crate::{Playwright, PlaywrightError};

/// The page an op acts on. Admission (the run view's profile and window check) happens once,
/// in [`OpPage::admit`]; every call after it goes to a raw `ProtocolSession` for the same
/// session id (decisions.md). On a bound view an admitted session checks the page's scope
/// around every message, about 7 CDP messages per call, which a 6-message click cannot afford.
pub(crate) struct OpPage<'op> {
    pw: &'op Playwright,
    admitted: &'op AdmittedPage,
    state: Arc<PageState>,
    /// When the op's own Playwright timeout passes (`OpScope::deadline`); `None` without one.
    deadline: Option<Instant>,
}

/// How long one frame's world may take to install, its hidden-root scan included, from the
/// start of the install (`SealedWorld::install`). A healthy frame takes a few milliseconds; one
/// that misses it is installed again while the op's window has room (`LateRetry`), each try
/// ending at the window's end at the latest (`TryCut`).
pub(crate) const FRAME_BUDGET: Duration = Duration::from_secs(1);

/// How long an op without a Playwright timeout (`timeout: 0`) goes on trying a frame that missed
/// its budget, per stall: from the first try of an install, and a new window only once the last
/// one completed. It bounds each continuous stall, so a frame that stays late
/// fails the op when its window ends. It is not one total for the op: an untimed poll (an
/// `expect` with `timeout: 0`, which upstream waits on forever) would then fail a later short
/// stall for an earlier one that had already recovered.
///
/// Measured under 16 `yes` burners (load average 27 to 299): of 764 frame budgets over 156
/// conformance cases, every one completed, the p99 in 150 ms and the slowest in 1.0 s, on its
/// second try. 5 s holds that slowest one and four more late tries; a shorter window would only
/// refuse a hung frame sooner.
pub(crate) const UNTIMED_RETRY: Duration = Duration::from_secs(5);

/// How long an op goes on trying a frame that missed its install budget (`FRAME_BUDGET`): until
/// the op's own deadline, as upstream waits on a slow frame until the op times out, or, when the
/// op has none, for [`UNTIMED_RETRY`] per stall: from the first try of an install, and a new
/// window only once the last one completed. Another try
/// starts only while the window still holds one as long as the last, and none runs past the
/// window's end ([`LateRetry::cut`]), so a frame late to the end fails the op with Neo's message
/// on why, by the op's deadline, before the op's own timeout could replace it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LateRetry {
    until: Instant,
}

impl LateRetry {
    /// Whether another try as long as `last` still ends inside the window.
    pub(crate) fn has_room(self, last: Duration) -> bool {
        self.until.saturating_duration_since(Instant::now()) >= last
    }

    /// Where every try after the first is cut: at the window's end. The last try's length is
    /// no bound on the next one's.
    pub(crate) fn cut(self) -> TryCut {
        TryCut(Some(self.until))
    }
}

/// Where one try of an install is cut. The first try is not ([`TryCut::default`]): it has its
/// full frame budget, so a frame late on it missed Neo's budget, and an op timeout that ends
/// sooner is the op's own, inside `OpScope::within`. Every later try is cut at its
/// [`LateRetry`] window's end, and a frame cut there is late, as one past its budget is, and the
/// op fails with Neo's refusal, true to its word since the first try gave the frame its whole
/// budget.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TryCut(Option<Instant>);

impl TryCut {
    /// `work`'s output, or `None` when the cut came first. Not an `async fn`, whose state would
    /// hold `work` twice, as its argument and inside the timeout: an install's is large.
    pub(crate) fn before<F: Future>(self, work: F) -> impl Future<Output = Option<F::Output>> {
        match self.0 {
            Some(until) => Either::Left(time::timeout_at(until, work).map(Result::ok)),
            None => Either::Right(work.map(Some)),
        }
    }
}

impl<'op> OpPage<'op> {
    /// Admits `page` for this op and finds its state, creating it on the tab's first op.
    pub(crate) async fn admit(
        pw: &'op Playwright,
        scope: &'op OpScope<'_>,
        page: TabId,
    ) -> Result<Self, PlaywrightError> {
        let admitted = scope.admit(pw.browser(), page).await?;
        let state = pw.pages().page(pw.browser(), admitted).await;
        let session = pw
            .browser()
            .protocol_session(admitted.session.session_id.clone());
        pw.media()
            .ensure_root(&admitted.core_page, &session, state.media())
            .await
            .map_err(|error| scope.core_error(&error))?;
        Ok(Self {
            pw,
            admitted,
            state,
            deadline: scope.deadline(),
        })
    }

    /// A page already checked through the run's view, for an event batch's read-only work.
    /// This does not select an action target in the scope; several chooser records can be
    /// adopted by one pump operation without weakening the single-tab write invariant.
    pub(crate) async fn observed(
        pw: &'op Playwright,
        scope: &'op OpScope<'_>,
        admitted: &'op AdmittedPage,
    ) -> Self {
        Self {
            pw,
            admitted,
            state: pw.pages().page(pw.browser(), admitted).await,
            deadline: scope.deadline(),
        }
    }

    /// Admits the native tab named by a target's page brand.
    pub(crate) async fn admit_handle(
        pw: &'op Playwright,
        scope: &'op OpScope<'_>,
        handle: &PageRef,
    ) -> Result<Self, PlaywrightError> {
        let page = handle.tab();
        Self::admit(pw, scope, page).await
    }

    /// The tab.
    pub(crate) fn tab(&self) -> TabId {
        self.admitted.tab
    }

    /// The core cache id, held only for this operation and passed directly to core.
    pub(crate) fn core_page(&self) -> &PageId {
        &self.admitted.core_page
    }

    /// The tab's sealed worlds.
    pub(crate) fn frames(&self) -> &FrameWorlds {
        self.state.frames()
    }

    /// The tab's state, for what outlives the op: its input lock, its focus lease.
    pub(crate) fn state(&self) -> &Arc<PageState> {
        &self.state
    }

    /// The tab's CDP target.
    pub(crate) fn target_id(&self) -> &TargetId {
        &self.admitted.session.target_id
    }

    /// The tab's root session, where all input goes.
    pub(crate) fn root_session(&self) -> ProtocolSession {
        self.pw
            .browser()
            .protocol_session(self.admitted.session.session_id.clone())
    }

    /// The browser session, for core's frame registry.
    pub(crate) fn browser(&self) -> &BrowserSession {
        self.pw.browser()
    }

    /// The main frame on the page's own session. A page target's main frame id is its target
    /// id, and stays so across cross-process navigations (measured).
    pub(crate) fn main_endpoint(&self) -> Endpoint {
        Endpoint {
            session: self
                .pw
                .browser()
                .protocol_session(self.admitted.session.session_id.clone()),
            frame: FrameId(self.admitted.session.target_id.as_str().to_owned()),
            kind: SessionKind::Page,
        }
    }

    /// A child frame on the session core's frame registry gave it: the page's own for a
    /// same-process child of the main frame, else an OOPIF's (or the one an OOPIF's
    /// same-process child shares).
    pub(crate) fn child_endpoint(&self, session: ProtocolSession, frame: FrameId) -> Endpoint {
        let kind = if session.session_id() == Some(&self.admitted.session.session_id) {
            SessionKind::Page
        } else {
            SessionKind::Oopif
        };
        Endpoint {
            session,
            frame,
            kind,
        }
    }

    /// The window in which this op tries a frame that missed its install budget again, from
    /// now: each install takes its own as it starts its first try, which for an op without a
    /// timeout opens [`UNTIMED_RETRY`] afresh.
    pub(crate) fn late_retry(&self) -> LateRetry {
        LateRetry {
            until: self
                .deadline
                .unwrap_or_else(|| Instant::now() + UNTIMED_RETRY),
        }
    }

    /// The world of `endpoint`'s frame, installed on first use. An install whose hidden-root
    /// scan missed the frame's budget (`ProbeFailure::Late`) left no world behind, so it is
    /// tried afresh while the op's [`LateRetry`] window has room, each try after the first cut
    /// at the window's end (an install's steps before its scan have no budget of their own);
    /// the last one's `Late` is the op's. A cut install leaves its frame's cell empty for the
    /// next caller.
    pub(crate) async fn world(
        &self,
        endpoint: &Endpoint,
        place: Place,
    ) -> Result<SealedWorld, ProbeFailure> {
        // A new OOPIF may be discovered before its attachment event is replayed. Synchronize
        // its page preferences before its first frame query or main-world script.
        self.pw
            .media()
            .ensure(&endpoint.session, self.state.media())
            .await
            .map_err(ProbeFailure::from_core)?;
        let installer = Installer {
            sessions: self.pw.sessions(),
            epoch: self.pw.epoch(),
        };
        let retry = self.late_retry();
        let mut cut = TryCut::default();
        loop {
            let started = Instant::now();
            let installing = self
                .state
                .frames()
                .world(installer, endpoint, place.clone());
            match cut
                .before(installing)
                .await
                .unwrap_or(Err(ProbeFailure::Late))
            {
                Err(ProbeFailure::Late) if retry.has_room(started.elapsed()) => cut = retry.cut(),
                installed => return installed,
            }
        }
    }
}
