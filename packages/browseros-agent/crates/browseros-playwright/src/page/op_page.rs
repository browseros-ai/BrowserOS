//! One op's page: the tab it admitted, the tab's state, and the endpoints its worlds live on.

use std::sync::Arc;

use browseros_core::{BrowserSession, FrameId, PageId, ProtocolSession, TargetId};

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
}

impl<'op> OpPage<'op> {
    /// Admits `page` for this op and finds its state, creating it on the tab's first op.
    pub(crate) async fn admit(
        pw: &'op Playwright,
        scope: &'op OpScope<'_>,
        page: &PageId,
    ) -> Result<Self, PlaywrightError> {
        let admitted = scope.admit(page).await?;
        let state = pw.pages().page(pw.browser(), admitted).await;
        Ok(Self {
            pw,
            admitted,
            state,
        })
    }

    /// Admits the page a target's brand names, once its tab checks out
    /// ([`PageRef::checked`]).
    pub(crate) async fn admit_handle(
        pw: &'op Playwright,
        scope: &'op OpScope<'_>,
        handle: &PageRef,
    ) -> Result<Self, PlaywrightError> {
        let page = handle.checked(pw.browser(), scope).await?;
        Self::admit(pw, scope, &page).await
    }

    /// The crate instance the op runs in.
    pub(crate) fn playwright(&self) -> &'op Playwright {
        self.pw
    }

    /// The tab.
    pub(crate) fn id(&self) -> &PageId {
        &self.admitted.id
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

    /// The world of `endpoint`'s frame, installed on first use.
    pub(crate) async fn world(
        &self,
        endpoint: &Endpoint,
        place: Place,
    ) -> Result<SealedWorld, ProbeFailure> {
        let installer = Installer {
            sessions: self.pw.sessions(),
            epoch: self.pw.epoch(),
            page: self.id(),
        };
        self.state.frames().world(installer, endpoint, place).await
    }
}
