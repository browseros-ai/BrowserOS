//! A transparent capture's temporary browser state and ownership of the shared capture queue.
//! The queue outlives a cancelled capture until its background reset has been attempted, so
//! an audit or desktop image cannot run inside an SDK screenshot's transparent interval.

use crate::{CoreError, ProtocolSession};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    runtime::Handle,
    sync::{Mutex, OwnedMutexGuard},
    time,
};

/// One temporary override and its exclusive ownership of the session's capture queue.
#[must_use = "the guard retains capture ownership through background restoration"]
pub(super) struct TransparentBackground {
    session: ProtocolSession,
    queue: Option<OwnedMutexGuard<()>>,
}

impl TransparentBackground {
    pub(super) async fn acquire(
        session: &ProtocolSession,
        queue: Arc<Mutex<()>>,
    ) -> Result<Self, CoreError> {
        let queue = queue.lock_owned().await;
        // Register before sending: losing the set reply does not mean it had no effect.
        let guard = Self {
            session: session.clone(),
            queue: Some(queue),
        };
        session
            .send::<_, Value>(
                "Emulation.setDefaultBackgroundColorOverride",
                json!({
                    "color": { "r": 0, "g": 0, "b": 0, "a": 0 },
                }),
            )
            .await?;
        Ok(guard)
    }

    pub(super) async fn restore(mut self) {
        reset(&self.session).await;
        self.queue = None;
    }
}

impl Drop for TransparentBackground {
    fn drop(&mut self) {
        let Some(queue) = self.queue.take() else {
            return;
        };
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        let session = self.session.clone();
        drop(runtime.spawn(async move {
            let _queue = queue;
            reset(&session).await;
        }));
    }
}

async fn reset(session: &ProtocolSession) {
    // A dead renderer must not keep all later captures queued forever. CDP still receives
    // the reset before the queue is released, even when its acknowledgement is lost.
    let _reset = time::timeout(
        Duration::from_secs(5),
        session.send::<_, Value>("Emulation.setDefaultBackgroundColorOverride", json!({})),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::super::{ScreenshotBackground, capture_cdp};
    use crate::{
        CdpConnection, ProtocolSession, SessionId,
        testing::{TestConnection, cdp_pending},
    };
    use serde_json::json;
    use std::{error::Error, sync::Arc, time::Duration};
    use tokio::{task, time};

    /// Audit JPEGs cannot enter a transparent interval, including while a cancelled capture's
    /// reset acknowledgement is stalled. The reset's bound eventually releases later captures.
    #[tokio::test(start_paused = true)]
    async fn cancellation_keeps_other_captures_queued_through_background_reset()
    -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([
            ("Emulation.setDefaultBackgroundColorOverride", json!({})),
            ("Emulation.setDefaultBackgroundColorOverride", cdp_pending()),
            ("Page.captureScreenshot", cdp_pending()),
            ("Page.captureScreenshot", json!({ "data": "cG5n" })),
        ]);
        let session = ProtocolSession::for_session(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            SessionId::from("transparent-capture-cancel"),
        );
        let png_session = session.clone();
        let png = tokio::spawn(async move {
            capture_cdp(
                &png_session,
                json!({ "format": "png" }),
                ScreenshotBackground::Transparent,
            )
            .await
        });
        for _ in 0..20 {
            task::yield_now().await;
        }
        let jpeg = tokio::spawn(async move {
            capture_cdp(
                &session,
                json!({ "format": "jpeg" }),
                ScreenshotBackground::Default,
            )
            .await
        });
        for _ in 0..20 {
            task::yield_now().await;
        }
        assert_eq!(
            connection
                .calls()?
                .iter()
                .filter(|call| call.method == "Page.captureScreenshot")
                .count(),
            1
        );
        png.abort();
        let _cancelled = png.await;
        for _ in 0..20 {
            task::yield_now().await;
        }
        assert_eq!(
            connection
                .calls()?
                .iter()
                .filter(|call| call.method == "Emulation.setDefaultBackgroundColorOverride")
                .count(),
            2
        );
        assert!(
            !jpeg.is_finished(),
            "the shared capture queue must outlive cancelled PNG capture"
        );
        time::advance(Duration::from_secs(6)).await;
        assert_eq!(jpeg.await??, "cG5n");
        let order: Vec<_> = connection
            .calls()?
            .iter()
            .filter_map(|call| match call.method.as_str() {
                "Emulation.setDefaultBackgroundColorOverride" => {
                    Some(if call.params.get("color").is_some() {
                        "set"
                    } else {
                        "reset"
                    })
                }
                "Page.captureScreenshot" => Some(if call.params["format"] == "png" {
                    "png"
                } else {
                    "jpeg"
                }),
                _ => None,
            })
            .collect();
        assert_eq!(order, ["set", "png", "reset", "jpeg"]);
        Ok(())
    }

    /// The set may have reached the browser when its reply is lost. No later image is allowed
    /// before the compensating reset, even when no pixel capture was ever sent.
    #[tokio::test(start_paused = true)]
    async fn a_lost_set_reply_is_reset_before_the_next_capture() -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([
            ("Emulation.setDefaultBackgroundColorOverride", cdp_pending()),
            ("Emulation.setDefaultBackgroundColorOverride", json!({})),
            ("Page.captureScreenshot", json!({ "data": "cG5n" })),
        ]);
        let session = ProtocolSession::for_session(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            SessionId::from("transparent-set-cancel"),
        );
        let png_session = session.clone();
        let png = tokio::spawn(async move {
            capture_cdp(
                &png_session,
                json!({ "format": "png" }),
                ScreenshotBackground::Transparent,
            )
            .await
        });
        for _ in 0..20 {
            task::yield_now().await;
        }
        let jpeg = tokio::spawn(async move {
            capture_cdp(
                &session,
                json!({ "format": "jpeg" }),
                ScreenshotBackground::Default,
            )
            .await
        });
        for _ in 0..20 {
            task::yield_now().await;
        }
        assert!(
            connection
                .calls()?
                .iter()
                .all(|call| call.method != "Page.captureScreenshot")
        );
        png.abort();
        let _cancelled = png.await;
        assert_eq!(jpeg.await??, "cG5n");
        let calls = connection.calls()?;
        let reset = calls
            .iter()
            .position(|call| {
                call.method == "Emulation.setDefaultBackgroundColorOverride"
                    && call.params == json!({})
            })
            .ok_or("reset was not sent")?;
        let capture = calls
            .iter()
            .position(|call| call.method == "Page.captureScreenshot")
            .ok_or("JPEG was not captured")?;
        assert!(
            reset < capture,
            "lost set replies must not leak transparency to later images"
        );
        Ok(())
    }
}
