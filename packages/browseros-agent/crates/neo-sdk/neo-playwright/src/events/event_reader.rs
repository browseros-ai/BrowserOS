//! What a run gets for each record it receives: the script's event, with the run's own
//! tab ids. A new tab's target id is mapped through the run's view, so a run only ever sees
//! tabs its view admits.

use std::time::Duration;

use browseros_core::{FrameId, TabId, TargetId};
use serde::Serialize;
use ts_rs::TS;

use crate::events::event_hub::EventRecord;
use crate::events::file_chooser::FileChooser;
use crate::events::frame_lifecycle::{EventFrame, FrameLifecycle};
use crate::events::page_event::{
    DialogId, DialogKind, DialogOpened, Location, OpenedPage, PageEvent, WindowKind,
};
use crate::events::run_events::QueuedCall;
use crate::events::{PageEventKind, Subscription};
use crate::op_scope::OpScope;
use crate::page::PageHandle;
use crate::page::serialized_value::SerializedValue;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// How many times a read lists the tabs again for a new tab the view does not list yet.
const LISTING_RETRIES: u32 = 5;
const LISTING_WAIT: Duration = Duration::from_millis(100);

/// One event as `events.wait` and `events.next` return it:
/// `{ "seq": 12, "at": 1700000000123, "event": "popup", "page": .., .. }`. `event` is
/// Playwright's name and `page` the page it fires on; the context's `page` event names the new
/// tab there. `seq` is the record's place in the log; a `waitForEvent` whose predicate rejects
/// the event asks again with `after` set to it.
///
/// The log is read once per listener, from that listener's own position, so one opened tab
/// reaches the opener's `popup` listener and the context's `page` listener as two events.
#[derive(Debug, Serialize, TS)]
#[ts(rename = "EventRecord")]
pub(crate) struct Delivered {
    pub(crate) seq: u64,
    /// When the watcher recorded the event, not when the page acted: ms since the epoch on the
    /// clock that stamps takeover lock times (`EpochMillis::now`, as `neo_takeover::Locks`'
    /// `created_at` and `ended.at`). claw-sdk's door compares `at` with those to withhold what a
    /// page did while its owner held the tab, and the prelude drops a context-level event
    /// recorded before the context's listener attached.
    // The log record's `EventRecord::at_ms`, never the time a read delivered it.
    at: u64,
    // ts-rs prints the flattened tagged enum as `{ seq, at } & ({ event: 'popup', .. } | ..)`.
    #[serde(flatten)]
    payload: Payload,
}

impl Delivered {
    /// Same-record mirrors follow Playwright's emitter ownership: frames reach the page
    /// before the context, while a dialog and its closure reach the context before the page.
    pub(crate) fn order(&self) -> (u64, bool) {
        let context = match &self.payload {
            Payload::FrameAttached { context, .. }
            | Payload::FrameNavigated { context, .. }
            | Payload::FrameDetached { context, .. } => *context,
            // Playwright's BrowserContext owns dialog dispatch: it emits a dialog, and its
            // close, on itself first, then on the page.
            Payload::Dialog { context, .. } | Payload::DialogClosed { context, .. } => !context,
            _ => false,
        };
        (self.seq, context)
    }
}

