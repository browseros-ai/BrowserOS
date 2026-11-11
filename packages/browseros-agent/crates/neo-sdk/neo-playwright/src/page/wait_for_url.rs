//! `page.waitForURL(url, { waitUntil })` and `frame.waitForURL`, with upstream client's
//! semantics: a URL that already matches waits only for the load state; otherwise every
//! navigation of the frame is checked, in order, until one matches, and then its load state is
//! waited for.
//!
//! ```text
//!  DocumentWatch (subscribe, lifecycle on, current URL) ── matchUrl(current)? ── yes ─┐
//!     no ── navigated ── `  navigated to "<url>"` ── matchUrl(url)? ── no: next ─────┤
//!                                                    yes ──────────────────────────┴─ waitUntil
//! ```
//!
//! The pattern is matched in the frame's sealed world, because a `RegExp` must run with
//! JavaScript's semantics. The URL matched is the one the navigation reported, not the page's
//! `location` later, so a page that moves on at once still matches the step it passed through.
//! A function predicate stays in the prelude. The frame-following and URL-matching helpers
//! also serve `waitForNavigation` after it selects a journaled navigation.
//!
//! A child frame that navigates to another site moves to another renderer, and so to another
//! session, while it stays the same frame, as upstream's frame model keeps it: the navigation
//! that may match commits there, and the old session only says the frame left it. So the wait
//! follows the frame: found again by its id through the page's tree, as a frame form's guard
//! follows its frame, watched on its new session, and its current URL matched before the wait
//! goes on. A frame the page no longer lists fails as upstream's navigation waiter does,
//! `Navigating frame was detached!` (measured with Playwright 1.64.1, tier 1 frame-url-moved).

use std::time::Duration;

use browseros_core::FrameId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::op_output::to_output;
use crate::op_scope::OpScope;
use crate::page::document_watch::{DocumentWatch, WatchError};
use crate::page::evaluate_guard::{self, TreeFrame};
use crate::page::navigation::watch_error;
use crate::page::world_call::{self, unexpected, with_world};
use crate::page::{LoadState, OpPage};
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, UrlPattern};
use crate::resolve::failed;
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// Upstream's navigation waiter's failure for a frame the page removed mid-wait.
const NAVIGATING_FRAME_DETACHED: &str = "Navigating frame was detached!";

/// How long a wait gives a frame between finding it in the page's tree and its new session
/// holding it, before it looks again: the renderer it moved to may not have it yet.
const MOVING: Duration = Duration::from_millis(50);

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

/// What `waitForURL` returns: the frame's URL, for the prelude's synchronous `url()`.
#[derive(Debug, Serialize)]
struct UrlResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

/// Upstream's first log line names a string pattern only.
pub(crate) fn logged(pattern: &UrlPattern) -> String {
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
    let page = OpPage::admit_handle(pw, scope, &input.page).await?;
    let waiting = format!(
        "waiting for navigation{} until \"{}\"",
        logged(&input.url),
        input.wait_until
    );
    let pattern = input.url;
    let frame_id = input.frame;
    // The frame is found under the timeout too, as upstream's covers the whole call: a parent
    // whose renderer never answers holds the op no longer than that.
    let wait = async {
        let Followed::Watching(mut followed) =
            Followed::find(pw, &page, scope, frame_id.as_ref()).await?
        else {
            return Err(scope.fail(ErrorCode::TargetClosed, "Frame was detached"));
        };
        let probed = |failure| failed(scope, failure);
        let watched = |error| watch_error(scope, &error);
        if !followed
            .matches_current(&page, &pattern)
            .await
            .map_err(probed)?
        {
            scope.log(waiting);
            loop {
                let navigated = match followed.watch.navigated().await {
                    Ok(navigated) => navigated,
                    Err(WatchError::Left) => {
                        followed = follow(pw, &page, scope, frame_id.as_ref()).await?;
                        if followed
                            .matches_current(&page, &pattern)
                            .await
                            .map_err(probed)?
                        {
                            break;
                        }
                        continue;
                    }
                    Err(error) => return Err(watched(error)),
                };
                if let Some(unreachable) = navigated.unreachable {
                    let message = format!("the browser could not load {unreachable}");
                    return Err(scope.fail(ErrorCode::NavigationFailed, &message));
                }
                scope.log(format!("  navigated to \"{}\"", navigated.url));
                if matches(&page, &followed.frame, &pattern, navigated.url)
                    .await
                    .map_err(probed)?
                {
                    break;
                }
            }
        }
        followed
            .watch
            .reached(input.wait_until)
            .await
            .map_err(watched)?;
        Ok(followed.watch.url().map(str::to_owned))
    };
    let url = match scope.within(wait).await {
        Ok(waited) => waited?,
        Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
    };
    to_output(scope, &UrlResult { url })
}

