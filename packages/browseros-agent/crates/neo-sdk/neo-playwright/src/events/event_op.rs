//! The event, dialog and download ops, their registration metadata and their handlers.
//!
//! The download ops wait in the hub's download table (`download_table.rs`) for the browser to
//! finish the file, then answer with the path the browser saved it at, or copy it for `saveAs`.
//! Neo keeps the browser's own download directory, so `download.path()` is a file there, not a
//! copy of Neo's: the host keeps its own copy with the run's record when the run ends.

use std::path::{self, Path, PathBuf};
use std::pin::pin;
use std::{io, slice};

use browseros_core::{CoreError, ProtocolSession};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::fs;

use crate::events::dialog_table::{ClosedBy, Unanswerable};
use crate::events::event_hub::{now_ms, receives};
use crate::events::event_reader::{Delivered, Tabs, WindowPopup, deliver};
use crate::events::page_event::DialogId;
use crate::events::subscription::Unknown;
use crate::events::{ListenerKey, PageEvent, PageEventKind, Subscription};
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_output::to_output;
use crate::op_scope::{Expired, OpScope, Sent};
use crate::op_spec::{OpSpec, OpTarget, PAGE};
use crate::target::PageRef;
use crate::{DownloadState, ErrorCode, Playwright, PlaywrightError, file_save};

listed_enum! {
    /// Listening, the pump, one-shot waits, held dialogs and finished downloads.
    pub(crate) enum EventOp {
        Listen,
        Next,
        Wait,
        FileChooserIntercept,
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
            Self::FileChooserIntercept => OpSpec::read(
                "events.interceptFileChooser",
                OpTarget::None,
                "acquire or release this run's native file chooser interception lease",
            ),
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
                "wait for a download to finish and copy it to a path on Neo's disk",
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

/// A synchronous JS interest owns one lease before an activation can be dispatched.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InterceptInput {
    page: PageRef,
    lease: u64,
    on: bool,
}

/// `dialog.accept { page, dialog, promptText? }` and `dialog.dismiss { page, dialog }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DialogInput {
    page: PageRef,
    dialog: DialogId,
    #[serde(default)]
    prompt_text: Option<String>,
}

/// `download.path { download }`, behind `download.path()` and `download.failure()`. `download`
/// is the browser's guid, which the `download` event handed the script.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathInput {
    download: String,
}

/// `download.saveAs { download, path }`. A relative `path` resolves against Neo's working
/// directory, as Playwright resolves it against the client's.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveAsInput {
    download: String,
    path: String,
}

/// What `download.path` answers: exactly one of the two is set.
#[derive(Debug, Serialize)]
struct Finished {
    path: Option<String>,
    failure: Option<String>,
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
        EventOp::FileChooserIntercept => intercept(pw, scope, args.decode(scope)?).await,
        EventOp::Listen => listen(pw, scope, args.decode(scope)?).await,
        EventOp::Next => {
            let NextInput {} = args.decode(scope)?;
            next(pw, scope).await
        }
        EventOp::Wait => wait(pw, scope, args.decode(scope)?).await,
        EventOp::DialogAccept => answer(pw, scope, args.decode(scope)?, Answer::Accept).await,
        EventOp::DialogDismiss => answer(pw, scope, args.decode(scope)?, Answer::Dismiss).await,
        EventOp::DownloadPath => download_path(pw, scope, args.decode(scope)?).await,
        EventOp::DownloadSaveAs => save_as(pw, scope, args.decode(scope)?).await,
    }
}

async fn intercept(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: InterceptInput,
) -> Result<Value, PlaywrightError> {
    let tab = input.page.tab();
    if input.on {
        let admitted = scope.admit(pw.browser(), tab).await?;
        let lease = scope
            .view()
            .files()
            .intercept_choosers(admitted.core_page.clone())
            .await
            .map_err(|error| scope.core_error(&error))?;
        scope
            .run_state()
            .events
            .keep_interception(input.lease, tab, lease);
    } else if let Some(mut lease) = scope.run_state().events.take_interception(input.lease, tab) {
        // Cleanup uses its original epoch/session binding, even if the page closed or moved.
        lease
            .release()
            .await
            .map_err(|error| scope.core_error(&error))?;
    }
    Ok(json!({}))
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
        None if PageEventKind::context(event).is_some() => Ok(Subscription::Context(
            PageEventKind::context(event).ok_or_else(|| context_event(scope, event))?,
        )),
        None => Err(context_event(scope, event)),
    }
}

