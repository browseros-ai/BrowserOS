//! A frame's document as a whole, in its sealed world: the main frame's, or that of the child
//! frame a `frame.*` form names. `page.title()` and `page.content()` read it there, so every
//! field value the serialization touches comes through the seal (`value="••••"`), never through
//! the page's own getters. `page.setContent()` replaces it from there, through the main-world
//! gateway, because the HTML's scripts run in the page's main world.
//!
//! ```text
//!  setContent ── EvaluateGuard::open_document: every frame of the site swept ── write: the
//!     probe's `setContent` from the sealed world, then Page.documentOpened and its waitUntil
//!     state ── Pending::sweep: the same sweep again ── close: `{}` or a masked failure
//! ```

use browseros_core::FrameId;
use futures_util::future;
use serde::Deserialize;
use serde_json::Value;

use crate::op_scope::OpScope;
use crate::page::evaluate_guard::{self, EvaluateGuard, TreeFrame};
use crate::page::site_sweep::SiteSweep;
use crate::page::world_call::{self, unexpected, with_world};
use crate::page::{LoadState, OpPage};
use crate::probe::{self, DocumentPart, ProbeReply, ProbeRequest};
use crate::resolve::failed;
use crate::target::PageRef;
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

/// Reads `part` of the frame's document. The seal masks the fields the markup serializes; the
/// values of every sensitive field in the frame's site then mask a typed value the page copied
/// elsewhere in its markup or its title, which no getter of the seal sees (an attribute, a
/// heading), from its own fields or from any frame of its site. They also go to the host, as
/// evaluate's and snapshot's do, so its mask at every exit knows them.
pub(crate) async fn read(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DocumentInput,
    part: DocumentPart,
) -> Result<Value, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &input.page).await?;
    let world = match world_call::frame_world(&page, scope, input.frame.as_ref()).await {
        Ok(world) => world,
        // upstream's `title()` swallows its failure and answers "" (PW:25766), so a detached
        // frame's title is "" (measured); its `content()` fails as the frame route does.
        Err(error)
            if part == DocumentPart::Title
                && input.frame.is_some()
                && error.error_code() == Some(ErrorCode::TargetClosed) =>
        {
            return Ok(Value::String(String::new()));
        }
        Err(error) => return Err(error),
    };
    let TreeFrame { endpoint, place } =
        evaluate_guard::frame_endpoint(&page, world.session(), world.frame().clone());
    let request = &ProbeRequest::Document { part };
    let admitted = &page;
    let (text, sweep, site) = with_world(&page, &endpoint, &place, |world| async move {
        let (reply, site) = future::join(
            probe::call(&world, request),
            SiteSweep::of(admitted, world.clone()),
        )
        .await;
        match reply? {
            ProbeReply::DocumentText { text, sweep } => Ok((text, sweep, site)),
            other => Err(unexpected("document", other)),
        }
    })
    .await
    .map_err(|failure| failed(scope, failure))?;
    let values = site.values(pw, scope, page.id(), sweep)?;
    Ok(Value::String(values.mask_text(text)))
}

/// Replaces the frame's document with `html` and waits for `waitUntil`, as upstream does,
/// through the main-world gateway: between two sweeps of every frame of the frame's site.
///
/// The probe writes from the sealed world, but the HTML's scripts and event handlers run in the
/// main world, with the page's origin and whatever the old document's scripts kept: a window or
/// an element of a same-site frame that `document.open` removed still reads raw (measured). So
/// nothing is written until every same-site frame was swept, and the frames the new document
/// holds are swept again once it reached `waitUntil`; the host learns every value either found.
///
/// Playwright-core never sees `load` on a hosted BrowserOS, because it waits for a console
/// message after `document.open()`, which a hosted browser never sends; the guard's watch needs
/// none. The write runs in the sealed world, so the probe can unboot in the same call, and the
/// new document takes a new seq before any other op can mint a ref in it.
pub(crate) async fn set_content(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: SetContentInput,
) -> Result<Value, PlaywrightError> {
    // Measured: Chromium never fires `networkIdle` again for a written document, so this wait
    // could only time out.
    if input.wait_until == LoadState::NetworkIdle {
        return Err(scope.fail(
            ErrorCode::Unsupported,
            "waitUntil \"networkidle\" cannot be observed after setContent in Neo; use \"load\"",
        ));
    }
    // Logged before the frame is looked for, as upstream logs it, so a timeout while the frame's
    // world installs carries the line in its call log.
    scope.log(format!(
        "setting frame content, waiting until \"{}\"",
        input.wait_until
    ));
    let guard = EvaluateGuard::open_document(pw, scope, &input.page, input.frame.as_ref()).await?;
    let pending = guard.write(scope, &input.html, input.wait_until).await?;
    let (guard, written) = pending.sweep(scope).await?;
    guard.close(written)
}

#[cfg(test)]
mod tests;
