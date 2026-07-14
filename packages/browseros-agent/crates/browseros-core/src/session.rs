use crate::{
    CoreError, PageId,
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
use browseros_cdp::CdpEvent;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, broadcast, mpsc};

#[derive(Clone, Default)]
pub struct BrowserSessionHooks {
    pub page_manager: PageManagerHooks,
}

pub struct BrowserSession {
    connection: Arc<dyn CdpConnection>,
    pub pages: Arc<PageManager>,
    pub page_signals: Arc<PageSignals>,
    pub windows: Arc<WindowManager>,
    frames: Arc<FrameRegistry>,
    observers: Mutex<HashMap<PageId, Arc<Observer>>>,
}

impl BrowserSession {
    #[must_use]
    pub fn new(connection: Arc<dyn CdpConnection>, hooks: BrowserSessionHooks) -> Arc<Self> {
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
        let pages = Arc::new(PageManager::new(connection.clone(), page_hooks));
        let windows = Arc::new(WindowManager::new(connection.clone()));
        let session = Arc::new(Self {
            connection: connection.clone(),
            pages,
            page_signals,
            windows,
            frames,
            observers: Mutex::new(HashMap::new()),
        });
        Self::spawn_detach_listener(session.clone());
        session
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

    fn spawn_detach_listener(session: Arc<Self>) {
        let mut events = session.connection.events();
        tokio::spawn(async move {
            loop {
                let Ok(event) = events.recv().await else {
                    break;
                };
                if event.method == "Target.detachedFromTarget" {
                    handle_detached_event(&session, event).await;
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
        .map(crate::SessionId::from);
    if let Some(session_id) = session_id {
        session.page_signals.detach_session(&session_id);
        session.pages.detach_session(&session_id).await;
    }
}
