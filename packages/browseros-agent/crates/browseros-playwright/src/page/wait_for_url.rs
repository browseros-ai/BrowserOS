//! `page.waitForURL(url, { waitUntil })`, with upstream client's semantics: a URL that already
//! matches waits only for the load state; otherwise every navigation of the main frame is
//! checked, in order, until one matches, and then its load state is waited for.
//!
//! ```text
//!  DocumentWatch (subscribe, lifecycle on, current URL) ── matchUrl(current)? ── yes ─┐
//!     no ── navigated ── `  navigated to "<url>"` ── matchUrl(url)? ── no: next ─────┤
//!                                                    yes ──────────────────────────┴─ waitUntil
//! ```
//!
//! The pattern is matched in the main frame's sealed world, because a `RegExp` must run with
//! JavaScript's semantics. The URL matched is the one the navigation reported, not the page's
//! `location` later, so a page that moves on at once still matches the step it passed through.
//! A function predicate stays in the prelude.

use browseros_core::FrameId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::op_output::to_output;
use crate::op_scope::OpScope;
use crate::page::document_watch::DocumentWatch;
use crate::page::navigation::watch_error;
use crate::page::world_call::{main_frame_only, unexpected, with_world};
use crate::page::{LoadState, OpPage};
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, UrlPattern};
use crate::resolve::failed;
use crate::target::PageRef;
use crate::world::{Endpoint, Place};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `page.waitForURL(url, { waitUntil })`: `{ page, frame?, url, waitUntil? }`. The prelude
/// sends a string as `{ glob }` (one without wildcards matches exactly, as upstream) and a
/// `RegExp` as `{ regexSource, regexFlags }`, the probe's own pattern.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WaitForUrlInput {
    page: PageRef,
    #[serde(default)]
    frame: Option<FrameId>,
    url: UrlPattern,
    #[serde(default)]
    wait_until: LoadState,
}

/// What `waitForURL` returns: the main frame's URL, for the prelude's synchronous `page.url()`.
#[derive(Debug, Serialize)]
struct UrlResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

/// Upstream's first log line names a string pattern only.
fn logged(pattern: &UrlPattern) -> String {
    match pattern {
        UrlPattern::Glob { glob } => format!(" to \"{glob}\""),
        UrlPattern::Regex { .. } => String::new(),
    }
}

/// Runs `page.waitForURL`.
pub(crate) async fn wait_for_url(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WaitForUrlInput,
) -> Result<Value, PlaywrightError> {
    main_frame_only(scope, input.frame.as_ref())?;
    let page = OpPage::admit_handle(pw, scope, &input.page).await?;
    let main = page.main_endpoint();
    let session_id = main
        .session
        .session_id()
        .cloned()
        .ok_or_else(|| scope.fail(ErrorCode::Internal, "the page has no session"))?;
    let waiting = format!(
        "waiting for navigation{} until \"{}\"",
        logged(&input.url),
        input.wait_until
    );
    let pattern = input.url;
    let mut watch = DocumentWatch::new(
        pw.browser(),
        main.session.clone(),
        session_id,
        main.frame.clone(),
    );
    let wait = async {
        let watched = |error| watch_error(scope, &error);
        let probed = |failure| failed(scope, failure);
        // Subscribed before the current URL is read, so a navigation in between is still seen.
        watch.read_current().await.map_err(watched)?;
        watch.enable_lifecycle().await.map_err(watched)?;
        let current = watch.url().unwrap_or_default().to_owned();
        if !matches(&page, &main, &pattern, current)
            .await
            .map_err(probed)?
        {
            scope.log(waiting);
            loop {
                let navigated = watch.navigated().await.map_err(watched)?;
                if let Some(unreachable) = navigated.unreachable {
                    let message = format!("the browser could not load {unreachable}");
                    return Err(scope.fail(ErrorCode::NavigationFailed, &message));
                }
                scope.log(format!("  navigated to \"{}\"", navigated.url));
                if matches(&page, &main, &pattern, navigated.url)
                    .await
                    .map_err(probed)?
                {
                    break;
                }
            }
        }
        watch.reached(input.wait_until).await.map_err(watched)
    };
    match scope.within(wait).await {
        Ok(waited) => waited?,
        Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
    }
    let url = watch.url().map(str::to_owned);
    to_output(scope, &UrlResult { url })
}

/// Whether `url` matches `pattern`, as the page's JavaScript decides. A document that went away
/// under the main frame's world is matched in the new document's world.
async fn matches(
    page: &OpPage<'_>,
    main: &Endpoint,
    pattern: &UrlPattern,
    url: String,
) -> Result<bool, ProbeFailure> {
    let request = &ProbeRequest::MatchUrl {
        pattern: pattern.clone(),
        url,
    };
    with_world(page, main, &Place::Main, |world| async move {
        match probe::call(&world, request).await? {
            ProbeReply::UrlMatch { matches, .. } => Ok(matches),
            other => Err(unexpected("matchUrl", other)),
        }
    })
    .await
}
