//! What every sealed-world read is masked with: the raw values of every sensitive field in the
//! read frame's site, across every CDP session of the page. The page text an action or a
//! matcher reports goes through the same mask ([`swept_now`], [`refused`]): a strict
//! violation's previews, the element a click names as in its way, a select's values, and a
//! matcher's `Received` and its `locator resolved to` lines.
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
//! Playwright's engine cuts some page text inside the page before any of this can mask it: a
//! preview keeps 50 characters, a locator hint keeps the words before a cut. So those replies
//! need the site's values in the page first ([`SiteValues::probe_mask`]): a read whose reply
//! always previews (`located`, `pins`) sweeps before its call and sends them along
//! (`Resolver::attempt_masked`), and any other request that meets such text, a strict
//! violation say, answers `needsMask` and is asked again with them (`resolve::remask`). Either
//! way the site is swept once more after a reply with cut text in it, because a field of
//! another frame may have changed in between ([`SiteValues::lacks_any_of`]). Actions ask the
//! same way (`resolve::ask`), and an `expect` poll, which always previews, sends the values
//! with it (`resolve::ask_masked`, `Resolver::attempt_masked`). The one cut outside a probe
//! call, the hit-target interceptor's description of what a trusted event hit, is masked whole
//! in the page, and its verdict describes the miss again inside its own call.
//!
//! A frame of another site is never swept: its script cannot reach the read frame's
//! documents, so whatever it holds can only arrive by a message the frame chose to send.

use browseros_core::PageId;
use browseros_core::sensitive_fields::FieldValues;
use serde_json::Value;

