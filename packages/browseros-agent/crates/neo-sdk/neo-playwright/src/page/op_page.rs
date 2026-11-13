//! One op's page: the tab it admitted, the tab's state, and the endpoints its worlds live on.

use std::sync::Arc;

use browseros_core::{BrowserSession, FrameId, PageId, ProtocolSession, TabId, TargetId};

use crate::op_scope::{AdmittedPage, OpScope};
use crate::page::PageState;
use crate::page::frame_routes::RootAttachment;
use crate::probe::ProbeFailure;
use crate::target::PageRef;
use crate::world::{Endpoint, FrameWorlds, Installer, IsolatedWorld, Place};
use crate::{Playwright, PlaywrightError};

/// The page an op acts on. Admission (the run view's profile and window check) happens once,
/// in [`OpPage::admit`]; every call after it goes to a raw `ProtocolSession` for the same
/// session id (decisions.md). On a bound view an admitted session checks the page's scope
/// around every message, about 7 CDP messages per call, which a 6-message click cannot afford.
pub(crate) struct OpPage<'op> {
    pw: &'op Playwright,
    admitted: &'op AdmittedPage,
    state: Arc<PageState>,
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
        })
    }

    /// A page already checked through the run's view, for an event batch's read-only work.
    /// This does not select an action target in the scope; several chooser records can be
    /// adopted by one pump operation without weakening the single-tab write invariant.
    pub(crate) async fn observed(pw: &'op Playwright, admitted: &'op AdmittedPage) -> Self {
        Self {
            pw,
            admitted,
            state: pw.pages().page(pw.browser(), admitted).await,
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

    /// The tab's isolated worlds.
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
        }
    }

    /// Whether this tab may have any frame besides the snapshot's exact frame, checked
    /// against the run's existing admitted tree. A cache only proves absence with live,
    /// gap-free observation while checking it; uncertainty enables the scan rather than
    /// fetching topology or changing frame storage.
    ///
    /// A tab that took a private entry always answers true (ruling R53). A marked box keeps the
    /// prototype of the frame it was created in; once the page moved it into this document and
    /// removed that frame, the tree proves one frame, and only the document-wide scan still
    /// finds that prototype to guard it.
    pub(crate) fn may_have_other_frame(&self, scope: &OpScope<'_>, world: &IsolatedWorld) -> bool {
        if self.pw.took_private_text(self.tab()) {
            return true;
        }
        let fence = self.pw.event_fence();
        if self.frames().lifecycle_has_gaps() || !self.pw.events_live_through(fence) {
            return true;
        }
        let Some(session) = world.session_id() else {
            return true;
        };
        let epoch = self.browser().connection().connection_epoch();
        let root = RootAttachment {
            session: self.admitted.session.session_id.clone(),
            frame: FrameId(self.target_id().as_str().to_owned()),
            epoch,
        };
        let other_frame = scope.run_state().frames.may_have_other_frame(
            self.tab(),
            &root,
            world.frame(),
            session,
            self.pw.events(),
        );
        other_frame
            || self.frames().lifecycle_has_gaps()
            || self.browser().connection().connection_epoch() != epoch
            || self.pw.event_fence() != fence
            || !self.pw.events_live_through(fence)
    }

    /// The world of `endpoint`'s frame, installed on first use; concurrent first uses share one
    /// install (`FrameWorlds::world`). The op's own timeout bounds it.
    pub(crate) async fn world(
        &self,
        endpoint: &Endpoint,
        place: Place,
    ) -> Result<IsolatedWorld, ProbeFailure> {
        // A new OOPIF may be discovered before its attachment event is replayed. Synchronize
        // its page preferences before its first frame query or main-world script.
        self.pw
            .media()
            .ensure(&endpoint.session, self.state.media())
            .await
            .map_err(ProbeFailure::from_core)?;
        let installer = Installer {
            browser: self.pw.browser(),
            epoch: self.pw.epoch(),
        };
        self.state.frames().world(installer, endpoint, place).await
    }
}
