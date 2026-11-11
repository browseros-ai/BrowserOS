//! The page's navigations: `goto`, `reload`, `goBack` and `goForward` send one command to the
//! selected frame and wait as `waitUntil` asks; `waitForLoadState` sends nothing and waits for the
//! document a frame shows now.
//!
//! ```text
//!  admit ── DocumentWatch (subscribe, lifecycle on) ──
//!     send_effect ── Page.navigate, reload or navigateToHistoryEntry ── that document's
//!     commit ── its waitUntil state ── the response
//! ```
//!
//! The response comes from the new document's navigation timing entry (`responseStatus`), so
//! the crate never enables the Network domain: `headers()` is `{}` and `statusText` is the
//! standard reason phrase for the status. A net error reads as Playwright's
//! `page.goto: net::ERR_NAME_NOT_RESOLVED at <url>`.

use std::time::Duration;

use browseros_cdp::CdpError;
use browseros_core::{CoreError, FrameId, PageJs, ProtocolSession, SessionId, TabId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::time;

use crate::op_output::to_output;
use crate::op_scope::{AdmittedPage, OpScope, Sent};
use crate::page::document_watch::{
    Arrival, CommitOutcome, DocumentWatch, Expect, FrameTreeReply, WatchError, decode,
};
use crate::page::frame_routes::RootAttachment;
use crate::page::{LoadState, frame_tree, subtree_load};
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// How long the response read may take once the document is there. It runs inside the op's
/// timeout too; this only keeps a busy renderer from holding a finished navigation.
const TIMING_READ: Duration = Duration::from_secs(2);

/// How long a failed `goto` waits for Chromium to commit its error page before it reports the
/// failure.
const ERROR_PAGE_COMMIT: Duration = Duration::from_secs(1);

/// `page.goto(url, { waitUntil, referer })` and `frame.goto`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct GotoInput {
    page: PageRef,
    /// The frame `frame.goto` names, a frame id from `page.frames()`; absent is the main frame.
    #[serde(default)]
    frame: Option<FrameId>,
    url: String,
    #[serde(default)]
    wait_until: LoadState,
    /// Sent as the request's `Referer`, with the `unsafeUrl` policy as upstream does, so the
    /// page's own referrer policy cannot drop it.
    #[serde(default)]
    referer: Option<String>,
}

/// `page.reload`, `page.goBack` and `page.goForward`: `{ page, waitUntil? }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct NavigateInput {
    page: PageRef,
    #[serde(default)]
    wait_until: LoadState,
}

/// `page.waitForLoadState(state)` and `frame.waitForLoadState(state)`: `{ page, frame?, state? }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct LoadStateInput {
    page: PageRef,
    /// A frame id from `page.frames()`; absent is the main frame.
    #[serde(default)]
    frame: Option<FrameId>,
    #[serde(default)]
    state: LoadState,
}

/// What starts a navigation of a page or one of its child frames.
#[derive(Clone, Debug)]
pub(crate) enum Navigation {
    /// `Page.navigate`, which names the document it starts by loader id. `frame` is the
    /// frame `frame.goto` named, absent for the main frame.
    Goto {
        frame: Option<FrameId>,
        url: String,
        referer: Option<String>,
    },
    /// `Page.reload`: done once any new document commits.
    Reload,
    /// One step through the tab's history (`-1` back, `1` forward); a same-document entry
    /// counts as arriving.
    History(i64),
}

/// What a navigation op returns. `url` refreshes the prelude's synchronous `page.url()`.
#[derive(Debug, Serialize)]
pub(crate) struct NavigationResult {
    pub(crate) response: Option<Response>,
    pub(crate) url: Option<String>,
}

/// Playwright's `Response` for the main document, from its navigation timing entry.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Response {
    url: String,
    status: u16,
    status_text: &'static str,
    ok: bool,
    /// Always empty: reading headers needs the Network domain, which Neo leaves off.
    headers: Map<String, Value>,
}