use crate::op_scope::OpScope;
use crate::page::evaluate_guard::{self, FRAME_BUDGET, Found, TreeFrame, Unswept};
use crate::page::{OpPage, page_value};
use crate::probe::{PageText, ProbeFailure, ProbeMask, ProbeReply, Ran};
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
    /// fails the op closed as `neo / unsupported`, as a main-world script is refused: the
    /// page text may hold that frame's value, and Neo cannot tell. The read's own frame gone
    /// under the sweep fails the op too.
    pub(crate) fn values(
        self,
        pw: &Playwright,
        scope: &OpScope<'_>,
        page: &PageId,
        own: FieldValues,
    ) -> Result<SiteValues, PlaywrightError> {
        self.values_or_gone(pw, scope, page, own)?
            .map_err(|gone| resolve::failed(scope, gone))
    }

    /// [`SiteSweep::values`] for a request sent after its sweep, which has no reply yet: the
    /// read's own frame gone under the sweep is [`ProbeFailure::Gone`], the answer the request
    /// would get in that world too. The resolver's liveness rule takes it as it takes a call
    /// that came back gone, so a poll that lands mid-navigation lands nowhere and the next one
    /// installs in the new document. Ending the op there instead failed upstream's waits that
    /// outlive a cross-process navigation.
    pub(crate) fn values_or_gone(
        self,
        pw: &Playwright,
        scope: &OpScope<'_>,
        page: &PageId,
        own: FieldValues,
    ) -> Result<Result<SiteValues, ProbeFailure>, PlaywrightError> {
        let Found { values, unswept } = self.0;
        let values = own.union(values);
        if !values.is_empty() {
            pw.hooks().values_swept(page.clone(), &values);
        }
        match unswept {
            None => Ok(Ok(SiteValues(values))),
            Some(Unswept::Gone) => Ok(Err(ProbeFailure::Gone(Ran::No))),
            Some(Unswept::Late) => Err(scope.fail(
                ErrorCode::Unsupported,
                &format!(
                    "a frame of the page's site did not answer Neo's privacy sweep within {}ms, \
                     so page text that may hold its values is withheld; try again once the frame \
                     responds",
                    FRAME_BUDGET.as_millis()
                ),
            )),
            Some(Unswept::Lost) => Err(scope.fail(
                ErrorCode::Unsupported,
                "Neo lost its connection to a frame of the page's site and could not reach it \
                 again for its privacy sweep, so page text that may hold its values is withheld; \
                 try again",
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

    /// Page text that is one string (an element's preview, what a matcher resolved to), masked
    /// once, for a message or a call-log line.
    pub(crate) fn mask_line(&self, text: PageText) -> String {
        match self.mask(text) {
            Value::String(text) => text,
            other => other.to_string(),
        }
    }

    /// Page text outside a read's value (a document's markup or title, a strict violation's
    /// previews), masked as one string.
    pub(crate) fn mask_text(&self, text: String) -> String {
        page_value::mask_text(&self.0, text)
    }

    /// The values themselves, for the snapshot's stitch, which masks every frame's strings with
    /// them and every frame's own sweep together.
    pub(crate) fn into_values(self) -> FieldValues {
        self.0
    }

    /// The values as a probe call carries them, to mask page text before the engine cuts it.
    pub(crate) fn probe_mask(&self) -> ProbeMask<'_> {
        ProbeMask::of(&self.0)
    }

    /// Whether `after`, the site swept again once a call that carried these values answered,
    /// holds a value the call's page text may have been cut without: one long enough to be in
    /// a mask that neither these values nor `own` hold. `own` is the sweep a reply came with,
    /// the probe's own frame as the call found it, which the probe added to these.
    pub(crate) fn lacks_any_of(&self, own: Option<&FieldValues>, after: &Self) -> bool {
        let holds = |values: &FieldValues, value: &str| values.registrable().any(|v| v == value);
        after
            .0
            .registrable()
            .any(|value| !holds(&self.0, value) && !own.is_some_and(|own| holds(own, value)))
    }

    /// These values and `other`'s.
    pub(crate) fn union(self, other: Self) -> Self {
        Self(self.0.union(other.0))
    }

    /// These values and `own`, the sweep a reply came with: the read frame's own fields as the
    /// call found them, after this site's sweep. Any of them go to the host as well.
    pub(crate) fn with_own(self, pw: &Playwright, page: &PageId, own: FieldValues) -> Self {
        if own.is_empty() {
            return self;
        }
        pw.hooks().values_swept(page.clone(), &own);
        Self(self.0.union(own))
    }

    /// A reply that ends an op with the page text in it masked: a strict violation's message
    /// and a not-an-iframe preview. Every other reply holds no page text.
    pub(crate) fn mask_reply(&self, reply: ProbeReply) -> ProbeReply {
        match reply {
            ProbeReply::Strict { message } => ProbeReply::Strict {
                message: self.mask_text(message),
            },
            ProbeReply::NotFrame { preview } => ProbeReply::NotFrame {
                preview: self.mask_text(preview),
            },
            other => other,
        }
    }

    /// `resolve::refused` of `reply`, its page text masked first.
    pub(crate) fn refused(&self, scope: &OpScope<'_>, reply: ProbeReply) -> PlaywrightError {
        resolve::refused(scope, self.mask_reply(reply))
    }
}

#[cfg(test)]
impl SiteValues {
    /// Values as a site's sweep would find them, for unit tests without a browser.
    pub(crate) fn found(values: FieldValues) -> Self {
        Self(values)
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
        Ok(values) => values.refused(scope, ProbeReply::Strict { message }),
        Err(withheld) => withheld,
    }
}

/// The values of `world`'s site, swept now, for page text an op reports outside a read's value:
/// the element a click names as in its way, a select's values. `own` is what the op knows to be
/// a sensitive field's value besides the sweep (a sensitive select's selected values). Actions
/// sweep only when a reply carries such text, so a click that lands sends no sweep; a frame of
/// the site that misses its budget fails the op closed, as [`SiteSweep::values`] does.
pub(crate) async fn swept_now(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    world: SealedWorld,
    own: FieldValues,
) -> Result<SiteValues, PlaywrightError> {
    SiteSweep::of(page, world)
        .await
        .values(page.playwright(), scope, page.id(), own)
}
