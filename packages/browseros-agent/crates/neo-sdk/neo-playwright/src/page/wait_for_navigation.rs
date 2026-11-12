//! Navigation waits use the event journal's call-time fence and cursor, so a click can run
//! concurrently and a URL predicate can reject one event without losing the next. The
//! document watcher supplies lifecycle replay; the response uses goto's navigation timing
//! source. No Network-domain subscription is needed.

use browseros_core::FrameId;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use crate::events::{NavigationEvent, PageEvent};
use crate::op_scope::OpScope;
use crate::page::document_watch::WatchError;
use crate::page::frame_tree::{self, FrameInfo};
use crate::page::navigation::{self, NavigationResult, Response};
use crate::page::{LoadState, OpPage, subtree_load, wait_for_url};
use crate::probe::{ProbeFailure, UrlPattern};
use crate::target::PageRef;
use crate::wire::options;
use crate::{ErrorCode, Playwright, PlaywrightError, resolve};

/// The prelude stamps `since` and `after` before its first await. Predicates stay in the
/// agent realm and send the last returned cursor when they ask for another navigation.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WaitForNavigationInput {
    page: PageRef,
    #[serde(default)]
    #[ts(optional)]
    frame: Option<FrameId>,
    #[serde(default)]
    #[ts(optional, type = "import('../../../probe/protocol').UrlPattern")]
    url: Option<UrlPattern>,
    #[serde(default)]
    wait_until: LoadState,
    since: u64,
    after: u64,
}
options!(WaitForNavigationInput);

/// A bounded wait returns its cursor and complete frame facts together, so the client can
/// refresh synchronous accessors without an additional untimed browser operation.
// The fields are spelled out in the order the two flattened structs they replace wrote them (a
// navigation result, then a frame tree): ts-rs would print both structs' `frames` as one
// object's duplicate key. The JSON is the same, pinned by `tests::tests_that_the_reply_keeps_its_bytes`.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigationReply {
    response: Option<Response>,
    url: Option<String>,
    frame_snapshot_attempted: bool,
    frames: Vec<FrameInfo>,
    navigation_seq: u64,
}

