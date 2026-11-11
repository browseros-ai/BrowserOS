//! The event, dialog and download ops, their registration metadata and their handlers.

use std::pin::pin;

use browseros_core::{CoreError, ProtocolSession};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::events::dialog_table::{ClosedBy, Unanswerable};
use crate::events::event_hub::{now_ms, receives};
use crate::events::event_reader::{Delivered, Tabs, WindowPopup, deliver};
use crate::events::page_event::DialogId;
use crate::events::subscription::Unknown;
use crate::events::{ListenerKey, PageEvent, PageEventKind, Subscription};
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_output::to_output;
use crate::op_scope::{OpScope, Sent};
use crate::op_spec::{OpSpec, OpTarget, PAGE};
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError};

listed_enum! {
    /// Listening, the pump, one-shot waits, held dialogs and finished downloads.
    pub(crate) enum EventOp {
        Listen,
        Next,
        Wait,
        DialogAccept,
        DialogDismiss,
        DownloadPath,
        DownloadSaveAs,
    }
}

impl EventOp {
    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Listen => OpSpec::read(
                "events.listen",
                OpTarget::None,
                "start or stop this run's interest in a page or context event (on, once, off)",
            ),
            Self::Next => OpSpec::read(
                "events.next",
                OpTarget::None,
                "the prelude's event pump: a long poll for the events this run listens to",
            )
            .background(),
            Self::Wait => OpSpec::read(
                "events.wait",
                OpTarget::None,
                "the first matching event since the script called waitForEvent",
            ),
            Self::DialogAccept => OpSpec::write(
                "dialog.accept",
                PAGE,
                "accept a held dialog, with an optional prompt text",
            ),
            Self::DialogDismiss => OpSpec::write("dialog.dismiss", PAGE, "dismiss a held dialog"),
            Self::DownloadPath => OpSpec::read(
                "download.path",
                OpTarget::None,
                "wait for a download to finish: its path or its failure; also failure()",
            ),
            Self::DownloadSaveAs => OpSpec::write(
                "download.saveAs",
                OpTarget::None,
                "copy a finished download into the run's outputs directory",
            ),
        }
    }
}

/// `events.listen { page?, event, on, since? }`: `page.on(event)` (`on: true`) and
/// `page.off(event)` once its last listener goes (`on: false`); without `page`, the context's
/// `page` event. `since` is the script's `Date.now()` when it attached the listener.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListenInput {
    #[serde(default)]
    page: Option<PageRef>,
    event: String,
    on: bool,
    #[serde(default)]
    since: Option<u64>,
}

/// `events.wait { page?, event, since, after? }`: `page.waitForEvent(event)` and
/// `context.waitForEvent('page')`. `since` is the script's `Date.now()` when it called; a
/// predicate that rejects an event asks again with `after` set to that event's `seq`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitInput {
    #[serde(default)]
    page: Option<PageRef>,
    event: String,
    since: u64,
    #[serde(default)]
    after: Option<u64>,
}

/// `events.next {}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NextInput {}

/// `dialog.accept { page, dialog, promptText? }` and `dialog.dismiss { page, dialog }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DialogInput {
    page: PageRef,
    dialog: DialogId,
    #[serde(default)]
    prompt_text: Option<String>,
}

/// `download.path { download }` and `download.saveAs { download, path }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadInput {
    #[serde(rename = "download")]
    _download: String,
    #[serde(default, rename = "path")]
    _path: Option<String>,
}

/// What the pump returns: the events in log order, and how many records fell off the ring
/// before the run read them (the prelude logs it to the run's console).
#[derive(Debug, Serialize)]
struct Pumped {
    events: Vec<Delivered>,
    lost: u64,
}

/// Runs one event op.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: EventOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    match op {
        EventOp::Listen => listen(pw, scope, args.decode(scope)?).await,
        EventOp::Next => {
            let NextInput {} = args.decode(scope)?;
            next(pw, scope).await
        }
        EventOp::Wait => wait(pw, scope, args.decode(scope)?).await,
        EventOp::DialogAccept => answer(pw, scope, args.decode(scope)?, Answer::Accept).await,
        EventOp::DialogDismiss => answer(pw, scope, args.decode(scope)?, Answer::Dismiss).await,
        EventOp::DownloadPath | EventOp::DownloadSaveAs => {
            let DownloadInput { .. } = args.decode(scope)?;
            Err(scope.fail(
                ErrorCode::Unsupported,
                "Neo does not hand downloaded files to scripts in v1; the download event still \
                 reports url() and suggestedFilename()",
            ))
        }
    }
}

