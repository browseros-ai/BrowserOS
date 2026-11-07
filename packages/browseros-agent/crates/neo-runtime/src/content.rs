//! Neo: the document-wide frame calls of slice B3, each in the receiving frame's document:
//! `Frame.content`, `Frame.setContent` and `Frame.resolveSelector` (`locator.normalize`).
//! `Frame.waitForTimeout` is a wait (`waits.rs`); `Frame.ariaSnapshotJSON` is a read
//! (`reads.rs`).
//!
//! ```text
//!  Frame.content {includeShadow?}      prepare → documentContent in the frame's utility world
//!  Frame.setContent {html, waitUntil?} prepare → subscribe to the frame session's events
//!                                        → writeContent (document.open, write, close)
//!                                        → init, then DOMContentLoaded or load
//!  Frame.resolveSelector {selector}    locate (not strict) → generateSelector for the match
//!                                        → generateOwnerSelector for each iframe on the way
//!                                        → joined by enter-frame hops, main frame first
//! ```

use neo_runtime_spec::injected::{
    InjectedDocumentContentParams, InjectedGenerateOwnerSelectorParams,
    InjectedGenerateSelectorParams, InjectedWriteContentParams,
};
use neo_runtime_spec::serve::{
    FrameContentResult, FrameResolveSelectorResult, FrameSetContentParams, LifecycleEvent,
};

use browseros_cdp::{CdpEvent, SessionId};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::click::{PageError, Tab};
use crate::frame_selector;
use crate::navigate;
use crate::page::FrameTab;
use crate::progress::{CallError, PlaywrightError, Progress};

/// Neo: Playwright's `Frame._content` error, word for word, for a document that changed under
/// the read.
const NAVIGATING: &str =
    "Unable to retrieve content because the page is navigating and changing the content.";

/// Neo: the hop between two frame chunks, as Playwright's `resolveSelector` joins them.
const ENTER_FRAME: &str = " >> internal:control=enter-frame >> ";

/// Neo: one `Frame.setContent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SetContentOptions {
    pub(crate) html: String,
    pub(crate) wait_until: LifecycleEvent,
}

/// Neo: decodes `Frame.setContent`, refusing `networkidle` before any page call, as `goto` does.
impl SetContentOptions {
    pub(crate) fn from_params(params: FrameSetContentParams) -> Result<Self, PlaywrightError> {
        if params.wait_until == Some(LifecycleEvent::Networkidle) {
            return Err(PlaywrightError::error(
                "Neo does not support waitUntil: \"networkidle\" yet",
            ));
        }
        Ok(Self {
            html: params.html,
            // Playwright: `server/frames.ts` `Frame.setContent`: `const waitUntil = options.waitUntil === undefined ? 'load' : options.waitUntil;`
            wait_until: params.wait_until.unwrap_or(LifecycleEvent::Load),
        })
    }
}

/// Serves `Frame.content`: the frame's markup, its doctype first, through Playwright's
/// `documentContent`. A document that goes away under the read is Playwright's navigating error.
///
/// Playwright: `server/frames.ts` `Frame._content`: `return await injected.evaluate((injected, includeShadow) => injected.documentContent(includeShadow), !!options.includeShadow);`
pub(crate) async fn content(
    tab: &FrameTab,
    include_shadow: bool,
    progress: &Progress,
) -> Result<FrameContentResult, CallError> {
    progress
        .race(Tab::prepare(tab))
        .await?
        .map_err(read_error)?;
    let frame = progress.race(tab.context()).await?.map_err(read_error)?;
    let read = progress
        .race(frame.call(None, &InjectedDocumentContentParams { include_shadow }))
        .await?
        .map_err(read_error)?;
    Ok(FrameContentResult { value: read.value })
}

/// Serves `Frame.setContent`: the content written into the frame's document, then the wait for
/// `waitUntil` among the lifecycle events the reopened document fires, read from the frame's own
/// session: `document.open()` fires `init`, and only a `DOMContentLoaded` or `load` after it
/// counts. Chromium reads `readyState` as `complete` at `document.close()`, before the content's
/// images load, so the events decide. The page watcher's state is left as it was: Playwright,
/// too, clears the frame's lifecycle only for its own `setContent`.
///
/// Playwright: `server/frames.ts` `Frame.setContent`: `document.write(html);`
pub(crate) async fn set_content(
    tab: &FrameTab,
    options: &SetContentOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    progress.log(format!(
        "setting frame content, waiting until \"{}\"",
        lifecycle_name(options.wait_until)
    ));
    progress
        .race(Tab::prepare(tab))
        .await?
        .map_err(call_error)?;
    let frame = progress.race(tab.context()).await?.map_err(call_error)?;
    // Before the write: the reopened document fires its first events while the write is in
    // flight.
    let events = frame.host.events();
    progress
        .race(frame.call(
            None,
            &InjectedWriteContentParams {
                html: options.html.clone(),
            },
        ))
        .await?
        .map_err(call_error)?;
    if options.wait_until == LifecycleEvent::Commit {
        return Ok(());
    }
    progress
        .race(reopened(
            events,
            frame.host.session_id().clone(),
            frame.id.clone(),
            options.wait_until,
        ))
        .await?
        .map_err(call_error)
}

