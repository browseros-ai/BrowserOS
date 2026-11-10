//! The page's navigations: `goto`, `reload`, `goBack` and `goForward` send one command to the
//! main frame and wait as `waitUntil` asks; `waitForLoadState` sends nothing and waits for the
//! document a frame shows now.
//!
//! ```text
//!  (goto: a javascript: URL refused) ── admit ── DocumentWatch (subscribe, lifecycle on) ──
//!     send_effect ── Page.navigate, reload or navigateToHistoryEntry ── that document's
//!     commit ── its waitUntil state ── the response
//! ```
//!
//! The response comes from the new document's navigation timing entry (`responseStatus`), so
//! the crate never enables the Network domain: `headers()` is `{}` and `statusText` is the
//! standard reason phrase for the status. A net error reads as Playwright's
//! `page.goto: net::ERR_NAME_NOT_RESOLVED at <url>`.

use std::time::Duration;

use browseros_core::{CoreError, FrameId, PageId, PageJs, ProtocolSession, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::time;

use crate::op_output::to_output;
use crate::op_scope::{AdmittedPage, OpScope, Sent};
use crate::page::LoadState;
use crate::page::document_watch::{
    Arrival, CommitOutcome, DocumentWatch, Expect, FrameTreeReply, WatchError, decode,
};
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

/// What starts a navigation of the main frame.
#[derive(Clone, Debug)]
pub(crate) enum Navigation {
    /// `Page.navigate`, which names the document it starts by loader id. `frame` is the
    /// frame `frame.goto` named, which must be the main frame.
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
struct NavigationResult {
    response: Option<Response>,
    url: Option<String>,
}

/// Playwright's `Response` for the main document, from its navigation timing entry.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    url: String,
    status: u16,
    status_text: &'static str,
    ok: bool,
    /// Always empty: reading headers needs the Network domain, which Neo leaves off.
    headers: Map<String, Value>,
}

impl GotoInput {
    /// The page, the navigation and its `waitUntil`. A `javascript:` URL is refused here, before
    /// the page is admitted or anything is sent: Chromium runs it in the current document's main
    /// world, outside the sweep that guards every other main-world script, and only then
    /// reports the navigation aborted. That refusal is why `page.goto` does not declare
    /// `Capability::MainWorld`: every URL it sends runs the new document's own script in that
    /// document's origin, as a link would, or the agent's in a `data:` document's opaque
    /// origin, which reaches no other document.
    pub(crate) fn into_parts(
        self,
        scope: &OpScope<'_>,
    ) -> Result<(PageRef, Navigation, LoadState), PlaywrightError> {
        if runs_script(&self.url) {
            return Err(scope.fail(
                ErrorCode::Unsupported,
                "Neo does not open javascript: URLs, whose script runs in the page's main world \
                 outside Neo's privacy sweep; run the script with page.evaluate",
            ));
        }
        let navigation = Navigation::Goto {
            frame: self.frame,
            url: complete_user_url(self.url),
            referer: self.referer,
        };
        Ok((self.page, navigation, self.wait_until))
    }
}

/// Whether `url` is a `javascript:` URL, its scheme read as the URL Standard and Chromium read
/// it: the C0 controls and spaces around the URL dropped, the tabs and newlines inside it
/// removed, then everything before the first `:`, compared without ASCII case. So
/// ` JavaScript:` and `java\tscript:` are, and `/javascript:` (no scheme) and `java script:`
/// (no URL Chromium navigates to) are not.
fn runs_script(url: &str) -> bool {
    let url: String = url
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    url.split_once(':')
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("javascript"))
}

impl NavigateInput {
    /// The page and its `waitUntil`.
    pub(crate) fn into_parts(self) -> (PageRef, LoadState) {
        (self.page, self.wait_until)
    }
}