/// The page event `name`, or the failure a script gets for it.
fn page_event(scope: &OpScope<'_>, name: &str) -> Result<PageEventKind, PlaywrightError> {
    PageEventKind::parse(name).map_err(|unknown| match unknown {
        Unknown::Unsupported(hint) => scope.fail(ErrorCode::Unsupported, &hint),
        Unknown::NotAnEvent => scope.fail(
            ErrorCode::InvalidInput,
            &format!("\"{name}\" is not a page event"),
        ),
    })
}

/// The subscription a listen or wait names. A page subscription admits its page, which also
/// gives the target id the watcher's records carry.
async fn subscription(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: Option<PageRef>,
    event: &str,
) -> Result<Subscription, PlaywrightError> {
    match page {
        Some(handle) => {
            let event = page_event(scope, event)?;
            let page = handle.tab();
            let admitted = scope.admit(pw.browser(), page).await?;
            Ok(Subscription::Page {
                tab: page,
                target: admitted.session.target_id.clone(),
                event,
            })
        }
        None if event == "page" => Ok(Subscription::ContextPage),
        None => Err(context_event(scope, event)),
    }
}

/// The failure for a context event other than `page`.
fn context_event(scope: &OpScope<'_>, event: &str) -> PlaywrightError {
    match PageEventKind::parse(event) {
        Ok(_) | Err(Unknown::Unsupported(_)) => scope.fail(
            ErrorCode::Unsupported,
            &format!(
                "context.on('{event}') is not available in Neo; listen on the page: \
                 page.on('{event}')"
            ),
        ),
        Err(Unknown::NotAnEvent) => scope.fail(
            ErrorCode::InvalidInput,
            &format!("\"{event}\" is not a context event"),
        ),
    }
}

async fn listen(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ListenInput,
) -> Result<Value, PlaywrightError> {
    let run = scope.run_state();
    if !input.on {
        let key = match &input.page {
            Some(page) => ListenerKey::Page(page.tab(), page_event(scope, &input.event)?),
            None if input.event == "page" => ListenerKey::ContextPage,
            None => return Err(context_event(scope, &input.event)),
        };
        run.events.unlisten(&key);
        return Ok(json!({}));
    }
    let subscription = subscription(pw, scope, input.page, &input.event).await?;
    let since = input.since.unwrap_or_else(now_ms);
    let hub = pw.events();
    hub.bind(run);
    run.events
        .listen(subscription, since, hub.position_at(since));
    // A pump that already read past an event this listener should get reads again.
    hub.wake();
    Ok(json!({}))
}

/// `events.next`: the pump. Waits until some listener of the run has events, and returns them.
/// It has no Playwright timeout; the host cancels it when the run ends.
async fn next(pw: &Playwright, scope: &OpScope<'_>) -> Result<Value, PlaywrightError> {
    let run = scope.run_state();
    let hub = pw.events();
    hub.bind(run);
    loop {
        let mut changed = pin!(hub.changed());
        changed.as_mut().enable();
        let mut events = Vec::new();
        let mut lost = 0;
        let mut read_up_to = Vec::new();
        let mut tabs = Tabs::default();
        for (subscription, listening) in run.events.listeners() {
            let read = hub.read_after(listening.after);
            lost += read.lost;
            for record in &read.records {
                if receives(&subscription, listening.since_ms, record)
                    && let Some(event) =
                        deliver(scope, &subscription, record, &mut tabs, WindowPopup::Skip).await?
                {
                    events.push(event);
                }
            }
            read_up_to.push((subscription, read.head));
        }
        run.events.advance(&read_up_to);
        if !events.is_empty() || lost > 0 {
            events.sort_by_key(|event| event.seq);
            return to_output(scope, &Pumped { events, lost });
        }
        changed.await;
    }
}