/// Neo: waits until the frame `frame` on session `session` fires `wait_until` after the `init`
/// of its reopened document.
async fn reopened(
    mut events: broadcast::Receiver<CdpEvent>,
    session: SessionId,
    frame: String,
    wait_until: LifecycleEvent,
) -> Result<(), PageError> {
    let mut opened = false;
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(RecvError::Closed) => return Err(PageError::Closed),
            Err(RecvError::Lagged(_)) => {
                return Err(PageError::Contract(
                    "neo-runtime: lifecycle events were dropped while setContent waited"
                        .to_string(),
                ));
            }
        };
        if event.method != "Page.lifecycleEvent"
            || event.session_id.as_ref() != Some(&session)
            || event.params["frameId"].as_str() != Some(frame.as_str())
        {
            continue;
        }
        match event.params["name"].as_str() {
            Some("init") => opened = true,
            Some("DOMContentLoaded")
                if opened && wait_until == LifecycleEvent::Domcontentloaded =>
            {
                return Ok(());
            }
            Some("load") if opened => return Ok(()),
            _ => {}
        }
    }
}

/// Serves `Frame.resolveSelector`: Playwright's simple selector for the first match, behind the
/// simple selector of each iframe on the way from the main frame.
///
/// Playwright: `server/frames.ts` `Frame.resolveSelector`: `return injected.generateSelectorSimple(node as unknown as Element);`
pub(crate) async fn resolve_selector(
    tab: &FrameTab,
    selector: &str,
    progress: &Progress,
) -> Result<FrameResolveSelectorResult, CallError> {
    progress
        .race(Tab::prepare(tab))
        .await?
        .map_err(call_error)?;
    let located = progress
        .race(frame_selector::locate(tab, selector, false))
        .await?
        .map_err(call_error)?;
    let Some(located) = located else {
        return Err(no_element(selector));
    };
    let generated = progress
        .race(located.frame.call(
            None,
            &InjectedGenerateSelectorParams {
                selector: located.selector.clone(),
            },
        ))
        .await?
        .map_err(call_error)?;
    let Some(element) = generated.generated else {
        return Err(no_element(selector));
    };
    let mut parts = Vec::with_capacity(located.owners.len() + 1);
    for owner in &located.owners {
        let generated = progress
            .race(owner.frame.call(
                None,
                &InjectedGenerateOwnerSelectorParams {
                    element: owner.element.clone(),
                },
            ))
            .await?
            .map_err(call_error)?;
        parts.push(generated.generated);
    }
    parts.push(element);
    if parts.iter().any(String::is_empty) {
        return Err(
            PlaywrightError::error(format!("Unable to generate locator for {selector}")).into(),
        );
    }
    Ok(FrameResolveSelectorResult {
        resolved_selector: parts.join(ENTER_FRAME),
        locator_code: None,
    })
}

/// Playwright: `server/frames.ts` `Frame.resolveSelector`: `if (!element)`
fn no_element(selector: &str) -> CallError {
    PlaywrightError::error(format!("No element matching {selector}")).into()
}

/// Neo: the name Playwright's log gives a lifecycle event.
fn lifecycle_name(state: LifecycleEvent) -> &'static str {
    match state {
        LifecycleEvent::Commit => "commit",
        LifecycleEvent::Domcontentloaded => "domcontentloaded",
        LifecycleEvent::Load => "load",
        LifecycleEvent::Networkidle => "networkidle",
    }
}

/// Neo: a content read's page error; a document that changed under it is Playwright's
/// navigating error, as is a read the page made throw.
fn read_error(error: PageError) -> CallError {
    match error {
        PageError::Closed => PlaywrightError::target_closed().into(),
        PageError::Contract(message) => PlaywrightError::error(message).into(),
        PageError::NotConnected | PageError::Threw(_) => PlaywrightError::error(NAVIGATING).into(),
    }
}

/// Neo: any other page error, as the page reported it.
fn call_error(error: PageError) -> CallError {
    navigate::page_error(error).into()
}

#[cfg(test)]
mod tests {
    //! The decoding steps; the page side is covered by the real-browser tests.

    use super::*;

    #[test]
    fn tests_that_set_content_waits_for_load_unless_told_and_refuses_networkidle() {
        let params = |wait_until| FrameSetContentParams {
            html: "<p>x</p>".to_string(),
            wait_until,
        };
        assert_eq!(
            SetContentOptions::from_params(params(None))
                .expect("load")
                .wait_until,
            LifecycleEvent::Load
        );
        assert_eq!(
            SetContentOptions::from_params(params(Some(LifecycleEvent::Commit)))
                .expect("commit")
                .wait_until,
            LifecycleEvent::Commit
        );
        assert!(
            SetContentOptions::from_params(params(Some(LifecycleEvent::Networkidle)))
                .expect_err("networkidle")
                .message
                .contains("networkidle")
        );
    }
}