/// What happened, tagged with Playwright's event name.
#[derive(Debug, Serialize, TS)]
#[serde(tag = "event")]
#[ts(rename = "EventPayload")]
enum Payload {
    #[serde(rename = "workerstate")]
    WorkerState {
        page: PageHandle,
        worker: super::worker_table::WorkerInfo,
        closed: bool,
    },
    #[serde(rename = "worker")]
    Worker {
        page: PageHandle,
        worker: super::worker_table::WorkerInfo,
    },
    #[serde(rename = "workerclosed")]
    WorkerClosed {
        page: PageHandle,
        worker: super::worker_table::WorkerInfo,
    },
    /// On the opener's page: the tab it opened.
    #[serde(rename = "popup")]
    Popup { page: PageHandle, popup: NewTab },
    /// On the context: a new tab and its opener, if that is open.
    #[serde(rename = "page")]
    Page {
        page: PageHandle,
        url: String,
        opener: Option<PageHandle>,
    },
    #[serde(rename = "close")]
    Close { page: PageHandle },
    #[serde(rename = "crash")]
    Crash { page: PageHandle },
    #[serde(rename = "load")]
    Load { page: PageHandle, frame: EventFrame },
    #[serde(rename = "domcontentloaded")]
    DomContentLoaded { page: PageHandle, frame: EventFrame },
    #[serde(rename = "pageload")]
    PageLoad { page: PageHandle, frame: EventFrame },
    #[serde(rename = "frameattached")]
    FrameAttached {
        page: PageHandle,
        frame: EventFrame,
        context: bool,
    },
    #[serde(rename = "framenavigated")]
    FrameNavigated {
        page: PageHandle,
        frame: EventFrame,
        context: bool,
    },
    #[serde(rename = "framedetached")]
    FrameDetached {
        page: PageHandle,
        frame: EventFrame,
        context: bool,
    },
    /// `context` marks the copy for the context's listeners, as on `dialogclosed`.
    #[serde(rename = "dialog")]
    Dialog {
        page: PageHandle,
        dialog: Dialog,
        context: bool,
    },
    #[serde(rename = "dialogclosed")]
    DialogClosed {
        page: PageHandle,
        dialog: Dialog,
        context: bool,
    },
    #[serde(rename = "console")]
    Console {
        page: PageHandle,
        message: ConsoleMessage,
    },
    #[serde(rename = "pageerror")]
    PageError { page: PageHandle, error: PageError },
    #[serde(rename = "download")]
    Download {
        page: PageHandle,
        download: Download,
    },
    #[serde(rename = "filechooser")]
    FileChooser {
        page: PageHandle,
        chooser: FileChooser,
    },
    /// A call of a binding the run exposed: the stub's JSON `payload` (`{ name, seq,
    /// serializedArgs }`), the execution context to answer it in (`bindingCall.resolve`), and
    /// that context's frame, `null` for the main frame or a frame the watcher did not see made.
    /// Named `executionContext`, not `context`, which the prelude reads as a context-level
    /// event's flag.
    #[serde(rename = "binding", rename_all = "camelCase")]
    Binding {
        page: PageHandle,
        frame: Option<FrameId>,
        execution_context: i64,
        payload: String,
    },
}

/// The tab a `popup` opened, with the URL the prelude caches for its synchronous `url()`.
#[derive(Debug, Serialize, TS)]
#[ts(rename = "PopupTab")]
struct NewTab {
    page: PageHandle,
    url: String,
}

/// Playwright's `Dialog`; `id` is what `dialog.accept` and `dialog.dismiss` name.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "DialogInfo")]
struct Dialog {
    id: DialogId,
    #[serde(rename = "type")]
    kind: DialogKind,
    message: String,
    default_value: String,
}

/// Playwright's `ConsoleMessage`: `type()`, `text()`, `args()`, `location()`, `timestamp()`.
#[derive(Debug, Serialize, TS)]
#[ts(rename = "ConsoleInfo")]
struct ConsoleMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    worker: Option<super::worker_table::WorkerInfo>,
    #[serde(rename = "type")]
    kind: String,
    text: String,
    args: Vec<ConsoleArgument>,
    location: Location,
    timestamp: f64,
}

/// One of `args()`: the argument's value when the console call's report held all of it
/// (`page_event::ConsoleArg`), and what it prints as, its `toString()`.
#[derive(Debug, Serialize, TS)]
#[ts(rename = "ConsoleArgInfo")]
struct ConsoleArgument {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    value: Option<SerializedValue>,
    preview: String,
}

/// The `Error` Playwright's `pageerror` passes.
#[derive(Debug, Serialize, TS)]
#[ts(rename = "PageErrorInfo")]
struct PageError {
    name: String,
    message: String,
    stack: String,
}

