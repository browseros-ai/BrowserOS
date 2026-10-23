//! The capture connection: one CDP websocket shared by every running capture, separate from the
//! agent's. It attaches its own flat session per capture (CDP session ids belong to one socket)
//! and routes screencast frames to their capture by that session id.

use crate::error::{AppError, AppResult};
use browseros_cdp::{CdpEvent, SessionId};
use browseros_core::CdpConnection;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::ports::CaptureTransport;

/// How often the link checks that its socket is still the one its sessions were attached on.
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);

pub(super) struct Link {
    cdp: Arc<dyn CdpConnection>,
    /// Flat session ids are valid only on the socket generation they were attached on.
    epoch: u64,
    frames: Mutex<HashMap<String, mpsc::UnboundedSender<Value>>>,
    detached: Mutex<HashMap<String, CancellationToken>>,
    /// Cancelled once the socket dropped or reconnected: every capture on it must end, and the
    /// hub opens a new link for the next one.
    pub(super) broken: CancellationToken,
}

/// A capture's own CDP session on one tab.
pub(super) struct Attached {
    pub(super) id: SessionId,
    /// Cancelled when the browser detaches the session, which is how a closed tab shows up.
    pub(super) detached: CancellationToken,
}

impl Link {
    pub(super) async fn open(
        transport: &dyn CaptureTransport,
        shutdown: CancellationToken,
    ) -> AppResult<Arc<Self>> {
        let cdp = transport.connect().await?;
        // Frames bypass the client's broadcast ring: at several per second and up to ~100 KB
        // each, that ring would copy them once per subscriber and hold hundreds of them.
        let frames = cdp.events_targeted("Page.screencastFrame");
        let events = cdp.events();
        let link = Arc::new(Self {
            epoch: cdp.connection_epoch(),
            cdp,
            frames: Mutex::new(HashMap::new()),
            detached: Mutex::new(HashMap::new()),
            broken: shutdown.child_token(),
        });
        tokio::spawn(route_frames(link.clone(), frames));
        tokio::spawn(watch_detaches(link.clone(), events));
        tokio::spawn(watchdog(link.clone()));
        Ok(link)
    }

    pub(super) async fn attach(&self, target_id: &str) -> AppResult<Attached> {
        let reply = self
            .call(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
                None,
            )
            .await?;
        let id = reply
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Internal("attachToTarget returned no sessionId".into()))?;
        let detached = self.broken.child_token();
        lock(&self.detached).insert(id.to_string(), detached.clone());
        Ok(Attached {
            id: SessionId::from(id),
            detached,
        })
    }

    /// Screencast frames for one attached session. Register before starting the screencast.
    pub(super) fn frames(&self, session: &Attached) -> mpsc::UnboundedReceiver<Value> {
        let (tx, rx) = mpsc::unbounded_channel();
        lock(&self.frames).insert(session.id.0.clone(), tx);
        rx
    }

    pub(super) async fn send(
        &self,
        method: &str,
        params: Value,
        session: &Attached,
    ) -> AppResult<Value> {
        self.call(method, params, Some(&session.id)).await
    }

    /// Forgets the session and detaches it; best effort, the tab may already be gone.
    pub(super) async fn release(&self, session: &Attached) {
        lock(&self.frames).remove(&session.id.0);
        lock(&self.detached).remove(&session.id.0);
        let _ = self
            .call(
                "Target.detachFromTarget",
                json!({ "sessionId": session.id.0 }),
                None,
            )
            .await;
    }

    async fn call(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> AppResult<Value> {
        self.cdp
            .send(method, params, session)
            .await
            .map_err(|error| AppError::Internal(format!("{method}: {error}")))
    }
}

async fn route_frames(link: Arc<Link>, mut frames: mpsc::UnboundedReceiver<CdpEvent>) {
    loop {
        let event = tokio::select! {
            () = link.broken.cancelled() => return,
            event = frames.recv() => event,
        };
        let Some(event) = event else {
            link.broken.cancel();
            return;
        };
        let Some(session_id) = event.session_id else {
            continue;
        };
        if let Some(tx) = lock(&link.frames).get(&session_id.0) {
            let _ = tx.send(event.params);
        }
    }
}

async fn watch_detaches(link: Arc<Link>, mut events: broadcast::Receiver<CdpEvent>) {
    loop {
        let event = tokio::select! {
            () = link.broken.cancelled() => return,
            event = events.recv() => event,
        };
        match event {
            Ok(event) if event.method == "Target.detachedFromTarget" => {
                let session_id = event.params.get("sessionId").and_then(Value::as_str);
                if let Some(token) = session_id.and_then(|id| lock(&link.detached).remove(id)) {
                    token.cancel();
                }
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                link.broken.cancel();
                return;
            }
        }
    }
}

async fn watchdog(link: Arc<Link>) {
    let mut tick = tokio::time::interval(WATCHDOG_INTERVAL);
    loop {
        tokio::select! {
            () = link.broken.cancelled() => return,
            _ = tick.tick() => {}
        }
        if !link.cdp.is_connected() || link.cdp.connection_epoch() != link.epoch {
            link.broken.cancel();
            return;
        }
    }
}

/// A poisoned map only means another task panicked mid-insert; the map itself is still usable.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
