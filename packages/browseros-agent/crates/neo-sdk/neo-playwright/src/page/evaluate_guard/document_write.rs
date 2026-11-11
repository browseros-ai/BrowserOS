//! `setContent`'s document write on the main-world gateway. The probe writes the HTML into the
//! guard's frame from the sealed world, but the HTML's scripts and handlers run in the main world
//! from the write on, with the page's origin and whatever the old document's scripts kept. So the
//! write and the wait for its load state are together the guard's one send, between its sweeps.
//!
//! ```text
//!  open_document   require(MainWorld) ── admit ── the frame, within the op's timeout ── sweep
//!                  of the frame's site, to the op's deadline at the latest
//!  write           DocumentWatch on the frame's session (subscribe, Page and lifecycle on) ──
//!                  send_effect_reaching(the group's other tabs) ── probe `setContent` with a
//!                  user gesture: document.open, write, close; the world and its seq stay ──
//!                  Page.documentOpened ── waitUntil, each step within the op's timeout, a
//!                  retry's sweep to its deadline ──▶ Pending, holding how it ended
//!  Pending::sweep  the same sweep, now of the new document's frames too
//!  close           `{}`, or the failure with every swept value masked in its text
//! ```

use std::future::Future;

use browseros_core::FrameId;
use browseros_core::sensitive_fields::FieldValues;
use serde_json::{Value, json};

use super::{
    DOCUMENT_REPLACED, DocumentWrite, EvaluateGuard, GuardCore, Guarded, Pending, Written,
};
use crate::op_scope::{OpScope, Sent};
use crate::op_spec::Capability;
use crate::page::document_watch::DocumentWatch;
use crate::page::{LoadState, OpPage, evaluate_guard, navigation, page_value, world_call};
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, Ran};
use crate::target::PageRef;
use crate::world::SealedWorld;
use crate::{ErrorCode, Playwright, PlaywrightError, resolve};

impl<'op> EvaluateGuard<'op, DocumentWrite> {
    /// Opens the gateway for a document write, as `open` does for a script: the declaration
    /// before anything else, then the page, the frame whose document the write replaces, and
    /// the first sweep of that frame's site. Upstream's timeout covers the whole `setContent`
    /// call, so finding the frame and installing its world run within the op's timeout: a
    /// renderer that never answers the install holds the op no longer than that, with nothing
    /// sent. The sweep runs outside it, its every try ending at the op's deadline at the latest
    /// (`GuardCore::sweep_once`), so a timeout never drops it with what the frames that answered
    /// found.
    pub(crate) async fn open_document(
        pw: &'op Playwright,
        scope: &'op OpScope<'_>,
        page: &PageRef,
        frame: Option<&FrameId>,
    ) -> Result<Self, PlaywrightError> {
        scope.require(Capability::MainWorld)?;
        let page = OpPage::admit_handle(pw, scope, page).await?;
        let world = bounded(scope, world_call::frame_world(&page, scope, frame)).await?;
        let frame = evaluate_guard::frame_endpoint(&page, world.session(), world.frame().clone());
        let mut core = GuardCore {
            pw,
            page,
            frame,
            follow: None,
            swept: FieldValues::default(),
            group: Vec::new(),
            sent: false,
        };
        core.sweep(scope, DocumentWrite::NOT_SENT, || true).await?;
        Ok(Self {
            core,
            guarded: DocumentWrite,
        })
    }

    /// Writes `html` and waits for `wait_until`, each step within the op's timeout. Once anything
    /// was written, how that ended, the timeout included, stays in the [`Pending`]: the page's
    /// scripts ran, so even a failure leaves only after the sweep that follows them. A failure
    /// before the write (the host's refusal, a watch that could not start) sent nothing, and
    /// leaves at once.
    pub(crate) async fn write(
        mut self,
        scope: &OpScope<'_>,
        html: &str,
        wait_until: LoadState,
    ) -> Result<Pending<'op, DocumentWrite>, PlaywrightError> {
        match self.core.write(scope, html, wait_until).await {
            Err(error) if !self.core.sent => Err(error),
            ended => Ok(Pending {
                core: self.core,
                guarded: self.guarded,
                outcome: Written(ended),
            }),
        }
    }

    /// The write's end, after the sweep that followed it: the op's `{}`, or its failure with
    /// every value the sweeps found masked in its text. A write's output exists only here, so
    /// `setContent` cannot skip the sweep after its load state.
    pub(crate) fn close(self, written: Written) -> Result<Value, PlaywrightError> {
        let swept = &self.core.swept;
        match written.0 {
            Ok(()) => Ok(json!({})),
            Err(error) => Err(error.mask_message(|text| page_value::mask_text(swept, text))),
        }
    }
}

