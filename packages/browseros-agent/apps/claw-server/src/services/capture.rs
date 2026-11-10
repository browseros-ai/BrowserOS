//! claw-server's door to the shared capture hub in `neo-capture`.
//!
//! The hub, its streams and its CDP transport moved to the crate so claw-sdk can serve the same
//! live views. What stays here is the one port only claw-server can answer: which tabs one of
//! *its* sessions may show ([`LiveSessionTabs`], over the MCP session map, the audit and the
//! cockpit's tab ranking). The re-exports keep the paths claw-server's routes and integration
//! tests were written against (`services::capture::{CaptureHub, testing::…}`), so none of them
//! changed when the hub moved out.

use crate::{
    db::AuditLog,
    error::{AppError, AppResult},
    ids::SessionId,
    services::{cockpit::SessionVisualService, sessions::Sessions},
};
use futures_util::future::BoxFuture;
use neo_capture::CaptureResult;
use std::sync::Arc;

pub use neo_capture::{
    CaptureError, CaptureFailure, CaptureHub, CaptureTab, CaptureTransport, CdpPortTransport,
    Frame, Frames, Quality, SessionTabs, SessionView, VideoChunks, Watch, Watcher, testing,
};

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

    async fn lookup(&self, session_id: &str) -> AppResult<Option<SessionView>> {
        if !self.sessions.contains(&SessionId::new(session_id)).await {
            let known = self.audit_log.get_task_summary(session_id).await?.is_some();
            return Ok(known.then(|| SessionView {
                live: false,
                tabs: Vec::new(),
            }));
        }
        let tabs = self
            .visuals
            .ranked_tabs(session_id)
            .await?
            .into_iter()
            .map(|tab| CaptureTab {
                tab_id: tab.tab_id,
                target_id: tab.target_id,
                url: tab.url,
            })
            .collect();
        Ok(Some(SessionView { live: true, tabs }))
    }
}

impl SessionTabs for LiveSessionTabs {
    fn view<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, CaptureResult<Option<SessionView>>> {
        Box::pin(async move {
            self.lookup(session_id)
                .await
                .map_err(|error| CaptureFailure::new(error.to_string()))
        })
    }
}

/// A hub failure reaches claw-server's handlers as the internal error it was before the move,
/// with the same message, so `/v1` still answers a generic 500 and logs the cause.
impl From<CaptureFailure> for AppError {
    fn from(failure: CaptureFailure) -> Self {
        Self::Internal(failure.message().to_string())
    }
}
