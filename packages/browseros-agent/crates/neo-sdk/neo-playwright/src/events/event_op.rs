//! The event, dialog and download ops, their registration metadata, route rows and handlers.
//!
//! ```text
//!  EventOp::route ── Run      → decode → handler → encode (`{}`, the pump's batch, a record, ..)
//!                 └─ Describe → OpWire → prelude generated/event.ts (EventRecord and the inputs)
//! ```
//!
//! The download ops wait in the hub's download table (`download_table.rs`) for the browser to
//! finish the file, then answer with the path the browser saved it at, or copy it for `saveAs`.
//! Neo keeps the browser's own download directory, so `download.path()` is a file there, not a
//! copy of Neo's: the host keeps its own copy with the run's record when the run ends.

use std::path::{self, Path, PathBuf};
use std::pin::pin;
use std::{io, slice};

use browseros_core::{CoreError, ProtocolSession, RuntimeLease, TabId};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::fs;
use ts_rs::TS;

use crate::events::dialog_table::{ClosedBy, Unanswerable};
use crate::events::event_hub::{now_ms, receives};
use crate::events::event_reader::{Delivered, Tabs, WindowPopup, binding_call, deliver};
use crate::events::page_event::DialogId;
use crate::events::subscription::Unknown;
use crate::events::{ListenerKey, PageEvent, PageEventKind, Subscription};
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::{Expired, OpScope, Sent};
use crate::op_spec::{OpSpec, OpTarget, PAGE};
use crate::target::PageRef;
use crate::wire::{self, Describe, Nothing, OpWire, Route, Run, options};
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
    /// The op's wire, described from the same route rows and handler types `invoke` runs.
    pub(crate) fn wire(self) -> OpWire {
        route(self, Describe)
    }

    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::FileChooserIntercept => OpSpec::read(
                "events.interceptFileChooser",
                OpTarget::None,
                "acquire or release this run's native file chooser interception lease",
            ),
            // on/off are synchronous APIs: their setup promise belongs to the prelude,
            // never to the script. A returning run may cancel it without an unawaited-op notice.
            Self::Listen => OpSpec::read(
                "events.listen",
                OpTarget::None,
                "start or stop this run's interest in a page or context event (on, once, off)",
            )
            .background(),
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

/// `events.listen`, in either of its two shapes: a script's interest, or the prelude's cleanup.
// Both arrive as one flat object (`ListenWire`) and are told apart once, at the decode, so the
// handler matches on the shape instead of checking field combinations; ts-rs prints the enum
// as the union of the two.
#[derive(Debug, TS)]
#[ts(untagged, rename = "EventListenInput")]
enum ListenInput {
    /// A script's interest in a page or context event.
    Interest(ListenInterest),
    /// The prelude's cleanup of a worker subscription whose page already closed.
    Retired(ListenRetired),
}
options!(ListenInput);

/// `page.on(event)` (`on: true`) and `page.off(event)` once its last listener goes
/// (`on: false`); without `page`, the context's event. `since` is the script's `Date.now()` when
/// it attached the listener (or removed it). The event ops name no target the door resolves, so
/// their page always has its tab: the prelude opens the run's lazy page first.
#[derive(Debug, TS)]
#[ts(rename = "EventListenInterest")]
struct ListenInterest {
    #[ts(optional = nullable)]
    page: Option<PageRef>,
    event: String,
    on: bool,
    #[ts(optional)]
    since: Option<u64>,
}

/// Internal worker bookkeeping can end after its page closed. This names only a key in this
/// run's subscription table, without resolving or acting on the retired tab, so it carries no
/// page and only ever removes the `workerstate` interest.
// The underscored fields are the fixed values the decode required: they carry nothing, and
// exist so the TypeScript names them.
#[derive(Debug, TS)]
#[ts(rename = "EventListenRetired")]
struct ListenRetired {
    #[ts(rename = "page")]
    _page: (),
    #[ts(rename = "retiredTab")]
    tab: TabId,
    #[ts(rename = "event", type = "'workerstate'")]
    _event: (),
    #[ts(rename = "on", type = "false")]
    _on: (),
    #[ts(rename = "since", optional)]
    _since: Option<u64>,
}