impl GotoInput {
    /// The page, the navigation and its `waitUntil`.
    pub(crate) fn into_parts(self) -> (PageRef, Navigation, LoadState) {
        let navigation = Navigation::Goto {
            frame: self.frame,
            url: complete_user_url(self.url),
            referer: self.referer,
        };
        (self.page, navigation, self.wait_until)
    }
}

impl NavigateInput {
    /// The page and its `waitUntil`.
    pub(crate) fn into_parts(self) -> (PageRef, LoadState) {
        (self.page, self.wait_until)
    }
}

/// Upstream first constructs a URL (without a base here), then `completeUserURL` adds
/// `http://` to bare local addresses. Use that same spelling in the command and its error:
/// Chromium executes `javascript:` before replying `net::ERR_ABORTED`, even with URL whitespace.
fn complete_user_url(url: String) -> String {
    let url = url::Url::parse(&url).map_or(url, String::from);
    if url.starts_with("localhost") || url.starts_with("127.0.0.1") {
        format!("http://{url}")
    } else {
        url
    }
}

/// Runs `navigation` on `page`'s main frame and waits for `wait_until`.
pub(crate) async fn navigate(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: TabId,
    navigation: Navigation,
    wait_until: LoadState,
) -> Result<Value, PlaywrightError> {
    let admitted = scope.admit(pw.browser(), page).await?;
    let session_id = admitted.session.session_id.clone();
    let session = pw.browser().protocol_session(session_id.clone());
    let main = main_frame(admitted);
    let frame = match &navigation {
        Navigation::Goto { frame, .. } => frame.clone().unwrap_or_else(|| main.clone()),
        Navigation::Reload | Navigation::History(_) => main.clone(),
    };
    let (session_id, session) = if frame == main {
        (session_id, session)
    } else {
        match scope
            .within(frame_host(pw, scope, page, (session_id, session), &frame))
            .await
        {
            Ok(result) => result?,
            Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
        }
    };
    scope.log(match &navigation {
        Navigation::Goto { url, .. } => {
            format!("navigating to \"{url}\", waiting until \"{wait_until}\"")
        }
        Navigation::Reload | Navigation::History(_) => {
            format!("waiting for navigation until \"{wait_until}\"")
        }
    });
    let mut watch = DocumentWatch::new(pw.browser(), session.clone(), session_id, frame.clone());
    let target = NavigationFrame {
        pw,
        page,
        id: frame.clone(),
    };
    let drive = async {
        let response = drive(scope, &session, &target, &mut watch, navigation, wait_until).await?;
        if wait_until == LoadState::NetworkIdle {
            subtree_load::network_idle(pw, scope, page, &frame).await?;
        }
        Ok::<_, PlaywrightError>(response)
    };
    let response = match scope.within(drive).await {
        Ok(response) => response?,
        Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
    };
    let result = NavigationResult {
        response,
        url: watch.url().map(str::to_owned),
    };
    to_output(scope, &result)
}

/// A tab's main frame id is its target id.
fn main_frame(admitted: &AdmittedPage) -> FrameId {
    FrameId(admitted.session.target_id.as_str().to_owned())
}

