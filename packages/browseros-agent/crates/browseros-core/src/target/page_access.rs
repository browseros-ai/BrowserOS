//! Reusable page capabilities retain both the call's scope and the page's target incarnation.
//! Observer state remains shared; only acquisition and protocol dispatch carry these guards.
use super::{BrowserView, CapturedScope, NativeWindow, cdp_policy};
use crate::{
    CoreError, FrameId, PageAdmission, ProtocolSession, SessionId, TargetError, TargetId,
    connection::ProtocolAdmission,
    pages::{PageInfo, PageSession},
};
use browseros_cdp::CdpError;
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
        }));
        Ok(session)
    }
    /// The endpoint for the main frame (`None`) or one of this page's child frames. A child
    /// is bound to this page through the parents that resolved it: Page.getFrameTree omits
    /// remote frames, so each parent must keep reporting the frame's owner element instead
    /// (see `FrameProtocol`). A same-process child shares its parent's endpoint and admission.
    pub(crate) fn frame_session(
        self: &Arc<Self>,
        session: ProtocolSession,
        frame: Option<FrameId>,
    ) -> BoxFuture<'_, Result<ProtocolSession, CoreError>> {
        // Boxed: ref resolution, input and the run bridge embed this future, and inlining the
        // owner climb grew them enough to overflow a worker stack in debug builds.
        Box::pin(async move {
            let root = self.session().await?;
            let Some(frame) = frame else {
                return if session.same_session(&root.session) {
                    Ok(root.session)
                } else {
                    Err(cdp_policy::unsupported("frame_session"))
                };
            };
            self.child_session(root.session, session, frame).await
        })
    }

    async fn child_session(
        self: &Arc<Self>,
        root: ProtocolSession,
        session: ProtocolSession,
        frame: FrameId,
    ) -> Result<ProtocolSession, CoreError> {
        let frames = self.view.session.frames();
        // Climb the recorded parents to this page's root. Each frame must still be on the
        // endpoint its parent read it through. An OOPIF that detached or re-attached waits for
        // a new capture rather than lending its node ids to another process. An intermediate
        // parent must be an OOPIF's own endpoint, so the climb only reaches targets recorded
        // under this page.
        let mut child = frame.clone();
        let mut endpoint = session.session_id().cloned();
        let mut chain = vec![(frame, session)];
        loop {
            let link = frames
                .frame_link(&child)
                .await
                .filter(|link| endpoint.as_ref() == Some(&link.endpoint))
                .ok_or_else(|| self.refused(&child))?;
            if root.session_id() == Some(&link.parent) {
                break;
            }
            let owner = frames
                .oopif_for_session(&link.parent)
                .await
                .filter(|owner| chain.iter().all(|(seen, _)| seen != owner))
                .ok_or_else(|| self.refused(&child))?;
            let parent =
                ProtocolSession::for_session(self.view.session.connection(), link.parent.clone());
            chain.push((owner.clone(), parent));
            child = owner;
            endpoint = Some(link.parent);
        }
        // Down from the root, each owner is read through a parent the hops above have just
        // vouched for; the capability then rechecks every hop on each call.
        let mut hops = Vec::new();
        let mut admitted = root;
        for (frame, session) in chain.into_iter().rev() {
            // A same-process child shares its parent's endpoint and so its admission.
            if session.same_session(&admitted) {
                continue;
            }
            let (Some(parent), Some(endpoint)) = (
                admitted.session_id().cloned(),
                session.session_id().cloned(),
            ) else {
                return Err(self.refused(&frame));
            };
            let backend = self
                .frame_owner(&parent, &frame)
                .await?
                .ok_or_else(|| self.refused(&frame))?;
            hops.push(FrameHop {
                parent,
                frame,
                endpoint,
                backend,
            });
            admitted = self.frame_capability(session, hops.clone())?;
        }
        Ok(admitted)
    }

    pub(crate) fn epoch_guard(&self) -> Result<Option<crate::connection::EpochGuard>, CoreError> {
        self.view.epoch_guard()
    }

    /// `session` admitted as a child frame endpoint through `hops`, root first.
    pub(super) fn frame_capability(
        self: &Arc<Self>,
        session: ProtocolSession,
        hops: Vec<FrameHop>,
    ) -> Result<ProtocolSession, CoreError> {
        Ok(session
            .with_epoch_guard(self.epoch_guard()?)
            .with_admission(Arc::new(FrameProtocol {
                access: self.clone(),
                hops,
            })))
    }

    fn refused(&self, frame: &FrameId) -> CoreError {
        TargetError::FrameNotInPage {
            page: self.page.clone(),
            frame: frame.clone(),
        }
        .into()
    }

    /// The owner element `parent` reports for `frame`, asked on the bare endpoint: callers
    /// have already vouched for `parent`, and its own admission would recheck every hop above
    /// it. A protocol error means `parent` no longer holds the frame, a refusal like another
    /// owner; transport errors keep their own type.
    async fn frame_owner(
        &self,
        parent: &SessionId,
        frame: &FrameId,
    ) -> Result<Option<i64>, CoreError> {
        let parent = ProtocolSession::for_session(self.view.session.connection(), parent.clone())
            .with_epoch_guard(self.epoch_guard()?);
        match parent
            .send_value("DOM.getFrameOwner", json!({"frameId":frame.as_str()}))
            .await
        {
            Ok(owner) => Ok(owner["backendNodeId"].as_i64()),
            Err(CoreError::Cdp(CdpError::Protocol { .. })) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The page still owns its root endpoint at the captured incarnation.
    async fn owns_root(&self, root: &SessionId, fresh: &PageInfo) -> Result<(), CoreError> {
        let owner = self.view.session.pages.page_for_session(root).await;
        if owner
            .is_none_or(|owner| owner.page_id != fresh.page_id || owner.target_id != self.target)
        {
            return Err(TargetError::PageTargetChanged(fresh.page_id.clone()).into());
        }
        Ok(())
    }
}
/// One step of a child frame endpoint's admission: `parent` must report `frame`'s owner
/// element as `backend`, and the registry must still map `frame` to `endpoint`.
#[derive(Clone)]
pub(super) struct FrameHop {
    pub(super) parent: SessionId,
    pub(super) frame: FrameId,
    pub(super) endpoint: SessionId,
    pub(super) backend: i64,
}
/// Rechecks a child frame endpoint in one pass before and after every call: the page and its
/// root endpoint as `PageProtocol` does, every hop from the root down, then the page again.
/// Asking each parent through its own admission instead rechecked all hops above it twice,
/// doubling the cost with each level of nesting.
struct FrameProtocol {
    access: Arc<PageAccess>,
    hops: Vec<FrameHop>,
}
impl ProtocolAdmission for FrameProtocol {
    fn check<'a>(
        &'a self,
        method: &'a str,
        params: &'a Value,
    ) -> BoxFuture<'a, Result<(), CoreError>> {
        Box::pin(async move {
            let fresh = self.access.fresh().await?;
            cdp_policy::page_method(method, params)?;
            let root = self
                .hops
                .first()
                .map(|hop| &hop.parent)
                .ok_or_else(|| cdp_policy::unsupported("frame_session"))?;
            self.access.owns_root(root, &fresh).await?;
            let frames = self.access.view.session.frames();
            for hop in &self.hops {
                if self.access.frame_owner(&hop.parent, &hop.frame).await? != Some(hop.backend)
                    || !frames
                        .frame_session_matches(&hop.frame, &hop.endpoint)
                        .await
                {
                    return Err(self.access.refused(&hop.frame));
                }
            }
            // The owner reads are round trips: the page may have moved during them.
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
            self.access.owns_root(&self.root_session, &fresh).await
        })
    }
}

