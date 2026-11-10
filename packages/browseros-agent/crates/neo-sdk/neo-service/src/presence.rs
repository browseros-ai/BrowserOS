//! Neo: one persistent emulating session per watched page, owned by its page-link generation.
//!
//! Viewer departure never detaches presence. Detaching an emulating session clears focus even
//! if another run is attached. Only page closure, link loss or service close ends ownership.

use tokio::sync::Mutex;
use tokio::time;

use std::collections::HashMap;
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use neo_capture::{Attached, CaptureFailure, CaptureResult, Link};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// Pages share only the short map access. Browser awaits hold that page's own state lock.
#[derive(Default)]
pub(crate) struct Presence(Mutex<HashMap<String, Arc<Mutex<PagePresence>>>>);

/// Retain the attachment across an interrupted emulation handshake so retries reuse it.
#[derive(Default)]
struct PagePresence {
    attached: Option<Attached>,
    ready: bool,
}

impl Presence {
    pub(crate) async fn ensure(
        &self,
        link: &Arc<Link>,
        page: &str,
    ) -> CaptureResult<CancellationToken> {
        let entry = {
            let mut pages = self.0.lock().await;
            // Only unreferenced entries can be swept. An ensure that already borrowed the
            // entry must remain the sole initializer for that page, even during detachment.
            pages.retain(|_, entry| {
                Arc::strong_count(entry) > 1
                    || entry.try_lock().map_or(true, |state| {
                        state
                            .attached
                            .as_ref()
                            .is_none_or(|s| !s.detached().is_cancelled())
                    })
            });
            Arc::clone(pages.entry(page.to_string()).or_default())
        };
        let mut state = entry.lock().await;
        if state
            .attached
            .as_ref()
            .is_some_and(|s| s.detached().is_cancelled())
        {
            *state = PagePresence::default();
        }
        if state.attached.is_none() {
            // Persist immediately after attach, before the renderer-answered command. A
            // request deadline may drop ensure while that reply is pending. The next viewer
            // must reuse this session, and a late emulation reply must never trigger detach.
            state.attached = Some(link.attach(page).await?);
        }
        let attached = state
            .attached
            .as_ref()
            .ok_or_else(|| CaptureFailure::new("presence attachment disappeared"))?;
        let detached = attached.detached().clone();
        if !state.ready {
            tokio::select! {
                () = detached.cancelled() => return Err(CaptureFailure::new("page presence ended")),
                reply = link.send("Emulation.setFocusEmulationEnabled", json!({"enabled": true}), attached) => { reply?; }
            }
            state.ready = true;
        }
        Ok(detached)
    }

    pub(crate) async fn close(&self, link: &Arc<Link>) {
        let entries = mem::take(&mut *self.0.lock().await);
        for entry in entries.into_values() {
            if let Some(attached) = entry.lock().await.attached.take() {
                // Shutdown already cancelled every body and pending ensure. Bound best-effort
                // cleanup so an unavailable browser cannot keep the host alive forever.
                let _ = time::timeout(Duration::from_secs(2), link.release(&attached)).await;
            }
        }
    }
}
