//! Translates native tab ids exposed by Playwright into host page ids.
//! Callers retain the page id only for the operation admitted against the current cache.

use crate::AppState;
use browseros_core::{PageId, TabId};

pub async fn page_for_tab(state: &AppState, tab_id: i64) -> Option<PageId> {
    let browser = state.browser.session().await?;
    browser
        .pages
        .get_info_by_tab(TabId(tab_id))
        .await
        .map(|page| page.page_id)
}