/// The command and every wait after it, all inside the op's timeout.
async fn drive(
    scope: &OpScope<'_>,
    session: &ProtocolSession,
    target: &NavigationFrame<'_>,
    watch: &mut DocumentWatch,
    navigation: Navigation,
    wait_until: LoadState,
) -> Result<Option<Response>, PlaywrightError> {
    let failed = |error: WatchError| watch_error(scope, &error);
    let core = |error: CoreError| scope.core_error(&error);
    let arrival = match navigation {
        Navigation::Goto { url, referer, .. } => {
            let sent = scope.send_effect()?;
            // Renderer commands can wait behind an uncommitted navigation. Sending the
            // replacement must not depend on their replies: it is what unblocks them.
            // The watch subscribed before both sends, so lifecycle replay cannot lose a commit.
            let (enabled, reply) = tokio::join!(
                watch.enable_lifecycle(),
                send_navigate(session, &sent, &target.id, &url, referer.as_deref())
            );
            enabled.map_err(failed)?;
            let reply = reply.map_err(core)?;
            // A download is answered with an error text too, so it goes first, as upstream.
            if reply.is_download {
                return Err(scope.fail(ErrorCode::NavigationFailed, "Download is starting"));
            }
            if let Some(error) = reply.error_text.filter(|error| !error.is_empty()) {
                // `javascript:` executes in the current document before Chromium reports
                // ERR_ABORTED. Forward the browser's failure as Playwright does; it does not
                // mean the script had no effect.
                // Chromium answers before it commits its error page, under the same loader
                // (measured: 2 ms later). Waiting for that commit keeps it in this op; left
                // for the next navigation, it would read as that one being interrupted.
                if let Some(loader) = &reply.loader_id {
                    let _ = time::timeout(ERROR_PAGE_COMMIT, watch.committed(loader)).await;
                }
                return Err(scope.fail(ErrorCode::NavigationFailed, &format!("{error} at {url}")));
            }
            if let Some(loader) = reply.loader_id {
                match target.committed(scope, watch, &loader).await? {
                    CommitOutcome::Committed(commit) => Arrival::NewDocument(commit),
                    CommitOutcome::Interrupted(other) => {
                        let message = format!(
                            "Navigation to \"{url}\" is interrupted by another navigation to \"{other}\""
                        );
                        return Err(scope.fail(ErrorCode::NavigationFailed, &message));
                    }
                }
            } else {
                // A fragment keeps its loader and its already-fired load states. Read
                // after sending, since reading a stalled old renderer first deadlocks.
                watch.read_current().await.map_err(failed)?;
                Arrival::SameDocument
            }
        }
        Navigation::Reload => {
            let sent = scope.send_effect()?;
            let (enabled, reloaded) =
                tokio::join!(watch.enable_lifecycle(), send_reload(session, &sent));
            enabled.map_err(failed)?;
            reloaded.map_err(core)?;
            watch.arrived(Expect::NewDocument).await.map_err(failed)?
        }
        Navigation::History(delta) => {
            // The current document's states, for a same-document step that keeps it.
            watch.read_current().await.map_err(failed)?;
            watch.enable_lifecycle().await.map_err(failed)?;
            let Some(entry) = history_entry(session, delta).await.map_err(core)? else {
                // Nothing to go to: upstream returns null without navigating.
                return Ok(None);
            };
            let sent = scope.send_effect()?;
            send_history_step(session, &sent, entry)
                .await
                .map_err(core)?;
            watch.arrived(Expect::AnyNavigation).await.map_err(failed)?
        }
    };
    if let Arrival::NewDocument(commit) = &arrival
        && let Some(unreachable) = &commit.unreachable
    {
        let message = format!("the browser could not load {unreachable}");
        return Err(scope.fail(ErrorCode::NavigationFailed, &message));
    }
    // Keep the selected document's response while it still exists. A later navigation can
    // finish this load wait, but its timing entry must never stand in for the earlier one.
    let current_session = watch.session().clone();
    let response = match &arrival {
        Arrival::NewDocument(commit) => {
            tokio::select! {
                response = read_response(&current_session, &commit.url, Some(&target.id), Some(&commit.loader)) => response,
                // Timing is supplemental. A later renderer navigation can hold its CDP
                // reads until that new response arrives; it must not delay this goto.
                _ = watch.navigation_started() => None,
            }
        }
        Arrival::SameDocument => None,
    };
    target.reached(scope, watch, wait_until).await?;
    Ok(response)
}

/// A stable Frame identity across CDP session changes. Following is read-only: the original
/// navigation command is never replayed when an OOPIF moves to another renderer.
struct NavigationFrame<'a> {
    pw: &'a Playwright,
    page: TabId,
    id: FrameId,
}

