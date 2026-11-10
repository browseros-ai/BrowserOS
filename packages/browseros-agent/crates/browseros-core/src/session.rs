use crate::{
    CoreError, NativeCapabilities, PageId, SessionId, TargetId,
    connection::CdpConnection,
    frames::FrameRegistry,
    input::Input,
    navigation::Navigation,
    observer::Observer,
    page_signals::PageSignals,
    pages::{PageManager, PageManagerHooks},
    screenshot::{
        ScreenshotCaptureOptions, ScreenshotCaptureResult, capture_screenshot_with_annotations,
    },
    windows::WindowManager,
};
use browseros_cdp::{CdpError, CdpEvent};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use tokio::sync::{Mutex, broadcast, mpsc};

#[derive(Clone, Default)]
pub struct BrowserSessionHooks {
    pub page_manager: PageManagerHooks,
}

/// Shared process runtime and global page/frame registries. Hosts use this for infrastructure;
/// task execution receives a BrowserView so capture and admission cannot be bypassed.
pub struct BrowserSession {
    connection: Arc<dyn CdpConnection>,
    /// Native identity of the browser behind `connection`, fixed for the session. Every view
    /// reads it through its session, so capture, dispatch and raw CDP share one gate decision;
    /// hosts derive discovery from the same value they pass in here.
    native: NativeCapabilities,
    pub(crate) file_transfers: Arc<crate::file_transfers::TransferState>,
    pub pages: Arc<PageManager>,
    pub page_signals: Arc<PageSignals>,
    pub windows: Arc<WindowManager>,
    frames: Arc<FrameRegistry>,
    observers: Mutex<HashMap<PageId, Arc<Observer>>>,
}

impl BrowserSession {
    /// A session whose native identity is unknown, so no gated native feature is used.
    /// Hosts that know the browser's version use [`Self::new_with_native`].
    #[must_use]
    pub fn new(connection: Arc<dyn CdpConnection>, hooks: BrowserSessionHooks) -> Arc<Self> {
        Self::new_with_native(connection, hooks, NativeCapabilities::default())
    }

    #[must_use]
    pub fn new_with_native(
        connection: Arc<dyn CdpConnection>,
        hooks: BrowserSessionHooks,
        native: NativeCapabilities,
    ) -> Arc<Self> {
        let session = Arc::new_cyclic(|session| {
            Self::assemble(
                Arc::new(EvictingConnection {
                    inner: connection,
                    session: session.clone(),
                }),
                hooks,
                native,
            )
        });
        Self::spawn_detach_listener(session.clone());
        session
    }

