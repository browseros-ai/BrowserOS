use crate::{CoreError, FrameId, PageId, ProtocolSession, SessionId, connection::CdpConnection};
use browseros_cdp::{CdpEvent, target};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct FrameTarget {
    pub session: ProtocolSession,
    pub ax_params: Value,
    /// Frame whose isolated world must host DOM cursor scanning. Root targets use their session's
    /// default document; same-process and out-of-process children require an explicit frame.
    pub runtime_frame_id: Option<FrameId>,
}

pub struct FrameRegistry {
    cdp: Arc<dyn CdpConnection>,
    oopif_sessions: Mutex<HashMap<FrameId, SessionId>>,
    page_sessions: Mutex<HashMap<PageId, SessionId>>,
}

impl FrameRegistry {
    #[must_use]
    pub fn new(cdp: Arc<dyn CdpConnection>) -> Arc<Self> {
        let registry = Arc::new(Self {
            cdp,
            oopif_sessions: Mutex::new(HashMap::new()),
            page_sessions: Mutex::new(HashMap::new()),
        });
        Self::spawn_event_listener(registry.clone());
        registry
    }

    pub async fn register_page(
        &self,
        page_session: ProtocolSession,
        page_id: PageId,
        session_id: SessionId,
    ) -> Result<(), CoreError> {
        self.page_sessions.lock().await.insert(page_id, session_id);
        let _ = page_session
            .send::<_, Value>(
                "Target.setAutoAttach",
                json!({
                    "autoAttach": true,
                    "waitForDebuggerOnStart": false,
                    "flatten": true
                }),
            )
            .await;
        Ok(())
    }

    pub async fn resolve_frame_target(
        &self,
        page_id: PageId,
        frame_id: Option<FrameId>,
    ) -> Result<FrameTarget, CoreError> {
        let page_session_id = self
            .page_sessions
            .lock()
            .await
            .get(&page_id)
            .cloned()
            .ok_or_else(|| CoreError::Message(format!("Page {page_id} has no attached session")))?;
        let Some(frame_id) = frame_id else {
            return Ok(FrameTarget {
                session: ProtocolSession::for_session(self.cdp.clone(), page_session_id),
                ax_params: json!({}),
                runtime_frame_id: None,
            });
        };
        if let Some(oopif) = self.oopif_sessions.lock().await.get(&frame_id).cloned() {
            return Ok(FrameTarget {
                session: ProtocolSession::for_session(self.cdp.clone(), oopif),
                ax_params: json!({}),
                runtime_frame_id: Some(frame_id),
            });
        }
        Ok(FrameTarget {
            session: ProtocolSession::for_session(self.cdp.clone(), page_session_id),
            ax_params: json!({ "frameId": frame_id.0 }),
            runtime_frame_id: Some(frame_id),
        })
    }

    fn spawn_event_listener(registry: Arc<Self>) {
        let mut events = registry.cdp.events();
        tokio::spawn(async move {
            loop {
                let Ok(event) = events.recv().await else {
                    break;
                };
                registry.handle_event(event).await;
            }
        });
    }

    async fn handle_event(&self, event: CdpEvent) {
        match event.method.as_str() {
            "Target.attachedToTarget" => {
                let parsed = serde_json::from_value::<target::AttachedToTargetEvent>(event.params);
                if let Ok(params) = parsed {
                    self.on_attached(params).await;
                }
            }
            "Target.detachedFromTarget" => {
                let parsed =
                    serde_json::from_value::<target::DetachedFromTargetEvent>(event.params);
                if let Ok(params) = parsed {
                    self.on_detached(&SessionId::from(params.session_id)).await;
                }
            }
            _ => {}
        }
    }

    async fn on_attached(&self, params: target::AttachedToTargetEvent) {
        if params.target_info.r#type != "iframe" {
            return;
        }
        let frame_id = FrameId(params.target_info.target_id);
        let session_id = SessionId::from(params.session_id);
        self.oopif_sessions
            .lock()
            .await
            .insert(frame_id, session_id.clone());
        let session = ProtocolSession::for_session(self.cdp.clone(), session_id);
        if params.waiting_for_debugger {
            let _ = session
                .send::<_, Value>("Runtime.runIfWaitingForDebugger", json!({}))
                .await;
        }
        let _ = session.send::<_, Value>("DOM.enable", json!({})).await;
        let _ = session
            .send::<_, Value>("Accessibility.enable", json!({}))
            .await;
        let _ = session
            .send::<_, Value>(
                "Target.setAutoAttach",
                json!({
                    "autoAttach": true,
                    "waitForDebuggerOnStart": false,
                    "flatten": true
                }),
            )
            .await;
    }

    async fn on_detached(&self, session_id: &SessionId) {
        self.oopif_sessions
            .lock()
            .await
            .retain(|_, existing| existing != session_id);
    }
}
