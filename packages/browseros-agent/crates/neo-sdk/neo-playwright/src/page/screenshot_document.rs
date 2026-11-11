//! Native capture can lose its reply when navigation replaces the document mid-snapshot.
//! Abandon that obsolete read; the screenshot owner restores its guards before retrying.

use std::future::Future;
use std::time::Duration;

use tokio::time;

use crate::op_scope::OpScope;
use crate::page::document_watch::{DocumentWatch, Expect};
use crate::page::{OpPage, navigation};
use crate::{ErrorCode, PlaywrightError};

/// The caller retains preparation and capture ownership through either outcome's cleanup.
pub(super) enum Attempt {
    Captured(String),
    Navigated,
    RetryPreparation,
}

pub(super) async fn capture<F>(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    pixels: F,
) -> Result<Attempt, PlaywrightError>
where
    F: Future<Output = Result<Attempt, PlaywrightError>>,
{
    let endpoint = page.main_endpoint();
    let session = endpoint.session.session_id().cloned().ok_or_else(|| {
        scope.fail(
            ErrorCode::Internal,
            "The screenshot page has no target session",
        )
    })?;
    // Admission already enabled Page events. Share DocumentWatch's broadcast: a targeted
    // subscription would consume the commit needed by a concurrent reload's own watcher.
    let mut watch = DocumentWatch::new(page.browser(), endpoint.session, session, endpoint.frame);
    // Seed the loader for lag recovery, but do not hold otherwise-ready pixels behind stalled
    // renderer metadata. The already-subscribed watch can still observe live commits without it.
    if let Ok(current) = time::timeout(Duration::from_secs(1), watch.read_current()).await {
        current.map_err(|error| navigation::watch_error(scope, &error))?;
    }
    tokio::select! {
        biased;
        changed = watch.arrived(Expect::NewDocument) => {
            changed.map_err(|error| navigation::watch_error(scope, &error))?;
            // Dropping the pixel future frees its CDP reply slot. The caller still owns its
            // preparation guard, including any shared core lease held for explicit masks.
            Ok(Attempt::Navigated)
        },
        data = pixels => data,
    }
}
