//! Navigation waits use the event journal's call-time fence and cursor, so a click can run
//! concurrently and a URL predicate can reject one event without losing the next. The
//! document watcher supplies lifecycle replay; the response uses goto's navigation timing
//! source. No Network-domain subscription is needed.

use browseros_core::FrameId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::events::{NavigationEvent, PageEvent};
use crate::op_output::to_output;
use crate::op_scope::OpScope;
use crate::page::document_watch::WatchError;
use crate::page::navigation::{self, NavigationResult};
use crate::page::{LoadState, OpPage, wait_for_url};
use crate::probe::UrlPattern;
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError, resolve};

/// The prelude stamps `since` and `after` before its first await. Predicates stay in the
/// agent realm and send the last returned cursor when they ask for another navigation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WaitForNavigationInput {
    page: PageRef,
    #[serde(default)]
    frame: Option<FrameId>,
    #[serde(default)]
    url: Option<UrlPattern>,
    #[serde(default)]
    wait_until: LoadState,
    since: u64,
    after: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NavigationReply {
    #[serde(flatten)]
    navigation: NavigationResult,
    navigation_seq: u64,
}

/// Waits for a navigation after the script's call, even when the current URL already matches.
pub(crate) async fn wait_for_navigation(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WaitForNavigationInput,
) -> Result<Value, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &input.page).await?;
    let main = page.main_endpoint();
    let frame = input.frame.as_ref().unwrap_or(&main.frame);
    let suffix = input
        .url
        .as_ref()
        .map_or_else(String::new, wait_for_url::logged);
    scope.log(format!(
        "waiting for navigation{suffix} until \"{}\"",
        input.wait_until
    ));
    let wait = async {
        // Enabling Page on the child's session is necessary for an OOPIF's events. The
        // journal was already subscribed before this op, so setup cannot lose a main-frame
        // navigation that races a Promise.all trigger.
        if *frame != main.frame {
            let root_id = main
                .session
                .session_id()
                .cloned()
                .ok_or_else(|| scope.fail(ErrorCode::Internal, "the page has no session"))?;
            let (_, session) = navigation::frame_host(
                pw,
                scope,
                page.tab(),
                (root_id, main.session.clone()),
                frame,
            )
            .await?;
            session
                .send_value("Page.enable", json!({}))
                .await
                .map_err(|error| scope.core_error(&error))?;
        }
        let hub = pw.events();
        let mut after = input.after.max(hub.position_at(input.since));
        loop {
            let changed = hub.changed();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let read = hub.read_after(after);
            if read.lost > 0 {
                return Err(scope.fail(
                    ErrorCode::NavigationFailed,
                    "Navigation events were lost while waiting",
                ));
            }
            for record in read.records {
                if record.at_ms < input.since {
                    continue;
                }
                match record.event {
                    PageEvent::Closed(target) if target.as_str() == main.frame.as_str() => {
                        return Err(scope.fail(
                            ErrorCode::TargetClosed,
                            "Target page, context or browser has been closed",
                        ));
                    }
                    PageEvent::FrameDetached(detached) if detached == *frame => {
                        return Err(
                            scope.fail(ErrorCode::TargetClosed, "Navigating frame was detached!")
                        );
                    }
                    PageEvent::Navigation(navigation) if navigation.frame == *frame => {
                        if let Some(navigation) =
                            complete_navigation(pw, scope, &page, &input, frame, navigation).await?
                        {
                            return Ok(NavigationReply {
                                navigation,
                                navigation_seq: record.seq,
                            });
                        }
                    }
                    _ => {}
                }
            }
            after = read.head;
            changed.await;
        }
    };
    match scope.within(wait).await {
        Ok(result) => to_output(scope, &result?),
        Err(_expired) => Err(scope.timed_out(ErrorCode::WaitTimeout)),
    }
}

/// Resolve a committed frame on its current renderer, check its event URL, then await the
/// lifecycle state. Matching the event URL preserves transient navigation steps even if the
/// document has already moved on by the time the isolated world answers.
async fn complete_navigation(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    input: &WaitForNavigationInput,
    frame: &FrameId,
    event: NavigationEvent,
) -> Result<Option<NavigationResult>, PlaywrightError> {
    if let Some(unreachable) = event.unreachable {
        return Err(scope.fail(
            ErrorCode::NavigationFailed,
            &format!("the browser could not load {unreachable}"),
        ));
    }
    scope.log(format!("  navigated to \"{}\"", event.url));
    // A child may have moved to another CDP session. Its stable frame id is the journal's
    // correlation key; worlds and lifecycle observations must use its new session.
    let mut followed = wait_for_url::follow(pw, page, scope, input.frame.as_ref()).await?;
    if let Some(pattern) = &input.url
        && !wait_for_url::matches(page, &followed.frame, pattern, event.url.clone())
            .await
            .map_err(|failure| resolve::failed(scope, failure))?
    {
        return Ok(None);
    }
    loop {
        match followed.watch.reached(input.wait_until).await {
            Ok(()) => break,
            Err(WatchError::Left) => {
                followed = wait_for_url::follow(pw, page, scope, input.frame.as_ref()).await?;
            }
            Err(error) => return Err(navigation::watch_error(scope, &error)),
        }
    }
    let response = if event.new_document {
        navigation::read_response(&followed.frame.endpoint.session, &event.url, Some(frame)).await
    } else {
        None
    };
    Ok(Some(NavigationResult {
        response,
        url: Some(event.url),
    }))
}
