//! Reusable page capabilities retain both the call's scope and the page's target incarnation.
//! Observer state remains shared; only acquisition and protocol dispatch carry these guards.
use super::{BrowserView, CapturedScope, NativeWindow, cdp_policy, contains_frame};
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
        let live = self.view.validate_live().await?;
        let admission = Admission {
            access: self,
            scope: live.as_ref().map(|live| &live.scope),
            windows: live.as_ref().map_or(&[], |live| live.windows.as_slice()),
        };
        let mut session = self
            .view
            .session
            .pages
            .get_session_admitted_with_epoch(
                self.page.clone(),
                &admission,
                self.view.epoch_guard()?,
            )
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
        Ok(session
            .with_epoch_guard(self.view.epoch_guard()?)
            .with_admission(Arc::new(PageProtocol {
                access: self.clone(),
                root_session: root.session_id,
                frame: Some((frame_id, endpoint)),
            })))
    }

    pub(crate) fn epoch_guard(&self) -> Result<Option<crate::connection::EpochGuard>, CoreError> {
        self.view.epoch_guard()
    }

    pub(super) async fn frame_capability(
        self: &Arc<Self>,
        session: ProtocolSession,
        frame: FrameId,
        parent: ProtocolSession,
        backend: i64,
    ) -> Result<ProtocolSession, CoreError> {
        let endpoint = session
            .session_id()
            .cloned()
            .ok_or_else(|| cdp_policy::unsupported("frame_session"))?;
        Ok(session
            .with_epoch_guard(self.epoch_guard()?)
            .with_admission(Arc::new(FrameProtocol {
                access: self.clone(),
                parent,
                frame,
                endpoint,
                backend,
            })))
    }
}
struct FrameProtocol {
    access: Arc<PageAccess>,
    parent: ProtocolSession,
    frame: FrameId,
    endpoint: SessionId,
    backend: i64,
}
impl ProtocolAdmission for FrameProtocol {
    fn check<'a>(
        &'a self,
        method: &'a str,
        params: &'a Value,
    ) -> BoxFuture<'a, Result<(), CoreError>> {
        Box::pin(async move {
            self.access.fresh().await?;
            cdp_policy::page_method(method, params)?;
            let owner = self
                .parent
                .send_value("DOM.getFrameOwner", json!({"frameId":self.frame.as_str()}))
                .await?;
            if owner["backendNodeId"].as_i64() != Some(self.backend)
                || !self
                    .access
                    .view
                    .session
                    .frames()
                    .frame_session_matches(&self.frame, &self.endpoint)
                    .await
            {
                return Err(CoreError::Message(format!(
                    "Frame {} is no longer part of page {:?}",
                    self.frame.as_str(),
                    self.access.page
                )));
            }
            self.access.fresh().await?;
            Ok(())
        })
    }
}
struct Admission<'a> {
    access: &'a PageAccess,
    scope: Option<&'a CapturedScope>,
    windows: &'a [NativeWindow],
}
impl PageAdmission for Admission<'_> {
    fn admit(&self, fresh: &PageInfo) -> Result<(), CoreError> {
        if let Some(scope) = self.scope {
            self.access.view.check_epoch(scope)?;
            scope.admit_page(fresh, self.windows)?;
        }
        if let Some(guard) = self.access.epoch_guard()? {
            guard.check(self.access.view.session.connection().as_ref())?;
        }
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
                )
                .with_epoch_guard(self.access.epoch_guard()?);
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

