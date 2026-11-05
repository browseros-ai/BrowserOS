//! What a run gets for each record it receives: the script's event, with the run's own
//! `PageId`s. A new tab's target id is mapped through the run's view, so a run only ever sees
//! tabs its view admits.

use std::time::Duration;

use browseros_core::TargetId;
use serde::Serialize;

use crate::events::Subscription;
use crate::events::event_hub::EventRecord;
use crate::events::page_event::{DialogId, Location, OpenedPage, PageEvent, WindowKind};
use crate::op_scope::OpScope;
use crate::page::PageHandle;
use crate::{ErrorCode, PlaywrightError};

/// How many times a read lists the tabs again for a new tab the view does not list yet.
const LISTING_RETRIES: u32 = 5;
const LISTING_WAIT: Duration = Duration::from_millis(100);

/// One event as the prelude receives it: `{ "seq": 12, "event": "popup", "page": .., .. }`.
/// `seq` is the record's place in the log; a `waitForEvent` whose predicate rejects the event
/// asks again with `after` set to it.
#[derive(Debug, Serialize)]
pub(crate) struct Delivered {
    pub(crate) seq: u64,
    #[serde(flatten)]
    payload: Payload,
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
                    .map(|page| (page.target_id, PageHandle::new(page.page_id, page.tab_id)))
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
    scope: &OpScope<'_>,
    subscription: &Subscription,
    record: &EventRecord,
    tabs: &mut Tabs,
    windows: WindowPopup,
) -> Result<Option<Delivered>, PlaywrightError> {
    let listener = match subscription {
        Subscription::Page { page, tab, .. } => Some(PageHandle::new(page.clone(), *tab)),
        Subscription::ContextPage => None,
    };
    let payload = match (&record.event, listener) {
        (PageEvent::Opened(opened), Some(page)) => {
            let Some(popup) = popup_tab(scope, opened, tabs, windows).await? else {
                return Ok(None);
            };
            Payload::Popup {
                page,
                popup: NewTab {
                    page: popup,
                    url: opened.url.clone(),
                },
            }
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
        (PageEvent::Closed(_), Some(page)) => Payload::Close { page },
        (PageEvent::Dialog(dialog), Some(page)) => Payload::Dialog {
            page,
            dialog: Dialog {
                id: dialog.id,
                kind: dialog.kind.as_str(),
                message: dialog.message.clone(),
                default_value: dialog.default_value.clone(),
            },
        },
        (PageEvent::Console(message), Some(page)) => Payload::Console {
            page,
            message: ConsoleMessage {
                kind: message.kind.clone(),
                text: message.text.clone(),
                location: message.location.clone(),
                timestamp: message.timestamp,
            },
        },
        (PageEvent::PageError(error), Some(page)) => Payload::PageError {
            page,
            error: PageError {
                name: error.name.clone(),
                message: error.message.clone(),
                stack: error.stack.clone(),
            },
        },
        (PageEvent::Download(download), Some(page)) => Payload::Download {
            page,
            download: Download {
                id: download.guid.clone(),
                url: download.url.clone(),
                suggested_filename: download.suggested_filename.clone(),
            },
        },
        // The context receives only new tabs.
        (
            PageEvent::Closed(_)
            | PageEvent::Dialog(_)
            | PageEvent::Console(_)
            | PageEvent::PageError(_)
            | PageEvent::Download(_),
            None,
        ) => return Ok(None),
    };
    Ok(Some(Delivered {
        seq: record.seq,
        payload,
    }))
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
    use browseros_core::{PageId, TabId};
    use serde_json::json;

    use super::{ConsoleMessage, Delivered, NewTab, Payload};
    use crate::events::page_event::Location;
    use crate::page::PageHandle;

    /// Page `id` on tab `id * 100`.
    fn handle(id: u32) -> PageHandle {
        PageHandle::new(PageId(id), TabId(i64::from(id) * 100))
    }

    /// The prelude's `EventRecord` (wire.ts) reads these exact shapes.
    #[test]
    fn tests_that_events_serialize_as_the_prelude_reads_them() -> Result<(), serde_json::Error> {
        let popup = Delivered {
            seq: 8,
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
            json!({ "seq": 8, "event": "popup", "page": { "$neo": "page", "id": 7, "tab": 700 },
                    "popup": { "page": { "$neo": "page", "id": 30, "tab": 3000 }, "url": "https://login.test/" } })
        );
        let tab = Delivered {
            seq: 8,
            payload: Payload::Page {
                page: handle(30),
                url: "https://login.test/".to_owned(),
                opener: Some(handle(7)),
            },
        };
        assert_eq!(
            serde_json::to_value(&tab)?,
            json!({ "seq": 8, "event": "page", "page": { "$neo": "page", "id": 30, "tab": 3000 },
                    "url": "https://login.test/", "opener": { "$neo": "page", "id": 7, "tab": 700 } })
        );
        let console = Delivered {
            seq: 9,
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
            json!({ "seq": 9, "event": "console", "page": { "$neo": "page", "id": 7, "tab": 700 },
                    "message": { "type": "log", "text": "hi", "timestamp": 5.0,
                                 "location": { "url": "", "lineNumber": 0, "columnNumber": 0 } } })
        );
        Ok(())
    }
}
