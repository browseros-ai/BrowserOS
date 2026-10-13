//! Reusable page capabilities retain both the call's scope and the page's target incarnation.
//! Observer state remains shared; only acquisition and protocol dispatch carry these guards.
use super::{BrowserView, CapturedScope, NativeWindow, cdp_policy};
use crate::{
    CoreError, FrameId, PageAdmission, ProtocolSession, SessionId, TargetError, TargetId,
    connection::ProtocolAdmission,
    pages::{PageInfo, PageSession},
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct PageAccess {
    view: BrowserView,
    page: crate::PageId,
    target: TargetId,
}
impl PageAccess {
    pub(super) fn new(view: BrowserView, page: PageInfo) -> Self {
        Self {
            view,
            page: page.page_id,
            target: page.target_id,
        }
    }
    pub(crate) async fn fresh(&self) -> Result<PageInfo, CoreError> {
        let fresh = self.view.page(self.page.clone()).await?;
        if fresh.target_id != self.target {
            return Err(TargetError::PageTargetChanged(self.page.clone()).into());
        }
        Ok(fresh)
    }
    pub(crate) async fn session(self: &Arc<Self>) -> Result<PageSession, CoreError> {
        let live = self
            .view
            .validate_live()
            .await?
            .ok_or_else(|| cdp_policy::unsupported("page_session"))?;
        let admission = Admission {
            access: self,
            scope: &live.scope,
            windows: &live.windows,
        };
        let mut session = self
            .view
            .session
            .pages
            .get_session_admitted(self.page.clone(), &admission)
            .await?;
        self.fresh().await?;
        session.session = session.session.with_admission(Arc::new(PageProtocol {
            access: self.clone(),
            root_session: session.session_id.clone(),
            frame: None,
        }));
        Ok(session)
    }
    pub(crate) async fn frame_session(
        self: &Arc<Self>,
        session: ProtocolSession,
        frame: Option<FrameId>,
    ) -> Result<ProtocolSession, CoreError> {
        let root = self.session().await?;
        if session.same_session(&root.session) {
            return Ok(root.session);
        }
        let frame_id = frame.ok_or_else(|| cdp_policy::unsupported("frame_session"))?;
        let endpoint = session
            .session_id()
            .cloned()
            .ok_or_else(|| cdp_policy::unsupported("frame_session"))?;
        // OOPIFs are internal descendants, never independent page capabilities. Bind the child
        // endpoint to a frame still present in this page's tree and its current registry session.
        Ok(session.with_admission(Arc::new(PageProtocol {
            access: self.clone(),
            root_session: root.session_id,
            frame: Some((frame_id, endpoint)),
        })))
    }
}
struct Admission<'a> {
    access: &'a PageAccess,
    scope: &'a CapturedScope,
    windows: &'a [NativeWindow],
}
impl PageAdmission for Admission<'_> {
    fn admit(&self, fresh: &PageInfo) -> Result<(), CoreError> {
        self.access.view.check_epoch(self.scope)?;
        self.scope.admit_page(fresh, self.windows)?;
        if fresh.target_id != self.access.target {
            return Err(TargetError::PageTargetChanged(fresh.page_id.clone()).into());
        }
        Ok(())
    }
}
struct PageProtocol {
    access: Arc<PageAccess>,
    root_session: SessionId,
    frame: Option<(FrameId, SessionId)>,
}
impl ProtocolAdmission for PageProtocol {
    fn check<'a>(
        &'a self,
        method: &'a str,
        params: &'a Value,
    ) -> BoxFuture<'a, Result<(), CoreError>> {
        Box::pin(async move {
            let fresh = self.access.fresh().await?;
            cdp_policy::page_method(method, params)?;
            let owner = self
                .access
                .view
                .session
                .pages
                .page_for_session(&self.root_session)
                .await;
            if owner.is_none_or(|owner| {
                owner.page_id != fresh.page_id || owner.target_id != self.access.target
            }) {
                return Err(TargetError::PageTargetChanged(fresh.page_id).into());
            }
            if let Some((frame, endpoint)) = &self.frame {
                let root = ProtocolSession::for_session(
                    self.access.view.session.connection(),
                    self.root_session.clone(),
                );
                let tree = root.send_value("Page.getFrameTree", json!({})).await?;
                if !contains_frame(&tree["frameTree"], frame.as_str())
                    || !self
                        .access
                        .view
                        .session
                        .frames()
                        .frame_session_matches(frame, endpoint)
                        .await
                {
                    return Err(TargetError::PageTargetChanged(fresh.page_id).into());
                }
                self.access.fresh().await?;
            }
            Ok(())
        })
    }
}
fn contains_frame(tree: &Value, id: &str) -> bool {
    tree["frame"]["id"] == id
        || tree["childFrames"]
            .as_array()
            .is_some_and(|children| children.iter().any(|child| contains_frame(child, id)))
}