impl NavigationFrame<'_> {
    async fn committed(
        &self,
        scope: &OpScope<'_>,
        watch: &mut DocumentWatch,
        loader: &str,
    ) -> Result<CommitOutcome, PlaywrightError> {
        loop {
            match watch.committed(loader).await {
                Ok(commit) => return Ok(commit),
                Err(error) if moved(&error) => {
                    *watch = self.follow(scope).await?;
                    if watch
                        .loader()
                        .is_some_and(|current| !current.is_empty() && current != loader)
                    {
                        return Ok(CommitOutcome::Interrupted(
                            watch.url().unwrap_or_default().to_owned(),
                        ));
                    }
                }
                Err(error) => return Err(watch_error(scope, &error)),
            }
        }
    }

    async fn reached(
        &self,
        scope: &OpScope<'_>,
        watch: &mut DocumentWatch,
        state: LoadState,
    ) -> Result<(), PlaywrightError> {
        loop {
            match watch.reached(state).await {
                Ok(()) => return Ok(()),
                Err(error) if moved(&error) => *watch = self.follow(scope).await?,
                Err(error) => return Err(watch_error(scope, &error)),
            }
        }
    }

    async fn follow(&self, scope: &OpScope<'_>) -> Result<DocumentWatch, PlaywrightError> {
        let admitted = scope.admit(self.pw.browser(), self.page).await?;
        let root_id = admitted.session.session_id.clone();
        let root = self.pw.browser().protocol_session(root_id.clone());
        let (session_id, session) =
            fresh_frame_host(self.pw, scope, self.page, (root_id, root), &self.id).await?;
        let mut watch = DocumentWatch::new(self.pw.browser(), session, session_id, self.id.clone());
        watch
            .enable_page()
            .await
            .map_err(|error| watch_error(scope, &error))?;
        watch
            .read_current()
            .await
            .map_err(|error| watch_error(scope, &error))?;
        watch
            .enable_lifecycle()
            .await
            .map_err(|error| watch_error(scope, &error))?;
        Ok(watch)
    }
}

fn moved(error: &WatchError) -> bool {
    matches!(
        error,
        WatchError::Left | WatchError::Core(CoreError::Cdp(CdpError::SessionGone))
    )
}

/// `page.waitForLoadState` and `frame.waitForLoadState`: the frame's current document, or the
/// one that replaces it while the op waits, reaches `state`.
pub(crate) async fn wait_for_load_state(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: LoadStateInput,
) -> Result<Value, PlaywrightError> {
    let page = input.page.tab();
    let admitted = scope.admit(pw.browser(), page).await?;
    let main = main_frame(admitted);
    let frame = input.frame.unwrap_or_else(|| main.clone());
    let root_id = admitted.session.session_id.clone();
    let root = pw.browser().protocol_session(root_id.clone());
    let (session_id, session) = if frame == main {
        (root_id, root)
    } else {
        frame_host(pw, scope, page, (root_id, root), &frame).await?
    };
    let mut watch = DocumentWatch::new(pw.browser(), session, session_id, frame.clone());
    let wait = async {
        watch.enable_page().await?;
        watch.read_current().await?;
        watch.enable_lifecycle().await?;
        watch.reached(input.state).await
    };
    match scope.within(wait).await {
        Ok(Ok(())) => {
            if input.state == LoadState::NetworkIdle {
                match scope
                    .within(subtree_load::network_idle(pw, scope, page, &frame))
                    .await
                {
                    Ok(result) => result?,
                    Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
                }
            }
            Ok(json!({}))
        }
        Ok(Err(error)) => Err(watch_error(scope, &error)),
        Err(_expired) => Err(scope.timed_out(ErrorCode::WaitTimeout)),
    }
}

/// The session that hosts child frame `frame`: the page's own session for a same-process
/// frame, else the frame's own, which core's frame registry attaches or already holds.
pub(crate) async fn frame_host(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: TabId,
    root: (SessionId, ProtocolSession),
    frame: &FrameId,
) -> Result<(SessionId, ProtocolSession), PlaywrightError> {
    let admitted = scope.admit(pw.browser(), page).await?;
    let attachment = RootAttachment {
        session: root.0.clone(),
        frame: main_frame(admitted),
        epoch: pw.browser().connection().connection_epoch(),
    };
    if let Some(session) = scope
        .run_state()
        .frames
        .cached(page, &attachment, frame, pw.events())
    {
        let session_id = session
            .session_id()
            .cloned()
            .ok_or_else(|| scope.fail(ErrorCode::Internal, "the frame has no session"))?;
        return Ok((session_id, session));
    }
    fresh_frame_host(pw, scope, page, root, frame).await
}

