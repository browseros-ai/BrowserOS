//! Playwright's network-idle state belongs to the entire frame subtree. CDP's lifecycle
//! replay supplies each document's state without enabling Network; stable document snapshots
//! prevent a replaced or newly attached child from borrowing an earlier subtree's readiness.

use browseros_core::{FrameId, ProtocolSession, SessionId, TabId};
use futures_util::future;

use crate::op_scope::OpScope;
use crate::page::document_watch::DocumentWatch;
use crate::page::{LoadState, frame_tree, navigation};
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
        let waits = documents.iter().map(|document| {
            idle_frame(
                pw,
                scope,
                page,
                (root_id.clone(), root.clone()),
                &document.frame,
                frame,
            )
        });
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
    requested: &FrameId,
) -> Result<bool, PlaywrightError> {
    let (session_id, session) = match navigation::frame_host(pw, scope, page, root, frame).await {
        Ok(host) => host,
        Err(error) if frame != requested && error.error_code() == Some(ErrorCode::TargetClosed) => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let mut watch = DocumentWatch::new(pw.browser(), session, session_id, frame.clone());
    let wait = async {
        watch.enable_page().await?;
        watch.read_current().await?;
        watch.enable_lifecycle().await?;
        watch.reached(LoadState::NetworkIdle).await
    };
    match wait.await {
        Ok(()) => Ok(true),
        Err(error) => {
            let error = navigation::watch_error(scope, &error);
            if frame != requested && error.error_code() == Some(ErrorCode::TargetClosed) {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::testing::TestConnection;
    use browseros_core::{BrowserView, FrameId, TabId};
    use serde_json::json;

    use crate::op::Op;
    use crate::op_scope::OpScope;
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;
    use crate::test_support::{self, RecordingHooks};
    use crate::{ErrorCode, Playwright};

    #[tokio::test]
    async fn child_disappearance_during_lookup_retries_but_a_missing_root_fails()
    -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([]);
        let test_support::OneTab { browser } = test_support::one_tab(&connection).await?;
        connection.always("Target.getTargets", json!({ "targetInfos": [] }))?;
        connection.always(
            "Page.getFrameTree",
            json!({ "frameTree": {
                "frame": { "id": "target-7", "loaderId": "L", "url": "https://a.test/" }
            }}),
        )?;
        let pw = Playwright::new(Arc::clone(&browser), Arc::new(RecordingHooks::default()));
        let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
        let scope = OpScope::new(
            &env,
            Op::from_name("page.goto").ok_or("goto op")?,
            CallHeader::default(),
        );
        let admitted = scope.admit(&browser, TabId(7)).await?;
        let root_id = admitted.session.session_id.clone();
        let root = browser.protocol_session(root_id.clone());
        let child = FrameId("removed".to_owned());
        assert!(
            !super::idle_frame(
                &pw,
                &scope,
                TabId(7),
                (root_id.clone(), root.clone()),
                &child,
                &FrameId("target-7".to_owned())
            )
            .await?
        );
        let error = super::idle_frame(&pw, &scope, TabId(7), (root_id, root), &child, &child)
            .await
            .err()
            .ok_or("missing root must fail")?;
        assert_eq!(error.error_code(), Some(ErrorCode::TargetClosed));
        Ok(())
    }
}