/// `events.listen` as it decodes: either shape, flat. Decoding this first keeps every decode
/// failure, and the refusal of a cleanup that names anything but its key, in the words it had
/// when the handler checked the fields itself (neo-server's cleanup test reads `retiredTab`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ListenWire {
    #[serde(default)]
    page: Option<PageRef>,
    event: String,
    on: bool,
    #[serde(default)]
    since: Option<u64>,
    #[serde(default)]
    retired_tab: Option<TabId>,
}

impl<'de> Deserialize<'de> for ListenInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let ListenWire {
            page,
            event,
            on,
            since,
            retired_tab,
        } = ListenWire::deserialize(deserializer)?;
        let Some(tab) = retired_tab else {
            return Ok(Self::Interest(ListenInterest {
                page,
                event,
                on,
                since,
            }));
        };
        if on || page.is_some() || event != "workerstate" {
            return Err(de::Error::custom(
                "retiredTab only removes internal worker bookkeeping without a page target",
            ));
        }
        Ok(Self::Retired(ListenRetired {
            _page: (),
            tab,
            _event: (),
            _on: (),
            _since: since,
        }))
    }
}

/// `events.wait { page?, event, since, after? }`: `page.waitForEvent(event)` and
/// `context.waitForEvent('page')`. `since` is the script's `Date.now()` when it called; a
/// predicate that rejects an event asks again with `after` set to that event's `seq`.
///
/// `call.timeoutMs` counts from this op's own start, so the prelude sends what is left of the
/// `waitForEvent`'s one deadline and names the script's timeout in the message. While it waits
/// for `dialog`, the run counts as listening, so the dialog is held for it.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "EventWaitInput")]
struct WaitInput {
    #[serde(default)]
    #[ts(optional = nullable)]
    page: Option<PageRef>,
    event: String,
    since: u64,
    #[serde(default)]
    #[ts(optional = nullable)]
    after: Option<u64>,
}
options!(WaitInput);

/// `events.next {}`: the background pump. It names no cursor: the run's state keeps one read
/// position per listener, so a listener attached after the pump read past an event still gets it.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "EventNextInput")]
struct NextInput {}
options!(NextInput);

/// A synchronous JS interest owns one lease before an activation can be dispatched.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "InterceptFileChooserInput")]
struct InterceptInput {
    page: PageRef,
    lease: u64,
    on: bool,
}
options!(InterceptInput);

/// `dialog.accept { page, dialog, promptText? }` and `dialog.dismiss { page, dialog }`. One
/// input for both, so `dialog.dismiss` accepts a `promptText` it never sends and ignores it
/// (its row names it); one input per op would be a later clean-up.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "DialogInput")]
struct DialogInput {
    page: PageRef,
    dialog: DialogId,
    #[serde(default)]
    #[ts(optional)]
    prompt_text: Option<String>,
}
options!(DialogInput);

/// `download.path { download }`, behind `download.path()` and `download.failure()`. `download`
/// is the browser's guid, which the `download` event handed the script.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "DownloadPathInput")]
struct PathInput {
    download: String,
}
options!(PathInput);

/// `download.saveAs { download, path }`. A relative `path` resolves against Neo's working
/// directory, as Playwright resolves it against the client's.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "DownloadSaveAsInput")]
struct SaveAsInput {
    download: String,
    path: String,
}
options!(SaveAsInput);

/// `{}`: what interception, listening and a dialog's answer have always answered. Upstream's
/// commands answer nothing and the prelude reads nothing from it; an empty object rather than
/// `Nothing` keeps the reply's bytes.
#[derive(Debug, Serialize, TS)]
#[ts(rename = "EventAcknowledged")]
struct Acknowledged {}

/// What `download.path` answers: exactly one of the two is set.
#[derive(Debug, Serialize, TS)]
#[ts(rename = "DownloadFinished")]
struct Finished {
    path: Option<String>,
    failure: Option<String>,
}

