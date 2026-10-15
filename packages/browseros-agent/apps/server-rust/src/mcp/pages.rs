//! The scheduled run's background page (`about:blank`, opened unfocused before the turn and
//! closed after it) through the host's live browser connection. The page is opened by the
//! server, not by an agent session, so it starts unowned and ungrouped, as TS's
//! `browser.newPage(url, { background: true })` did.

use crate::chat::BackgroundPages;
use browseros_core::{BrowserSession, PageId, pages::NewPageOptions};
use browseros_mcp_host::HostCtx;
use std::sync::Arc;

/// [`BackgroundPages`] over the host's current CDP epoch. Every failure is reported, never
/// retried: the engine falls back to the request's browser context, as TS did.
pub struct HostPages(pub Arc<HostCtx>);

impl HostPages {
    async fn browser(&self) -> anyhow::Result<Arc<BrowserSession>> {
        self.0
            .browser()
            .session()
            .await
            .ok_or_else(|| anyhow::anyhow!("browser is not connected"))
    }
}

#[async_trait::async_trait]
impl BackgroundPages for HostPages {
    async fn open_background(&self, url: &str) -> anyhow::Result<u32> {
        let page = self
            .browser()
            .await?
            .pages
            .new_page(
                url,
                NewPageOptions {
                    background: Some(true),
                    ..NewPageOptions::default()
                },
            )
            .await?;
        Ok(page.0)
    }

    async fn window_of(&self, page_id: u32) -> anyhow::Result<Option<i64>> {
        let browser = self.browser().await?;
        let live = browser.pages.list().await?;
        Ok(live
            .into_iter()
            .find(|page| page.page_id == PageId(page_id))
            .and_then(|page| page.window_id)
            .map(|window| window.0))
    }

    async fn close(&self, page_id: u32) -> anyhow::Result<()> {
        self.browser().await?.pages.close(PageId(page_id)).await?;
        Ok(())
    }
}