/// Discover through the owning page, bypassing hints after a known renderer swap.
async fn fresh_frame_host(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: TabId,
    root: (SessionId, ProtocolSession),
    frame: &FrameId,
) -> Result<(SessionId, ProtocolSession), PlaywrightError> {
    let admitted = scope.admit(pw.browser(), page).await?;
    let since = pw.events().head();
    let tree = frame_tree::read(pw.browser(), &admitted.core_page, &root.1)
        .await
        .map_err(|error| scope.core_error(&error))?;
    let session = tree
        .endpoint(frame)
        .cloned()
        .ok_or_else(|| scope.fail(ErrorCode::TargetClosed, "Frame was detached"))?;
    let session_id = session
        .session_id()
        .cloned()
        .ok_or_else(|| scope.fail(ErrorCode::Internal, "the frame has no session"))?;
    tree.remember(pw, scope, page, &root.1, since);
    Ok((session_id, session))
}

/// A watch that could not answer, as the op's error.
pub(crate) fn watch_error(scope: &OpScope<'_>, error: &WatchError) -> PlaywrightError {
    match error {
        WatchError::Core(error) => scope.core_error(error),
        WatchError::FrameDetached | WatchError::Left => {
            scope.fail(ErrorCode::TargetClosed, "Frame was detached")
        }
    }
}

/// `Page.navigate`'s reply.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigateReply {
    /// Absent for a navigation inside the current document.
    #[serde(default)]
    loader_id: Option<String>,
    #[serde(default)]
    error_text: Option<String>,
    /// The URL answered with a file, so the browser downloads it and the page stays.
    #[serde(default)]
    is_download: bool,
}

/// Starts a navigation of the main frame.
async fn send_navigate(
    session: &ProtocolSession,
    _sent: &Sent<'_>,
    frame: &FrameId,
    url: &str,
    referer: Option<&str>,
) -> Result<NavigateReply, CoreError> {
    let params = match referer {
        Some(referer) => {
            json!({ "url": url, "frameId": frame, "referrer": referer, "referrerPolicy": "unsafeUrl" })
        }
        None => json!({ "url": url, "frameId": frame }),
    };
    decode(session.send_value("Page.navigate", params).await?)
}

async fn send_reload(session: &ProtocolSession, _sent: &Sent<'_>) -> Result<(), CoreError> {
    session.send_value("Page.reload", json!({})).await?;
    Ok(())
}

async fn send_history_step(
    session: &ProtocolSession,
    _sent: &Sent<'_>,
    entry: i64,
) -> Result<(), CoreError> {
    session
        .send_value("Page.navigateToHistoryEntry", json!({ "entryId": entry }))
        .await?;
    Ok(())
}

/// `Page.getNavigationHistory`'s reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct History {
    current_index: i64,
    entries: Vec<HistoryEntry>,
}

#[derive(Deserialize)]
struct HistoryEntry {
    id: i64,
}

/// The id of the history entry `delta` steps from the current one, if there is one.
async fn history_entry(session: &ProtocolSession, delta: i64) -> Result<Option<i64>, CoreError> {
    let history: History = decode(
        session
            .send_value("Page.getNavigationHistory", json!({}))
            .await?,
    )?;
    Ok(history
        .current_index
        .checked_add(delta)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| history.entries.get(index))
        .map(|entry| entry.id))
}

/// The main document's status and final URL. Navigation Timing exposes both to an isolated
/// world without touching the page's own scripts or enabling the Network domain.
const TIMING_JS: &str = "(() => { const entry = performance.getEntriesByType('navigation')[0]; \
return entry ? { status: entry.responseStatus || 0, url: entry.name } : null; })()";

/// What [`TIMING_JS`] returns.
#[derive(Deserialize)]
struct Timing {
    status: u16,
    url: String,
}

