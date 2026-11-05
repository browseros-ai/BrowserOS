//! Prompt notice of closed tabs.

use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::CdpEvent;
use browseros_core::BrowserSession;
use serde_json::json;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::time::{self, MissedTickBehavior};

/// How often the watch looks for a replaced socket. The client publishes no reconnect event,
/// and target discovery dies with the old socket, so this bounds how long closes go unseen
/// after a reconnect.
const EPOCH_CHECK: Duration = Duration::from_secs(1);

/// Keeps core's page registry honest about tabs someone else closed.
///
/// Core notices a closed tab only on its own close or its next tab listing, so a tab the owner
/// closed would keep its slots, its lock key and its idle entry until something listed tabs.
/// This asks Chromium for target events on every connection (`Target.setDiscoverTargets` does
/// not survive a reconnect) and lists tabs when a target is destroyed. The listing's
/// reconciliation fires core's detach hook, which drops the tab from `TabRegistry`. A burst of
/// destroy events costs one listing, and so does a reconnect, which also catches tabs closed
/// while the socket was down.
///
/// Runs for the process's life: the session's event channel outlives every reconnect.
pub(crate) async fn tab_watch(session: Arc<BrowserSession>) {
    let connection = session.connection();
    // Subscribed before discovery is asked for, so no destroy event can slip in between.
    let mut events = session.cdp_events();
    let mut checks = time::interval(EPOCH_CHECK);
    checks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut watched_epoch = None;
    loop {
        tokio::select! {
            _ = checks.tick() => {
                let epoch = connection.connection_epoch();
                if !connection.is_connected() || watched_epoch == Some(epoch) {
                    continue;
                }
                // At this epoch, so a socket replaced mid-call fails here and is retried on the
                // next check instead of being marked as watched.
                let discover = connection
                    .send_at_epoch(epoch, "Target.setDiscoverTargets", json!({ "discover": true }), None)
                    .await;
                match discover {
                    Ok(_) => {
                        watched_epoch = Some(epoch);
                        reconcile(&session).await;
                    }
                    Err(error) => tracing::debug!(%error, epoch, "target discovery not enabled yet"),
                }
            }
            event = events.recv() => match event {
                Ok(CdpEvent { method, .. }) if method == "Target.targetDestroyed" => {
                    drain(&mut events);
                    reconcile(&session).await;
                }
                Ok(_) => {}
                // Missed events may have been destroys: reconcile anyway.
                Err(RecvError::Lagged(_)) => reconcile(&session).await,
                Err(RecvError::Closed) => return,
            },
        }
    }
}

/// Drops whatever is queued, so a burst of destroys costs one listing. The listing reads every
/// tab, so the dropped events carry nothing it would miss.
fn drain(events: &mut Receiver<CdpEvent>) {
    loop {
        match events.try_recv() {
            Ok(_) | Err(TryRecvError::Lagged(_)) => {}
            Err(TryRecvError::Empty | TryRecvError::Closed) => return,
        }
    }
}

async fn reconcile(session: &BrowserSession) {
    if let Err(error) = session.pages.list().await {
        tracing::debug!(%error, "tab listing failed; the next destroy event or reconnect retries it");
    }
}