/// Playwright's `Download`: `url()` and `suggestedFilename()`. `id` is the browser's download
/// guid, which `download.path` and `download.saveAs` name.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "DownloadInfo")]
struct Download {
    id: String,
    url: String,
    suggested_filename: String,
}

impl Delivered {
    /// The browser's guid of the download this event hands the script, if it is a `download`.
    pub(crate) fn download(&self) -> Option<&str> {
        match &self.payload {
            Payload::Download { download, .. } => Some(&download.id),
            _ => None,
        }
    }
}

/// What a read does with a popup that opened as a separate window (D9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowPopup {
    /// `waitForEvent('popup')` fails `neo / unsupported`, rather than wait for a tab.
    Fail,
    /// The pump leaves it out: the run's view would not admit it as a tab.
    Skip,
}

/// The run's tabs, listed through its view at most once per read, and again for a tab that
/// just opened.
#[derive(Debug, Default)]
pub(crate) struct Tabs {
    listed: Option<Vec<(TargetId, PageHandle)>>,
}

impl Tabs {
    /// The handle of `target` if the run's view admits it, waiting briefly for a tab that
    /// opened a moment ago and is not listed yet.
    async fn new_tab(
        &mut self,
        scope: &OpScope<'_>,
        target: &TargetId,
    ) -> Result<Option<PageHandle>, PlaywrightError> {
        self.opened_tab(scope, target, || None).await
    }

    /// [`Self::new_tab`] for a tab the watcher recorded opening. The listing names live tabs
    /// only, so one that closed before it named it (`window.open()` then `close()` in one task)
    /// is handed out by `closed()`, its native id once the log holds its close: upstream emits
    /// such a page, already closed. The reply's page facts then report it closed to the
    /// script, as for any tab core no longer has (neo-server's `PageFact::new`).
    async fn opened_tab(
        &mut self,
        scope: &OpScope<'_>,
        target: &TargetId,
        closed: impl Fn() -> Option<TabId>,
    ) -> Result<Option<PageHandle>, PlaywrightError> {
        for retry in 0..=LISTING_RETRIES {
            if let Some(page) = self.known(scope, target).await? {
                return Ok(Some(page));
            }
            if let Some(tab) = closed() {
                scope.run_state().events.remember_page(target, tab);
                return Ok(Some(PageHandle::new(tab)));
            }
            if retry == LISTING_RETRIES || scope.sleep(LISTING_WAIT).await.is_err() {
                break;
            }
            self.listed = None;
        }
        Ok(None)
    }

    /// The handle of `target` in the current listing.
    async fn known(
        &mut self,
        scope: &OpScope<'_>,
        target: &TargetId,
    ) -> Result<Option<PageHandle>, PlaywrightError> {
        if self.listed.is_none() {
            let listing = scope
                .view()
                .list_pages()
                .await
                .map_err(|error| scope.core_error(&error))?;
            self.listed = Some(
                listing
                    .pages
                    .into_iter()
                    .map(|page| (page.target_id, PageHandle::new(page.tab_id)))
                    .collect(),
            );
        }
        let known = self.listed.as_ref().and_then(|listed| {
            listed
                .iter()
                .find(|(listed, _)| listed == target)
                .map(|(_, page)| page.clone())
        });
        if let Some(page) = &known {
            scope.run_state().events.remember_page(target, page.tab());
        }
        Ok(known)
    }
}