/// Upstream's `completeUserURL`: a bare `localhost` or `127.0.0.1` address gets `http://`.
fn complete_user_url(url: String) -> String {
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
    page: &PageId,
    navigation: Navigation,
    wait_until: LoadState,
) -> Result<Value, PlaywrightError> {
    let admitted = scope.admit(page).await?;
    // The watch below follows the main frame's documents only, so a child frame's goto would
    // wait on the wrong loader. Refused before anything is sent.
    if let Navigation::Goto {
        frame: Some(frame), ..
    } = &navigation
        && *frame != main_frame(admitted)
    {
        return Err(scope.fail(
            ErrorCode::Unsupported,
            "Neo navigates only a page's main frame; navigate the page, or set the iframe's \
             src with page.evaluate",
        ));
    }
    let session_id = admitted.session.session_id.clone();
    let session = pw.browser().protocol_session(session_id.clone());
    scope.log(match &navigation {
        Navigation::Goto { url, .. } => {
            format!("navigating to \"{url}\", waiting until \"{wait_until}\"")
        }
        Navigation::Reload | Navigation::History(_) => {
            format!("waiting for navigation until \"{wait_until}\"")
        }
    });
    let mut watch = DocumentWatch::new(
        pw.browser(),
        session.clone(),
        session_id,
        main_frame(admitted),
    );
    let drive = drive(scope, &session, &mut watch, navigation, wait_until);
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
    watch: &mut DocumentWatch,
    navigation: Navigation,
    wait_until: LoadState,
) -> Result<Option<Response>, PlaywrightError> {
    let failed = |error: WatchError| watch_error(scope, &error);
    let core = |error: CoreError| scope.core_error(&error);
    watch.enable_lifecycle().await.map_err(failed)?;
    let arrival = match navigation {
        Navigation::Goto { url, referer, .. } => {
            let sent = scope.send_effect()?;
            let reply = send_navigate(session, &sent, &url, referer.as_deref())
                .await
                .map_err(core)?;
            // A download is answered with an error text too, so it goes first, as upstream.
            if reply.is_download {
                return Err(scope.fail(ErrorCode::NavigationFailed, "Download is starting"));
            }
            if let Some(error) = reply.error_text.filter(|error| !error.is_empty()) {
                // Chromium answers before it commits its error page, under the same loader
                // (measured: 2 ms later). Waiting for that commit keeps it in this op; left
                // for the next navigation, it would read as that one being interrupted.
                if let Some(loader) = &reply.loader_id {
                    let _ = time::timeout(ERROR_PAGE_COMMIT, watch.committed(loader)).await;
                }
                return Err(scope.fail(ErrorCode::NavigationFailed, &format!("{error} at {url}")));
            }
            match reply.loader_id {
                Some(loader) => match watch.committed(&loader).await.map_err(failed)? {
                    CommitOutcome::Committed(commit) => Arrival::NewDocument(commit),
                    CommitOutcome::Interrupted(other) => {
                        let message = format!(
                            "Navigation to \"{url}\" is interrupted by another navigation to \"{other}\""
                        );
                        return Err(scope.fail(ErrorCode::NavigationFailed, &message));
                    }
                },
                // No loader: Chromium already moved inside the current document.
                None => Arrival::SameDocument,
            }
        }
        Navigation::Reload => {
            let sent = scope.send_effect()?;
            send_reload(session, &sent).await.map_err(core)?;
            watch.arrived(Expect::NewDocument).await.map_err(failed)?
        }
        Navigation::History(delta) => {
            // The current document's states, for a same-document step that keeps it.
            watch.read_current().await.map_err(failed)?;
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
    watch.reached(wait_until).await.map_err(failed)?;
    Ok(match arrival {
        Arrival::NewDocument(commit) => read_response(session, &commit.url).await,
        Arrival::SameDocument => None,
    })
}

/// `page.waitForLoadState` and `frame.waitForLoadState`: the frame's current document, or the
/// one that replaces it while the op waits, reaches `state`.
pub(crate) async fn wait_for_load_state(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: LoadStateInput,
) -> Result<Value, PlaywrightError> {
    let page = input.page.checked(pw.browser(), scope).await?;
    let admitted = scope.admit(&page).await?;
    let main = main_frame(admitted);
    let frame = input.frame.unwrap_or_else(|| main.clone());
    let root_id = admitted.session.session_id.clone();
    let root = pw.browser().protocol_session(root_id.clone());
    let (session_id, session) = if frame == main {
        (root_id, root)
    } else {
        frame_host(pw, scope, &page, (root_id, root), &frame).await?
    };
    let mut watch = DocumentWatch::new(pw.browser(), session, session_id, frame);
    let wait = async {
        watch.enable_page().await?;
        watch.read_current().await?;
        watch.enable_lifecycle().await?;
        watch.reached(input.state).await
    };
    match scope.within(wait).await {
        Ok(Ok(())) => Ok(json!({})),
        Ok(Err(error)) => Err(watch_error(scope, &error)),
        Err(_expired) => Err(scope.timed_out(ErrorCode::WaitTimeout)),
    }
}

/// The session that hosts child frame `frame`: the page's own session for a same-process
/// frame, else the frame's own, which core's frame registry attaches or already holds.
async fn frame_host(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: &PageId,
    root: (SessionId, ProtocolSession),
    frame: &FrameId,
) -> Result<(SessionId, ProtocolSession), PlaywrightError> {
    let tree: FrameTreeReply = root
        .1
        .send_value("Page.getFrameTree", json!({}))
        .await
        .and_then(decode)
        .map_err(|error| scope.core_error(&error))?;
    if tree
        .frame_tree
        .into_frames()
        .any(|known| known.id == frame.as_str())
    {
        return Ok(root);
    }
    let target = pw
        .browser()
        .frames()
        .resolve_frame_target(page.clone(), Some(frame.clone()), None, true)
        .await
        .map_err(|error| scope.core_error(&error))?;
    let session_id = target
        .session
        .session_id()
        .cloned()
        .ok_or_else(|| scope.fail(ErrorCode::TargetClosed, "Frame was detached"))?;
    Ok((session_id, target.session))
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
    url: &str,
    referer: Option<&str>,
) -> Result<NavigateReply, CoreError> {
    let params = match referer {
        Some(referer) => {
            json!({ "url": url, "referrer": referer, "referrerPolicy": "unsafeUrl" })
        }
        None => json!({ "url": url }),
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
async fn read_response(session: &ProtocolSession, url: &str) -> Option<Response> {
    if url.starts_with("about:") || url.starts_with("data:") {
        return None;
    }
    let timing = time::timeout(TIMING_READ, timing_entry(session))
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

#[expect(
    clippy::disallowed_methods,
    reason = "the navigation timing entry holds a status code and the document URL, never a form-control value"
)]
async fn timing_entry(session: &ProtocolSession) -> Result<Option<Timing>, CoreError> {
    // Core's DOM-only world: the sealed world is for element work, and this read needs none.
    PageJs::isolated_only(session)
        .timeout(TIMING_READ)
        .eval(TIMING_JS)
        .await
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
    use std::error::Error;
    use std::sync::Arc;

    use neo_trace::ErrorOwner;
    use serde_json::json;

    use super::{complete_user_url, runs_script, status_text};
    use crate::test_env::{self, TestEnv};
    use crate::test_support::RecordingHooks;
    use crate::{Answer, ErrorCode, Op, Playwright};

    #[test]
    fn tests_that_a_javascript_url_is_script_however_it_is_spelled() {
        for (url, script) in [
            ("javascript:void(document.title = 'x')", true),
            ("JavaScript:void(0)", true),
            // The browser drops what surrounds a URL, and the tabs and newlines inside it.
            (" \u{1}java\tscr\nipt:void(0)\r\n", true),
            ("https://bank.example/javascript:void(0)", false),
            (
                "data:text/html,<script>document.title = 'x'</script>",
                false,
            ),
            ("about:blank", false),
            ("/javascript:void(0)", false),
            ("java script:void(0)", false),
            ("javascript", false),
        ] {
            assert_eq!(runs_script(url), script, "{url:?}");
        }
    }

    /// Offline, any op that reaches the browser fails as disconnected, so `unsupported` with no
    /// host call shows the refusal came first.
    #[tokio::test]
    async fn tests_that_goto_refuses_a_javascript_url_before_anything_is_sent()
    -> Result<(), Box<dyn Error>> {
        let playwright = Playwright::new(
            test_env::offline_session(),
            Arc::new(RecordingHooks::default()),
        );
        let goto = Op::from_name("page.goto").ok_or("page.goto is an op")?;
        for frame in [None, Some("child-frame")] {
            let env = TestEnv::new();
            let input = json!({ "page": { "$neo": "page", "id": 7, "tab": 1007 },
                                "frame": frame, "url": " JavaScript:void(document.title = 'ran')" });
            let error = playwright
                .invoke(&env, goto, input)
                .await
                .err()
                .ok_or("a javascript: URL is refused")?;
            assert_eq!(
                (error.error_code(), error.owner(), error.answer()),
                (
                    Some(ErrorCode::Unsupported),
                    ErrorOwner::Neo,
                    Answer::Definite
                ),
                "{error}"
            );
            assert!(error.message().contains("javascript:"), "{error}");
            assert!(env.calls().is_empty(), "nothing was rechecked or sent");
        }
        Ok(())
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
            "https://example.com"
        );
    }

    #[test]
    fn tests_that_status_text_is_the_reason_phrase() {
        assert_eq!(status_text(200), "OK");
        assert_eq!(status_text(404), "Not Found");
        assert_eq!(status_text(299), "", "no phrase for an unregistered code");
    }
}
