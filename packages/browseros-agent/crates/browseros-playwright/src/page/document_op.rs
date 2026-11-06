//! The main frame's document as a whole, in its sealed world. `page.title()` and
//! `page.content()` read it there, so every field value the serialization touches comes
//! through the seal (`value="••••"`), never through the page's own getters. `page.setContent()`
//! replaces it there.
//!
//! ```text
//!  setContent ── DocumentWatch (subscribe, lifecycle on) ── send_effect ── probe `setContent`:
//!     document.open, write, close; the probe unboots ── invalidate: the old seq retires ──
//!     install: the new document's seq ── Page.documentOpened, then its waitUntil state
//! ```

use browseros_core::FrameId;
use futures_util::future;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::op_scope::{OpScope, Sent};
use crate::page::document_watch::DocumentWatch;
use crate::page::evaluate_guard::DOCUMENT_REPLACED;
use crate::page::navigation::watch_error;
use crate::page::site_sweep::SiteSweep;
use crate::page::world_call::{main_frame_only, unexpected, with_world};
use crate::page::{LoadState, OpPage};
use crate::probe::{self, DocumentPart, ProbeFailure, ProbeReply, ProbeRequest, Ran};
use crate::resolve::failed;
use crate::target::PageRef;
use crate::world::{Endpoint, Place, SealedWorld};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `page.title()` and `page.content()`: `{ page, frame? }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DocumentInput {
    page: PageRef,
    /// A child frame, for `frame.title()`; absent is the main frame.
    #[serde(default)]
    frame: Option<FrameId>,
}

/// `page.setContent(html, { waitUntil })`: `{ page, frame?, html, waitUntil? }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SetContentInput {
    page: PageRef,
    #[serde(default)]
    frame: Option<FrameId>,
    html: String,
    #[serde(default)]
    wait_until: LoadState,
}

/// Reads `part` of the main frame's document. The seal masks the fields the markup serializes;
/// the values of every sensitive field in the main frame's site then mask a typed value the
/// page copied elsewhere in its markup or its title, which no getter of the seal sees (an
/// attribute, a heading), from its own fields or from any frame of its site. They also go to
/// the host, as evaluate's and snapshot's do, so its mask at every exit knows them.
pub(crate) async fn read(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DocumentInput,
    part: DocumentPart,
) -> Result<Value, PlaywrightError> {
    main_frame_only(scope, input.frame.as_ref())?;
    let page = OpPage::admit_handle(pw, scope, &input.page).await?;
    let request = &ProbeRequest::Document { part };
    let admitted = &page;
    let (text, sweep, site) = with_world(
        &page,
        &page.main_endpoint(),
        &Place::Main,
        |world| async move {
            let (reply, site) = future::join(
                probe::call(&world, request),
                SiteSweep::of(admitted, world.clone()),
            )
            .await;
            match reply? {
                ProbeReply::DocumentText { text, sweep } => Ok((text, sweep, site)),
                other => Err(unexpected("document", other)),
            }
        },
    )
    .await
    .map_err(|failure| failed(scope, failure))?;
    let values = site.values(pw, scope, page.id(), sweep)?;
    Ok(Value::String(values.mask_text(text)))
}

/// Replaces the main frame's document with `html` and waits for `waitUntil`, as upstream does.
///
/// Measured on BrowserOS, both ways upstream and CDP offer (document.open, write and close from
/// an isolated world, and `Page.setDocumentContent`) behave the same: the loader stays, the
/// window and every world outlive the write, and Chromium fires `Page.documentOpened`, then
/// `init`, `DOMContentLoaded` and `load` again under that loader. Playwright-core never sees
/// `load` on BrowserOS because it waits for a console message after `document.open()`, which a
/// hosted browser never sends; this watch needs none. The write runs in the sealed world, so the
/// probe can unboot in the same call, and the new document takes a new seq before any other op
/// can mint a ref in it: a ref from before then fails instead of naming a new element.
pub(crate) async fn set_content(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: SetContentInput,
) -> Result<Value, PlaywrightError> {
    main_frame_only(scope, input.frame.as_ref())?;
    // Measured: Chromium never fires `networkIdle` again for a written document, so this wait
    // could only time out.
    if input.wait_until == LoadState::NetworkIdle {
        return Err(scope.fail(
            ErrorCode::Unsupported,
            "waitUntil \"networkidle\" cannot be observed after setContent in Neo; use \"load\"",
        ));
    }
    let page = OpPage::admit_handle(pw, scope, &input.page).await?;
    let main = page.main_endpoint();
    let session_id = main
        .session
        .session_id()
        .cloned()
        .ok_or_else(|| scope.fail(ErrorCode::Internal, "the page has no session"))?;
    scope.log(format!(
        "setting frame content, waiting until \"{}\"",
        input.wait_until
    ));
    let mut watch = DocumentWatch::new(
        pw.browser(),
        main.session.clone(),
        session_id,
        main.frame.clone(),
    );
    let replace = async {
        let watched = |error| watch_error(scope, &error);
        watch.read_current().await.map_err(watched)?;
        watch.enable_lifecycle().await.map_err(watched)?;
        let old = replace_document(scope, &page, &main, &input.html).await?;
        // The world outlived document.open, unbooted: its seq retires, and the new document
        // is booted under the next one.
        page.frames().invalidate(&old);
        page.world(&main, Place::Main)
            .await
            .map_err(|failure| failed(scope, failure))?;
        watch.opened().await.map_err(watched)?;
        watch.reached(input.wait_until).await.map_err(watched)
    };
    match scope.within(replace).await {
        Ok(replaced) => replaced?,
        Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
    }
    Ok(json!({}))
}

/// Writes `html` in the main frame's world and returns that world, now unbooted. A world whose
/// document changed since its install fails `Gone` without running the request, so the write is
/// tried once more in the new document's world, after the host's policies run again. A write a
/// navigation cut off while it was pending may have run, the HTML's inline scripts with it, so
/// it is never sent again: the op fails with the message upstream's setContent gives when that
/// navigation destroys its evaluation's context.
async fn replace_document(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    main: &Endpoint,
    html: &str,
) -> Result<SealedWorld, PlaywrightError> {
    let installed = |page_world: Result<SealedWorld, ProbeFailure>| {
        page_world.map_err(|failure| failed(scope, failure))
    };
    let mut world = installed(page.world(main, Place::Main).await)?;
    let mut retried = false;
    loop {
        let sent = scope.send_effect()?;
        match write_document(&world, html, &sent).await {
            Ok(ProbeReply::Replaced) => return Ok(world),
            Ok(other) => return Err(failed(scope, unexpected("setContent", other))),
            Err(ProbeFailure::Gone(Ran::Maybe)) => {
                return Err(scope.fail(ErrorCode::PageError, DOCUMENT_REPLACED));
            }
            Err(ProbeFailure::Gone(Ran::No)) if !retried => {
                retried = true;
                page.frames().invalidate(&world);
                world = installed(page.world(main, Place::Main).await)?;
            }
            Err(failure) => return Err(failed(scope, failure)),
        }
    }
}

/// The probe's `setContent`, which replaces the page's document.
async fn write_document(
    world: &SealedWorld,
    html: &str,
    _sent: &Sent<'_>,
) -> Result<ProbeReply, ProbeFailure> {
    let request = ProbeRequest::SetContent {
        html: html.to_owned(),
    };
    probe::call(world, &request).await
}

#[cfg(test)]
mod tests;
