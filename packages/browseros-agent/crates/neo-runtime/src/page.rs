//! The run's attached tab as browser calls see it: the run's CDP session, owned so that it
//! lives exactly as long as anything that may still write on it.
//!
//! Lifetime: [`PageSession`] owns the run's [`Session`], whose `Drop` detaches, and the run
//! shares it as `Arc<PageSession>`. The dispatcher holds one, and so will every task that writes
//! on the session. The session therefore detaches when the last holder lets go, after the run's
//! realm is gone and after any such task is done, so no end-of-run step has to order a drain
//! before a detach. This also covers a caller that drops the run's future: the realm and the
//! dispatcher drop with it, and the page follows.

use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError, SessionId, target};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::time;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::run::PageId;

/// Detaching is best effort. It runs in the background after a run, so this bound only keeps
/// a hung browser from piling up detach tasks.
const DETACH_TIMEOUT: Duration = Duration::from_secs(1);

/// The run's CDP session on its tab, which owns the attach: dropping it detaches. The
/// `Runtime`'s connection outlives every run, so a session nobody detaches stays attached
/// until the browser exits. Tying the detach to the value covers every way a run ends after
/// the browser answered the attach: the script's end, a failed or stopped setup, an answer
/// that arrives after the run stopped, and a caller that drops the run's future.
pub(crate) struct Session {
    pub(crate) cdp: CdpClient,
    pub(crate) id: SessionId,
    /// browseros-cdp's socket incarnation, read just before the attach is sent: the
    /// incarnation `id` belongs to. Input is written with `send_at_epoch(epoch, …)`, so it is
    /// never sent on a reconnected socket under a session id that does not exist there.
    #[expect(
        dead_code,
        reason = "read by the mouse writer, Frame.click's first input step"
    )]
    epoch: u64,
    /// Drop is synchronous, so the detach runs as a task on this runtime.
    tokio: Handle,
}

impl Session {
    pub(crate) async fn attach(
        cdp: CdpClient,
        page: PageId,
        tokio: Handle,
    ) -> Result<Self, CdpError> {
        let epoch = cdp.epoch();
        let attached: target::AttachToTargetResult = cdp
            .send_typed(
                "Target.attachToTarget",
                target::AttachToTargetParams {
                    target_id: page.as_str().to_string(),
                    flatten: Some(true),
                },
                None,
            )
            .await?;
        Ok(Self {
            cdp,
            id: SessionId::from(attached.session_id),
            epoch,
            tokio,
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let cdp = self.cdp.clone();
        let session = self.id.clone();
        self.tokio.spawn(async move {
            let detach = cdp.send_typed::<_, Value>(
                "Target.detachFromTarget",
                target::DetachFromTargetParams {
                    session_id: Some(session.as_str().to_string()),
                    target_id: None,
                },
                None,
            );
            match time::timeout(DETACH_TIMEOUT, detach).await {
                Ok(Ok(_)) => {}
                // Expected once the tab has closed: the session went with it.
                Ok(Err(error)) => debug!(%session, %error, "detaching a run's session failed"),
                Err(_) => debug!(%session, "detaching a run's session timed out"),
            }
        });
    }
}

/// The run's attached tab, shared by the dispatcher and every task that writes on its session.
pub(crate) struct PageSession {
    session: Session,
    /// Cancelled when the tab closes; every call's `Progress` races it, so a call on a closed
    /// tab ends with Playwright's `TargetClosedError` instead of waiting out its timeout.
    closed: CancellationToken,
}

impl PageSession {
    pub(crate) fn new(session: Session) -> Self {
        Self {
            session,
            closed: CancellationToken::new(),
        }
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        &self.session.id
    }

    pub(crate) fn closed(&self) -> CancellationToken {
        self.closed.clone()
    }
}
