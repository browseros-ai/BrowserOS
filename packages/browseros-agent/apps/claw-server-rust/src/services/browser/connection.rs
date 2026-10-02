use super::TabRegistry;
use crate::services::sessions::PageOwnership;
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use browseros_core::{
    BrowserSession, BrowserSessionHooks,
    pages::{OnPageDetached, PageManagerHooks},
};
use serde::Serialize;
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{RwLock, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConnectionState {
    pub connected: bool,
    pub epoch: u64,
    pub last_error: Option<String>,
}

/// The link as the client sees it right now.
///
/// Separate from [`BrowserConnectionState`], which is refreshed by a one second
/// poll. Mixing the two lets a caller report a connected link whose socket is
/// already down, so anything deciding whether tools will work reads this instead.
#[derive(Debug, Clone, Copy)]
pub struct BrowserLinkStatus {
    pub connected: bool,
    /// How long the link has been down. `None` when up, and also before the first
    /// connection ever succeeds.
    pub down_for: Option<Duration>,
    /// Whether a link was ever established in this process. Distinguishes "lost a
    /// working link" from "never reached the browser", which need different advice.
    pub ever_connected: bool,
}

pub struct BrowserService {
    cdp_port: u16,
    ownership: Arc<PageOwnership>,
    state_tx: watch::Sender<BrowserConnectionState>,
    initial_attempt_tx: watch::Sender<bool>,
    session: Arc<RwLock<Option<Arc<browseros_core::BrowserSession>>>>,
    /// Kept so callers can ask the client how long the link has been down rather
    /// than this service keeping a second copy of that answer.
    client: Arc<RwLock<Option<CdpClient>>>,
    tab_registry: Arc<TabRegistry>,
    cancel: CancellationToken,
}

impl BrowserService {
    #[must_use]
    pub fn new(
        cdp_port: u16,
        ownership: Arc<PageOwnership>,
        tab_registry: Arc<TabRegistry>,
    ) -> Arc<Self> {
        let (state_tx, _) = watch::channel(BrowserConnectionState {
            connected: false,
            epoch: 0,
            last_error: None,
        });
        let (initial_attempt_tx, _) = watch::channel(false);
        Arc::new(Self {
            cdp_port,
            ownership,
            state_tx,
            initial_attempt_tx,
            session: Arc::new(RwLock::new(None)),
            client: Arc::new(RwLock::new(None)),
            tab_registry,
            cancel: CancellationToken::new(),
        })
    }

    pub fn start(self: &Arc<Self>) -> JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            service.reattach_loop().await;
        })
    }

    #[must_use]
    pub fn state(&self) -> BrowserConnectionState {
        self.state_tx.borrow().clone()
    }

    pub async fn session(&self) -> Option<Arc<browseros_core::BrowserSession>> {
        self.session
            .read()
            .await
            .clone()
            .filter(|session| session.is_connected())
    }

    /// The link as the client sees it right now, not as the one second poll last
    /// published it.
    ///
    /// Callers deciding whether browser tools will work, or what to tell an agent
    /// that cannot run one, need every field to come from the same read.
    pub async fn link_status(&self) -> BrowserLinkStatus {
        let epoch = self.state_tx.borrow().epoch;
        match self.client.read().await.as_ref() {
            Some(client) => BrowserLinkStatus {
                connected: client.is_connected(),
                down_for: client.down_for(),
                ever_connected: epoch > 0,
            },
            // No client at all: either the first connection has not succeeded yet,
            // or the service was stopped. The reattach loop is still retrying.
            None => BrowserLinkStatus {
                connected: false,
                down_for: None,
                ever_connected: epoch > 0,
            },
        }
    }

    pub async fn wait_for_initial_attempt(&self) {
        let mut receiver = self.initial_attempt_tx.subscribe();
        while !*receiver.borrow() {
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }

    #[doc(hidden)]
    pub async fn connect_once_for_testing(&self) -> Result<(), browseros_cdp::CdpError> {
        let opts = self.connect_options();
        let client = CdpClient::connect(opts).await?;
        *self.session.write().await = Some(self.browser_session(client.clone()).await);
        *self.client.write().await = Some(client.clone());
        self.state_tx.send_replace(BrowserConnectionState {
            connected: true,
            epoch: client.epoch(),
            last_error: None,
        });
        Ok(())
    }

    #[doc(hidden)]
    pub async fn set_session_for_testing(&self, session: Arc<BrowserSession>) {
        *self.session.write().await = Some(session);
    }

    /// Publishes a connection snapshot the way the one second poll does, so the
    /// gap between the snapshot and the live link can be exercised.
    #[doc(hidden)]
    pub fn publish_state_for_testing(&self, state: BrowserConnectionState) {
        self.state_tx.send_replace(state);
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    fn connect_options(&self) -> ConnectOptions {
        ConnectOptions {
            port: self.cdp_port,
            connect_timeout: Duration::from_secs(2),
            connect_max_retries: 1,
            reconnect_policy: ReconnectPolicy::KeepTrying,
            reconnect_delay: Duration::from_secs(1),
            reconnect_max_retries: usize::MAX,
            ..ConnectOptions::new(self.cdp_port)
        }
    }

    async fn browser_session(&self, client: CdpClient) -> Arc<BrowserSession> {
        let epoch = client.epoch();
        let ownership = self.ownership.clone();
        let on_page_detached: OnPageDetached = Arc::new(move |page_id| {
            let ownership = ownership.clone();
            tokio::spawn(async move {
                ownership.remove_page(&page_id).await;
            });
        });
        let session = BrowserSession::new(
            Arc::new(client),
            BrowserSessionHooks {
                page_manager: PageManagerHooks {
                    on_page_detached: Some(on_page_detached),
                    ..PageManagerHooks::default()
                },
            },
        );
        if let Err(error) = self
            .tab_registry
            .observe_session(session.clone(), epoch)
            .await
        {
            warn!(epoch, error = %error, "failed to seed tab target map");
        }
        session
    }

    async fn reattach_loop(self: Arc<Self>) {
        let mut backoff = Duration::from_secs(1);
        let mut initial_attempt_pending = true;
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            let opts = self.connect_options();
            match CdpClient::connect(opts).await {
                Ok(client) => {
                    let session = self.browser_session(client.clone()).await;
                    *self.session.write().await = Some(session);
                    *self.client.write().await = Some(client.clone());
                    let epoch = client.epoch();
                    self.state_tx.send_replace(BrowserConnectionState {
                        connected: true,
                        epoch,
                        last_error: None,
                    });
                    if initial_attempt_pending {
                        self.initial_attempt_tx.send_replace(true);
                        initial_attempt_pending = false;
                    }
                    info!(epoch, "connected to BrowserOS CDP");
                    self.monitor_client(client).await;
                    *self.session.write().await = None;
                    *self.client.write().await = None;
                    backoff = Duration::from_secs(1);
                }
                Err(err) => {
                    let epoch = self.state_tx.borrow().epoch;
                    self.state_tx.send_replace(BrowserConnectionState {
                        connected: false,
                        epoch,
                        last_error: Some(err.to_string()),
                    });
                    if initial_attempt_pending {
                        self.initial_attempt_tx.send_replace(true);
                        initial_attempt_pending = false;
                    }
                    warn!(error = %err, retry_ms = backoff.as_millis(), "CDP connect failed; retrying");
                    tokio::select! {
                        () = self.cancel.cancelled() => return,
                        () = tokio::time::sleep(backoff) => {}
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    }

    async fn monitor_client(&self, client: CdpClient) {
        let mut last_connected = true;
        let mut last_epoch = client.epoch();
        loop {
            tokio::select! {
                () = self.cancel.cancelled() => {
                    client.disconnect().await;
                    return;
                }
                () = tokio::time::sleep(Duration::from_secs(1)) => {
                    let connected = client.is_connected();
                    let epoch = client.epoch();
                    let transitioned = connected != last_connected || epoch != last_epoch;
                    // Two different jobs, and conflating them breaks one or the other.
                    //
                    // A new epoch needs its own event listener, exactly once: the
                    // listener is bound to an epoch and exits when that changes, and
                    // the attach that starts one happens per client, not per socket,
                    // so an in-client reconnect has to attach again here or tab
                    // events stop being processed.
                    //
                    // A map that is merely incomplete on the epoch it already has
                    // needs seeding alone, retried until it succeeds. Routing that
                    // through the attach would subscribe another listener every pass.
                    if connected && let Some(session) = self.session().await {
                        if transitioned {
                            if let Err(error) = self
                                .tab_registry
                                .observe_session(session, epoch)
                                .await
                            {
                                warn!(epoch, error = %error, "failed to seed tab target map after reconnect");
                            }
                        } else if !self.tab_registry.is_ready(epoch)
                            && let Err(error) = self.tab_registry.reseed(&session, epoch).await
                        {
                            warn!(epoch, error = %error, "failed to seed tab target map; retrying");
                        }
                    }
                    if transitioned {
                        match (connected, client.down_for()) {
                            (true, _) => info!(epoch, "browser link is up"),
                            (false, Some(down_for)) => warn!(
                                epoch,
                                down_ms = down_for.as_millis(),
                                "browser link is down; reconnecting"
                            ),
                            (false, None) => {
                                warn!(epoch, "browser link is down; reconnecting");
                            }
                        }
                        self.state_tx.send_replace(BrowserConnectionState {
                            connected,
                            epoch,
                            last_error: if connected {
                                None
                            } else {
                                Some("browser link lost, reconnecting".to_string())
                            },
                        });
                        last_connected = connected;
                        last_epoch = epoch;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browseros_cdp::{CdpError, CdpEvent, SessionId};
    use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection};
    use futures_util::future::BoxFuture;
    use serde_json::Value;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tempfile::TempDir;
    use tokio::sync::broadcast;

    struct TestConnection {
        connected: AtomicBool,
        events: broadcast::Sender<CdpEvent>,
    }

    impl TestConnection {
        fn new(connected: bool) -> Arc<Self> {
            let (events, _) = broadcast::channel(1);
            Arc::new(Self {
                connected: AtomicBool::new(connected),
                events,
            })
        }
    }

    impl CdpConnection for TestConnection {
        fn send<'a>(
            &'a self,
            _method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async { Ok(serde_json::json!({})) })
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
            self.events.subscribe()
        }

        fn is_connected(&self) -> bool {
            self.connected.load(Ordering::SeqCst)
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    async fn service() -> anyhow::Result<(Arc<BrowserService>, TempDir)> {
        let root = tempfile::tempdir()?;
        let database =
            crate::db::Database::open(root.path().join(crate::db::DATABASE_FILENAME)).await?;
        let audit_log = Arc::new(crate::db::AuditLog::new(database.clone()));
        let session_tabs = Arc::new(crate::db::SessionTabLedger::new(database));
        let sessions = crate::services::sessions::Sessions::new(
            audit_log,
            session_tabs.clone(),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(60),
        );
        Ok((
            BrowserService::new(0, sessions.ownership(), TabRegistry::new(session_tabs)),
            root,
        ))
    }

    /// The published snapshot is refreshed by a one second poll, so between a link
    /// dropping and the next poll it still claims the link is up. Readiness and the
    /// tool error read the live link instead, and this pins that they do.
    #[tokio::test]
    async fn link_status_ignores_a_stale_connected_snapshot() -> anyhow::Result<()> {
        let (service, _root) = service().await?;
        service.publish_state_for_testing(BrowserConnectionState {
            connected: true,
            epoch: 7,
            last_error: None,
        });

        let link = service.link_status().await;

        assert!(
            !link.connected,
            "a snapshot cannot make a link with no client look connected"
        );
        assert!(service.state().connected, "the snapshot is still stale");
        assert!(
            link.ever_connected,
            "a non-zero epoch means a link existed once, so the advice is reconnect"
        );
        Ok(())
    }

    /// Before anything has ever connected the epoch is zero, which is the one case
    /// where the browser might genuinely not be running.
    #[tokio::test]
    async fn link_status_reports_never_connected_before_the_first_link() -> anyhow::Result<()> {
        let (service, _root) = service().await?;

        let link = service.link_status().await;

        assert!(!link.connected);
        assert!(!link.ever_connected);
        assert!(link.down_for.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn session_filters_stored_disconnected_browser_session() -> anyhow::Result<()> {
        let (service, _root) = service().await?;
        let connection = TestConnection::new(false);
        let browser = BrowserSession::new(connection, BrowserSessionHooks::default());

        service.set_session_for_testing(browser).await;

        assert!(service.session().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn session_returns_stored_connected_browser_session() -> anyhow::Result<()> {
        let (service, _root) = service().await?;
        let connection = TestConnection::new(true);
        let browser = BrowserSession::new(connection, BrowserSessionHooks::default());

        service.set_session_for_testing(browser).await;

        assert!(service.session().await.is_some());
        Ok(())
    }
}
