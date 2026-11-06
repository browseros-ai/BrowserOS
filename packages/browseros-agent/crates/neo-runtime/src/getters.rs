//! Live reads of page state, one CDP call each and nothing cached. Each source was chosen
//! because it matches what real Playwright returns at the same moment, including while a
//! navigation is in flight (see `sync-getters-design.md`).

use browseros_cdp::{CdpClient, CdpError, SessionId, browser};
use serde::Deserialize;
use serde_json::{Value, json};

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

/// `page.url()`: the committed URL of the session's current history entry, or `None` once
/// the tab is closed. Playwright keeps the old URL until a navigation commits, and so does
/// navigation history; `Page.getFrameTree` and `location.href` block until the commit, and
/// the target and tab infos already show the destination.
pub(crate) async fn url(
    cdp: &CdpClient,
    session: &SessionId,
    page: &PageId,
) -> Result<Option<String>, CdpError> {
    let history_error = match cdp
        .send("Page.getNavigationHistory", json!({}), Some(session))
        .await
    {
        Ok(reply) => return current_url(reply).map(Some),
        Err(error) => error,
    };
    // Every source errors once the tab is gone; only the tab check can say that is why.
    if is_closed(cdp, page).await? {
        Ok(None)
    } else {
        Err(history_error)
    }
}

fn current_url(reply: Value) -> Result<String, CdpError> {
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
