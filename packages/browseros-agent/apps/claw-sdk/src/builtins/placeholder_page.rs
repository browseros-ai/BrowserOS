//! Four Playwright-shaped page ops, enough for `newPage → goto → title → close`, until
//! sdk-playwright registers the real ones under the same names.

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use browseros_core::navigation::GotoOutcome;
use browseros_core::pages::NewPageOptions;
use browseros_trace::{Delivery, Effect};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::browser::PageRef;
use crate::dispatch::{
    ErrorCode, LeafError, Listing, Method, MethodSpec, OpCtx, OpFailure, OperationError,
    PageChange, PageTarget, RegistryBuilder, RegistryError,
};

/// The one line in the `run` description for all four.
const PAGES_LINE: &str = "Pages (a placeholder subset until the Playwright API lands): \
    context.newPage({ url? }) → page, page.goto(url) → { status, download }, page.title() → the \
    tab's title, page.close().";
/// Where a new tab starts, as Playwright's `newPage`.
const BLANK: &str = "about:blank";

/// Registers the four ops.
pub(super) fn placeholder_page(builder: &mut RegistryBuilder) -> Result<(), RegistryError> {
    builder.define(NewPage)?;
    builder.define(Goto)?;
    builder.define(Title)?;
    builder.define(Close)
}

/// `context.newPage`'s input. The door also calls it with `{}` to open a run's lazy default
/// page, so every field stays optional.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPageInput {
    /// Loaded right after the tab opens, by a `page.goto` child op.
    #[serde(default)]
    url: Option<String>,
}

/// An input that names only a page.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageInput {
    page: PageTarget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GotoInput {
    page: PageTarget,
    url: String,
}

/// What a load learned, as `page.goto` returns it.
#[derive(Serialize)]
struct GotoOutput {
    /// The main document's HTTP status, when the page exposes one.
    status: Option<u16>,
    /// The URL answered with a file: the browser downloaded it and the page stayed put.
    download: bool,
}

/// `context.newPage({ url? })`: a background tab, marked agent-opened so the janitor may close
/// it once idle.
struct NewPage;

/// `page.goto(url)`: navigates and waits for the load.
struct Goto;

/// `page.title()`: the tab-strip title, which is the document's `<title>`, or its URL when it
/// has none. Not `document.title` through a page script: that needs a `PageJs` exception
/// (STYLE.md P3), and sdk-playwright's sealed-world `title` replaces this anyway.
struct Title;

/// `page.close()`.
struct Close;

impl Method for NewPage {
    type Input = NewPageInput;
    type Output = PageRef;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            listing: Listing::Grouped(PAGES_LINE),
            ..MethodSpec::new("context.newPage", Effect::Write)
        }
    }

    fn target(_input: &NewPageInput) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, input: NewPageInput) -> Result<PageRef, LeafError> {
        op.recheck()?;
        op.mark_sent();
        // An unbound view opens a foreground tab by default, which would take the owner's focus
        // in the live view.
        let options = NewPageOptions {
            background: Some(true),
            ..NewPageOptions::default()
        };
        // Boxed: core's page open is a large future. A stop during `Browser.createTab` can leave
        // a tab the door never learns about; it stays `Found` and the janitor leaves it.
        let info = op
            .race(Box::pin(op.run().view().new_page(BLANK, options)))
            .await?;
        let page = PageRef::from(&info);
        // Before the load, so a tab whose `url` fails is still agent-opened and in the reply's
        // page facts, where the script can find it.
        op.report_page(&page, PageChange::Opened);
        if let Some(url) = input.url {
            // A child op on the new tab, never a load inside this one: this op has no target,
            // so the door's rules would check its writes as touching no page, and the trace
            // would record the load on no tab. The owner can lock the tab as soon as it exists.
            op.invoke("page.goto", json!({ "page": page, "url": url }))
                .await
                .map_err(load_failure)?;
        }
        Ok(page)
    }
}

