//! Translates MCP page arguments for help and pursue before those handlers use host page ids.

use crate::AppState;
use browseros_core::{PageId, TabId};
use serde_json::Value;

#[derive(Clone, Copy)]
pub(super) enum PageNaming {
    HostPage,
    NativeTab,
}

/// Keep the agent's id for replies and the host's id for browser access.
pub(super) struct NamedPage {
    pub(super) said: i64,
    pub(super) page: PageId,
}

impl PageNaming {
    pub(super) fn pursue_page(self, raw_args: &Value) -> Option<i64> {
        match self {
            // Match the catalog's pursue parser, which accepts page zero.
            Self::HostPage => raw_args
                .get("page")
                .and_then(Value::as_u64)
                .and_then(|page| u32::try_from(page).ok())
                .map(i64::from),
            Self::NativeTab => native_tab(raw_args),
        }
    }

    pub(super) async fn host_page(self, state: &AppState, said: i64) -> Option<PageId> {
        match self {
            Self::HostPage => u32::try_from(said).ok().map(PageId),
            Self::NativeTab => {
                // The SDK can create a tab on its own CDP connection before the host sees it.
                let browser = state.browser.session().await?;
                browser.pages.list().await.ok()?;
                browser
                    .pages
                    .get_info_by_tab(TabId(said))
                    .await
                    .map(|page| page.page_id)
            }
        }
    }

    pub(super) async fn help_page(
        self,
        state: &AppState,
        raw_args: &Value,
    ) -> Result<NamedPage, &'static str> {
        match self {
            Self::HostPage => {
                let page = browseros_mcp::extract_page_id(true, raw_args).ok_or(
                    "request_human_help: \"page\" is required (the id of the page that needs a person, the same number you pass to snapshot and act).",
                )?;
                Ok(NamedPage {
                    said: i64::from(page),
                    page: PageId(page),
                })
            }
            Self::NativeTab => {
                let said = native_tab(raw_args).ok_or(
                    "request_human_help: page is required. Pass page.tabId from a run script.",
                )?;
                let page = self.host_page(state, said).await.ok_or(
                    "request_human_help: that page.tabId is not open in the connected browser. Find the tab with context.pages() in run and ask again.",
                )?;
                Ok(NamedPage { said, page })
            }
        }
    }

    pub(super) fn help_page_not_yours(self, said: i64) -> String {
        let hint = match self {
            Self::HostPage => {
                "Pass the id of a page that is still open, the same number you use with snapshot and act."
            }
            Self::NativeTab => {
                "Pass page.tabId from a tab that is still open, found with context.pages() in run."
            }
        };
        format!("request_human_help: page {said} is not one of your open pages. {hint}")
    }

    pub(super) fn help_reply_names_tab(self) -> bool {
        matches!(self, Self::NativeTab)
    }

    pub(super) fn new_task_hint(self) -> &'static str {
        match self {
            Self::HostPage => "name_session",
            Self::NativeTab => "neo.createTask(name)",
        }
    }
}

fn native_tab(raw_args: &Value) -> Option<i64> {
    raw_args
        .get("page")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)
}

// Run effects use the host's existing cache when correlating and retrying individual SDK steps.
pub(super) async fn page_for_tab(state: &AppState, tab_id: i64) -> Option<PageId> {
    let browser = state.browser.session().await?;
    browser
        .pages
        .get_info_by_tab(TabId(tab_id))
        .await
        .map(|page| page.page_id)
}