/// What the pump returns: the events in log order, and how many records fell off the ring
/// before the run read them (the prelude logs it to the run's console). claw-sdk's door reads
/// both fields to release a batch.
#[derive(Debug, Serialize, TS)]
struct Pumped {
    events: Vec<Delivered>,
    lost: u64,
}

/// Why `dialog.*` and `download.*` carry an id upstream's commands lack.
const DIALOG: (&str, &str) = (
    "dialog",
    "the host-owned dialog's id, instead of an upstream Dialog channel",
);
const DOWNLOAD: (&str, &str) = (
    "download",
    "the host-owned download's id, instead of an upstream Artifact channel",
);
/// Why a dialog's answer is `{}` where upstream's is void.
const ANSWERED: &str = "a dialog's answer has always been {}; the prelude reads nothing from it";

/// Runs one event op through the shared decode/call/encode boundary.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: EventOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    route(op, Run { pw, scope, args }).await
}

/// Run and Describe share this exhaustive list: an op cannot run without the row that
/// describes its wire. Describing never admits a page or touches the hub.
fn route<'a, R: Route<'a>>(op: EventOp, r: R) -> R::Out {
    match op {
        EventOp::FileChooserIntercept => r.op(
            wire::replaces(
                "Page.updateSubscription",
                "Neo leases chooser interception per run, so the run's end releases it; \
                 upstream subscribes by event name and an enabled flag",
            ),
            intercept,
        ),
        EventOp::Listen => r.op(
            wire::neo_only(
                "run-scoped listener bookkeeping, including cleanup after a tab closed; \
                 upstream subscribes per channel",
            ),
            listen,
        ),
        EventOp::Next => r.op(
            wire::neo_only("the prelude's background event pump, one cursor per listener"),
            next,
        ),
        EventOp::Wait => r.op(
            wire::neo_only("a journal wait by event and time; upstream waits on channel events"),
            wait,
        ),
        EventOp::DialogAccept => r.op(
            wire::command("Dialog.accept")
                .adds(&[DIALOG])
                .result(ANSWERED),
            accept,
        ),
        EventOp::DialogDismiss => r.op(
            wire::command("Dialog.dismiss")
                .adds(&[
                    DIALOG,
                    (
                        "promptText",
                        "dialog.accept's input, shared: dismiss accepts it and ignores it",
                    ),
                ])
                .result(ANSWERED),
            dismiss,
        ),
        EventOp::DownloadPath => r.op(
            wire::command("Artifact.pathAfterFinished").adds(&[DOWNLOAD]),
            download_path,
        ),
        EventOp::DownloadSaveAs => {
            r.op(wire::command("Artifact.saveAs").adds(&[DOWNLOAD]), save_as)
        }
    }
}