    /// Builds the registries over the evicting `connection`. Every registry must get this one,
    /// because a registry handed the host's connection would keep routing to dead ids.
    fn assemble(
        connection: Arc<dyn CdpConnection>,
        hooks: BrowserSessionHooks,
        native: NativeCapabilities,
    ) -> Self {
        let file_transfers = crate::file_transfers::TransferState::new(connection.clone());
        let frames = FrameRegistry::new(connection.clone());
        let page_signals = PageSignals::new(connection.clone());
        let frame_hook = {
            let frames = frames.clone();
            let page_signals = page_signals.clone();
            let user_hook = hooks.page_manager.on_session_attached.clone();
            Arc::new(
                move |session: crate::ProtocolSession,
                      page_id: PageId,
                      session_id: crate::SessionId| {
                    let frames = frames.clone();
                    let page_signals = page_signals.clone();
                    let user_hook = user_hook.clone();
                    Box::pin(async move {
                        page_signals.attach_page(page_id.clone(), session_id.clone());
                        frames
                            .register_page(session.clone(), page_id.clone(), session_id.clone())
                            .await?;
                        if let Some(user_hook) = user_hook {
                            user_hook(session, page_id, session_id).await?;
                        }
                        Ok(())
                    })
                        as futures_util::future::BoxFuture<'static, Result<(), CoreError>>
                },
            )
        };
        let detach_hook = {
            let page_signals = page_signals.clone();
            let user_hook = hooks.page_manager.on_page_detached.clone();
            Arc::new(move |page_id: PageId| {
                page_signals.forget_page(&page_id);
                if let Some(user_hook) = &user_hook {
                    user_hook(page_id);
                }
            })
        };
        let page_hooks = PageManagerHooks {
            on_session_attached: Some(frame_hook),
            on_page_detached: Some(detach_hook),
        };
        let pages = Arc::new(
            PageManager::new(connection.clone(), page_hooks)
                .with_file_transfers(file_transfers.clone()),
        );
        let windows = Arc::new(WindowManager::new(connection.clone()));
        Self {
            connection,
            native,
            file_transfers,
            pages,
            page_signals,
            windows,
            frames,
            observers: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn native(&self) -> NativeCapabilities {
        self.native
    }

    pub(crate) fn frames(&self) -> &Arc<FrameRegistry> {
        &self.frames
    }

    pub async fn observe(&self, page_id: PageId) -> Arc<Observer> {
        let mut observers = self.observers.lock().await;
        if let Some(observer) = observers.get(&page_id) {
            return observer.clone();
        }
        let observer = Arc::new(Observer::new(
            self.pages.clone(),
            self.frames.clone(),
            page_id.clone(),
        ));
        observers.insert(page_id, observer.clone());
        observer
    }

    pub async fn input(&self, page_id: PageId) -> Input {
        Input::new(
            self.observe(page_id.clone()).await,
            self.pages.clone(),
            page_id,
        )
    }

    #[must_use]
    pub fn nav(&self, page_id: PageId) -> Navigation {
        Navigation::new(self.pages.clone(), self.page_signals.clone(), page_id)
    }

    pub async fn screenshot(
        &self,
        page_id: PageId,
        options: ScreenshotCaptureOptions,
    ) -> Result<ScreenshotCaptureResult, CoreError> {
        let page = self.pages.get_session(page_id.clone()).await?;
        capture_screenshot_with_annotations(page.session, self.observe(page_id).await, options)
            .await
    }

    /// Captures only while the reusable page id still names the expected target incarnation.
    /// The checks bracket CDP capture so a rebind can neither capture the replacement nor
    /// publish a result for the old target after it closes.
    pub async fn screenshot_for_target(
        &self,
        page_id: PageId,
        target_id: &TargetId,
        options: ScreenshotCaptureOptions,
    ) -> Result<Option<ScreenshotCaptureResult>, CoreError> {
        let live = self.pages.refresh(page_id.clone()).await?;
        if live
            .as_ref()
            .is_none_or(|page| page.target_id.as_str() != target_id.as_str())
        {
            return Ok(None);
        }
        let page = self.pages.get_session(page_id.clone()).await?;
        if page.target_id.as_str() != target_id.as_str() {
            return Ok(None);
        }
        let capture = capture_screenshot_with_annotations(
            page.session,
            self.observe(page_id.clone()).await,
            options,
        )
        .await?;
        let live = self.pages.refresh(page_id).await?;
        if live
            .as_ref()
            .is_none_or(|page| page.target_id.as_str() != target_id.as_str())
        {
            return Ok(None);
        }
        Ok(Some(capture))
    }

    /// A protocol handle for a session the host attached itself, such as an extension service
    /// worker, outside the page registry. Scripts on it still go through [`crate::PageJs`].
    #[must_use]
    pub fn protocol_session(&self, session_id: crate::SessionId) -> crate::ProtocolSession {
        crate::ProtocolSession::for_session(self.connection.clone(), session_id)
    }

    /// The page's main-frame document identity (`<frame id>:<loader id>`, as
    /// snapshot refs carry it). A cross-document navigation changes it; a
    /// same-document one keeps it.
    pub async fn main_document_id(
        &self,
        page_id: PageId,
    ) -> Result<Option<crate::snapshot::DocumentId>, CoreError> {
        let page = self.pages.get_session(page_id).await?;
        crate::observer::main_document_id(&page.session).await
    }

    pub async fn cdp(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&crate::SessionId>,
    ) -> Result<Value, CoreError> {
        self.connection
            .send(method, params, session_id)
            .await
            .map_err(CoreError::from)
    }

    pub async fn cdp_json(
        &self,
        method: &str,
        params_json: &str,
        session_id: Option<&crate::SessionId>,
    ) -> Result<String, CoreError> {
        self.connection
            .send_raw_json(method, params_json, session_id)
            .await
            .map_err(CoreError::from)
    }

    pub async fn cdp_json_for_page(
        &self,
        page_id: PageId,
        method: &str,
        params_json: &str,
    ) -> Result<String, CoreError> {
        let page = self.pages.get_session(page_id).await?;
        self.connection
            .send_raw_json(method, params_json, Some(&page.session_id))
            .await
            .map_err(CoreError::from)
    }

    #[must_use]
    /// The raw protocol connection, for work that addresses a target directly
    /// rather than a page.
    pub fn connection(&self) -> Arc<dyn CdpConnection> {
        self.connection.clone()
    }

    pub fn cdp_events(&self) -> broadcast::Receiver<CdpEvent> {
        self.connection.events()
    }

    /// See [`crate::connection::CdpConnection::events_targeted`].
    #[must_use]
    pub fn cdp_events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.connection.events_targeted(method)
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connection.is_connected()
    }

    pub async fn dispose(&self) {}

    /// Drops `session_id` from every cache that routes commands by session. The detach listener
    /// and the connection's refusal path both use it. Removal is by value, so a late refusal
    /// for a dead session never evicts the session that replaced it. It sends nothing, so the
    /// connection can run it from inside a send without recursing. It takes the page, frame,
    /// signal and file-transfer registry locks, so no code may hold one of those locks across a
    /// send on this session's connection.
    pub(crate) async fn forget_session(&self, session_id: &SessionId) {
        self.page_signals.detach_session(session_id);
        self.file_transfers.forget_session(session_id);
        self.pages.detach_session(session_id).await;
        self.frames.forget_session(session_id).await;
    }

    fn spawn_detach_listener(session: Arc<Self>) {
        let mut events = session.connection.events();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if event.method == "Target.detachedFromTarget" {
                            handle_detached_event(&session, event).await;
                        }
                    }
                    // Survive a CDP event-buffer overflow on a frame-heavy page instead of
                    // leaving detached sessions uncleaned for the rest of the session.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}

async fn handle_detached_event(session: &BrowserSession, event: CdpEvent) {
    let session_id = event
        .params
        .get("sessionId")
        .and_then(Value::as_str)
        .map(SessionId::from);
    if let Some(session_id) = session_id {
        session.forget_session(&session_id).await;
    }
}

/// The host's connection as this session's registries see it. A detach event is not the only
/// notice that a session died. The event can be lost to a lagged listener, or land before the
/// attach that caches the id, and the registries would then route every later command on the
/// tab to the dead id. Chromium still refuses each such command with "no session with given
/// id", so a refusal forgets the session it was sent on before the caller sees it, and the
/// caller's next attempt attaches anew. Hosts pass their own connection and never see this.
struct EvictingConnection {
    inner: Arc<dyn CdpConnection>,
    /// Weak because the session owns this connection through its registries.
    session: Weak<BrowserSession>,
}

impl EvictingConnection {
    /// Awaits one send, forgetting the session it went to when Chromium refused it as gone.
    async fn evicting<T>(
        &self,
        session: Option<&SessionId>,
        names_other: impl FnOnce() -> bool,
        reply: BoxFuture<'_, Result<T, CdpError>>,
    ) -> Result<T, CdpError> {
        let reply = reply.await;
        if matches!(reply, Err(CdpError::SessionGone))
            && let Some(session_id) = session
            && !names_other()
            && let Some(owner) = self.session.upgrade()
        {
            owner.forget_session(session_id).await;
        }
        reply
    }
}

/// Whether `params` name a session other than the one the command went to, as
/// `Target.detachFromTarget` sent on a parent does. Chromium's refusal is then about the named
/// session, so the one the command was sent on stays.
fn names_other_session(params: &Value, session: Option<&SessionId>) -> bool {
    params
        .get("sessionId")
        .and_then(Value::as_str)
        .is_some_and(|named| session.is_none_or(|session| session.as_str() != named))
}

fn raw_names_other_session(params_json: &str, session: Option<&SessionId>) -> bool {
    serde_json::from_str::<Value>(params_json)
        .is_ok_and(|params| names_other_session(&params, session))
}

impl CdpConnection for EvictingConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        let names_other = names_other_session(&params, session);
        Box::pin(self.evicting(
            session,
            move || names_other,
            self.inner.send(method, params, session),
        ))
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(self.evicting(
            session,
            move || raw_names_other_session(params_json, session),
            self.inner.send_raw_json(method, params_json, session),
        ))
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        let names_other = names_other_session(&params, session);
        Box::pin(
            self.evicting(
                session,
                move || names_other,
                self.inner
                    .send_at_epoch(expected_epoch, method, params, session),
            ),
        )
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(
            self.evicting(
                session,
                move || raw_names_other_session(params_json, session),
                self.inner
                    .send_raw_json_at_epoch(expected_epoch, method, params_json, session),
            ),
        )
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.inner.events()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.inner.events_targeted(method)
    }

    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    fn connection_epoch(&self) -> u64 {
        self.inner.connection_epoch()
    }
}
