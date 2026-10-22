//! Bridges host grants to MCP catalog invalidation for both protocol lifecycles.
//! Legacy notifications belong to a transport; modern notifications belong to a cancellable
//! subscription. Neither watcher retains the service or its application state.
use browseros_core::FeatureFlags;
use rmcp::{ErrorData, Peer, RoleServer, service::SubscriptionContext};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// One legacy watcher per service, cancelled when its owning transport service is dropped.
#[derive(Default)]
pub struct FeatureNotifications {
    started: AtomicBool,
    cancel: CancellationToken,
}

impl FeatureNotifications {
    pub fn start(&self, mut flags: watch::Receiver<FeatureFlags>, peer: Peer<RoleServer>) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = cancel.cancelled() => {},
                () = async {
                    while flags.changed().await.is_ok() {
                        flags.borrow_and_update();
                        if peer.notify_tool_list_changed().await.is_err() { break; }
                    }
                } => {},
            }
        });
    }
}

impl Drop for FeatureNotifications {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Modern peers receive changes only on a subscription that accepted tool invalidations.
/// Cancellation also interrupts a blocked send, so a disconnected client retains no task.
pub async fn listen(
    mut flags: watch::Receiver<FeatureFlags>,
    context: SubscriptionContext,
) -> Result<(), ErrorData> {
    if context.accepted().tools_list_changed != Some(true) {
        context.cancelled().await;
        return Ok(());
    }
    tokio::select! {
        () = context.cancelled() => {},
        () = async {
            while flags.changed().await.is_ok() {
                flags.borrow_and_update();
                if context.sink().notify_tool_list_changed().await.is_err() { break; }
            }
        } => {},
    }
    Ok(())
}