/// The script's event for `record`, which `subscription` receives, or `None` when the run's
/// view does not admit the tab it names.
pub(crate) async fn deliver(
    pw: &Playwright,
    scope: &OpScope<'_>,
    subscription: &Subscription,
    record: &EventRecord,
    tabs: &mut Tabs,
    windows: WindowPopup,
) -> Result<Option<Delivered>, PlaywrightError> {
    let listener = match subscription {
        Subscription::Page { tab, .. } => Some(PageHandle::new(*tab)),
        Subscription::ContextPage => None,
        Subscription::Context(PageEventKind::Dialog | PageEventKind::DialogClosed) => {
            let (PageEvent::Dialog(dialog) | PageEvent::DialogClosed(dialog)) = &record.event
            else {
                return Ok(None);
            };
            // A dialog's closing can be delivered after its page closed. Never require a
            // live tab to recover an identity this run already approved while it existed.
            // A dialog can open in a tab no op admitted yet (a popup's `alert`): the listing
            // names it, as it names the context's new pages.
            if let Some(tab) = scope.run_state().events.known_page(&dialog.target) {
                Some(PageHandle::new(tab))
            } else {
                tabs.new_tab(scope, &dialog.target).await?
            }
        }
        Subscription::Context(_) => {
            let target = match &record.event {
                PageEvent::FrameLifecycle(frame) => &frame.target,
                PageEvent::Load { target, .. } => target,
                PageEvent::DialogClosed(dialog) => &dialog.target,
                _ => return Ok(None),
            };
            tabs.new_tab(scope, target).await?
        }
    };
    let context = matches!(subscription, Subscription::Context(_));
    report_navigation(scope, listener.as_ref(), &record.event);
    let payload = if let (
        Subscription::Page {
            event: PageEventKind::WorkerState,
            tab,
            ..
        },
        PageEvent::Worker { worker, closed, .. },
    ) = (subscription, &record.event)
    {
        Payload::WorkerState {
            page: PageHandle::new(*tab),
            worker: worker.clone(),
            closed: *closed,
        }
    } else if let Some(payload) = lifecycle_payload(&record.event, listener.clone(), context) {
        payload
    } else if let Some(payload) = ordinary_payload(
        pw,
        scope,
        &record.event,
        record.seq,
        listener,
        tabs,
        windows,
    )
    .await?
    {
        payload
    } else {
        return Ok(None);
    };
    Ok(Some(Delivered {
        seq: record.seq,
        at: record.at_ms,
        payload,
    }))
}

/// Public lifecycle values need no browser read: their metadata belongs to the event.
fn lifecycle_payload(
    event: &PageEvent,
    listener: Option<PageHandle>,
    context: bool,
) -> Option<Payload> {
    let payload = match (event, listener) {
        (PageEvent::Dialog(dialog), Some(page)) => Payload::Dialog {
            page,
            dialog: dialog_info(dialog),
            context,
        },
        (PageEvent::DialogClosed(dialog), Some(page)) => Payload::DialogClosed {
            page,
            dialog: dialog_info(dialog),
            context,
        },
        (PageEvent::FrameLifecycle(FrameLifecycle { kind, frame, .. }), Some(page)) => {
            let frame = frame.clone();
            match kind {
                PageEventKind::FrameAttached => Payload::FrameAttached {
                    page,
                    frame,
                    context,
                },
                PageEventKind::FrameNavigated => Payload::FrameNavigated {
                    page,
                    frame,
                    context,
                },
                PageEventKind::FrameDetached => Payload::FrameDetached {
                    page,
                    frame,
                    context,
                },
                _ => return None,
            }
        }
        (
            PageEvent::Load {
                kind: PageEventKind::Load,
                frame,
                ..
            },
            Some(page),
        ) => {
            if context {
                Payload::PageLoad {
                    page,
                    frame: frame.clone(),
                }
            } else {
                Payload::Load {
                    page,
                    frame: frame.clone(),
                }
            }
        }
        (
            PageEvent::Load {
                kind: PageEventKind::DomContentLoaded,
                frame,
                ..
            },
            Some(page),
        ) => Payload::DomContentLoaded {
            page,
            frame: frame.clone(),
        },
        _ => return None,
    };
    Some(payload)
}