#[cfg(test)]
mod depth_tests;

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

    /// A page `root` embedding the OOPIF `cross`, which holds the same-process frame `same`
    /// and the OOPIF `deep`: A -> B -> {B, A}. `split_tree` answers Page.getFrameTree as
    /// Chromium does, without remote frames, so only DOM owners connect the sessions.
    struct Native {
        tree: Mutex<Value>,
        split_tree: std::sync::atomic::AtomicBool,
        owner_changed: std::sync::atomic::AtomicBool,
        tab: Mutex<Value>,
        calls: Mutex<Vec<(String, Option<SessionId>)>>,
        mouse: Mutex<Vec<(Option<SessionId>, Value)>>,
        epoch: AtomicU64,
        events: broadcast::Sender<CdpEvent>,
    }

    impl Native {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                tree: Mutex::new(json!({"frame":{"id":"root"},"childFrames":[{
                    "frame":{"id":"cross"},"childFrames":[{"frame":{"id":"same"}},{"frame":{"id":"deep"}}]
                }]})),
                split_tree: std::sync::atomic::AtomicBool::new(false),
                owner_changed: std::sync::atomic::AtomicBool::new(false),
                tab: Mutex::new(json!({"tabId":1,"targetId":"page","windowId":11,
                    "url":"https://local.test","title":"test","isActive":true,"isLoading":false,
                    "loadProgress":1.0,"isPinned":false,"isHidden":false,
                    "browserProfileId":"eng","liveBrowserContextId":"live-eng"})),
                calls: Mutex::new(Vec::new()),
                mouse: Mutex::new(Vec::new()),
                epoch: AtomicU64::new(1),
                events: broadcast::channel(32).0,
            })
        }

        fn called(&self, method: &str) -> Result<bool, Box<dyn std::error::Error>> {
            Ok(self
                .calls
                .lock()
                .map_err(|_| "poisoned")?
                .iter()
                .any(|(called, _)| called == method))
        }

        /// The target an endpoint is attached to: `session-cross-2` re-attaches `cross`.
        fn target_of(session: Option<&SessionId>) -> Option<&str> {
            session.map(|id| id.as_str().strip_suffix("-2").unwrap_or(id.as_str()))
        }

        /// The document each session or same-process frame shows: its controls, then owners.
        fn ax_tree(params: &Value, session: Option<&SessionId>) -> Value {
            let (button, name, backend, owners): (_, _, _, &[(&str, i64)]) =
                match (Self::target_of(session), params["frameId"].as_str()) {
                    (Some("session-cross"), Some("same")) => {
                        ("same-button", "Nested frame target", 21, &[])
                    }
                    (Some("session-cross"), None) => (
                        "cross-button",
                        "Remote frame target",
                        11,
                        &[("same-owner", 2), ("deep-owner", 3)],
                    ),
                    (Some("session-deep"), None) => ("deep-button", "Deep frame target", 31, &[]),
                    _ => ("root-button", "Root target", 10, &[("cross-owner", 1)]),
                };
            let mut children = vec![button];
            children.extend(owners.iter().map(|(id, _)| *id));
            let mut nodes = vec![
                json!({"nodeId":"document","role":{"type":"internalRole","value":"RootWebArea"},"childIds":children}),
                json!({"nodeId":button,"backendDOMNodeId":backend,"role":{"type":"internalRole","value":"button"},"name":{"type":"computedString","value":name}}),
            ];
            nodes.extend(owners.iter().map(|(id, backend)| {
                json!({"nodeId":id,"backendDOMNodeId":backend,"role":{"type":"internalRole","value":"Iframe"},"name":{"type":"computedString","value":id}})
            }));
            json!({ "nodes": nodes })
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
                        match Self::target_of(session) {
                            Some("session-cross") => {
                                json!({"frameTree":{"frame":{"id":"cross"},"childFrames":[{"frame":{"id":"same"}}]}})
                            }
                            Some("session-deep") => json!({"frameTree":{"frame":{"id":"deep"}}}),
                            _ => json!({"frameTree":{"frame":{"id":"root"}}}),
                        }
                    } else {
                        json!({"frameTree":self.tree.lock().map_err(|_| CdpError::SessionGone)?.clone()})
                    }
                }
                "DOM.getDocument" => {
                    if Self::target_of(session) == Some("session-cross") {
                        json!({"root":{"nodeType":9,"children":[
                            {"nodeName":"IFRAME","frameId":"same","backendNodeId":2,"contentDocument":{"nodeType":9}},
                            {"nodeName":"IFRAME","frameId":"deep","backendNodeId":3}
                        ]}})
                    } else if session.is_some_and(|id| id.as_str() == "session-deep") {
                        json!({"root":{"nodeType":9}})
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
                    // Like Chromium, only the endpoint whose document holds the owner answers.
                    let backend = match (Self::target_of(session), params["frameId"].as_str()) {
                        (Some("session-page"), Some("cross")) => 1,
                        (Some("session-cross"), Some("same")) => 2,
                        (Some("session-cross"), Some("deep")) => 3,
                        _ => {
                            return Err(CdpError::Protocol {
                                code: -32000,
                                message: "Frame with the given id does not belong to the target."
                                    .into(),
                            });
                        }
                    };
                    json!({"backendNodeId":if self.owner_changed.load(Ordering::SeqCst) { 99 } else { backend }})
                }
                "DOM.describeNode" => match params["backendNodeId"].as_i64() {
                    Some(1) => json!({"node":{"frameId":"cross"}}),
                    Some(2) => json!({"node":{"contentDocument":{"frameId":"same"}}}),
                    Some(3) => json!({"node":{"frameId":"deep"}}),
                    _ => json!({"node":{}}),
                },
                "Accessibility.getFullAXTree" => Self::ax_tree(&params, session),
                "DOM.resolveNode" => json!({"object":{"objectId":"node"}}),
                "DOM.getContentQuads" => {
                    // Centered on (backend, backend) in the endpoint's own coordinates.
                    let at = params["backendNodeId"].as_f64().unwrap_or_default();
                    json!({"quads":[[at - 1.0, at - 1.0, at + 1.0, at - 1.0, at + 1.0, at + 1.0, at - 1.0, at + 1.0]]})
                }
                "Input.dispatchMouseEvent" => {
                    self.mouse
                        .lock()
                        .map_err(|_| CdpError::SessionGone)?
                        .push((session.cloned(), params));
                    json!({})
                }
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
        fixture_in(true).await
    }

    /// `bound` views are profile-scoped and admit frames; unbound ones, on a browser without
    /// profile targeting, have no admission at all.
    async fn fixture_in(
        bound: bool,
    ) -> Result<(Arc<Native>, BrowserView, crate::PageId), CoreError> {
        let native = Native::new();
        let version = bound.then_some("0.51.1.3");
        let browser = Browser::new_with_native(
            native.clone(),
            NativeCapabilities::from_browseros_version(version),
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
            Err(CoreError::Target(TargetError::FrameNotInPage { .. }))
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

    /// Breaks one link a frame endpoint's admission depends on.
    async fn apply_change(native: &Native, view: &BrowserView, change: &str) -> TestResult {
        match change {
            "owner" => native.owner_changed.store(true, Ordering::SeqCst),
            "tree" => *native.tree.lock().map_err(|_| "poisoned")? = json!({"frame":{"id":"root"}}),
            "scope" => {
                native.tab.lock().map_err(|_| "poisoned")?["liveBrowserContextId"] = json!("other")
            }
            "target" => native.tab.lock().map_err(|_| "poisoned")?["targetId"] = json!("replaced"),
            "epoch" => {
                native.epoch.fetch_add(1, Ordering::SeqCst);
            }
            "session" => {
                // The OOPIF re-attaches under a new endpoint, as after a detach and recovery.
                native.events.send(CdpEvent {
                    method: "Target.attachedToTarget".into(),
                    params: json!({"sessionId":"session-cross-2","waitingForDebugger":false,
                        "targetInfo":{"targetId":"cross","type":"iframe","title":"","url":"",
                            "attached":true,"canAccessOpener":false}}),
                    session_id: Some("session-page".into()),
                })?;
                registry_settled(view, "session-cross-2", true).await?;
            }
            "detach" => {
                native.events.send(CdpEvent {
                    method: "Target.detachedFromTarget".into(),
                    params: json!({"sessionId":"session-cross"}),
                    session_id: Some("session-page".into()),
                })?;
                registry_settled(view, "session-cross", false).await?;
            }
            other => return Err(format!("unknown change {other}").into()),
        }
        Ok(())
    }

    /// Waits for the registry's event listener to map `cross` to `session`, or to drop it.
    async fn registry_settled(view: &BrowserView, session: &str, mapped: bool) -> TestResult {
        let (frame, session) = (FrameId("cross".into()), SessionId::from(session));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while view
                .session
                .frames()
                .frame_session_matches(&frame, &session)
                .await
                != mapped
            {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }

    fn ref_named(
        snapshot: &crate::observer::SnapshotResult,
        name: &str,
    ) -> Result<crate::Ref, String> {
        snapshot
            .refs
            .entries_in_order()
            .into_iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.ref_id.clone())
            .ok_or_else(|| format!("no ref for {name:?} in:\n{}", snapshot.text))
    }

    #[tokio::test]
    async fn tests_that_scoped_snapshots_read_frames_missing_from_the_page_tree() -> TestResult {
        let (native, view, page) = fixture().await?;
        native.split_tree.store(true, Ordering::SeqCst);
        let snapshot = view.observe(page).await?.snapshot().await?;
        for (name, frame) in [
            ("Root target", None),
            ("Remote frame target", Some("cross")),
            ("Nested frame target", Some("same")),
            ("Deep frame target", Some("deep")),
        ] {
            let reference = ref_named(&snapshot, name)?;
            let entry = snapshot.refs.get(&reference).ok_or("ref vanished")?;
            assert_eq!(
                entry.frame_id.as_ref().map(FrameId::as_str),
                frame,
                "{name}"
            );
        }
        // Owners are proven through DOM.getFrameOwner, never by serializing whole documents.
        assert!(!native.called("DOM.getDocument")?);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_scoped_frame_refs_click_natively_through_their_own_session() -> TestResult {
        let (native, view, page) = fixture().await?;
        native.split_tree.store(true, Ordering::SeqCst);
        let observer = view.observe(page.clone()).await?;
        let snapshot = observer.snapshot().await?;
        let input = view.input(page).await?;
        for (name, session, backend) in [
            ("Root target", "session-page", 10.0),
            ("Remote frame target", "session-cross", 11.0),
            // Same-process frames inside an OOPIF share their OOPIF's endpoint.
            ("Nested frame target", "session-cross", 21.0),
            ("Deep frame target", "session-deep", 31.0),
        ] {
            let reference = ref_named(&snapshot, name)?;
            let resolved = observer.resolve_ref(&reference).await?;
            assert_eq!(
                resolved.session.session_id().map(SessionId::as_str),
                Some(session),
                "{name}"
            );
            native.mouse.lock().map_err(|_| "poisoned")?.clear();
            let point = input
                .click(&reference, crate::input::ClickOptions::default())
                .await?;
            // A dispatched point means trusted mouse input; the scripted fallback returns None.
            let expected = crate::input::Point {
                x: backend,
                y: backend,
            };
            assert_eq!(point, Some(expected), "{name}");
            let mouse = native.mouse.lock().map_err(|_| "poisoned")?.clone();
            let pressed = mouse
                .iter()
                .find(|(_, event)| event["type"] == "mousePressed")
                .ok_or_else(|| format!("{name}: no mousePressed in {mouse:?}"))?;
            // Frame-local coordinates go to the same endpoint, untransformed.
            assert_eq!(pressed.0.as_ref().map(SessionId::as_str), Some(session));
            assert_eq!(
                (pressed.1["x"].as_f64(), pressed.1["y"].as_f64()),
                (Some(backend), Some(backend))
            );
            assert!(
                mouse
                    .iter()
                    .all(|(endpoint, _)| endpoint.as_ref().map(SessionId::as_str) == Some(session)),
                "{name}: {mouse:?}"
            );
        }
        assert!(!native.called("DOM.getDocument")?);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_retained_frame_refs_recheck_owner_scope_session_and_epoch() -> TestResult {
        for name in [
            "Remote frame target",
            "Nested frame target",
            "Deep frame target",
        ] {
            for change in [
                "owner", "tree", "scope", "target", "epoch", "session", "detach",
            ] {
                let (native, view, page) = fixture().await?;
                native.split_tree.store(true, Ordering::SeqCst);
                let observer = view.observe(page.clone()).await?;
                let reference = ref_named(&observer.snapshot().await?, name)?;
                let resolved = observer.resolve_ref(&reference).await?;
                apply_change(&native, &view, change).await?;
                let result = resolved
                    .session
                    .send_value("Runtime.evaluate", json!({"expression":"1"}))
                    .await;
                assert!(result.is_err(), "{name}/{change}: {result:?}");
                assert!(!native.called("Runtime.evaluate")?, "{name}/{change}");
                match change {
                    // Admission refusals are typed, so callers can tell them from the browser.
                    "owner" | "session" | "detach" => assert!(
                        matches!(
                            result,
                            Err(CoreError::Target(TargetError::FrameNotInPage { .. }))
                        ),
                        "{name}/{change}: {result:?}"
                    ),
                    // A transport failure while asking for the owner keeps its own type.
                    "tree" => assert!(
                        matches!(result, Err(CoreError::Cdp(CdpError::SessionGone))),
                        "{name}/{change}: {result:?}"
                    ),
                    _ => {}
                }
                if change == "owner" {
                    // The frame is still this page's through its current owner, which a fresh
                    // resolution verifies anew; only the retained endpoint was bound to the old one.
                    continue;
                }
                // Re-resolving is refused too: nothing rebinds the ref to whatever is there now.
                let again = match view.observe(page).await {
                    Ok(observer) => observer.resolve_ref(&reference).await.map(|_| ()),
                    Err(error) => Err(error),
                };
                assert!(again.is_err(), "{name}/{change}: {again:?}");
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_detached_oopif_refs_never_fall_back_to_the_page_session() -> TestResult {
        // Unbound views have no frame admission, so the registry alone must refuse.
        for bound in [true, false] {
            let (native, view, page) = fixture_in(bound).await?;
            native.split_tree.store(true, Ordering::SeqCst);
            let snapshot = view.observe(page.clone()).await?.snapshot().await?;
            apply_change(&native, &view, "detach").await?;
            native.calls.lock().map_err(|_| "poisoned")?.clear();
            let input = view.input(page).await?;
            // The detach dropped the frames' links. Answering them with the page's own
            // endpoint would resolve these node ids against unrelated nodes of another process.
            for name in [
                "Remote frame target",
                "Nested frame target",
                "Deep frame target",
            ] {
                let reference = ref_named(&snapshot, name)?;
                let result = input
                    .click(&reference, crate::input::ClickOptions::default())
                    .await;
                assert!(
                    matches!(
                        result,
                        Err(CoreError::Target(TargetError::FrameNotInPage { .. }))
                    ),
                    "bound={bound} {name}: {result:?}"
                );
            }
            let calls = native.calls.lock().map_err(|_| "poisoned")?.clone();
            assert!(
                !calls.iter().any(|(method, _)| method.starts_with("DOM.")
                    || method.starts_with("Input.")
                    || method.starts_with("Runtime.")),
                "bound={bound}: {calls:?}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_old_refs_stay_refused_after_a_walk_records_a_reattached_frame() -> TestResult
    {
        for bound in [true, false] {
            let (native, view, page) = fixture_in(bound).await?;
            native.split_tree.store(true, Ordering::SeqCst);
            let old = view.observe(page.clone()).await?.snapshot().await?;
            apply_change(&native, &view, "session").await?;
            // A download or upload walk re-resolves `cross` through its parent, which records
            // the new endpoint for the frame. The old ref's node id was read through the old one.
            view.frame_session(page.clone(), FrameId("cross".into()))
                .await?;
            let input = view.input(page.clone()).await?;
            let result = input
                .click(
                    &ref_named(&old, "Remote frame target")?,
                    crate::input::ClickOptions::default(),
                )
                .await;
            assert!(
                matches!(
                    result,
                    Err(CoreError::Target(TargetError::FrameNotInPage { .. }))
                ),
                "bound={bound}: {result:?}"
            );
            assert!(native.mouse.lock().map_err(|_| "poisoned")?.is_empty());

            // A fresh capture reads the frame through its new endpoint, and its refs work.
            let fresh = view.observe(page.clone()).await?.snapshot().await?;
            let point = input
                .click(
                    &ref_named(&fresh, "Remote frame target")?,
                    crate::input::ClickOptions::default(),
                )
                .await?;
            assert_eq!(point, Some(crate::input::Point { x: 11.0, y: 11.0 }));
            let mouse = native.mouse.lock().map_err(|_| "poisoned")?.clone();
            assert!(
                mouse
                    .iter()
                    .all(|(endpoint, _)| endpoint.as_ref().map(SessionId::as_str)
                        == Some("session-cross-2")),
                "bound={bound}: {mouse:?}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_frames_without_a_parent_in_this_page_are_refused() -> TestResult {
        let (native, view, page) = fixture().await?;
        let access = Arc::new(PageAccess::new(
            view.clone(),
            view.page(page.clone()).await?,
        ));
        let foreign = ProtocolSession::for_session(native.clone(), "session-foreign".into());
        assert!(matches!(
            access
                .frame_session(foreign.clone(), Some(FrameId("foreign".into())))
                .await,
            Err(CoreError::Target(TargetError::FrameNotInPage { .. }))
        ));
        // A parent the registry knows but that no OOPIF of this page owns vouches for nothing.
        let other_page = ProtocolSession::for_session(native.clone(), "session-other".into());
        view.session
            .frames()
            .resolve_frame_target(
                page,
                Some(FrameId("foreign".into())),
                Some(&other_page),
                false,
            )
            .await?;
        assert!(matches!(
            access
                .frame_session(foreign, Some(FrameId("foreign".into())))
                .await,
            Err(CoreError::Target(TargetError::FrameNotInPage { .. }))
        ));
        assert!(
            !native
                .calls
                .lock()
                .map_err(|_| "poisoned")?
                .iter()
                .any(|(_, session)| session.as_ref().is_some_and(|id| {
                    matches!(id.as_str(), "session-foreign" | "session-other")
                }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_retained_frames_recheck_membership_scope_and_epoch() -> TestResult {
        for frame in ["same", "cross"] {
            for change in ["tree", "scope", "epoch", "target", "owner"] {
                let (native, view, page) = fixture().await?;
                let session = view.frame_session(page, FrameId(frame.into())).await?;
                apply_change(&native, &view, change).await?;
                let result = session
                    .send_value("Runtime.evaluate", json!({"expression":"1"}))
                    .await;
                assert!(result.is_err(), "{frame}/{change}: {result:?}");
                if change == "owner" {
                    assert!(
                        matches!(
                            result,
                            Err(CoreError::Target(TargetError::FrameNotInPage { .. }))
                        ),
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