/// Waits for a navigation after the script's call, even when the current URL already matches.
pub(crate) async fn wait_for_navigation(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WaitForNavigationInput,
) -> Result<NavigationReply, PlaywrightError> {
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
        enable_frame(pw, scope, &page, frame).await?;
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
                    PageEvent::FrameSwapped(swapped) if swapped == *frame => {
                        let followed =
                            wait_for_url::follow(pw, &page, scope, input.frame.as_ref()).await?;
                        let event = NavigationEvent {
                            frame: frame.clone(),
                            url: followed.watch.url().unwrap_or_default().to_owned(),
                            loader: followed.watch.loader().map(str::to_owned),
                            new_document: true,
                            unreachable: None,
                        };
                        if let Some(navigation) =
                            complete_navigation(pw, scope, &page, &input, frame, event).await?
                        {
                            return navigation_reply(pw, scope, &page, navigation, record.seq)
                                .await;
                        }
                    }
                    PageEvent::Navigation(navigation) if navigation.frame == *frame => {
                        if let Some(navigation) =
                            complete_navigation(pw, scope, &page, &input, frame, navigation).await?
                        {
                            return navigation_reply(pw, scope, &page, navigation, record.seq)
                                .await;
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
        Ok(result) => result,
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
    if let Some(pattern) = &input.url {
        loop {
            match wait_for_url::matches(page, &followed.frame, pattern, event.url.clone()).await {
                Ok(true) => break,
                Ok(false) => return Ok(None),
                // Concurrent waits can each install a world while successive navigations
                // replace it. Matching the retained event URL is read-only; rediscover the
                // endpoint before retrying, so a swap is followed and a closed frame fails.
                // The enclosing navigation deadline bounds every attempt.
                Err(ProbeFailure::Gone(_)) => {
                    followed = wait_for_url::follow(pw, page, scope, input.frame.as_ref()).await?;
                }
                Err(failure) => return Err(resolve::failed(scope, failure)),
            }
        }
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
    if input.wait_until == LoadState::NetworkIdle {
        subtree_load::network_idle(pw, scope, page.tab(), frame).await?;
    }
    let response = if event.new_document && event.loader.is_some() {
        navigation::read_response(
            &followed.frame.endpoint.session,
            &event.url,
            Some(frame),
            event.loader.as_deref(),
        )
        .await
    } else {
        None
    };
    Ok(Some(NavigationResult::observed(response, event.url)))
}

/// Report frame facts under the same native timeout as the navigation; the prelude need not
/// issue an untimed page.frames operation after a successful bounded wait.
async fn navigation_reply(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    navigation: NavigationResult,
    navigation_seq: u64,
) -> Result<NavigationReply, PlaywrightError> {
    let since = pw.events().head();
    let frames = frame_tree::read(
        pw.browser(),
        page.core_page(),
        &page.main_endpoint().session,
    )
    .await
    .map_err(|error| scope.core_error(&error))?;
    frames.remember(pw, scope, page.tab(), &page.main_endpoint().session, since);
    // The wait's own result never carries frames (`NavigationResult::observed`); the tree just
    // read is the reply's.
    Ok(NavigationReply {
        response: navigation.response,
        url: navigation.url,
        frame_snapshot_attempted: navigation.frame_snapshot_attempted,
        frames: frames.into_frames(),
        navigation_seq,
    })
}

/// Existing page sessions already report navigation. Adopted child sessions need Page
/// enabled explicitly; a later renderer swap is journaled so the wait can follow and replay
/// the replacement's committed document even if it arrived before Page was enabled there.
async fn enable_frame(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    frame: &FrameId,
) -> Result<(), PlaywrightError> {
    let main = page.main_endpoint();
    if *frame == main.frame {
        return Ok(());
    }
    let root_id = main
        .session
        .session_id()
        .cloned()
        .ok_or_else(|| scope.fail(ErrorCode::Internal, "the page has no session"))?;
    let (_, session) =
        navigation::frame_host(pw, scope, page.tab(), (root_id, main.session), frame).await?;
    session
        .send_value("Page.enable", json!({}))
        .await
        .map_err(|error| scope.core_error(&error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::NavigationReply;
    use crate::page::frame_tree::{FrameTree, sample_tree};
    use crate::page::navigation::{NavigationResult, Response};
    use crate::test_support::TestResult;

    /// The reply is the bytes the two flattened structs it replaced wrote, a navigation result
    /// and then a frame tree, key order included: a later edit cannot drift unseen.
    #[test]
    fn tests_that_the_reply_keeps_its_bytes() -> TestResult {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Before {
            #[serde(flatten)]
            navigation: NavigationResult,
            #[serde(flatten)]
            frames: FrameTree,
            navigation_seq: u64,
        }
        let url = "https://main.test/".to_owned();
        for response in [Some(Response::sample()), None] {
            let answered = response.is_some();
            let before = Before {
                navigation: NavigationResult::observed(response, url.clone()),
                frames: sample_tree()?,
                navigation_seq: 7,
            };
            let reply = NavigationReply {
                response: answered.then(Response::sample),
                url: Some(url.clone()),
                frame_snapshot_attempted: true,
                frames: sample_tree()?.into_frames(),
                navigation_seq: 7,
            };
            let json = serde_json::to_string(&reply)?;
            assert_eq!(json, serde_json::to_string(&before)?);
            let response = if answered {
                r#"{"url":"https://main.test/","status":200,"statusText":"OK","ok":true,"headers":{}}"#
            } else {
                "null"
            };
            assert_eq!(
                json,
                format!(
                    r#"{{"response":{response},"url":"https://main.test/","frameSnapshotAttempted":true,"frames":[{{"id":"main","parentId":null,"name":"","url":"https://main.test/"}},{{"id":"child","parentId":"main","name":"ad","url":"https://child.test/"}}],"navigationSeq":7}}"#
                )
            );
        }
        Ok(())
    }
}