/// Non-lifecycle values may adopt a new tab or a native file chooser through the run's view.
async fn ordinary_payload(
    pw: &Playwright,
    scope: &OpScope<'_>,
    event: &PageEvent,
    seq: u64,
    listener: Option<PageHandle>,
    tabs: &mut Tabs,
    windows: WindowPopup,
) -> Result<Option<Payload>, PlaywrightError> {
    let payload = match (event, listener) {
        (PageEvent::Opened(opened), Some(page)) => {
            let Some(popup) = popup_tab(pw, scope, opened, tabs, windows).await? else {
                return Ok(None);
            };
            let popup = NewTab {
                page: popup,
                url: opened.url.clone(),
            };
            Payload::Popup { page, popup }
        }
        (PageEvent::Opened(opened), None) => {
            if opened.window == WindowKind::Window {
                return Ok(None);
            }
            let opener = match &opened.opener {
                Some(opener) => tabs.known(scope, opener).await?,
                None => None,
            };
            // A closed tab the listing cannot vouch for reaches the context only as a popup of
            // a page the view admits, never as some other window's tab.
            let closed = closed_tab(pw, opened);
            let Some(page) = tabs
                .opened_tab(scope, &opened.target, || {
                    opener.as_ref().and_then(|_| closed())
                })
                .await?
            else {
                return Ok(None);
            };
            Payload::Page {
                page,
                url: opened.url.clone(),
                opener,
            }
        }
        (PageEvent::Closed(target), Some(page)) => {
            scope.report_close(page.tab(), target, seq);
            Payload::Close { page }
        }
        (PageEvent::FileChooser(opened), Some(page)) => {
            // World installation is a large future; keep it off every event reader's stack.
            let Some(chooser) = Box::pin(FileChooser::adopt(pw, scope, &page, opened)).await?
            else {
                return Ok(None);
            };
            Payload::FileChooser { page, chooser }
        }
        (event, Some(page)) => match plain(event, page) {
            Some(payload) => payload,
            None => return Ok(None),
        },
        // The context receives only new tabs.
        (_, None) => return Ok(None),
    };
    Ok(Some(payload))
}

/// A queued binding call as the pump hands it to the prelude.
pub(crate) fn binding_call(queued: QueuedCall) -> Delivered {
    let QueuedCall {
        seq,
        at_ms,
        tab,
        call,
    } = queued;
    Delivered {
        seq,
        at: at_ms,
        payload: Payload::Binding {
            page: PageHandle::new(tab),
            frame: call.frame,
            execution_context: call.context,
            payload: call.payload,
        },
    }
}

/// The payload of an event a page listener receives as recorded, with nothing to look up:
/// `None` for the internal navigation facts, and for the events [`deliver`] builds itself
/// (lifecycles, new tabs, closes, choosers) or drops.
fn plain(event: &PageEvent, page: PageHandle) -> Option<Payload> {
    Some(match event {
        PageEvent::Worker { worker, closed, .. } => {
            if *closed {
                Payload::WorkerClosed {
                    page,
                    worker: worker.clone(),
                }
            } else {
                Payload::Worker {
                    page,
                    worker: worker.clone(),
                }
            }
        }
        PageEvent::Crashed(_) => Payload::Crash { page },
        PageEvent::Console(message) => Payload::Console {
            page,
            message: ConsoleMessage {
                worker: message.worker.clone(),
                kind: message.kind.clone(),
                text: message.text.clone(),
                args: message
                    .args
                    .iter()
                    .map(|arg| ConsoleArgument {
                        value: arg.value.clone().map(SerializedValue),
                        preview: arg.preview.clone(),
                    })
                    .collect(),
                location: message.location.clone(),
                timestamp: message.timestamp,
            },
        },
        PageEvent::PageError(error) => Payload::PageError {
            page,
            error: PageError {
                name: error.name.clone(),
                message: error.message.clone(),
                stack: error.stack.clone(),
            },
        },
        PageEvent::Download(download) => Payload::Download {
            page,
            download: Download {
                id: download.guid.clone(),
                url: download.url.clone(),
                suggested_filename: download.suggested_filename.clone(),
            },
        },
        PageEvent::Navigation(_)
        | PageEvent::FrameDetached(_)
        | PageEvent::FrameSwapped(_)
        | PageEvent::FrameLifecycle(_)
        | PageEvent::Dialog(_)
        | PageEvent::DialogClosed(_)
        | PageEvent::Load { .. }
        | PageEvent::Opened(_)
        | PageEvent::Closed(_)
        | PageEvent::FileChooser(_) => return None,
    })
}

