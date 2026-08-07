use crate::{CoreError, FrameId, PageId, ProtocolSession, SessionId, connection::CdpConnection};
use browseros_cdp::{CdpEvent, target};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct FrameTarget {
    pub session: ProtocolSession,
    pub ax_params: Value,
    /// Root and OOPIF sessions already inspect the intended document. Same-process children share
    /// their nearest parent target session, so cursor acquisition must target the resolved child
    /// Document object instead.
    pub cursor_uses_session_default: bool,
}

pub struct FrameRegistry {
    cdp: Arc<dyn CdpConnection>,
    connection_epoch: Mutex<u64>,
    oopif_sessions: Mutex<HashMap<FrameId, SessionId>>,
    same_process_sessions: Mutex<HashMap<FrameId, SessionId>>,
    page_sessions: Mutex<HashMap<PageId, SessionId>>,
}

impl FrameRegistry {
    #[must_use]
    pub fn new(cdp: Arc<dyn CdpConnection>) -> Arc<Self> {
        let connection_epoch = cdp.connection_epoch();
        let registry = Arc::new(Self {
            cdp,
            connection_epoch: Mutex::new(connection_epoch),
            oopif_sessions: Mutex::new(HashMap::new()),
            same_process_sessions: Mutex::new(HashMap::new()),
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
        self.clear_sessions_from_prior_connection().await;
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
        same_process_parent: Option<&ProtocolSession>,
    ) -> Result<FrameTarget, CoreError> {
        self.clear_sessions_from_prior_connection().await;
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
                cursor_uses_session_default: true,
            });
        };
        if let Some(oopif) = self.oopif_sessions.lock().await.get(&frame_id).cloned() {
            return Ok(FrameTarget {
                session: ProtocolSession::for_session(self.cdp.clone(), oopif),
                ax_params: json!({}),
                cursor_uses_session_default: true,
            });
        }
        // A non-target frame shares its nearest parent target's CDP session, which may itself be
        // an OOPIF. Cache that inheritance because later ref resolution knows the frame id but no
        // longer has the capture-time parent target.
        let inherited_session = same_process_parent
            .and_then(ProtocolSession::session_id)
            .cloned();
        let session_id = if let Some(inherited_session) = inherited_session {
            self.same_process_sessions
                .lock()
                .await
                .insert(frame_id.clone(), inherited_session.clone());
            inherited_session
        } else {
            self.same_process_sessions
                .lock()
                .await
                .get(&frame_id)
                .cloned()
                .unwrap_or(page_session_id)
        };
        Ok(FrameTarget {
            session: ProtocolSession::for_session(self.cdp.clone(), session_id),
            ax_params: json!({ "frameId": frame_id.0 }),
            cursor_uses_session_default: false,
        })
    }

    async fn clear_sessions_from_prior_connection(&self) {
        let current_epoch = self.cdp.connection_epoch();
        let mut cached_epoch = self.connection_epoch.lock().await;
        if *cached_epoch == current_epoch {
            return;
        }
        // Flattened target session ids are scoped to one CDP websocket. A reconnect can reuse
        // frame ids while every cached page, OOPIF, and inherited session id is already invalid.
        self.oopif_sessions.lock().await.clear();
        self.same_process_sessions.lock().await.clear();
        self.page_sessions.lock().await.clear();
        *cached_epoch = current_epoch;
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
        self.clear_sessions_from_prior_connection().await;
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
        self.clear_sessions_from_prior_connection().await;
        self.oopif_sessions
            .lock()
            .await
            .retain(|_, existing| existing != session_id);
        self.same_process_sessions
            .lock()
            .await
            .retain(|_, existing| existing != session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::FrameRegistry;
    use crate::{FrameId, PageId, ProtocolSession, SessionId, connection::CdpConnection};
    use browseros_cdp::{CdpError, CdpEvent};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    use tokio::sync::broadcast;

    struct NoopConnection {
        epoch: AtomicU64,
    }

    impl Default for NoopConnection {
        fn default() -> Self {
            Self {
                epoch: AtomicU64::new(1),
            }
        }
    }

    impl CdpConnection for NoopConnection {
        fn send<'a>(
            &'a self,
            _method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async { Ok(json!({})) })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            self.epoch.load(Ordering::SeqCst)
        }
    }

    #[tokio::test]
    async fn same_process_child_inherits_and_caches_its_oopif_parent_session()
    -> Result<(), crate::CoreError> {
        let cdp = Arc::new(NoopConnection::default());
        let registry = FrameRegistry::new(cdp.clone());
        let page_id = PageId(1);
        registry
            .page_sessions
            .lock()
            .await
            .insert(page_id.clone(), SessionId::from("page-session".to_string()));
        registry.oopif_sessions.lock().await.insert(
            FrameId("oopif".to_string()),
            SessionId::from("oopif-session".to_string()),
        );
        let oopif = registry
            .resolve_frame_target(page_id.clone(), Some(FrameId("oopif".to_string())), None)
            .await?;
        let nested_id = FrameId("nested".to_string());

        let nested = registry
            .resolve_frame_target(
                page_id.clone(),
                Some(nested_id.clone()),
                Some(&oopif.session),
            )
            .await?;
        assert!(nested.session.same_session(&oopif.session));
        assert_eq!(nested.ax_params, json!({"frameId": "nested"}));
        assert!(!nested.cursor_uses_session_default);

        let cached = registry
            .resolve_frame_target(page_id, Some(nested_id), None)
            .await?;
        assert!(cached.session.same_session(&oopif.session));
        assert!(!cached.session.same_session(&ProtocolSession::for_session(
            cdp,
            SessionId::from("page-session".to_string())
        )));
        Ok(())
    }

    #[tokio::test]
    async fn reconnect_discards_inherited_sessions_from_the_prior_connection()
    -> Result<(), crate::CoreError> {
        let cdp = Arc::new(NoopConnection::default());
        let registry = FrameRegistry::new(cdp.clone());
        let page_id = PageId(1);
        let child_id = FrameId("child".to_string());
        let first_page_session = ProtocolSession::for_session(
            cdp.clone(),
            SessionId::from("page-session-1".to_string()),
        );
        registry
            .register_page(
                first_page_session,
                page_id.clone(),
                SessionId::from("page-session-1".to_string()),
            )
            .await?;
        let first_parent = ProtocolSession::for_session(
            cdp.clone(),
            SessionId::from("oopif-session-1".to_string()),
        );
        let first_child = registry
            .resolve_frame_target(page_id.clone(), Some(child_id.clone()), Some(&first_parent))
            .await?;
        assert!(first_child.session.same_session(&first_parent));

        cdp.epoch.store(2, Ordering::SeqCst);
        let second_page_session = ProtocolSession::for_session(
            cdp.clone(),
            SessionId::from("page-session-2".to_string()),
        );
        registry
            .register_page(
                second_page_session.clone(),
                page_id.clone(),
                SessionId::from("page-session-2".to_string()),
            )
            .await?;

        let reconnected_child = registry
            .resolve_frame_target(page_id, Some(child_id), None)
            .await?;
        assert!(reconnected_child.session.same_session(&second_page_session));
        assert!(!reconnected_child.session.same_session(&first_parent));
        Ok(())
    }
}