async fn intercept(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: InterceptInput,
) -> Result<Acknowledged, PlaywrightError> {
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
    Ok(Acknowledged {})
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

/// Whether only `Runtime` reports `event` on a page session: console messages and uncaught
/// errors, and binding calls with the main-world contexts that name their frame. Core attaches
/// pages with `Runtime` off (a site times the inspector's console serialization to detect
/// DevTools), so a run that wants these takes a lease ([`lease_runtime`]).
pub(crate) const fn needs_runtime(event: PageEventKind) -> bool {
    matches!(
        event,
        PageEventKind::Console | PageEventKind::PageError | PageEventKind::Binding
    )
}

/// `Runtime` leases for what `subscription` listens to, floored at the script's `since_ms`:
/// its page's session, or every page open now for a context listener. Hosted BrowserOS sends
/// no console or exception events at all, so there a listener for them leases nothing.
async fn lease_runtime(
    pw: &Playwright,
    scope: &OpScope<'_>,
    subscription: &Subscription,
    since_ms: u64,
) -> Result<Vec<RuntimeLease>, PlaywrightError> {
    let event = match subscription {
        Subscription::Page { event, .. } | Subscription::Context(event) => *event,
        Subscription::ContextPage => return Ok(Vec::new()),
    };
    let hosted_silent =
        scope.hosted_mode() && matches!(event, PageEventKind::Console | PageEventKind::PageError);
    if !needs_runtime(event) || hosted_silent {
        return Ok(Vec::new());
    }
    let leases = pw.browser().page_signals.runtime_leases();
    #[expect(
        clippy::cast_precision_loss,
        reason = "epoch milliseconds stay far below 2^53"
    )]
    let floor = since_ms as f64;
    let core = |error: CoreError| scope.core_error(&error);
    if let Subscription::Page { tab, .. } = subscription {
        let admitted = scope.admit(pw.browser(), *tab).await?;
        let lease = leases
            .acquire(&admitted.session.session, floor)
            .await
            .map_err(core)?;
        return Ok(vec![lease]);
    }
    let listing = scope.view().list_pages().await.map_err(core)?;
    let mut held = Vec::new();
    for page in listing.pages {
        // A tab that closed since the listing has nothing left to report.
        if let Ok(session) = scope.view().page_session(page.page_id).await
            && let Ok(lease) = leases.acquire(&session.session, floor).await
        {
            held.push(lease);
        }
    }
    Ok(held)
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
) -> Result<Acknowledged, PlaywrightError> {
    let run = scope.run_state();
    let input = match input {
        ListenInput::Retired(retired) => {
            run.events
                .unlisten(&ListenerKey::Page(retired.tab, PageEventKind::WorkerState));
            return Ok(Acknowledged {});
        }
        ListenInput::Interest(interest) => interest,
    };
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
        return Ok(Acknowledged {});
    }
    let subscription = subscription(pw, scope, input.page, &input.event).await?;
    let since = input.since.unwrap_or_else(now_ms);
    let hub = pw.events();
    hub.bind(run);
    let observes_context = matches!(subscription, Subscription::Context(_));
    run.events
        .listen(subscription.clone(), since, hub.position_at(since));
    // Before the ack: the script's next action may log what it listens for.
    let leases = lease_runtime(pw, scope, &subscription, since).await?;
    run.events.hold_runtime(&subscription, leases);
    // Register before asynchronous observation setup: an off arriving during setup removes
    // the actual listener and cannot be undone when this on finishes.
    if observes_context {
        observe_context(scope).await?;
    }
    // A pump that already read past an event this listener should get reads again.
    hub.wake();
    Ok(Acknowledged {})
}