/// Opening and closing share one protocol representation and one client Dialog identity.
fn dialog_info(dialog: &DialogOpened) -> Dialog {
    Dialog {
        id: dialog.id,
        kind: dialog.kind,
        message: dialog.message.clone(),
        default_value: dialog.default_value.clone(),
    }
}

/// A console/dialog/etc. can follow renderer-initiated navigation. Delivery synchronizes
/// `page.url()` even when no SDK navigation ran, while a close carries its own terminal facts
/// and a crashed renderer navigated nowhere.
fn report_navigation(scope: &OpScope<'_>, page: Option<&PageHandle>, event: &PageEvent) {
    if let Some(page) = page
        // Lifecycle records already carry their event-time state. A fresh browser read here
        // both overwrites it and can delay a navigation event past the action that caused it.
        && !matches!(
            event,
            PageEvent::Closed(_)
                | PageEvent::Crashed(_)
                | PageEvent::FrameLifecycle(_)
                | PageEvent::Load { .. }
        )
    {
        scope.report_navigation(page.tab());
    }
}

/// The popup's page, or what the read does with a window.
async fn popup_tab(
    pw: &Playwright,
    scope: &OpScope<'_>,
    opened: &OpenedPage,
    tabs: &mut Tabs,
    windows: WindowPopup,
) -> Result<Option<PageHandle>, PlaywrightError> {
    if opened.window == WindowKind::Window {
        return match windows {
            WindowPopup::Fail => Err(scope.fail(
                ErrorCode::Unsupported,
                &format!(
                    "the popup ({}) opened as a separate window, which Neo cannot drive; Neo \
                     drives popups that open as tabs",
                    opened.url
                ),
            )),
            WindowPopup::Skip => Ok(None),
        };
    }
    tabs.opened_tab(scope, &opened.target, closed_tab(pw, opened))
        .await
}

