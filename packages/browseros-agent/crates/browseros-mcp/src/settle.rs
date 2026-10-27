//! The settle between an MCP action and the diff it returns.
//!
//! The algorithm is core's [`wait_for_action_settle`]; this resolves the page through the
//! request's captured view first, so profile, pin, live context and target-incarnation
//! admission decide which page it waits on, and gives it the action budget.

use browseros_core::{
    BrowserView, DocumentMark, PageId,
    settle::{SettleOutcome, wait_for_action_settle as settle_page},
    timeouts,
};
use tokio::time::timeout;

pub(crate) async fn wait_for_action_settle(
    pages: &BrowserView,
    page_id: PageId,
    mark: DocumentMark,
) -> SettleOutcome {
    let page = match timeout(
        timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT,
        pages.page_session(page_id.clone()),
    )
    .await
    {
        Ok(Ok(page)) => page,
        Ok(Err(error)) => {
            return SettleOutcome::Skipped {
                reason: error.to_string(),
            };
        }
        Err(_elapsed) => {
            return SettleOutcome::Skipped {
                reason: "the page did not answer before the settle could start".to_string(),
            };
        }
    };
    settle_page(
        &page,
        pages.signals(),
        &page_id,
        mark,
        timeouts::ACTION_SETTLE_DEFAULT_TIMEOUT,
    )
    .await
}
