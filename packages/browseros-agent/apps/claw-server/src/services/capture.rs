//! Adapts legacy session liveness and tab ranking to the shared capture engine.
//! Focus emulation remains owned by legacy tab claims, never by capture sessions.

use crate::db::AuditLog;
use crate::ids::SessionId;
use crate::services::cockpit::SessionVisualService;
use crate::services::sessions::Sessions;
use futures_util::future::BoxFuture;
use neo_capture::{CaptureFailure, CaptureResult, CaptureTab, SessionTabs, SessionView};
use std::sync::Arc;

/// Production `SessionTabs`: liveness from the in-memory session map, existence from the audit,
/// and tabs from the same ranking the cockpit preview uses, so both agree on a session's current
/// tab. Only tabs the session has open claims on are offered, which are exactly the ones focus
/// emulation keeps rendering in the background.
pub struct LiveSessionTabs {
    sessions: Arc<Sessions>,
    audit_log: Arc<AuditLog>,
    visuals: Arc<SessionVisualService>,
}

impl LiveSessionTabs {
    #[must_use]
    pub fn new(
        sessions: Arc<Sessions>,
        audit_log: Arc<AuditLog>,
        visuals: Arc<SessionVisualService>,
    ) -> Self {
        Self {
            sessions,
            audit_log,
            visuals,
        }
    }
}

impl SessionTabs for LiveSessionTabs {
    fn view<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, CaptureResult<Option<SessionView>>> {
        Box::pin(async move {
            if !self.sessions.contains(&SessionId::new(session_id)).await {
                let known = self
                    .audit_log
                    .get_task_summary(session_id)
                    .await
                    .map_err(|e| CaptureFailure::new(e.to_string()))?
                    .is_some();
                return Ok(known.then(|| SessionView {
                    live: false,
                    tabs: Vec::new(),
                }));
            }
            let tabs = self
                .visuals
                .ranked_tabs(session_id)
                .await
                .map_err(|e| CaptureFailure::new(e.to_string()))?
                .into_iter()
                .map(|tab| CaptureTab {
                    tab_id: tab.tab_id,
                    target_id: tab.target_id,
                    url: tab.url,
                })
                .collect();
            Ok(Some(SessionView { live: true, tabs }))
        })
    }
}
