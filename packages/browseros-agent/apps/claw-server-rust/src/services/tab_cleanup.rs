//! Reliable cleanup of finished-agent browser tabs.
//!
//! Releasing a session's tab ownership stamps the ledger but never closes the
//! Chrome tab. This reconciliation sweep closes any tab that was agent-owned but
//! now has no live owner, once a short grace has elapsed, sparing the tab the
//! user is currently viewing. It is idempotent and only acts on tabs the browser
//! still reports open, so it survives crashes, disconnects, ungrouped tabs, and
//! popup children.

use crate::{db::SessionTabLedger, error::AppResult, services::browser::BrowserService};
use browseros_core::PageId;
use std::{collections::HashMap, time::Duration};
use tracing::warn;

/// Grace after a session finishes before its still-open tabs are closed, so the
/// user can glance at the result before the tabs disappear.
const CLEANUP_GRACE: Duration = Duration::from_secs(3 * 60);

/// One tab the browser currently reports open.
struct LiveTab {
    tab_id: i64,
    is_active: bool,
}

/// The tabs to close this pass: orphaned owned tabs that are still open and not
/// the foreground tab. An active tab is spared and closed on a later pass once
/// the user moves off it; an orphaned tab the browser no longer lists is already
/// gone and needs nothing.
fn tabs_to_close(orphaned: &[i64], live: &[LiveTab]) -> Vec<i64> {
    let live_active: HashMap<i64, bool> =
        live.iter().map(|tab| (tab.tab_id, tab.is_active)).collect();
    orphaned
        .iter()
        .copied()
        .filter(|tab_id| matches!(live_active.get(tab_id), Some(false)))
        .collect()
}

/// Closes finished-agent tabs whose grace has elapsed, returning how many closed.
/// Browser errors (unreachable or a failed listing) abort the pass harmlessly;
/// the next tick retries once the browser is reachable again.
pub async fn sweep_orphaned_tabs(
    session_tabs: &SessionTabLedger,
    browser: &BrowserService,
    now: i64,
) -> AppResult<usize> {
    let cutoff = now.saturating_sub(i64::try_from(CLEANUP_GRACE.as_millis()).unwrap_or(i64::MAX));
    let orphaned = session_tabs.list_orphaned_owned_tabs(cutoff).await?;
    if orphaned.is_empty() {
        return Ok(0);
    }
    let Some(session) = browser.session().await else {
        return Ok(0);
    };
    let pages = match session.pages.list().await {
        Ok(pages) => pages,
        Err(error) => {
            warn!(error = %error, "agent tab cleanup could not list tabs");
            return Ok(0);
        }
    };
    let live: Vec<LiveTab> = pages
        .iter()
        .map(|page| LiveTab {
            tab_id: page.tab_id.0,
            is_active: page.is_active,
        })
        .collect();
    let page_by_tab: HashMap<i64, PageId> = pages
        .iter()
        .map(|page| (page.tab_id.0, page.page_id.clone()))
        .collect();

    // The candidate set is a snapshot; between it and the close a live session
    // could reclaim a tab or the user could bring one to the foreground. Re-check
    // both the instant before each close to shrink that window to a negligible
    // residual (CDP has no atomic close-if-unowned-and-inactive). The freshest
    // foreground read is a single `get_active` taken just before the loop.
    let foreground = session
        .pages
        .get_active()
        .await
        .ok()
        .flatten()
        .map(|page| page.tab_id.0);

    let mut closed = 0;
    for tab_id in tabs_to_close(&orphaned, &live) {
        let Some(page_id) = page_by_tab.get(&tab_id) else {
            continue;
        };
        if Some(tab_id) == foreground {
            continue;
        }
        if session_tabs.open_owner_exists(tab_id).await? {
            continue;
        }
        match session.pages.close(page_id.clone()).await {
            Ok(()) => closed += 1,
            Err(error) => warn!(tab_id, error = %error, "agent tab cleanup close failed"),
        }
    }
    Ok(closed)
}

#[cfg(test)]
mod tests {
    use super::{LiveTab, tabs_to_close};

    fn live(tab_id: i64, is_active: bool) -> LiveTab {
        LiveTab { tab_id, is_active }
    }

    #[test]
    fn closes_orphaned_live_inactive_tabs_only() {
        let orphaned = [11, 12, 13, 99];
        let live = [live(11, false), live(12, true), live(13, false)];
        // 11 and 13 are orphaned, live, and inactive -> close; 12 is the
        // foreground tab -> spared; 99 is no longer listed -> already gone.
        assert_eq!(tabs_to_close(&orphaned, &live), vec![11, 13]);
    }

    #[test]
    fn spares_the_foreground_tab() {
        assert!(tabs_to_close(&[42], &[live(42, true)]).is_empty());
    }

    #[test]
    fn skips_tabs_the_browser_no_longer_lists() {
        assert!(tabs_to_close(&[7], &[live(8, false)]).is_empty());
    }
}
