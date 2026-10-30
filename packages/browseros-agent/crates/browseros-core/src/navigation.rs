use crate::{
    CoreError, PageId, PageJs,
    page_signals::{DocumentMark, DocumentSince, PageSignals},
    pages::{PageManager, PageSession},
    target::page_access::PageAccess,
    timeouts,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::time::{sleep, timeout};

/// What a `goto` learned beyond arriving: the parts of `Page.navigate`'s answer and the new
/// document that a caller would otherwise mistake for an ordinary page load.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GotoOutcome {
    /// The main document's HTTP status, when the page exposes one.
    pub status: Option<u16>,
    /// The URL answered with a file, so the browser downloaded it and the page stayed put.
    pub download: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigateReply {
    #[serde(default)]
    error_text: Option<String>,
    #[serde(default)]
    is_download: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigationHistory {
    current_index: i64,
    entries: Vec<NavigationEntry>,
}

#[derive(Deserialize)]
struct NavigationEntry {
    id: i64,
}

/// The status line of the document the frame now shows. Navigation Timing exposes it to the
/// isolated world without touching the page's own scripts.
const HTTP_STATUS_JS: &str = "(()=>{const entry=performance.getEntriesByType('navigation')[0];\
return entry&&entry.responseStatus?entry.responseStatus:null;})()";

/// Navigation retains the admitted page incarnation through load waits; it cannot reacquire a
/// different context or page when a task rebinds or the browser reconnects.
pub struct Navigation {
    pages: Arc<PageManager>,
    page_signals: Arc<PageSignals>,
    page_id: PageId,
    access: Option<Arc<PageAccess>>,
}

impl Navigation {
    #[must_use]
    pub fn new(pages: Arc<PageManager>, page_signals: Arc<PageSignals>, page_id: PageId) -> Self {
        Self {
            pages,
            page_signals,
            page_id,
            access: None,
        }
    }

    pub(crate) fn with_access(mut self, access: Arc<PageAccess>) -> Self {
        self.access = Some(access);
        self
    }
    async fn page_session(&self) -> Result<PageSession, CoreError> {
        match &self.access {
            Some(access) => access.session().await,
            None => self.pages.get_session(self.page_id.clone()).await,
        }
    }

    /// Navigates and waits for the page to arrive. A URL the browser could not load (an
    /// unknown host, a refused connection) is an error rather than an arrival at Chrome's
    /// error page, and a URL that turns out to be a file reports the download instead of
    /// waiting for a page that never comes.
    pub async fn goto(&self, url: &str) -> Result<GotoOutcome, CoreError> {
        if let Some(access) = &self.access {
            access.fresh().await?;
        }
        self.throw_if_dialog_open()?;
        let page = self.page_session().await?;
        let mark = self.page_signals.document_mark(&self.page_id);
        let reply: NavigateReply = page
            .session
            .send("Page.navigate", json!({ "url": url }))
            .await?;
        // A download is answered with errorText net::ERR_ABORTED too, so it goes first.
        if reply.is_download {
            return Ok(GotoOutcome {
                status: None,
                download: true,
            });
        }
        if let Some(error) = reply.error_text.filter(|error| !error.is_empty()) {
            return Err(CoreError::Message(format!(
                "Navigation to {url} failed: {error}"
            )));
        }
        // Chromium answers Page.navigate only once the document committed, so a navigation
        // that has not started by now was a same-document one.
        wait_for_load(
            &page,
            &self.page_signals,
            &self.page_id,
            self.access.as_deref(),
            Arrival {
                mark,
                expects_new_document: false,
            },
        )
        .await?;
        let status = timeout(
            timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT,
            PageJs::isolated(&page.session).eval::<Option<u16>>(HTTP_STATUS_JS),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .flatten();
        Ok(GotoOutcome {
            status,
            download: false,
        })
    }

    pub async fn reload(&self) -> Result<(), CoreError> {
        if let Some(access) = &self.access {
            access.fresh().await?;
        }
        self.throw_if_dialog_open()?;
        let page = self.page_session().await?;
        let mark = self.page_signals.document_mark(&self.page_id);
        let _: Value = page.session.send("Page.reload", json!({})).await?;
        wait_for_load(
            &page,
            &self.page_signals,
            &self.page_id,
            self.access.as_deref(),
            Arrival {
                mark,
                expects_new_document: true,
            },
        )
        .await
    }

    /// Goes one entry back in the tab's history. False when there is none: the page is left
    /// as it was.
    pub async fn back(&self) -> Result<bool, CoreError> {
        self.history(-1).await
    }

    /// Goes one entry forward in the tab's history. False when there is none.
    pub async fn forward(&self) -> Result<bool, CoreError> {
        self.history(1).await
    }

    /// Steps the tab's session history from the browser side. Calling `history.back()` in the
    /// page needed its main thread, so a busy page stalled it, and with no entry it did
    /// nothing while the step still reported a navigation.
    async fn history(&self, delta: i64) -> Result<bool, CoreError> {
        if let Some(access) = &self.access {
            access.fresh().await?;
        }
        self.throw_if_dialog_open()?;
        let page = self.page_session().await?;
        let history: NavigationHistory = page
            .session
            .send("Page.getNavigationHistory", json!({}))
            .await?;
        let entry = history
            .current_index
            .checked_add(delta)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| history.entries.get(index));
        let Some(entry) = entry else {
            return Ok(false);
        };
        let mark = self.page_signals.document_mark(&self.page_id);
        let _: Value = page
            .session
            .send(
                "Page.navigateToHistoryEntry",
                json!({ "entryId": entry.id }),
            )
            .await?;
        wait_for_load(
            &page,
            &self.page_signals,
            &self.page_id,
            self.access.as_deref(),
            Arrival {
                mark,
                expects_new_document: true,
            },
        )
        .await?;
        Ok(true)
    }

    fn throw_if_dialog_open(&self) -> Result<(), CoreError> {
        if let Some(message) = self.page_signals.pending_dialog_line(&self.page_id) {
            Err(CoreError::Message(message))
        } else {
            Ok(())
        }
    }
}

/// What a navigation command asked for, so the load wait can tell when it arrived.
struct Arrival {
    /// Taken before the command, so only a navigation started after it counts.
    mark: DocumentMark,
    /// Whether the command is acknowledged before its navigation starts (reload, history),
    /// so the start gets a grace period. Without one, a navigation not started by the reply
    /// is taken to have stayed in the document.
    expects_new_document: bool,
}

/// Waits until the document `arrival` asked for has reached DOMContentLoaded, or loading
/// stopped without one, or it never started within the grace. Dialogs and admission are
/// checked each time round, like the readiness loop after it.
async fn wait_for_document(
    page: &PageSession,
    page_signals: &PageSignals,
    page_id: &PageId,
    access: Option<&PageAccess>,
    arrival: Arrival,
    deadline: tokio::time::Instant,
) -> Result<(), CoreError> {
    let grace_ends = tokio::time::Instant::now() + timeouts::NAVIGATION_START_GRACE;
    let mut changes = page_signals.document_changes();
    while tokio::time::Instant::now() < deadline {
        if let Some(access) = access {
            access.fresh().await?;
        }
        if let Some(message) = page_signals.pending_dialog_line(page_id) {
            return Err(CoreError::Message(message));
        }
        let until =
            match page_signals.document_since(page_id, page.target_id.as_str(), arrival.mark) {
                DocumentSince::Arrived => return Ok(()),
                DocumentSince::Loading => deadline,
                DocumentSince::Unchanged
                    if arrival.expects_new_document && tokio::time::Instant::now() < grace_ends =>
                {
                    grace_ends
                }
                DocumentSince::Unchanged => return Ok(()),
            };
        // Re-checked at least every quiet interval, so a dialog opening is never missed.
        let until = until
            .min(deadline)
            .min(tokio::time::Instant::now() + timeouts::ACTION_SETTLE_DOM_QUIET);
        if let Ok(Err(_closed)) = tokio::time::timeout_at(until, changes.changed()).await {
            return Ok(());
        }
    }
    Ok(())
}

/// Waits for a navigation to arrive somewhere worth acting on.
///
/// This used to wait for `document.readyState` to reach `complete`, which is
/// the load event and so waits on every ad iframe, tracking pixel and lazy
/// image the destination carries. Measured on this surface, that cost 3077ms
/// and 3410ms on two ordinary navigations whose useful work then took under
/// 10ms. What a caller actually needs is the content having stopped changing,
/// and `complete` remains one of the two ways to satisfy that.
///
/// Everything else here is unchanged on purpose: the dialog check still runs
/// each time round, a target error still propagates rather than being retried,
/// and `fresh()` still admits before each shared read.
///
/// Readiness alone cannot tell the old document from the new one: a reload or a
/// history step is acknowledged before its document replaces the old one, which
/// then reads as quiet. So the asked-for document has to arrive first.
async fn wait_for_load(
    page: &PageSession,
    page_signals: &PageSignals,
    page_id: &PageId,
    access: Option<&PageAccess>,
    arrival: Arrival,
) -> Result<(), CoreError> {
    let session = &page.session;
    let deadline = tokio::time::Instant::now() + timeouts::ARRIVAL_READY_BUDGET;
    wait_for_document(page, page_signals, page_id, access, arrival, deadline).await?;
    let mut previous: Option<u64> = None;
    while tokio::time::Instant::now() < deadline {
        // The sleep is an async handoff. Admit before reading the shared dialog cache, which
        // may now belong to a tab dragged out of this capture even before the next CDP send.
        if let Some(access) = access {
            access.fresh().await?;
        }
        if let Some(message) = page_signals.pending_dialog_line(page_id) {
            return Err(CoreError::Message(message));
        }
        let result = timeout(
            timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT,
            crate::settle::read_readiness(session),
        )
        .await;
        if let Ok(Err(error @ CoreError::Target(_))) = result {
            return Err(error);
        }
        // Navigation can tear down the execution context; errors and timeouts mean not ready yet, so keep polling.
        if let Ok(Ok(reading)) = result {
            if !reading.installed && previous == Some(reading.count) && reading.settled_enough() {
                break;
            }
            previous = Some(reading.count);
        }
        sleep(timeouts::ACTION_SETTLE_DOM_QUIET).await;
    }
    if let Some(access) = access {
        access.fresh().await?;
    }
    Ok(())
}
