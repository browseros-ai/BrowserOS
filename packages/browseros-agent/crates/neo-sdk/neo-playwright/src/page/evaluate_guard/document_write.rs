//! `setContent`'s document write on the main-world gateway. The probe writes the HTML into the
//! frame from the sealed world, but the HTML's scripts and handlers run in the main world from
//! the write on, so the write goes through the same declaration check and host recheck as a
//! script does.
//!
//! ```text
//!  DocumentWrite::open  require(MainWorld) ── admit ── the frame, within the op's timeout
//!  write                DocumentWatch on the frame's session (subscribe, Page and lifecycle on)
//!                       ── send_effect ── probe `setContent` with a user gesture: document.open,
//!                       write, close; the probe unboots ── the old seq retires, the new
//!                       document's world installs ── Page.documentOpened ── waitUntil, all within
//!                       the op's timeout
//! ```

use std::future::Future;

use browseros_core::FrameId;

use super::{DOCUMENT_REPLACED, TreeFrame, frame_endpoint};
use crate::op_scope::{OpScope, Sent};
use crate::op_spec::Capability;
use crate::page::document_watch::DocumentWatch;
use crate::page::{LoadState, OpPage, navigation, world_call};
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, Ran};
use crate::target::PageRef;
use crate::world::SealedWorld;
use crate::{ErrorCode, Playwright, PlaywrightError, resolve};

/// A frame whose document `setContent` replaces, found once its op's `MainWorld` declaration was
/// checked: the only way to [`DocumentWrite::write`].
#[must_use]
pub(crate) struct DocumentWrite<'op> {
    pw: &'op Playwright,
    page: OpPage<'op>,
    frame: TreeFrame,
}

impl<'op> DocumentWrite<'op> {
    /// Opens the gateway for a document write, as `EvaluateGuard::open` does for a script: the
    /// declaration before anything else, then the page and the frame whose document the write
    /// replaces. Upstream's timeout covers the whole `setContent` call, so finding the frame and
    /// installing its world run within the op's timeout: a renderer that never answers the
    /// install holds the op no longer than that, with nothing sent.
    pub(crate) async fn open(
        pw: &'op Playwright,
        scope: &'op OpScope<'_>,
        page: &PageRef,
        frame: Option<&FrameId>,
    ) -> Result<Self, PlaywrightError> {
        scope.require(Capability::MainWorld)?;
        let page = OpPage::admit_handle(pw, scope, page).await?;
        let world = bounded(scope, world_call::frame_world(&page, scope, frame)).await?;
        let frame = frame_endpoint(&page, world.session(), world.frame().clone());
        Ok(Self { pw, page, frame })
    }

    /// Writes `html` and waits for `wait_until`, all within the op's timeout.
    ///
    /// Measured on BrowserOS, both ways upstream and CDP offer (document.open, write and close
    /// from an isolated world, and `Page.setDocumentContent`) behave the same: the loader stays,
    /// the window and every world outlive the write, and Chromium fires `Page.documentOpened`,
    /// then `init`, `DOMContentLoaded` and `load` again under that loader. The watch subscribes
    /// before the write, so none of them is missed.
    pub(crate) async fn write(
        &self,
        scope: &OpScope<'_>,
        html: &str,
        wait_until: LoadState,
    ) -> Result<(), PlaywrightError> {
        let endpoint = self.frame.endpoint.clone();
        let session_id = endpoint
            .session
            .session_id()
            .cloned()
            .ok_or_else(|| scope.fail(ErrorCode::Internal, "the frame has no session"))?;
        let mut watch = DocumentWatch::new(
            self.pw.browser(),
            endpoint.session.clone(),
            session_id,
            endpoint.frame.clone(),
        );
        let watched = |error| navigation::watch_error(scope, &error);
        bounded(scope, async {
            watch.enable_page().await.map_err(watched)?;
            watch.read_current().await.map_err(watched)?;
            watch.enable_lifecycle().await.map_err(watched)?;
            let old = self.replace_document(scope, html).await?;
            // The world outlived document.open, unbooted: its seq retires, and the new document
            // is booted under the next one before any other op can mint a ref in it, so a ref
            // from before fails instead of naming a new element.
            self.page.frames().invalidate(&old);
            self.page
                .world(&endpoint, self.frame.place.clone())
                .await
                .map_err(|failure| resolve::failed(scope, failure))?;
            watch.opened().await.map_err(watched)?;
            watch.reached(wait_until).await.map_err(watched)
        })
        .await
    }

    /// Writes `html` in the frame's world and returns that world, now unbooted. A world whose
    /// document changed since its install fails `Gone(Ran::No)` without running the request, so
    /// the write is tried once more in the new document's world, after the host's policies run
    /// again. A write a navigation cut off while it was pending may have run, the HTML's inline
    /// scripts with it, so it is never sent again: the op fails with the message upstream's
    /// setContent gives when that navigation destroys its evaluation's context.
    async fn replace_document(
        &self,
        scope: &OpScope<'_>,
        html: &str,
    ) -> Result<SealedWorld, PlaywrightError> {
        let install = || async {
            self.page
                .world(&self.frame.endpoint, self.frame.place.clone())
                .await
                .map_err(|failure| resolve::failed(scope, failure))
        };
        let mut world = install().await?;
        let mut retried = false;
        loop {
            let sent = scope.send_effect()?;
            match write_document(&world, html, &sent).await {
                Ok(ProbeReply::Replaced) => return Ok(world),
                Ok(other) => {
                    return Err(resolve::failed(
                        scope,
                        world_call::unexpected("setContent", other),
                    ));
                }
                Err(ProbeFailure::Gone(Ran::Maybe)) => {
                    return Err(scope.fail(ErrorCode::PageError, DOCUMENT_REPLACED));
                }
                Err(ProbeFailure::Gone(Ran::No)) if !retried => {
                    retried = true;
                    self.page.frames().invalidate(&world);
                    world = install().await?;
                }
                Err(failure) => return Err(resolve::failed(scope, failure)),
            }
        }
    }
}

/// `step`'s outcome within the op's timeout, which upstream's covers the whole call with.
async fn bounded<T>(
    scope: &OpScope<'_>,
    step: impl Future<Output = Result<T, PlaywrightError>>,
) -> Result<T, PlaywrightError> {
    match scope.within(step).await {
        Ok(stepped) => stepped,
        Err(_expired) => Err(scope.timed_out(ErrorCode::WaitTimeout)),
    }
}

/// The probe's `setContent`, which replaces the frame's document, sent with the user gesture
/// upstream's utility-world write carries.
async fn write_document(
    world: &SealedWorld,
    html: &str,
    _sent: &Sent<'_>,
) -> Result<ProbeReply, ProbeFailure> {
    let request = ProbeRequest::SetContent {
        html: html.to_owned(),
    };
    probe::call_with_gesture(world, &request).await
}

#[cfg(test)]
mod tests;