#[cfg(test)]
mod frame_tests {
    use super::*;
    use crate::{Browser, CdpConnection, NativeCapabilities, TargetBinding, TargetIntent};
    use browseros_cdp::{CdpError, CdpEvent};
    use std::sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    };
    use tokio::sync::broadcast;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    struct Native {
        tree: Mutex<Value>,
        split_tree: std::sync::atomic::AtomicBool,
        owner_changed: std::sync::atomic::AtomicBool,
        tab: Mutex<Value>,
        calls: Mutex<Vec<(String, Option<SessionId>)>>,
        epoch: AtomicU64,
        events: broadcast::Sender<CdpEvent>,
    }

    impl Native {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                tree: Mutex::new(json!({"frame":{"id":"root"},"childFrames":[{
                    "frame":{"id":"cross"},"childFrames":[{"frame":{"id":"same"}}]
                }]})),
                split_tree: std::sync::atomic::AtomicBool::new(false),
                owner_changed: std::sync::atomic::AtomicBool::new(false),
                tab: Mutex::new(json!({"tabId":1,"targetId":"page","windowId":11,
                    "url":"https://local.test","title":"test","isActive":true,"isLoading":false,
                    "loadProgress":1.0,"isPinned":false,"isHidden":false,
                    "browserProfileId":"eng","liveBrowserContextId":"live-eng"})),
                calls: Mutex::new(Vec::new()),
                epoch: AtomicU64::new(1),
                events: broadcast::channel(32).0,
            })
        }

        fn reply(
            &self,
            expected: Option<u64>,
            method: &str,
            params: Value,
            session: Option<&SessionId>,
        ) -> Result<Value, CdpError> {
            if let Some(expected) = expected {
                let actual = self.connection_epoch();
                if expected != actual {
                    return Err(CdpError::ConnectionEpochChanged { expected, actual });
                }
            }
            self.calls
                .lock()
                .map_err(|_| CdpError::SessionGone)?
                .push((method.into(), session.cloned()));
            Ok(match method {
                "Browser.getProfiles" => {
                    json!({"profiles":[{"browserProfileId":"eng","name":"eng","directory":"directory-eng","loaded":true,"browserContextId":"live-eng"}]})
                }
                "Browser.getWindows" => {
                    json!({"windows":[{"windowId":11,"windowType":"normal","bounds":{},"isActive":true,"isVisible":true,"tabCount":1,"browserProfileId":"eng","liveBrowserContextId":"live-eng"}]})
                }
                "Browser.getTabs" => {
                    json!({"tabs":[self.tab.lock().map_err(|_| CdpError::SessionGone)?.clone()]})
                }
                "Browser.getTabInfo" => {
                    json!({"tab":self.tab.lock().map_err(|_| CdpError::SessionGone)?.clone()})
                }
                "Target.attachToTarget" => {
                    json!({"sessionId":format!("session-{}", params["targetId"].as_str().unwrap_or_default())})
                }
                "Page.getFrameTree" => {
                    if self.split_tree.load(Ordering::SeqCst) {
                        if session.is_some_and(|id| id.as_str() == "session-cross") {
                            json!({"frameTree":{"frame":{"id":"cross"},"childFrames":[{"frame":{"id":"same"}}]}})
                        } else {
                            json!({"frameTree":{"frame":{"id":"root"}}})
                        }
                    } else {
                        json!({"frameTree":self.tree.lock().map_err(|_| CdpError::SessionGone)?.clone()})
                    }
                }
                "DOM.getDocument" => {
                    if session.is_some_and(|id| id.as_str() == "session-cross") {
                        json!({"root":{"nodeType":9,"children":[{"nodeName":"IFRAME","frameId":"same","backendNodeId":2,"contentDocument":{"nodeType":9}}]}})
                    } else if self.tree.lock().map_err(|_| CdpError::SessionGone)?["childFrames"]
                        .is_array()
                    {
                        json!({"root":{"nodeType":9,"children":[{"nodeName":"DIV","shadowRoots":[{"shadowRootType":"closed","children":[{"nodeName":"IFRAME","frameId":"cross","backendNodeId":1}]}]}]}})
                    } else {
                        json!({"root":{"nodeType":9}})
                    }
                }
                "DOM.getFrameOwner" => {
                    if !self.tree.lock().map_err(|_| CdpError::SessionGone)?["childFrames"]
                        .is_array()
                    {
                        return Err(CdpError::SessionGone);
                    }
                    json!({"backendNodeId":if self.owner_changed.load(Ordering::SeqCst) { 99 } else if params["frameId"] == "same" { 2 } else { 1 }})
                }
                "DOM.describeNode" if params["backendNodeId"] == 2 => {
                    json!({"node":{"contentDocument":{"frameId":"same"}}})
                }
                "DOM.describeNode" => json!({"node":{"frameId":"cross"}}),
                "Runtime.evaluate" => json!({"result":{"value":session.map(SessionId::as_str)}}),
                _ => json!({}),
            })
        }
    }

    impl CdpConnection for Native {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move { self.reply(None, method, params, session) })
        }
        fn send_at_epoch<'a>(
            &'a self,
            epoch: u64,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move { self.reply(Some(epoch), method, params, session) })
        }
        fn send_raw_json<'a>(
            &'a self,
            method: &'a str,
            params: &'a str,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async move {
                Ok(serde_json::to_string(&self.reply(
                    None,
                    method,
                    serde_json::from_str(params)?,
                    session,
                )?)?)
            })
        }
        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.events.subscribe()
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn connection_epoch(&self) -> u64 {
            self.epoch.load(Ordering::SeqCst)
        }
    }

    async fn fixture() -> Result<(Arc<Native>, BrowserView, crate::PageId), CoreError> {
        let native = Native::new();
        let browser = Browser::new_with_native(
            native.clone(),
            NativeCapabilities::from_browseros_version(Some("0.51.1.3")),
        );
        browser
            .set_target(Some(TargetBinding {
                intent: TargetIntent {
                    browser_profile_id: crate::BrowserProfileId("eng".into()),
                    directory: "directory-eng".into(),
                    name: "eng".into(),
                    window: None,
                },
                revision: 1,
            }))
            .await;
        let view = browser.view().await;
        let page = view
            .list_pages()
            .await?
            .pages
            .into_iter()
            .next()
            .ok_or("missing page")?
            .page_id;
        Ok((native, view, page))
    }

    #[tokio::test]
    async fn tests_that_nested_same_process_frames_inherit_the_oopif_session() -> TestResult {
        let (_, view, page) = fixture().await?;
        for (frame, expected) in [
            ("root", "session-page"),
            ("cross", "session-cross"),
            ("same", "session-cross"),
        ] {
            let session = view
                .frame_session(page.clone(), FrameId(frame.into()))
                .await?;
            let result = session
                .send_value("Runtime.evaluate", json!({"expression":"location.href"}))
                .await?;
            assert_eq!(result["result"]["value"], expected);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_unknown_frames_are_refused() -> TestResult {
        let (native, view, page) = fixture().await?;
        assert!(matches!(
            view.frame_session(page, FrameId("foreign".into())).await,
            Err(CoreError::Message(_))
        ));
        assert!(
            !native
                .calls
                .lock()
                .map_err(|_| "poisoned")?
                .iter()
                .any(|(_, session)| session
                    .as_ref()
                    .is_some_and(|id| id.as_str() == "session-foreign"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_closed_shadow_oopifs_omitted_from_the_page_tree_are_resolved() -> TestResult
    {
        let (native, view, page) = fixture().await?;
        native.split_tree.store(true, Ordering::SeqCst);
        let session = view.frame_session(page, FrameId("same".into())).await?;
        let result = session
            .send_value("Runtime.evaluate", json!({"expression":"location.href"}))
            .await?;
        assert_eq!(result["result"]["value"], "session-cross");
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_retained_frames_recheck_membership_scope_and_epoch() -> TestResult {
        for frame in ["same", "cross"] {
            for change in ["tree", "scope", "epoch", "target", "owner"] {
                let (native, view, page) = fixture().await?;
                let session = view.frame_session(page, FrameId(frame.into())).await?;
                match change {
                    "owner" => native.owner_changed.store(true, Ordering::SeqCst),
                    "tree" => {
                        *native.tree.lock().map_err(|_| "poisoned")? =
                            json!({"frame":{"id":"root"}})
                    }
                    "scope" => {
                        native.tab.lock().map_err(|_| "poisoned")?["liveBrowserContextId"] =
                            json!("other")
                    }
                    "target" => {
                        native.tab.lock().map_err(|_| "poisoned")?["targetId"] = json!("replaced")
                    }
                    _ => {
                        native.epoch.fetch_add(1, Ordering::SeqCst);
                    }
                }
                let result = session
                    .send_value("Runtime.evaluate", json!({"expression":"1"}))
                    .await;
                assert!(result.is_err(), "{frame}/{change}: {result:?}");
                if change == "owner" {
                    assert!(
                        matches!(result, Err(CoreError::Message(_))),
                        "{frame}: {result:?}"
                    );
                }
                assert!(
                    !native
                        .calls
                        .lock()
                        .map_err(|_| "poisoned")?
                        .iter()
                        .any(|(method, _)| method == "Runtime.evaluate")
                );
            }
        }
        Ok(())
    }
}
