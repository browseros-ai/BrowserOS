//! Live reads of page state, nothing cached. Each source was chosen because it matches what
//! real Playwright returns at the same moment, including while a navigation is in flight (see
//! `sync-getters-design.md`). A read is normally one CDP call. The exception is a `url` read
//! that lands in the few milliseconds of a cross-document commit, when BrowserOS refuses
//! navigation history: it checks the tab is still open and retries until the new document
//! answers.

use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError, SessionId, browser};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time;
use tokio::time::Instant;

use crate::run::PageId;

/// BrowserOS's `Browser.getTabInfo` error message for a tab that no longer exists. It is the
/// only signal that tells "closed" apart from a failed call, so `Runtime::connect` checks the
/// browser still says it before any run relies on it.
pub(crate) const CLOSED_TAB_MESSAGE: &str = "No target with given id";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigationHistory {
    current_index: usize,
    entries: Vec<NavigationEntry>,
}

#[derive(Deserialize)]
struct NavigationEntry {
    url: String,
}

/// How long `page.url()` keeps re-reading navigation history while the tab is still open. At
/// every cross-document commit BrowserOS answers that read with "Not attached to an active
/// page" for under 10 ms (1 to 8 ms, one to three calls), then with the new URL; measured on
/// BrowserOS neo 155.0.8318.26 by the navigation probes in `sync-getters-design.md`. The
/// budget is far above that window only so that a session which never recovers fails loudly
/// instead of hanging the script.
const HISTORY_RETRY_BUDGET: Duration = Duration::from_millis(250);
/// Retries wait 1, 2, 4, 8, then 16 ms, so the first ones land inside the commit window.
const MAX_RETRY_BACKOFF: Duration = Duration::from_millis(16);

/// `page.url()`: the committed URL of the session's current history entry, or `None` once
/// the tab is closed. Playwright keeps the old URL until a navigation commits, and so does
/// navigation history. `Page.getFrameTree` and `location.href` block until the commit, and
/// the target and tab infos show the destination before it, so history is the only source
/// and nothing else stands in when it fails.
///
/// A failed history read means the tab closed or a navigation is committing; the tab check
/// tells them apart without reading the error text. Closed gives `None`. Open retries within
/// [`HISTORY_RETRY_BUDGET`] and then returns the last history error. Every caller races the
/// whole call against the run's deadline, so the budget never outlives the run.
pub(crate) async fn url(
    cdp: &CdpClient,
    session: &SessionId,
    page: &PageId,
) -> Result<Option<String>, CdpError> {
    let give_up_at = Instant::now() + HISTORY_RETRY_BUDGET;
    let mut backoff = Duration::from_millis(1);
    loop {
        let history_error = match current_url(cdp, session).await {
            Ok(url) => return Ok(Some(url)),
            Err(error) => error,
        };
        if is_closed(cdp, page).await? {
            return Ok(None);
        }
        if Instant::now() + backoff >= give_up_at {
            return Err(history_error);
        }
        time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_RETRY_BACKOFF);
    }
}

async fn current_url(cdp: &CdpClient, session: &SessionId) -> Result<String, CdpError> {
    let reply = cdp
        .send("Page.getNavigationHistory", json!({}), Some(session))
        .await?;
    let history: NavigationHistory = serde_json::from_value(reply)?;
    let entry_count = history.entries.len();
    history
        .entries
        .into_iter()
        .nth(history.current_index)
        .map(|entry| entry.url)
        .ok_or_else(|| {
            CdpError::Json(format!(
                "Page.getNavigationHistory: currentIndex {} is outside its {entry_count} entries",
                history.current_index
            ))
        })
}

/// `page.isClosed()`: whether BrowserOS still has a tab for this target.
pub(crate) async fn is_closed(cdp: &CdpClient, page: &PageId) -> Result<bool, CdpError> {
    let params = browser::GetTabInfoParams {
        target_id: Some(page.as_str().to_string()),
        tab_id: None,
    };
    let reply = cdp
        .send_typed::<_, Value>("Browser.getTabInfo", params, None)
        .await;
    match reply {
        Ok(_) => Ok(false),
        Err(CdpError::Protocol { message, .. }) if message.contains(CLOSED_TAB_MESSAGE) => Ok(true),
        Err(error) => Err(error),
    }
}