/// `events.wait`: the first event recorded at or after the script's `since` (and after
/// `after`) that the subscription receives.
async fn wait(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WaitInput,
) -> Result<Value, PlaywrightError> {
    let subscription = subscription(pw, scope, input.page, &input.event).await?;
    let run = scope.run_state();
    pw.events().bind(run);
    // While it waits, the watcher records and holds what this wait could receive.
    let _waiting = run.events.waiting(subscription.clone());
    let found = first_event(
        pw,
        scope,
        &subscription,
        input.since,
        input.after.unwrap_or(0),
    );
    match scope.within(found).await {
        Ok(found) => to_output(scope, &found?),
        Err(_expired) => {
            if let Some(aborted) = scope.aborted_error() {
                return Err(aborted);
            }
            let millis = scope.timeout().map_or(0, |timeout| timeout.as_millis());
            let message = format!(
                "Timeout {millis}ms exceeded while waiting for event \"{}\"",
                input.event
            );
            Err(scope.fail(ErrorCode::WaitTimeout, &message))
        }
    }
}

async fn first_event(
    pw: &Playwright,
    scope: &OpScope<'_>,
    subscription: &Subscription,
    since: u64,
    mut after: u64,
) -> Result<Delivered, PlaywrightError> {
    let hub = pw.events();
    let mut tabs = Tabs::default();
    loop {
        let mut changed = pin!(hub.changed());
        changed.as_mut().enable();
        for record in hub.read_after(after).records {
            after = record.seq;
            // A close ends every outstanding page wait except the close wait itself.
            // Match the admitted target, not just the reusable native tab id, and report
            // the same close identity the dispatcher uses to authorize cached page facts.
            if record.at_ms >= since
                && let Some((target, event)) = subscription.page_event()
                && event != PageEventKind::Close
                && let PageEvent::Closed(closed) = &record.event
                && closed == target
            {
                if let Subscription::Page { tab, .. } = subscription {
                    scope.report_close(*tab, target, record.seq);
                }
                return Err(scope.fail(
                    ErrorCode::TargetClosed,
                    "Target page, context or browser has been closed",
                ));
            }
            if receives(subscription, since, &record)
                && let Some(event) =
                    deliver(scope, subscription, &record, &mut tabs, WindowPopup::Fail).await?
            {
                return Ok(event);
            }
        }
        changed.await;
    }
}

/// Which answer a dialog op gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Accept,
    Dismiss,
}

async fn answer(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DialogInput,
    answer: Answer,
) -> Result<Value, PlaywrightError> {
    let verb = match answer {
        Answer::Accept => "accept",
        Answer::Dismiss => "dismiss",
    };
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    let target = admitted.session.target_id.clone();
    // From here the dialog is the script's: the grace and the reap leave it to this answer, and
    // `answering` reopens it for them unless the browser acknowledges the answer.
    let answering = match pw.events().answer_dialog(input.dialog, &target) {
        Ok(answering) => answering,
        // Core accepted the alert before any listener saw it: the script's answer is moot.
        Err(Unanswerable::Closed(ClosedBy::Core)) => return Ok(json!({})),
        Err(
            Unanswerable::Answering
            | Unanswerable::Closed(ClosedBy::Script | ClosedBy::Neo | ClosedBy::Browser),
        ) => {
            return Err(scope.fail(
                ErrorCode::InvalidInput,
                &format!("Cannot {verb} dialog which is already handled!"),
            ));
        }
        Err(Unanswerable::Unknown) => {
            return Err(scope.fail(ErrorCode::InvalidInput, "no such dialog on this page"));
        }
    };
    let session = pw.browser().protocol_session(answering.session().clone());
    let prompt = match answer {
        Answer::Accept => input.prompt_text,
        Answer::Dismiss => None,
    };
    let sent = scope.send_effect()?;
    send_answer(&session, &sent, answer == Answer::Accept, prompt.as_deref())
        .await
        .map_err(|error| scope.core_error(&error))?;
    answering.answered();
    Ok(json!({}))
}

/// `Page.handleJavaScriptDialog` on the session the dialog opened on.
async fn send_answer(
    session: &ProtocolSession,
    _sent: &Sent<'_>,
    accept: bool,
    prompt_text: Option<&str>,
) -> Result<(), CoreError> {
    let params = match prompt_text {
        Some(text) => json!({ "accept": accept, "promptText": text }),
        None => json!({ "accept": accept }),
    };
    session
        .send_value("Page.handleJavaScriptDialog", params)
        .await?;
    Ok(())
}