/// Context listeners observe existing tabs without requiring a prior page action. These are
/// core-owned observation sessions; event release still maps each returned tab through the run.
async fn observe_context(scope: &OpScope<'_>) -> Result<(), PlaywrightError> {
    let setup = async {
        let listing = scope.view().list_pages().await?;
        for page in listing.pages {
            scope
                .run_state()
                .events
                .remember_page(&page.target_id, page.tab_id);
            // A tab can close between listing and attachment. The watcher retries live tabs.
            if let Err(error) = scope.view().page_session(page.page_id).await {
                scope.log(format!(
                    "context lifecycle observation will retry attachment: {error}"
                ));
            }
        }
        Ok::<(), CoreError>(())
    };
    scope
        .within(setup)
        .await
        .map_err(|_expired| scope.timed_out(ErrorCode::WaitTimeout))?
        .map_err(|error| scope.core_error(&error))
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
            None if PageEventKind::context(&input.event).is_some() => ListenerKey::Context(
                PageEventKind::context(&input.event)
                    .ok_or_else(|| context_event(scope, &input.event))?,
            ),
            None => return Err(context_event(scope, &input.event)),
        };
        run.events.unlisten(&key);
        return Ok(json!({}));
    }
    let subscription = subscription(pw, scope, input.page, &input.event).await?;
    let since = input.since.unwrap_or_else(now_ms);
    let hub = pw.events();
    hub.bind(run);
    let observes_context = matches!(subscription, Subscription::Context(_));
    run.events
        .listen(subscription, since, hub.position_at(since));
    // Register before asynchronous observation setup: an off arriving during setup removes
    // the actual listener and cannot be undone when this on finishes.
    if observes_context {
        observe_context(scope).await?;
    }
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
    // An op that ends waits for a pump in flight to read past its events (RunEvents::taken_through).
    let _pumping = run.events.pumping();
    loop {
        let mut changed = pin!(hub.changed());
        changed.as_mut().enable();
        let mut events = Vec::new();
        let mut lost = 0;
        let mut read_up_to = Vec::new();
        let mut tabs = Tabs::default();
        let listeners = run.events.listeners();
        let after = listeners
            .iter()
            .map(|(_, listening)| listening.after)
            .min()
            .unwrap_or_else(|| hub.head());
        // All audiences share one journal cut. Appends during a tab lookup belong to the
        // next batch for everyone, so page/context mirrors cannot split across batches.
        let read = hub.read_after(after);
        for (subscription, listening) in listeners {
            lost += read
                .lost
                .saturating_sub(listening.after.saturating_sub(after));
            for record in &read.records {
                if record.seq > listening.after
                    && receives(&subscription, listening.since_ms, record)
                    && let Some(event) = deliver(
                        pw,
                        scope,
                        &subscription,
                        record,
                        &mut tabs,
                        WindowPopup::Skip,
                    )
                    .await?
                {
                    events.push(event);
                }
            }
            read_up_to.push((subscription, read.head));
        }
        let replying = !events.is_empty() || lost > 0;
        run.events.advance(&read_up_to, replying);
        if replying {
            events.sort_by_key(Delivered::order);
            // After the sort, so a run's download index is the order its script receives them.
            hand_downloads(pw, scope, &events);
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
    // The native browser cannot produce these in hosted mode. Refuse before installing a
    // wait lease or consuming its timeout; ordinary listeners remain accepted and silent.
    if scope.hosted_mode()
        && input.page.is_some()
        && matches!(input.event.as_str(), "console" | "pageerror")
    {
        return Err(scope.fail(
            ErrorCode::Unsupported,
            &format!(
                "Hosted BrowserOS does not emit '{}' events; page.waitForEvent('{}') is \
                 unavailable in hosted mode",
                input.event, input.event
            ),
        ));
    }
    let subscription = subscription(pw, scope, input.page, &input.event).await?;
    let run = scope.run_state();
    pw.events().bind(run);
    // While it waits, the watcher records and holds what this wait could receive.
    let _waiting = run.events.waiting(subscription.clone());
    if matches!(subscription, Subscription::Context(_)) {
        observe_context(scope).await?;
    }
    let found = first_event(
        pw,
        scope,
        &subscription,
        input.since,
        input.after.unwrap_or(0),
    );
    match scope.within(found).await {
        Ok(found) => {
            let found = found?;
            hand_downloads(pw, scope, slice::from_ref(&found));
            to_output(scope, &found)
        }
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
                && let Some(event) = deliver(
                    pw,
                    scope,
                    subscription,
                    &record,
                    &mut tabs,
                    WindowPopup::Fail,
                )
                .await?
            {
                return Ok(event);
            }
        }
        changed.await;
    }
}