/// `events.next`: the pump. Waits until some listener of the run has events, and returns them.
/// It has no Playwright timeout; the host cancels it when the run ends.
async fn next(
    pw: &Playwright,
    scope: &OpScope<'_>,
    NextInput {}: NextInput,
) -> Result<Pumped, PlaywrightError> {
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
        // Before the advance, which counts this reply when it carries anything.
        let (calls, dropped) = run.events.take_calls();
        events.extend(calls.into_iter().map(binding_call));
        lost += dropped;
        let replying = !events.is_empty() || lost > 0;
        run.events.advance(&read_up_to, replying);
        if replying {
            events.sort_by_key(Delivered::order);
            // After the sort, so a run's download index is the order its script receives them.
            hand_downloads(pw, scope, &events);
            return Ok(Pumped { events, lost });
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
) -> Result<Delivered, PlaywrightError> {
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
    let _runtime = lease_runtime(pw, scope, &subscription, input.since).await?;
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
            Ok(found)
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
) -> Result<Finished, PlaywrightError> {
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
    Ok(finished)
}

/// `download.saveAs`: once the browser finished the file, a copy at `path`, with its parent
/// directories made as Playwright makes them. A download that failed fails the save with its
/// failure, as upstream's does.
async fn save_as(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: SaveAsInput,
) -> Result<Nothing, PlaywrightError> {
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
    Ok(())
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

async fn accept(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DialogInput,
) -> Result<Acknowledged, PlaywrightError> {
    answer(pw, scope, input, Answer::Accept).await
}

async fn dismiss(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DialogInput,
) -> Result<Acknowledged, PlaywrightError> {
    answer(pw, scope, input, Answer::Dismiss).await
}

async fn answer(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DialogInput,
    answer: Answer,
) -> Result<Acknowledged, PlaywrightError> {
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
        Err(Unanswerable::Closed(ClosedBy::Core)) => return Ok(Acknowledged {}),
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
    Ok(Acknowledged {})
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

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::TabId;
    use serde::Serialize;
    use serde_json::{Value, json};

    use super::{Acknowledged, EventOp, Finished, ListenInput, Pumped};
    use crate::op::Op;
    use crate::op_output::to_output;
    use crate::op_scope::OpScope;
    use crate::target::CallHeader;
    use crate::test_env::{self, TestEnv};
    use crate::wire::Nothing;
    use crate::{ErrorCode, Playwright};

    /// The refusal of a cleanup that names anything but its key, as the handler worded it
    /// before the two shapes were told apart at the decode.
    const RETIRED_ONLY: &str =
        "retiredTab only removes internal worker bookkeeping without a page target";

    /// What an answer is on the wire: the JSON value the route's `to_output` makes of it, printed.
    fn wire<T: Serialize>(answer: &T) -> Result<String, Box<dyn Error>> {
        let env = TestEnv::new();
        let op = Op::from_name("events.next").ok_or("events.next is an op")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        Ok(serde_json::to_string(&to_output(&scope, answer)?)?)
    }

    /// Each shape the prelude sends (events.ts `sendListen`) decodes as its variant.
    #[test]
    fn tests_that_listen_decodes_the_two_shapes_the_prelude_sends() -> Result<(), Box<dyn Error>> {
        let interest: ListenInput = serde_json::from_value(json!({
            "page": { "$neo": "page", "tab": 1007 }, "event": "dialog", "on": true, "since": 111,
        }))?;
        let ListenInput::Interest(interest) = interest else {
            return Err("a page listener is an interest".into());
        };
        assert_eq!(interest.page.map(|page| page.tab()), Some(TabId(1007)));
        assert_eq!((interest.event.as_str(), interest.on), ("dialog", true));
        assert_eq!(interest.since, Some(111));
        let context: ListenInput = serde_json::from_value(
            json!({ "page": null, "event": "page", "on": false, "since": 5 }),
        )?;
        assert!(matches!(context, ListenInput::Interest(ref i) if i.page.is_none()));
        let retired: ListenInput = serde_json::from_value(json!({
            "page": null, "retiredTab": 1007, "event": "workerstate", "on": false, "since": 9,
        }))?;
        let ListenInput::Retired(retired) = retired else {
            return Err("a cleanup with retiredTab is retired".into());
        };
        assert_eq!(retired.tab, TabId(1007));
        Ok(())
    }

    /// A cleanup that also names a page, listens, or names another event is refused in the
    /// handler's old words; an unknown key still fails the field decode.
    #[test]
    fn tests_that_a_mixed_cleanup_is_refused_in_the_old_words() {
        for mixed in [
            json!({ "page": null, "retiredTab": 7, "event": "workerstate", "on": true, "since": 0 }),
            json!({ "page": { "$neo": "page", "tab": 7 }, "retiredTab": 7, "event": "workerstate",
                    "on": false, "since": 0 }),
            json!({ "page": null, "retiredTab": 7, "event": "console", "on": false, "since": 0 }),
        ] {
            let refused = serde_json::from_value::<ListenInput>(mixed).map(|_| ());
            assert_eq!(
                refused.map_err(|error| error.to_string()),
                Err(RETIRED_ONLY.to_owned())
            );
        }
        let unknown = serde_json::from_value::<ListenInput>(
            json!({ "event": "page", "on": true, "since": 0, "lease": 1 }),
        );
        assert!(unknown.is_err_and(|error| error.to_string().contains("unknown field `lease`")));
    }

    /// Through the op boundary the refusal reads `<api>: ...` and is `script / invalid_input`,
    /// as when the handler raised it; a valid cleanup answers `{}` without admitting a page.
    #[tokio::test(start_paused = true)]
    async fn tests_that_listen_answers_and_refuses_through_the_op_as_before()
    -> Result<(), Box<dyn Error>> {
        let playwright = Playwright::new(test_env::offline_session());
        let op = Op::from_name("events.listen").ok_or("events.listen is an op")?;
        let env = TestEnv::new();
        let cleaned = playwright
            .invoke(
                &env,
                op,
                json!({ "call": { "api": "page.off", "timeoutMs": 0 }, "page": null,
                        "retiredTab": 1007, "event": "workerstate", "on": false, "since": 0 }),
            )
            .await?;
        assert_eq!(wire(&cleaned)?, "{}");
        let refused = playwright
            .invoke(
                &env,
                op,
                json!({ "call": { "api": "page.on", "timeoutMs": 0 }, "page": null,
                        "retiredTab": 1007, "event": "workerstate", "on": true, "since": 0 }),
            )
            .await
            .err()
            .ok_or("a listening cleanup is refused")?;
        assert_eq!(refused.error_code(), Some(ErrorCode::InvalidInput));
        assert_eq!(refused.message(), format!("page.on: {RETIRED_ONLY}"));
        Ok(())
    }

    /// The answers' bytes: `{}` stays `{}` (listen, interception, both dialog answers),
    /// `saveAs` stays `null`, `download.path` names both fields, the pump its two.
    #[tokio::test]
    async fn tests_that_every_event_answer_keeps_its_bytes() -> Result<(), Box<dyn Error>> {
        assert_eq!(wire(&Acknowledged {})?, "{}");
        let saved: Nothing = ();
        assert_eq!(wire(&saved)?, "null");
        let done = Finished {
            path: Some("/runs/r/f.csv".to_owned()),
            failure: None,
        };
        assert_eq!(wire(&done)?, r#"{"failure":null,"path":"/runs/r/f.csv"}"#);
        let failed = Finished {
            path: None,
            failure: Some("canceled".to_owned()),
        };
        assert_eq!(wire(&failed)?, r#"{"failure":"canceled","path":null}"#);
        let idle = Pumped {
            events: Vec::new(),
            lost: 3,
        };
        assert_eq!(wire(&idle)?, r#"{"events":[],"lost":3}"#);
        for op in EventOp::ALL {
            let output = op.wire().output;
            let expected = match op {
                EventOp::Listen
                | EventOp::FileChooserIntercept
                | EventOp::DialogAccept
                | EventOp::DialogDismiss => "EventAcknowledged",
                EventOp::Next => "Pumped",
                EventOp::Wait => "EventRecord",
                EventOp::DownloadPath => "DownloadFinished",
                EventOp::DownloadSaveAs => "null",
            };
            assert_eq!(output, expected, "{}", op.spec().name);
        }
        Ok(())
    }

    /// The recorded prelude inputs (wire-examples.json) leave out listen, wait and
    /// interception, which run on the internal site; these are the literals the prelude sends.
    #[test]
    fn tests_that_the_preludes_internal_event_inputs_decode() -> Result<(), Box<dyn Error>> {
        let page = json!({ "$neo": "page", "tab": 1007 });
        let wait: super::WaitInput = serde_json::from_value(
            json!({ "page": page, "event": "popup", "since": 111, "after": null }),
        )?;
        assert_eq!((wait.since, wait.after), (111, None));
        let rewait: super::WaitInput = serde_json::from_value(
            json!({ "page": null, "event": "page", "since": 500, "after": 5 }),
        )?;
        assert_eq!((rewait.page.is_none(), rewait.after), (true, Some(5)));
        let leased: super::InterceptInput =
            serde_json::from_value(json!({ "page": page, "lease": 1, "on": true }))?;
        assert_eq!((leased.lease, leased.on), (1, true));
        let next: Value = json!({});
        let super::NextInput {} = serde_json::from_value(next)?;
        // `dialog.dismiss` shares `dialog.accept`'s input: a `promptText` decodes and is ignored.
        let dismissed: super::DialogInput = serde_json::from_value(
            json!({ "page": page, "dialog": 4, "promptText": "ignored by dismiss" }),
        )?;
        assert_eq!(dismissed.prompt_text.as_deref(), Some("ignored by dismiss"));
        Ok(())
    }
}