/// The response for the main document at `url`, or `None` for one that has none (about:blank
/// and data URLs, for which upstream's goto returns null too: measured by conformance Runner
/// P) or whose timing entry cannot be read in time.
pub(crate) async fn read_response(
    session: &ProtocolSession,
    url: &str,
    frame: Option<&FrameId>,
    loader: Option<&str>,
) -> Option<Response> {
    if url.starts_with("about:") || url.starts_with("data:") {
        return None;
    }
    let timing = time::timeout(TIMING_READ, correlated_timing(session, frame, loader))
        .await
        .ok()?
        .ok()
        .flatten()?;
    Some(Response {
        status_text: status_text(timing.status),
        ok: timing.status == 0 || (200..=299).contains(&timing.status),
        url: timing.url,
        status: timing.status,
        headers: Map::new(),
    })
}

/// Check on both sides of the isolated-world read: a commit between either CDP reply and
/// the timing evaluation must fall back to unavailable, rather than borrow another document's
/// response. This also distinguishes two reloads of the same URL.
async fn correlated_timing(
    session: &ProtocolSession,
    frame: Option<&FrameId>,
    loader: Option<&str>,
) -> Result<Option<Timing>, CoreError> {
    if let Some(loader) = loader
        && current_loader(session, frame).await?.as_deref() != Some(loader)
    {
        return Ok(None);
    }
    let timing = timing_entry(session, frame).await?;
    if let Some(loader) = loader
        && current_loader(session, frame).await?.as_deref() != Some(loader)
    {
        return Ok(None);
    }
    Ok(timing)
}

async fn current_loader(
    session: &ProtocolSession,
    frame: Option<&FrameId>,
) -> Result<Option<String>, CoreError> {
    let tree: FrameTreeReply = decode(session.send_value("Page.getFrameTree", json!({})).await?)?;
    Ok(tree
        .frame_tree
        .into_frames()
        .find(|known| frame.is_none_or(|frame| known.id == frame.as_str()))
        .map(|known| known.loader_id))
}

#[expect(
    clippy::disallowed_methods,
    reason = "the navigation timing entry holds a status code and the document URL, never a form-control value"
)]
async fn timing_entry(
    session: &ProtocolSession,
    frame: Option<&FrameId>,
) -> Result<Option<Timing>, CoreError> {
    // Core's DOM-only world: the sealed world is for element work, and this read needs none.
    let reader = PageJs::isolated_only(session).timeout(TIMING_READ);
    let reader = match frame {
        Some(frame) => reader.in_frame(frame),
        None => reader,
    };
    reader.eval(TIMING_JS).await
}

/// The standard reason phrase for `status`, which Chromium reports as `statusText` for
/// HTTP/1.1 responses. Empty for a code without one.
fn status_text(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        511 => "Network Authentication Required",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{complete_user_url, status_text};

    #[test]
    fn tests_that_goto_normalizes_url_spelling_as_upstream_does() {
        for url in [
            "javascript:void(0)",
            " JavaScript:void(0)",
            "java\tscr\nipt:void(0)",
        ] {
            assert_eq!(complete_user_url(url.to_owned()), "javascript:void(0)");
        }
        assert_eq!(
            complete_user_url(" FILE:///tmp/a b.html".to_owned()),
            "file:///tmp/a%20b.html"
        );
    }

    #[test]
    fn tests_that_a_bare_local_address_gets_a_scheme_as_upstream_does() {
        assert_eq!(
            complete_user_url("localhost:8080/a".to_owned()),
            "http://localhost:8080/a"
        );
        assert_eq!(
            complete_user_url("127.0.0.1/x".to_owned()),
            "http://127.0.0.1/x"
        );
        assert_eq!(
            complete_user_url("https://example.com".to_owned()),
            "https://example.com/"
        );
    }

    #[test]
    fn tests_that_status_text_is_the_reason_phrase() {
        assert_eq!(status_text(200), "OK");
        assert_eq!(status_text(404), "Not Found");
        assert_eq!(status_text(299), "", "no phrase for an unregistered code");
    }
}
