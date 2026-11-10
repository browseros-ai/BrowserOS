//! Neo: page identity and one generation of the page socket, separate from capture traffic.

use tokio::sync::Mutex;

use std::sync::Arc;

use futures_util::future::BoxFuture;
use neo_capture::{
    CaptureFailure, CaptureResult, CaptureTab, CaptureTransport, Link, SessionTabs, SessionView,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::presence::Presence;

/// A page link generation owns all presence sessions established on that websocket.
pub(crate) struct Generation {
    pub(crate) link: Arc<Link>,
    pub(crate) presence: Presence,
}

/// Retires a broken socket generation before any new lookup or presence attachment.
pub(crate) struct PageLink {
    transport: Arc<dyn CaptureTransport>,
    shutdown: CancellationToken,
    current: Mutex<Option<Arc<Generation>>>,
}

/// Browser-level lookup result used to pin a capture and construct HTTP metadata.
pub(crate) struct PageInfo {
    pub(crate) tab_id: i64,
    pub(crate) url: String,
}

impl PageLink {
    pub(crate) fn new(transport: Arc<dyn CaptureTransport>, shutdown: CancellationToken) -> Self {
        Self {
            transport,
            shutdown,
            current: Mutex::new(None),
        }
    }

    pub(crate) async fn generation(&self) -> CaptureResult<Arc<Generation>> {
        let mut current = self.current.lock().await;
        if self.shutdown.is_cancelled() {
            return Err(CaptureFailure::new("capture service is closed"));
        }
        if let Some(generation) = current.as_ref().filter(|g| !g.link.broken().is_cancelled()) {
            return Ok(Arc::clone(generation));
        }
        // Only connection establishment is serialized here. Page lookups, attach and focus
        // emulation happen after this lock is released, so a modal page cannot block others.
        let link = Link::open(self.transport.as_ref(), self.shutdown.clone()).await?;
        let generation = Arc::new(Generation {
            link,
            presence: Presence::default(),
        });
        *current = Some(Arc::clone(&generation));
        Ok(generation)
    }

    pub(crate) async fn lookup(&self, page: &str) -> CaptureResult<Option<PageInfo>> {
        self.generation().await?.lookup(page).await
    }

    pub(crate) async fn close(&self) {
        let generation = self.current.lock().await.take();
        if let Some(generation) = generation {
            generation.presence.close(&generation.link).await;
        }
    }
}

impl Generation {
    pub(crate) async fn lookup(&self, page: &str) -> CaptureResult<Option<PageInfo>> {
        let reply = match self
            .link
            .call("Target.getTargetInfo", json!({"targetId": page}))
            .await
        {
            Ok(reply) => reply,
            Err(error) if missing(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        let info = &reply["targetInfo"];
        if info["type"].as_str() != Some("page") {
            return Ok(None);
        }
        let tab_id = info["tabId"]
            .as_i64()
            .ok_or_else(|| CaptureFailure::new("page target has no BrowserOS tabId"))?;
        Ok(Some(PageInfo {
            tab_id,
            url: info["url"].as_str().unwrap_or_default().to_string(),
        }))
    }
}

pub(crate) fn missing(error: &CaptureFailure) -> bool {
    error.to_string().contains("No target with given id")
}

/// The hub's session identity is exactly one Neo page. Viewing it has no emulation side effect.
pub(crate) struct PageTabs(pub(crate) Arc<PageLink>);

impl SessionTabs for PageTabs {
    fn view<'a>(&'a self, page: &'a str) -> BoxFuture<'a, CaptureResult<Option<SessionView>>> {
        Box::pin(async move {
            Ok(self.0.lookup(page).await?.map(|info| SessionView {
                live: true,
                tabs: vec![CaptureTab {
                    tab_id: info.tab_id,
                    target_id: page.to_string(),
                    url: info.url,
                }],
            }))
        })
    }
}
