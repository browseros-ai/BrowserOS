//! What every sealed-world read is masked with: the raw values of every sensitive field in the
//! read frame's site, across every CDP session of the page.
//!
//! A read runs in one frame's sealed world, and the probe's reply carries that frame's own
//! sweep. But the page's own script reads every frame of its site through `frames[…]`, `parent`
//! and `top`, whatever sessions hold them, and can copy a password typed in one into its own
//! heading, title or attributes. So a read's page text is masked with the union of the sweeps
//! of every frame of its site: the frames the evaluate guard sweeps before a main-world script
//! (decisions.md, "Main-world sweep scope"), found and swept by the same code
//! (`evaluate_guard::sweep_site`), with its rules for a frame that misses its budget.
//!
//! ```text
//!  the read's probe call ──┐ at the same time, so the sweep's round trips overlap the read's
//!  SiteSweep::of(world) ───┘ every frame of the world's site, each within its budget
//!     └─ SiteSweep::values(the reply's own sweep) ── all of them to the host (values_swept)
//!          ├─ a frame of the site unswept: the read fails closed, its result withheld
//!          └─ SiteValues: each PageText, document text and strict message masked once
//! ```
//!
//! A frame of another site is never swept: its script cannot reach the read frame's
//! documents, so whatever it holds can only arrive by a message the frame chose to send.

use browseros_core::PageId;
use browseros_core::sensitive_fields::FieldValues;
use serde_json::Value;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::page::evaluate_guard::{self, FRAME_BUDGET, Found, TreeFrame, Unswept};
use crate::probe::{PageText, ProbeReply};
use crate::world::SealedWorld;
use crate::{ErrorCode, Playwright, PlaywrightError, resolve};

/// One read's sweep of its frame's site, taken in the world the read ran in, at the same time.
#[must_use]
pub(crate) struct SiteSweep(Found);

impl SiteSweep {
    /// Sweeps every frame of the page in `world`'s frame's site, `world`'s frame included.
    pub(crate) async fn of(page: &OpPage<'_>, world: SealedWorld) -> Self {
        let TreeFrame { endpoint, place } =
            evaluate_guard::frame_endpoint(page, world.session(), world.frame().clone());
        // Boxed: every read op's future holds one, inside `Playwright::invoke`'s.
        Self(Box::pin(evaluate_guard::sweep_site(page, &endpoint, &place)).await)
    }

    /// The values the read is masked with: `own`, the sweep its reply came with, and every
    /// other frame's. All of them go to the host, also when a frame was not swept, so its own
    /// mask at every exit knows them from now on. A frame of the site that missed its budget
    /// fails the read closed as `neo / unsupported`, as a main-world script is refused: the
    /// read's text may hold that frame's value, and Neo cannot tell.
    pub(crate) fn values(
        self,
        pw: &Playwright,
        scope: &OpScope<'_>,
        page: &PageId,
        own: FieldValues,
    ) -> Result<SiteValues, PlaywrightError> {
        let Found { values, unswept } = self.0;
        let values = own.union(values);
        if !values.is_empty() {
            pw.hooks().values_swept(page.clone(), &values);
        }
        match unswept {
            None => Ok(SiteValues(values)),
            Some(Unswept::Late) => Err(scope.fail(
                ErrorCode::Unsupported,
                &format!(
                    "a frame of the read frame's site did not answer Neo's privacy sweep within \
                     {}ms, so the read's result is withheld; try again once the frame responds",
                    FRAME_BUDGET.as_millis()
                ),
            )),
            Some(Unswept::Lost) => Err(scope.fail(
                ErrorCode::Unsupported,
                "Neo lost its connection to a frame of the read frame's site and could not reach \
                 it again for its privacy sweep, so the read's result is withheld; try again",
            )),
            Some(Unswept::Failed(failure)) => Err(resolve::failed(scope, failure)),
        }
    }
}

/// How one read is masked, chosen by what it reads (`Read::holds_page_text`).
#[must_use]
pub(crate) enum ReadMask {
    /// The read can hold page text: its site's sweep, taken with it.
    Site(SiteSweep),
    /// A state, a box or a frame's origin, which holds none.
    Textless,
}

impl ReadMask {
    /// The mask for a read in `world` that holds page text when `holds_page_text` says so;
    /// only then is the site swept.
    pub(crate) async fn of(page: &OpPage<'_>, world: SealedWorld, holds_page_text: bool) -> Self {
        if holds_page_text {
            Self::Site(SiteSweep::of(page, world).await)
        } else {
            Self::Textless
        }
    }

    /// `value` as the op returns it: masked with the read site's values, or as the probe sent
    /// it for a read that holds no page text.
    pub(crate) fn value(
        self,
        pw: &Playwright,
        scope: &OpScope<'_>,
        page: &PageId,
        own: FieldValues,
        value: PageText,
    ) -> Result<Value, PlaywrightError> {
        match self {
            Self::Site(site) => Ok(site.values(pw, scope, page, own)?.mask(value)),
            Self::Textless => Ok(value.textless()),
        }
    }
}

/// The raw values of every sensitive field in one read frame's site. It never prints them.
pub(crate) struct SiteValues(FieldValues);

impl SiteValues {
    /// A read's value, every string in it masked once.
    pub(crate) fn mask(&self, value: PageText) -> Value {
        value.masked(&self.0)
    }

    /// Page text outside a read's value (a document's markup or title, a strict violation's
    /// previews), masked as one string.
    pub(crate) fn mask_text(&self, text: String) -> String {
        evaluate_guard::mask_text(&self.0, text)
    }

    /// The values themselves, for the snapshot's stitch, which masks every frame's strings with
    /// them and every frame's own sweep together.
    pub(crate) fn into_values(self) -> FieldValues {
        self.0
    }
}

/// A reply that ends a read in `world`. A strict violation previews each match's markup, page
/// text the probe sends without a sweep, so its message is masked with the site's values first:
/// `mask`'s sweep when the read took one, else one taken now, on this error path only. Any
/// other reply is `resolve::refused`'s.
pub(crate) async fn refused(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: SealedWorld,
    reply: ProbeReply,
    mask: ReadMask,
) -> PlaywrightError {
    let ProbeReply::Strict { message } = reply else {
        return resolve::refused(scope, reply);
    };
    let site = match mask {
        ReadMask::Site(site) => site,
        ReadMask::Textless => SiteSweep::of(page, world).await,
    };
    match site.values(pw, scope, page.id(), FieldValues::default()) {
        Ok(values) => resolve::refused(
            scope,
            ProbeReply::Strict {
                message: values.mask_text(message),
            },
        ),
        Err(withheld) => withheld,
    }
}