/// The native tab of `opened` once the log holds its close (target ids are never reused). Only
/// for a popup whose opener the run's view admits: the popup listener's own page, or one the
/// context path found in the listing.
fn closed_tab<'a>(pw: &'a Playwright, opened: &'a OpenedPage) -> impl Fn() -> Option<TabId> + 'a {
    move || {
        opened
            .tab
            .filter(|_| pw.events().closed_at(&opened.target).is_some())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::{TabId, TargetId};
    use serde_json::json;

    use super::{
        ConsoleArgument, ConsoleMessage, Delivered, Dialog, Download, NewTab, Payload, Tabs,
        WindowPopup,
    };
    use crate::Playwright;
    use crate::events::event_hub::EventRecord;
    use crate::events::page_event::{DialogId, DialogKind, DialogOpened, Location, PageEvent};
    use crate::events::{PageEventKind, Subscription};
    use crate::op::Op;
    use crate::op_scope::OpScope;
    use crate::page::PageHandle;
    use crate::page::serialized_value::SerializedValue;
    use crate::target::CallHeader;
    use crate::test_env::{self, EnvCall, TestEnv};

    /// Page `id` on tab `id * 100`.
    fn handle(id: u32) -> PageHandle {
        PageHandle::new(TabId(i64::from(id) * 100))
    }

    /// The prelude's `EventRecord` (wire.ts) reads these exact shapes.
    #[test]
    fn tests_that_events_serialize_as_the_prelude_reads_them() -> Result<(), serde_json::Error> {
        let popup = Delivered {
            seq: 8,
            at: 1_700_000_000_008,
            payload: Payload::Popup {
                page: handle(7),
                popup: NewTab {
                    page: handle(30),
                    url: "https://login.test/".to_owned(),
                },
            },
        };
        assert_eq!(
            serde_json::to_value(&popup)?,
            json!({ "seq": 8, "at": 1_700_000_000_008_u64, "event": "popup", "page": { "$neo": "page", "tab": 700 },
                    "popup": { "page": { "$neo": "page", "tab": 3000 }, "url": "https://login.test/" } })
        );
        let tab = Delivered {
            seq: 8,
            at: 1_700_000_000_008,
            payload: Payload::Page {
                page: handle(30),
                url: "https://login.test/".to_owned(),
                opener: Some(handle(7)),
            },
        };
        assert_eq!(
            serde_json::to_value(&tab)?,
            json!({ "seq": 8, "at": 1_700_000_000_008_u64, "event": "page", "page": { "$neo": "page", "tab": 3000 },
                    "url": "https://login.test/", "opener": { "$neo": "page", "tab": 700 } })
        );
        let console = Delivered {
            seq: 9,
            at: 1_700_000_000_009,
            payload: Payload::Console {
                page: handle(7),
                message: ConsoleMessage {
                    worker: None,
                    kind: "log".to_owned(),
                    text: "hi".to_owned(),
                    args: vec![ConsoleArgument {
                        value: Some(SerializedValue(json!("hi"))),
                        preview: "hi".to_owned(),
                    }],
                    location: Location::default(),
                    timestamp: 5.0,
                },
            },
        };
        assert_eq!(
            serde_json::to_value(&console)?,
            json!({ "seq": 9, "at": 1_700_000_000_009_u64, "event": "console", "page": { "$neo": "page", "tab": 700 },
                    "message": { "type": "log", "text": "hi", "timestamp": 5.0,
                                 "args": [{ "value": "hi", "preview": "hi" }],
                                 "location": { "url": "", "lineNumber": 0, "columnNumber": 0 } } })
        );
        Ok(())
    }

    /// claw-sdk's door withholds an event by comparing its `at` with takeover lock times, so
    /// `at` is the record's own stamp, not the time it was read.
    #[tokio::test]
    async fn tests_that_a_delivered_event_carries_its_record_time() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let op = Op::from_name("events.wait").ok_or("events.wait is an op")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        let target = TargetId("T".to_owned());
        let subscription = Subscription::Page {
            tab: TabId(700),
            target: target.clone(),
            event: PageEventKind::Close,
        };
        let record = EventRecord {
            seq: 3,
            at_ms: 1_700_000_000_123,
            event: PageEvent::Closed(target.clone()),
        };
        let delivered = super::deliver(
            &Playwright::new(test_env::offline_session()),
            &scope,
            &subscription,
            &record,
            &mut Tabs::default(),
            WindowPopup::Fail,
        )
        .await?
        .ok_or("a page's own close reaches its listener")?;
        assert!(
            env.calls()
                .contains(&EnvCall::ReportClose(TabId(700), target, 3))
        );
        assert_eq!(
            serde_json::to_value(&delivered)?,
            json!({ "seq": 3, "at": 1_700_000_000_123_u64, "event": "close",
                    "page": { "$neo": "page", "tab": 700 } })
        );
        Ok(())
    }

    /// `context.waitForEvent('dialog')` gets a dialog of any tab the run's view admits, on the
    /// page it opened in (upstream's `dialog.page()`), marked for the context and ordered
    /// before the page's copy of the same record; `page.on('crash')` gets its page's crash,
    /// which leaves the tab open.
    #[tokio::test]
    async fn tests_that_the_context_receives_a_dialog_and_a_page_its_crash()
    -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let op = Op::from_name("events.wait").ok_or("events.wait is an op")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        let pw = Playwright::new(test_env::offline_session());
        let popup = TargetId("P".to_owned());
        scope.run_state().events.remember_page(&popup, TabId(900));
        let opened = EventRecord {
            seq: 4,
            at_ms: 1_700_000_000_004,
            event: PageEvent::Dialog(DialogOpened {
                id: DialogId(5),
                target: popup.clone(),
                kind: DialogKind::Alert,
                message: "hello".to_owned(),
                default_value: String::new(),
            }),
        };
        let context = super::deliver(
            &pw,
            &scope,
            &Subscription::Context(PageEventKind::Dialog),
            &opened,
            &mut Tabs::default(),
            WindowPopup::Fail,
        )
        .await?
        .ok_or("the context receives a dialog of any admitted tab")?;
        assert_eq!(
            serde_json::to_value(&context)?,
            json!({ "seq": 4, "at": 1_700_000_000_004_u64, "event": "dialog",
                    "page": { "$neo": "page", "tab": 900 },
                    "dialog": { "id": 5, "type": "alert", "message": "hello",
                                "defaultValue": "" },
                    "context": true })
        );
        let page = super::deliver(
            &pw,
            &scope,
            &Subscription::Page {
                tab: TabId(900),
                target: popup.clone(),
                event: PageEventKind::Dialog,
            },
            &opened,
            &mut Tabs::default(),
            WindowPopup::Fail,
        )
        .await?
        .ok_or("the page receives its own dialog")?;
        assert!(context.order() < page.order(), "the context's copy first");

        let crashed = EventRecord {
            seq: 6,
            at_ms: 1_700_000_000_006,
            event: PageEvent::Crashed(popup.clone()),
        };
        let crash = super::deliver(
            &pw,
            &scope,
            &Subscription::Page {
                tab: TabId(900),
                target: popup,
                event: PageEventKind::Crash,
            },
            &crashed,
            &mut Tabs::default(),
            WindowPopup::Fail,
        )
        .await?
        .ok_or("a page's crash reaches its listener")?;
        assert_eq!(
            serde_json::to_value(&crash)?,
            json!({ "seq": 6, "at": 1_700_000_000_006_u64, "event": "crash",
                    "page": { "$neo": "page", "tab": 900 } })
        );
        assert!(
            env.calls().is_empty(),
            "a crash does not report its tab closed: {:?}",
            env.calls()
        );
        Ok(())
    }

    /// The payloads the earlier test leaves out, with the shapes generated/event.ts declares:
    /// a dialog's typed `type`, a download, and a binding call's camelCase fields.
    #[test]
    fn tests_that_dialog_download_and_binding_records_keep_their_bytes()
    -> Result<(), serde_json::Error> {
        let dialog = Delivered {
            seq: 3,
            at: 1_700_000_000_003,
            payload: Payload::Dialog {
                page: handle(7),
                dialog: Dialog {
                    id: DialogId(41),
                    kind: DialogKind::BeforeUnload,
                    message: "Leave?".to_owned(),
                    default_value: String::new(),
                },
                context: false,
            },
        };
        assert_eq!(
            serde_json::to_value(&dialog)?,
            json!({ "seq": 3, "at": 1_700_000_000_003_u64, "event": "dialog",
                    "page": { "$neo": "page", "tab": 700 },
                    "dialog": { "id": 41, "type": "beforeunload", "message": "Leave?",
                                "defaultValue": "" },
                    "context": false })
        );
        let download = Delivered {
            seq: 4,
            at: 1_700_000_000_004,
            payload: Payload::Download {
                page: handle(7),
                download: Download {
                    id: "guid-5".to_owned(),
                    url: "https://files.test/f.csv".to_owned(),
                    suggested_filename: "f.csv".to_owned(),
                },
            },
        };
        assert_eq!(
            serde_json::to_value(&download)?,
            json!({ "seq": 4, "at": 1_700_000_000_004_u64, "event": "download",
                    "page": { "$neo": "page", "tab": 700 },
                    "download": { "id": "guid-5", "url": "https://files.test/f.csv",
                                  "suggestedFilename": "f.csv" } })
        );
        let binding = Delivered {
            seq: 5,
            at: 1_700_000_000_005,
            payload: Payload::Binding {
                page: handle(7),
                frame: None,
                execution_context: 12,
                payload: "{}".to_owned(),
            },
        };
        assert_eq!(
            serde_json::to_value(&binding)?,
            json!({ "seq": 5, "at": 1_700_000_000_005_u64, "event": "binding",
                    "page": { "$neo": "page", "tab": 700 }, "frame": null,
                    "executionContext": 12, "payload": "{}" })
        );
        Ok(())
    }
}
