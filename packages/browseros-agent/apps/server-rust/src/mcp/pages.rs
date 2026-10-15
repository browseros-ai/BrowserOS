//! The chat layer's two direct uses of the browser, through the host's live connection:
//! the scheduled run's background page (`about:blank`, opened unfocused before the turn and
//! closed after it) and tab id → page id resolution for the request's browser context.
//! The background page is opened by the server, not by an agent session, so it starts
//! unowned and ungrouped, as TS's `browser.newPage(url, { background: true })` did.

use crate::chat::{BackgroundPages, TabResolver};
use browseros_core::{BrowserSession, PageId, pages::NewPageOptions};
use browseros_mcp_host::HostCtx;
use std::{collections::HashMap, sync::Arc};

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

/// [`TabResolver`] over the host's live page registry (TS `browser.resolveTabIds`): the app
/// names tabs by Chrome tab id, tools by page id. Without a browser nothing resolves.
pub struct HostTabs(pub Arc<HostCtx>);

#[async_trait::async_trait]
impl TabResolver for HostTabs {
    async fn resolve_tab_ids(&self, tab_ids: &[i64]) -> HashMap<i64, u32> {
        let Some(browser) = self.0.browser().session().await else {
            return HashMap::new();
        };
        let pages = match browser.pages.list().await {
            Ok(pages) => pages,
            Err(error) => {
                tracing::warn!(%error, "could not list pages to resolve tab ids");
                return HashMap::new();
            }
        };
        pages
            .into_iter()
            .filter(|page| tab_ids.contains(&page.tab_id.0))
            .map(|page| (page.tab_id.0, page.page_id.0))
            .collect()
    }
}
