//! The two things `CaptureHub` depends on, as ports so it can be exercised without a browser:
//! which tabs a session may show, and a CDP connection reserved for captures. Production
//! adapters live here; the test adapters are in `testing`.

use crate::{
    db::AuditLog,
    error::{AppError, AppResult},
    ids::SessionId,
    services::{cockpit::SessionVisualService, sessions::Sessions},
};
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use browseros_core::CdpConnection;
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};

/// What a live view may show of one session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionView {
    /// Whether its MCP session is still open; an ended session has nothing to watch.
    pub live: bool,
    /// The session's open tabs, the current one (latest tool call) first.
    pub tabs: Vec<CaptureTab>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureTab {
    pub tab_id: i64,
    pub target_id: String,
    pub url: String,
}

/// Resolves a session to the tabs a live view may show.
pub trait SessionTabs: Send + Sync {
    /// `None` when no session has this id.
    fn view<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, AppResult<Option<SessionView>>>;
}

/// Opens the CDP connection captures run on.
pub trait CaptureTransport: Send + Sync {
    /// Opens a fresh connection; the hub calls it again only after the previous one broke, and
    /// the previous connection may be closed then.
    fn connect(&self) -> BoxFuture<'_, AppResult<Arc<dyn CdpConnection>>>;
}

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
    fn view<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, AppResult<Option<SessionView>>> {
        Box::pin(async move {
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
        })
    }
}

/// Production `CaptureTransport`: a second websocket to the browser's CDP port. Frames are large
/// and frequent; on the agent's connection they would queue every command, and its keepalive,
/// behind them, and a stalled keepalive tears that connection down for every session.
pub struct CdpPortTransport {
    port: u16,
    current: tokio::sync::Mutex<Option<CdpClient>>,
}

impl CdpPortTransport {
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self {
            port,
            current: tokio::sync::Mutex::new(None),
        }
    }
}

impl CaptureTransport for CdpPortTransport {
    fn connect(&self) -> BoxFuture<'_, AppResult<Arc<dyn CdpConnection>>> {
        Box::pin(async move {
            let mut current = self.current.lock().await;
            // The client keeps reconnecting on its own, so the broken one must be told to stop.
            if let Some(previous) = current.take() {
                previous.disconnect().await;
            }
            let client = CdpClient::connect(ConnectOptions {
                port: self.port,
                connect_timeout: Duration::from_secs(2),
                connect_max_retries: 1,
                // The default policy exits the process when reconnects run out.
                reconnect_policy: ReconnectPolicy::KeepTrying,
                reconnect_delay: Duration::from_secs(1),
                reconnect_max_retries: usize::MAX,
                ..ConnectOptions::new(self.port)
            })
            .await
            .map_err(|error| AppError::Internal(format!("capture connection failed: {error}")))?;
            *current = Some(client.clone());
            Ok(Arc::new(client) as Arc<dyn CdpConnection>)
        })
    }
}