impl Method for Goto {
    type Input = GotoInput;
    type Output = GotoOutput;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            listing: Listing::Grouped(PAGES_LINE),
            ..MethodSpec::new("page.goto", Effect::Write)
        }
    }

    fn target(input: &GotoInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, input: GotoInput) -> Result<GotoOutput, LeafError> {
        let page = op.page()?;
        // Held through the load: a fixed-mode tab gets its viewport back first, so the site
        // lays out at the size the owner chose.
        let _viewport = op.viewport(page).await?;
        let nav = op.race(op.run().view().nav(page.id.clone())).await?;
        op.recheck()?;
        op.mark_sent();
        // The page's own failures are peeled off inside the race, so they are the website's;
        // whatever is left (a lost connection, a closed tab) keeps the door's classification.
        let loaded = op
            .race(async {
                match nav.goto(&input.url).await {
                    Ok(outcome) => Ok(Ok(outcome)),
                    Err(error) => website_failure(error).map(Err),
                }
            })
            .await;
        // Whatever the outcome, the URL may have changed, so the door refreshes the page's facts.
        op.report_page(page, PageChange::Navigated);
        let GotoOutcome { status, download } = loaded??;
        Ok(GotoOutput { status, download })
    }
}

impl Method for Title {
    type Input = PageInput;
    type Output = String;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            listing: Listing::Grouped(PAGES_LINE),
            ..MethodSpec::new("page.title", Effect::Read)
        }
    }

    fn target(input: &PageInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, _input: PageInput) -> Result<String, LeafError> {
        let page = op.page()?;
        // One `Browser.getTabInfo`, which also refreshes core's cache. A tab that closed since
        // the door resolved it is `UnknownPage`, which the race reports as `page_closed`.
        let info = op.race(op.run().view().page(page.id.clone())).await?;
        Ok(info.title)
    }
}

impl Method for Close {
    type Input = PageInput;
    type Output = ();

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            listing: Listing::Grouped(PAGES_LINE),
            ..MethodSpec::new("page.close", Effect::Write)
        }
    }

    fn target(input: &PageInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, _input: PageInput) -> Result<(), LeafError> {
        let page = op.page()?;
        op.recheck()?;
        op.mark_sent();
        // Core's close fires its detach hook, which drops the tab's registry entry and slots.
        op.race(op.run().view().close_page(page.id.clone())).await?;
        // A later use of this page in the run is then the script's mistake, not Neo's.
        op.report_page(page, PageChange::Closed);
        Ok(())
    }
}

/// `context.newPage({ url })`'s load failed as its child op: newPage fails with the load's
/// error. The tab opened, so newPage was sent either way; the load's delivery says whether
/// the outcome is known (`not_sent`: refused or failed before it left, `done`: the browser
/// answered) or the page may have loaded (`unknown`).
fn load_failure(failure: OpFailure) -> LeafError {
    match failure.delivery {
        Delivery::NotSent | Delivery::Done => LeafError::definite(failure.error),
        Delivery::Unknown => LeafError::ambiguous(failure.error),
    }
}

/// A navigation failure that is the page's doing, or the core error back for the door.
///
/// Core reports a load the browser could not make (`net::ERR_NAME_NOT_RESOLVED`, a refused
/// connection) and a dialog blocking the navigation as `CoreError::Message`: the browser
/// answered, so it is definite. A `Page.navigate` with no answer inside core's 60 s request
/// timeout is a site that never responded; Playwright throws `TimeoutError` for it, and the
/// load may still be under way, so it is ambiguous.
fn website_failure(error: CoreError) -> Result<LeafError, CoreError> {
    match error {
        CoreError::Message(message) => Ok(LeafError::definite(OperationError::website(
            ErrorCode::NAVIGATION_FAILED,
            message,
        ))),
        error @ CoreError::Cdp(CdpError::Timeout { .. }) => Ok(LeafError::ambiguous(
            OperationError::website(ErrorCode::NAVIGATION_FAILED, error).timed_out(),
        )),
        other => Err(other),
    }
}