/// The waited frame: where its world lives now, and the watch on the session that hosts it.
pub(crate) struct Watching {
    pub(crate) frame: TreeFrame,
    pub(crate) watch: DocumentWatch,
}

/// What looking the frame up again found.
enum Followed {
    /// It is in the page, watched on the session that hosts it now. Boxed: a watch holds its
    /// event receiver and its frame's state.
    Watching(Box<Watching>),
    /// The page's tree lists it, but that session's tree does not hold it yet: it is still
    /// moving to another renderer.
    Moving,
    /// The page no longer lists it.
    Detached,
}

impl Followed {
    /// The frame `id` names, the main frame when `None`, found through the page's tree as a
    /// frame form finds it, and watched on the session that hosts it now: subscribed first, then
    /// `Page` and lifecycle events on and its current document read, so a navigation in between
    /// is still seen.
    async fn find(
        pw: &Playwright,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
        id: Option<&FrameId>,
    ) -> Result<Self, PlaywrightError> {
        let Some(world) = world_call::listed_frame_world(page, scope, id).await? else {
            return Ok(Self::Detached);
        };
        let frame = evaluate_guard::frame_endpoint(page, world.session(), world.frame().clone());
        let session_id = frame
            .endpoint
            .session
            .session_id()
            .cloned()
            .ok_or_else(|| scope.fail(ErrorCode::Internal, "the frame has no session"))?;
        let mut watch = DocumentWatch::new(
            pw.browser(),
            frame.endpoint.session.clone(),
            session_id,
            frame.endpoint.frame.clone(),
        );
        let started = async {
            watch.enable_page().await?;
            watch.read_current().await?;
            watch.enable_lifecycle().await
        };
        match started.await {
            Ok(()) => Ok(Self::Watching(Box::new(Watching { frame, watch }))),
            Err(WatchError::FrameDetached) => Ok(Self::Moving),
            Err(error) => Err(watch_error(scope, &error)),
        }
    }
}

/// The waited frame after it left its session: found again, looked up anew while it is still
/// moving, or upstream's failure once the page no longer lists it. The op's timeout bounds it.
pub(crate) async fn follow(
    pw: &Playwright,
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    id: Option<&FrameId>,
) -> Result<Box<Watching>, PlaywrightError> {
    loop {
        match Followed::find(pw, page, scope, id).await? {
            Followed::Watching(watching) => return Ok(watching),
            Followed::Moving => {
                if scope.sleep(MOVING).await.is_err() {
                    return Err(scope.timed_out(ErrorCode::WaitTimeout));
                }
            }
            Followed::Detached => {
                return Err(scope.fail(ErrorCode::TargetClosed, NAVIGATING_FRAME_DETACHED));
            }
        }
    }
}

impl Watching {
    /// Whether the document the frame shows now matches `pattern`.
    async fn matches_current(
        &self,
        page: &OpPage<'_>,
        pattern: &UrlPattern,
    ) -> Result<bool, ProbeFailure> {
        let current = self.watch.url().unwrap_or_default().to_owned();
        matches(page, &self.frame, pattern, current).await
    }
}

/// Whether `url` matches `pattern`, as the page's JavaScript decides. A document that went away
/// under the frame's world is matched in the new document's world.
pub(crate) async fn matches(
    page: &OpPage<'_>,
    frame: &TreeFrame,
    pattern: &UrlPattern,
    url: String,
) -> Result<bool, ProbeFailure> {
    let request = &ProbeRequest::MatchUrl {
        pattern: pattern.clone(),
        url,
    };
    with_world(page, &frame.endpoint, &frame.place, |world| async move {
        match probe::call(&world, request).await? {
            ProbeReply::UrlMatch { matches, .. } => Ok(matches),
            other => Err(unexpected("matchUrl", other)),
        }
    })
    .await
}
