//! Neo: The two things `CaptureHub` depends on, as ports so it can be exercised without a browser:
//! which tabs a session may show, and a CDP connection reserved for captures. Production
//! adapters live here; the test adapters are in `testing`.

use tokio::sync::Mutex as AsyncMutex;

use crate::{CaptureConnection, CaptureFailure, CaptureResult};
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use futures_util::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;

/// What a live view may show of one session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionView {
    /// Whether its MCP session is still open; an ended session has nothing to watch.
    pub live: bool,
    /// The session's open tabs, the current one (latest tool call) first.
    pub tabs: Vec<CaptureTab>,
}

/// An eligible `BrowserOS` tab, with stable target identity and the current URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureTab {
    pub tab_id: i64,
    pub target_id: String,
    pub url: String,
}

/// Resolves a session to the tabs a live view may show.
pub trait SessionTabs: Send + Sync {
    /// `None` when no session has this id.
    fn view<'a>(&'a self, session_id: &'a str)
    -> BoxFuture<'a, CaptureResult<Option<SessionView>>>;
}

/// Opens the CDP connection captures run on.
pub trait CaptureTransport: Send + Sync {
    /// Opens a fresh connection; the hub calls it again only after the previous one broke, and
    /// the previous connection may be closed then.
    fn connect(&self) -> BoxFuture<'_, CaptureResult<Arc<dyn CaptureConnection>>>;
}

/// Production `CaptureTransport`: a second websocket to the browser's CDP port. Frames are large
/// and frequent; on the agent's connection they would queue every command, and its keepalive,
/// behind them, and a stalled keepalive tears that connection down for every session.
pub struct CdpPortTransport {
    port: u16,
    current: AsyncMutex<Option<CdpClient>>,
}

impl CdpPortTransport {
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self {
            port,
            current: AsyncMutex::new(None),
        }
    }
}

impl CdpPortTransport {
    /// Stops the owned reconnecting client once its service has drained.
    pub async fn close(&self) {
        if let Some(client) = self.current.lock().await.take() {
            client.disconnect().await;
        }
    }
}

impl CaptureTransport for CdpPortTransport {
    fn connect(&self) -> BoxFuture<'_, CaptureResult<Arc<dyn CaptureConnection>>> {
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
            .map_err(|error| CaptureFailure::new(format!("capture connection failed: {error}")))?;
            *current = Some(client.clone());
            Ok(Arc::new(client) as Arc<dyn CaptureConnection>)
        })
    }
}
