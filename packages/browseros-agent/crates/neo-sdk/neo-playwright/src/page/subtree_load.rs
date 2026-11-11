//! Playwright's network-idle state belongs to the entire frame subtree. CDP's lifecycle
//! replay supplies each document's state without enabling Network; stable document snapshots
//! prevent a replaced or newly attached child from borrowing an earlier subtree's readiness.

use browseros_core::{FrameId, ProtocolSession, SessionId, TabId};
use futures_util::future;

use crate::op_scope::OpScope;
use crate::page::document_watch::{DocumentWatch, WatchError};
use crate::page::navigation::{frame_host, watch_error};
use crate::page::{LoadState, frame_tree};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// Waits for every currently attached document under `frame`, then rechecks the topology.
/// The caller owns the deadline; detaching a child retries the subtree rather than failing
/// the parent's navigation. Each retry waits on lifecycle events, never a polling delay.
pub(crate) async fn network_idle(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: TabId,
    frame: &FrameId,
) -> Result<(), PlaywrightError> {
    loop {
        let admitted = scope.admit(pw.browser(), page).await?;
        let root_id = admitted.session.session_id.clone();
        let root = pw.browser().protocol_session(root_id.clone());
        let read = || frame_tree::read(pw.browser(), &admitted.core_page, &root);
        let documents = read()
            .await
            .map_err(|error| scope.core_error(&error))?
            .documents(frame);
        if documents.is_empty() {
            return Err(scope.fail(ErrorCode::TargetClosed, "Frame was detached"));
        }
        let waits = documents
            .iter()
            .map(|(id, _)| idle_frame(pw, scope, page, (root_id.clone(), root.clone()), id));
        if future::try_join_all(waits)
            .await?
            .into_iter()
            .all(|ready| ready)
            && read()
                .await
                .map_err(|error| scope.core_error(&error))?
                .documents(frame)
                == documents
        {
            return Ok(());
        }
    }
}

/// One frame's current loader and lifecycle replay, subscribed before any setup awaits.
async fn idle_frame(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: TabId,
    root: (SessionId, ProtocolSession),
    frame: &FrameId,
) -> Result<bool, PlaywrightError> {
    let (session_id, session) = frame_host(pw, scope, page, root, frame).await?;
    let mut watch = DocumentWatch::new(pw.browser(), session, session_id, frame.clone());
    let wait = async {
        watch.enable_page().await?;
        watch.read_current().await?;
        watch.enable_lifecycle().await?;
        watch.reached(LoadState::NetworkIdle).await
    };
    match wait.await {
        Ok(()) => Ok(true),
        Err(WatchError::Left | WatchError::FrameDetached) => Ok(false),
        Err(error) => Err(watch_error(scope, &error)),
    }
}