impl GuardCore<'_> {
    /// The write and its load state, on the guard's frame.
    ///
    /// Measured on BrowserOS, both ways upstream and CDP offer (document.open, write and close
    /// from an isolated world, and `Page.setDocumentContent`) behave the same: the loader stays,
    /// the window and every world outlive the write, and Chromium fires `Page.documentOpened`,
    /// then `init`, `DOMContentLoaded` and `load` again under that loader. The watch subscribes
    /// before the write, so none of them is missed.
    async fn write(
        &mut self,
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
            watch.enable_lifecycle().await.map_err(watched)
        })
        .await?;
        // The world outlives document.open with its engine and its seq, as upstream's frame
        // keeps its seq when no new document commits: the next snapshot's refs keep the frame's
        // prefix, and a ref or pin from before names a removed element, so it fails as stale.
        self.replace_document(scope, html).await?;
        bounded(scope, async {
            watch.opened().await.map_err(watched)?;
            watch.reached(wait_until).await.map_err(watched)
        })
        .await
    }

    /// Writes `html` in the frame's world, which stays booted. A world whose
    /// document changed since its install fails `Gone(Ran::No)` without running the request, so
    /// the write is tried once more in the new document's world. That document is not the one
    /// the sweep before the first write saw, and its scripts may have copied a value anywhere
    /// or opened another window, so its site is swept first, and only then do the host's
    /// policies run again and the write go out: no write without a sweep since the document's
    /// last change. A write a navigation cut off while it was pending may have run, the HTML's
    /// inline scripts with it, so it is never sent again: the op fails with the message
    /// upstream's setContent gives when that navigation destroys its evaluation's context.
    ///
    /// Each attempt is marked sent before it goes out, after the host's rules ran on the op's
    /// tab and on the group's other tabs the sweep before it found, whose scripts the written
    /// HTML can reach, so a failure after it leaves only after the guard's sweep, and the guard
    /// reports on drop. One that reached no document ran nothing, so it is unmarked: a sweep
    /// for the next attempt that fails leaves at once, saying the content was not set.
    async fn replace_document(
        &mut self,
        scope: &OpScope<'_>,
        html: &str,
    ) -> Result<(), PlaywrightError> {
        let installed = |world: Result<SealedWorld, ProbeFailure>| {
            world.map_err(|failure| resolve::failed(scope, failure))
        };
        let mut world = bounded(scope, async {
            installed(
                self.page
                    .world(&self.frame.endpoint, self.frame.place.clone())
                    .await,
            )
        })
        .await?;
        let mut retried = false;
        loop {
            let reach = bounded(scope, self.reach(scope)).await?;
            let sent = scope.send_effect_reaching(&reach)?;
            self.sent = true;
            let written = bounded(scope, async {
                Ok(write_document(&world, html, &sent).await)
            })
            .await?;
            match written {
                Ok(ProbeReply::Replaced) => return Ok(()),
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
                    self.sent = false;
                    self.page.frames().invalidate(&world);
                    world = bounded(scope, async {
                        installed(
                            self.page
                                .world(&self.frame.endpoint, self.frame.place.clone())
                                .await,
                        )
                    })
                    .await?;
                    // Not bounded: a sweep ends at the op's deadline by itself, and reports what
                    // it found.
                    self.sweep(scope, DocumentWrite::NOT_SENT, || true).await?;
                }
                Err(failure) => return Err(resolve::failed(scope, failure)),
            }
        }
    }
}

/// `step`'s outcome within the op's timeout, which upstream's covers the whole call with. Every
/// step of a write but a sweep runs this way: a sweep ends at the op's deadline by itself, so no
/// timeout drops it with the values it found.
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