/// `download.path`: the saved file's path once the browser finished it, or why it never will.
async fn download_path(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PathInput,
) -> Result<Value, PlaywrightError> {
    let finished = match finished(pw, scope, &input.download).await? {
        Saved::File(path) => Finished {
            path: Some(path.to_string_lossy().into_owned()),
            failure: None,
        },
        Saved::Failed(failure) => Finished {
            path: None,
            failure: Some(failure),
        },
    };
    to_output(scope, &finished)
}

/// `download.saveAs`: once the browser finished the file, a copy at `path`, with its parent
/// directories made as Playwright makes them. A download that failed fails the save with its
/// failure, as upstream's does.
async fn save_as(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: SaveAsInput,
) -> Result<Value, PlaywrightError> {
    let source = match finished(pw, scope, &input.download).await? {
        Saved::File(path) => path,
        Saved::Failed(failure) => return Err(scope.fail(ErrorCode::DownloadFailed, &failure)),
    };
    let target = path::absolute(&input.path).map_err(|error| {
        scope.fail(
            ErrorCode::InvalidInput,
            &format!("cannot save the download to \"{}\": {error}", input.path),
        )
    })?;
    let sent = scope.send_effect()?;
    copy_to(&sent, &source, &target).await.map_err(|error| {
        scope.fail(
            file_save::error_code(error.kind()),
            &format!(
                "cannot save the download to \"{}\": {error}",
                target.display()
            ),
        )
    })?;
    Ok(Value::Null)
}

/// Adds each download among `events` to the run's downloads, in order: the run keeps it from
/// here, so its record lists it even if the script never asks for the file.
fn hand_downloads(pw: &Playwright, scope: &OpScope<'_>, events: &[Delivered]) {
    let table = pw.events().downloads();
    for guid in events.iter().filter_map(Delivered::download) {
        if let Some(download) = table.get(guid) {
            scope.run_state().downloads.handed(download);
        }
    }
}

/// Where a finished download stands for the ops that read it.
enum Saved {
    File(PathBuf),
    Failed(String),
}

/// Waits for the download the script's `Download` names to finish. It has no timeout unless the
/// script gave one (`path()` and `saveAs()` take none); the run's own deadline still stops it.
async fn finished(
    pw: &Playwright,
    scope: &OpScope<'_>,
    guid: &str,
) -> Result<Saved, PlaywrightError> {
    // The run's own downloads first: the table may have stopped listing one the run holds.
    let download = scope
        .run_state()
        .downloads
        .find(guid)
        .or_else(|| pw.events().downloads().get(guid));
    let Some(download) = download else {
        return Err(scope.fail(
            ErrorCode::Unsupported,
            "Neo does not know this download; use the `Download` a `download` event of this run \
             handed the script",
        ));
    };
    match scope.within(download.finished()).await {
        Ok(DownloadState::Completed(path)) => Ok(Saved::File(path)),
        Ok(DownloadState::Failed(failure)) => Ok(Saved::Failed(failure)),
        // `finished` resolves only once the state is final.
        Ok(DownloadState::InProgress) => Err(scope.fail(
            ErrorCode::Internal,
            "the download's wait ended while it was still in progress",
        )),
        Err(Expired) => Err(scope.timed_out(ErrorCode::WaitTimeout)),
    }
}

/// Copies `source` to `target`. A save onto the file itself is already done: copying a file
/// over itself would empty it.
async fn copy_to(_sent: &Sent<'_>, source: &Path, target: &Path) -> io::Result<()> {
    if let (Ok(from), Ok(to)) = (
        fs::canonicalize(source).await,
        fs::canonicalize(target).await,
    ) && from == to
    {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::copy(source, target).await.map(drop)
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
