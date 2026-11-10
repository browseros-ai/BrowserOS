//! Neo: HTTP captures of the CDP page ids returned by the run tool, on two owned sockets.
//!
//! Mount `NeoCapture::router` beside MCP. It serves GET `/v1/pages/{pageId}/screenshot`,
//! `/screencast` and `/video`, where `pageId` is a CDP target id from `run`'s `pages` result.
//! Stills accept `type=jpeg|png` and JPEG `quality=0..100`; streams accept `quality=low|high`.
//! Stills are viewport/device-pixel captures. Response bodies are the only output; this service
//! keeps no files or trace records. Cancel the host token before draining HTTP, drain runs,
//! then call `close` to release presence and stop both reconnecting clients.
//!
//! Page ids are capabilities. The request guard matches `/mcp`, but headerless GET requests
//! can pass it, so a page-listing GET must never be added behind this guard alone. URLs in
//! capture headers are unmasked, including their query strings, as allowed by the host contract.
//!
//! A watched background page stays visible until it closes. `hasFocus()` resets when a run
//! on it ends. Losing the page-link socket also clears a live run's focus. Capture sessions
//! never emulate focus; detaching one that did would clear another session's emulation.

mod pages;
mod presence;
mod routes;
mod still;

use std::sync::Arc;

use axum::Router;
use neo_capture::{CaptureHub, CaptureTransport, CdpPortTransport};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use pages::{PageLink, PageTabs};

/// Stills, MJPEG and AV1 fMP4 for Neo pages. Connections open only on the first request.
#[derive(Clone)]
pub struct NeoCapture(Arc<Inner>);

/// Shared service lifetime. Response bodies borrow leases; only close releases presence.
struct Inner {
    hub: Arc<CaptureHub>,
    pages: Arc<PageLink>,
    stills: Semaphore,
    shutdown: CancellationToken,
    // Each transport owns one reconnecting client. The page socket must never replace the
    // hub's socket, nor either socket the run tool's client.
    ports: Option<[Arc<CdpPortTransport>; 2]>,
}

impl NeoCapture {
    /// Opens independent capture and page sockets lazily. Shutdown ends response bodies.
    #[must_use]
    pub fn start(cdp_port: u16, shutdown: CancellationToken) -> Self {
        let hub = Arc::new(CdpPortTransport::new(cdp_port));
        let pages = Arc::new(CdpPortTransport::new(cdp_port));
        Self::build(
            Arc::<CdpPortTransport>::clone(&hub),
            Arc::<CdpPortTransport>::clone(&pages),
            shutdown,
            Some([hub, pages]),
        )
    }

    /// The same service with independent scripted transports for contract tests.
    #[doc(hidden)]
    #[must_use]
    pub fn start_with(
        hub: Arc<dyn CaptureTransport>,
        pages: Arc<dyn CaptureTransport>,
        shutdown: CancellationToken,
    ) -> Self {
        Self::build(hub, pages, shutdown, None)
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "accept an owned host token while deriving service-local cancellation"
    )]
    fn build(
        hub: Arc<dyn CaptureTransport>,
        pages: Arc<dyn CaptureTransport>,
        shutdown: CancellationToken,
        ports: Option<[Arc<CdpPortTransport>; 2]>,
    ) -> Self {
        // A service may close itself without cancelling its host's sibling services.
        let shutdown = shutdown.child_token();
        let pages = Arc::new(PageLink::new(pages, shutdown.clone()));
        Self(Arc::new(Inner {
            hub: CaptureHub::new(
                Arc::new(PageTabs(Arc::clone(&pages))),
                hub,
                shutdown.clone(),
            ),
            pages,
            stills: Semaphore::new(4),
            shutdown,
            ports,
        }))
    }

    /// Mount after the host's `with_state`, beside its MCP router on the same listener.
    #[must_use = "merge this router into the host listener"]
    pub fn router(&self) -> Router {
        routes::router(self.clone())
    }

    /// Call only after the run service drained. Presence detach can clear a run's focus.
    pub async fn close(&self) {
        self.0.shutdown.cancel();
        self.0.pages.close().await;
        if let Some(ports) = &self.0.ports {
            for port in ports {
                port.close().await;
            }
        }
    }
}
