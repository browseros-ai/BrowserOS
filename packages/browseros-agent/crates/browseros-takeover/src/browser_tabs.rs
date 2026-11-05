//! The takeover parts' one door into Chromium, keyed by the browser's tab id.
//!
//! Every part and every host request speaks `TabId`, the key the contract and the trace records
//! use (decision 13); core speaks `PageId`, which only this process knows. This module is the
//! only place in the crate that maps one to the other, and the place later parts add their
//! browser calls: the window state, Ctrl+0, the viewport and the frames. Everything goes over
//! the agent connection's own session, the one the app's browser link owns, never a connection
//! of its own.

use crate::PartError;
use browseros_core::pages::PageInfo;
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, TabId};
use std::sync::Arc;

/// Chromium as the takeover parts reach it. Cheap to clone. The service builds one from the
/// session and viewports the app hands it and passes it to the parts.
#[derive(Clone)]
pub struct BrowserTabs {
    session: Arc<BrowserSession>,
    #[expect(
        dead_code,
        reason = "the fit (step 9a-2) reads and restores tab viewports through it"
    )]
    viewports: Arc<Viewports>,
}

impl BrowserTabs {
    /// The agent's session and the process's viewport state, both from the app's browser link.
    #[must_use]
    pub fn new(session: Arc<BrowserSession>, viewports: Arc<Viewports>) -> Self {
        Self { session, viewports }
    }

    /// The open tab's page. One `Browser.getTabs` round trip, so it also answers whether the
    /// tab is open.
    ///
    /// # Errors
    /// `TabNotFound` when no open tab has this id; `Browser` when the browser does not answer.
    pub async fn page(&self, tab: TabId) -> Result<PageInfo, PartError> {
        self.pages()
            .await?
            .into_iter()
            .find(|page| page.tab_id == tab)
            .ok_or(PartError::TabNotFound(tab))
    }

    /// Every open tab, in the browser's page order.
    ///
    /// # Errors
    /// `Browser` when the browser does not answer.
    pub async fn open_tabs(&self) -> Result<Vec<TabId>, PartError> {
        Ok(self
            .pages()
            .await?
            .into_iter()
            .map(|page| page.tab_id)
            .collect())
    }

    async fn pages(&self) -> Result<Vec<PageInfo>, PartError> {
        self.session.pages.list().await.map_err(PartError::Browser)
    }
}
