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
use tracing::{debug, warn};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConnectionState {
    pub connected: bool,
    pub epoch: u64,
    pub last_error: Option<String>,
}

pub struct BrowserService {
    cdp_port: u16,
    ownership: Arc<PageOwnership>,
    state_tx: watch::Sender<BrowserConnectionState>,
    initial_attempt_tx: watch::Sender<bool>,
    session: Arc<RwLock<Option<Arc<browseros_core::BrowserSession>>>>,
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
                    debug!(epoch, "connected to BrowserOS CDP");
                    self.monitor_client(client).await;
                    *self.session.write().await = None;
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
                    if connected != last_connected || epoch != last_epoch {
                        if connected
                            && let Some(session) = self.session().await
                            && let Err(error) = self.tab_registry.observe_session(session, epoch).await
                        {
                            warn!(epoch, error = %error, "failed to seed tab target map after reconnect");
                        }
                        self.state_tx.send_replace(BrowserConnectionState {
                            connected,
                            epoch,
                            last_error: if connected { None } else { Some("CDP disconnected; reconnecting".to_string()) },
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
        let database = crate::db::Database::open(root.path().join("audit.sqlite")).await?;
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
