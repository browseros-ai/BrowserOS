//! What a run gets for each record it receives: the script's event, with the run's own
//! tab ids. A new tab's target id is mapped through the run's view, so a run only ever sees
//! tabs its view admits.

use std::time::Duration;

use browseros_core::{FrameId, TargetId};
use serde::Serialize;

use crate::events::event_hub::EventRecord;
use crate::events::file_chooser::FileChooser;
use crate::events::frame_lifecycle::{EventFrame, FrameLifecycle};
use crate::events::page_event::{DialogId, Location, OpenedPage, PageEvent, WindowKind};
use crate::events::run_events::QueuedCall;
use crate::events::{PageEventKind, Subscription};
use crate::op_scope::OpScope;
use crate::page::PageHandle;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// How many times a read lists the tabs again for a new tab the view does not list yet.
const LISTING_RETRIES: u32 = 5;
const LISTING_WAIT: Duration = Duration::from_millis(100);

/// One event as `events.wait` and `events.next` return it:
/// `{ "seq": 12, "at": 1700000000123, "event": "popup", "page": .., .. }`. `seq` is the record's
/// place in the log; a `waitForEvent` whose predicate rejects the event asks again with `after`
/// set to it.
#[derive(Debug, Serialize)]
pub(crate) struct Delivered {
    pub(crate) seq: u64,
    /// The record's [`EventRecord::at_ms`]: when the watcher recorded the event, not when the
    /// page acted, in ms since the epoch from `EpochMillis::now`, the function that stamps
    /// `neo_takeover::Locks`' `created_at` and `ended.at`. claw-sdk's door compares `at`
    /// with those to withhold what a page did while its owner held the tab.
    at: u64,
    #[serde(flatten)]
    payload: Payload,
}

impl Delivered {
    /// Page listeners precede the context mirror of the same frame transition, irrespective
    /// of the listener map's iteration order (Playwright's client emits in this order).
    pub(crate) fn order(&self) -> (u64, bool) {
        let context = matches!(
            &self.payload,
            Payload::FrameAttached { context: true, .. }
                | Payload::FrameNavigated { context: true, .. }
                | Payload::FrameDetached { context: true, .. }
        );
        (self.seq, context)
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "event")]
enum Payload {
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
    #[serde(rename = "dialog")]
    Dialog { page: PageHandle, dialog: Dialog },
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
    /// A call of a binding the run exposed: the stub's JSON `payload`, the execution context to
    /// answer it in (`bindingCall.resolve`), and that context's frame, `null` for the main frame
    /// or a frame the watcher did not see made. Named `executionContext`, not `context`, which the
    /// prelude reads as a context-level event's flag.
    #[serde(rename = "binding", rename_all = "camelCase")]
    Binding {
        page: PageHandle,
        frame: Option<FrameId>,
        execution_context: i64,
        payload: String,
    },
}

#[derive(Debug, Serialize)]
struct NewTab {
    page: PageHandle,
    url: String,
}

/// Playwright's `Dialog`; `id` is what `dialog.accept` and `dialog.dismiss` name.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Dialog {
    id: DialogId,
    #[serde(rename = "type")]
    kind: &'static str,
    message: String,
    default_value: String,
}

/// Playwright's `ConsoleMessage`: `type()`, `text()`, `location()`, `timestamp()`.
#[derive(Debug, Serialize)]
struct ConsoleMessage {
    #[serde(rename = "type")]
    kind: String,
    text: String,
    location: Location,
    timestamp: f64,
}

/// The `Error` Playwright's `pageerror` passes.
#[derive(Debug, Serialize)]
struct PageError {
    name: String,
    message: String,
    stack: String,
}

/// Playwright's `Download`: `url()` and `suggestedFilename()`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
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
        for retry in 0..=LISTING_RETRIES {
            if let Some(page) = self.known(scope, target).await? {
                return Ok(Some(page));
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
        Ok(self.listed.as_ref().and_then(|listed| {
            listed
                .iter()
                .find(|(listed, _)| listed == target)
                .map(|(_, page)| page.clone())
        }))
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
        Subscription::Context(_) => {
            let target = match &record.event {
                PageEvent::FrameLifecycle(frame) => &frame.target,
                PageEvent::Load { target, .. } => target,
                _ => return Ok(None),
            };
            tabs.new_tab(scope, target).await?
        }
    };
    let context = matches!(subscription, Subscription::Context(_));
    report_navigation(scope, listener.as_ref(), &record.event);
    let payload = if let Some(payload) = lifecycle_payload(&record.event, listener.clone(), context)
    {
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
            let Some(popup) = popup_tab(scope, opened, tabs, windows).await? else {
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
            let Some(page) = tabs.new_tab(scope, &opened.target).await? else {
                return Ok(None);
            };
            let opener = match &opened.opener {
                Some(opener) => tabs.known(scope, opener).await?,
                None => None,
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
        PageEvent::Dialog(dialog) => Payload::Dialog {
            page,
            dialog: Dialog {
                id: dialog.id,
                kind: dialog.kind.as_str(),
                message: dialog.message.clone(),
                default_value: dialog.default_value.clone(),
            },
        },
        PageEvent::Console(message) => Payload::Console {
            page,
            message: ConsoleMessage {
                kind: message.kind.clone(),
                text: message.text.clone(),
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
        | PageEvent::Load { .. }
        | PageEvent::Opened(_)
        | PageEvent::Closed(_)
        | PageEvent::FileChooser(_) => return None,
    })
}

/// A console/dialog/etc. can follow renderer-initiated navigation. Delivery synchronizes
/// `page.url()` even when no SDK navigation ran, while a close carries its own terminal facts.
fn report_navigation(scope: &OpScope<'_>, page: Option<&PageHandle>, event: &PageEvent) {
    if let Some(page) = page
        // Lifecycle records already carry their event-time state. A fresh browser read here
        // both overwrites it and can delay a navigation event past the action that caused it.
        && !matches!(event, PageEvent::Closed(_) | PageEvent::FrameLifecycle(_) | PageEvent::Load { .. })
    {
        scope.report_navigation(page.tab());
    }
}

/// The popup's page, or what the read does with a window.
async fn popup_tab(
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
    tabs.new_tab(scope, &opened.target).await
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::{TabId, TargetId};
    use serde_json::json;

    use super::{ConsoleMessage, Delivered, NewTab, Payload, Tabs, WindowPopup};
    use crate::Playwright;
    use crate::events::event_hub::EventRecord;
    use crate::events::page_event::{Location, PageEvent};
    use crate::events::{PageEventKind, Subscription};
    use crate::op::Op;
    use crate::op_scope::OpScope;
    use crate::page::PageHandle;
    use crate::target::CallHeader;
    use crate::test_env::{self, EnvCall, TestEnv};
    use crate::test_support::RecordingHooks;

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
                    kind: "log".to_owned(),
                    text: "hi".to_owned(),
                    location: Location::default(),
                    timestamp: 5.0,
                },
            },
        };
        assert_eq!(
            serde_json::to_value(&console)?,
            json!({ "seq": 9, "at": 1_700_000_000_009_u64, "event": "console", "page": { "$neo": "page", "tab": 700 },
                    "message": { "type": "log", "text": "hi", "timestamp": 5.0,
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
            &Playwright::new(test_env::offline_session(), Arc::new(RecordingHooks)),
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
}
