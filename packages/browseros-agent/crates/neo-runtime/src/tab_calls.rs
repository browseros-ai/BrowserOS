//! Context and Page wire handlers. Frame handlers stay in `dispatch::serve` and receive
//! their own page's session; these lifecycle calls use the same registry and closure tokens.

use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::CdpError;
use neo_runtime_spec::serve::{
    BrowserContextCall, BrowserContextNewPageResult, ConsoleMessagesFilter,
    PageClearConsoleMessagesParams, PageClearConsoleMessagesResult, PageClearPageErrorsParams,
    PageClearPageErrorsResult, PageCall, PageChannel, PageConsoleMessagesParams,
    PageConsoleMessagesResult, PagePageErrorsParams, PagePageErrorsResult,
};
use serde_json::{Value, json};
use tracing::debug;

use crate::bridge;
use crate::dispatch::encode_result;
use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::tabs::{self, PageEntry, Tabs};

pub(crate) async fn context(
    tabs: &Tabs,
    call: BrowserContextCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match call {
        BrowserContextCall::NewPage(_) => {
            let created = progress.race(tabs.create()).await??;
            let entry = &created.entry;
            // Playwright 26137face server/browserContext.ts:575-595 waits for initialization
            // before returning a page. Its committed history ensures a main frame here.
            let mut backoff = Duration::from_millis(1);
            loop {
                let url = progress
                    .race(tabs.url(entry))
                    .await??
                    .ok_or_else(PlaywrightError::target_closed)?;
                if !url.is_empty() {
                    break;
                }
                // BrowserOS can briefly report an empty history URL while creating the
                // initial document. Returning now would seed the SDK with that empty URL.
                progress.wait(backoff).await?;
                backoff = (backoff * 2).min(Duration::from_millis(16));
            }
            let result = BrowserContextNewPageResult {
                page: PageChannel {
                    guid: bridge::page_guid(&entry.id),
                },
            };
            let value = serde_json::to_value(result)
                .map_err(|error| PlaywrightError::error(error.to_string()))?;
            created.keep();
            Ok(Some(value))
        }
    }
}

pub(crate) async fn page(
    tabs: &Tabs,
    entry: &Arc<PageEntry>,
    call: PageCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match call {
        // Playwright's server keeps a page's console and page errors from its first moment;
        // Neo starts on the first read, with what V8 kept of the tab's live documents
        // (`page_events.rs`). A clear enables first, so that replay cannot bring back what it
        // cleared.
        PageCall::ConsoleMessages(params) => {
            let page = console_page(tabs, entry, progress).await?;
            let all = params.filter == Some(ConsoleMessagesFilter::All);
            return encode_result::<PageConsoleMessagesParams>(&PageConsoleMessagesResult {
                messages: page.events.console.messages(all),
            });
        }
        PageCall::PageErrors(params) => {
            let page = console_page(tabs, entry, progress).await?;
            let all = params.filter == Some(ConsoleMessagesFilter::All);
            return encode_result::<PagePageErrorsParams>(&PagePageErrorsResult {
                errors: page.events.console.errors(all),
            });
        }
        PageCall::ClearConsoleMessages(_) => {
            console_page(tabs, entry, progress)
                .await?
                .events
                .console
                .clear_messages();
            return encode_result::<PageClearConsoleMessagesParams>(
                &PageClearConsoleMessagesResult {},
            );
        }
        PageCall::ClearPageErrors(_) => {
            console_page(tabs, entry, progress)
                .await?
                .events
                .console
                .clear_errors();
            return encode_result::<PageClearPageErrorsParams>(&PageClearPageErrorsResult {});
        }
        PageCall::Close(_) => close(tabs, entry, progress).await?,
        PageCall::RunBeforeUnload(_) => {
            // Playwright 26137face server/page.ts:852-861 and chromium/crPage.ts:264-270:
            // Page.close runs beforeunload; errors are logged and closure is not awaited.
            let page = progress.race(tabs.session(entry)).await??;
            let prepared = progress.race(page.prepare()).await?;
            checked(tabs, entry, progress, prepared).await?;
            progress.check()?;
            entry.request_close();
            tabs.finish_close(Arc::clone(&page), Arc::clone(entry));
            match progress.race(page.send("Page.close", json!({}))).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => debug!(%error, "running beforeunload failed"),
                // The watcher can report closure before Page.close's acknowledgement.
                Err(CallError::Playwright(error)) if error == PlaywrightError::target_closed() => {}
                Err(error) => return Err(error),
            }
        }
        PageCall::BringToFront(_) => {
            // Playwright 26137face server/chromium/crPage.ts:207 sends Page.bringToFront.
            let page = progress.race(tabs.session(entry)).await??;
            let brought = progress
                .race(page.send("Page.bringToFront", json!({})))
                .await?;
            checked(tabs, entry, progress, brought).await?;
            let info = progress
                .race(tabs::tab_info(tabs.cdp(), &entry.id))
                .await??
                .ok_or_else(PlaywrightError::target_closed)?;
            // BrowserOS headless accepts Page.bringToFront without selecting the tab strip
            // entry. Browser.activateTab changes that selection (see probes-runtime.md).
            if !info.is_active {
                let activated = progress
                    .race(tabs.cdp().send(
                        "Browser.activateTab",
                        json!({"targetId":entry.id.as_str()}),
                        None,
                    ))
                    .await?;
                checked(tabs, entry, progress, activated).await?;
            }
        }
    }
    Ok(None)
}

/// `entry`'s session with console messages and page errors reported (`PageSession::enable_runtime`).
async fn console_page(
    tabs: &Tabs,
    entry: &PageEntry,
    progress: &Progress,
) -> Result<Arc<PageSession>, CallError> {
    let page = progress.race(tabs.session(entry)).await??;
    let enabled = progress.race(page.enable_runtime()).await?;
    checked(tabs, entry, progress, enabled).await?;
    Ok(page)
}

/// A session without enabled domains has no watcher yet. If it disappeared, ask the
/// browser whether the tab closed before returning a raw session error to the script.
async fn checked<T>(
    tabs: &Tabs,
    entry: &PageEntry,
    progress: &Progress,
    result: Result<T, CdpError>,
) -> Result<T, CallError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if progress.race(tabs.is_closed(entry)).await?? {
                Err(PlaywrightError::target_closed().into())
            } else {
                Err(tabs::cdp_error(&error).into())
            }
        }
    }
}

async fn close(tabs: &Tabs, entry: &PageEntry, progress: &Progress) -> Result<(), CallError> {
    // Playwright 26137face server/page.ts:826-847 treats repeated closes as success and
    // waits until closure. Browser.getTabInfo also covers pages with no enabled watcher.
    if progress.race(tabs.is_closed(entry)).await?? {
        return Ok(());
    }
    entry.request_close();
    let closed = progress
        .race(tabs.cdp().send(
            "Target.closeTarget",
            json!({"targetId":entry.id.as_str()}),
            None,
        ))
        .await?;
    if let Err(error) = closed {
        debug!(%error, "closing a tab failed; checking whether it is gone");
    }
    let mut backoff = Duration::from_millis(1);
    loop {
        if progress.race(tabs.is_closed(entry)).await?? {
            entry.closed_by_run();
            return Ok(());
        }
        progress.wait(backoff).await?;
        backoff = (backoff * 2).min(Duration::from_millis(50));
    }
}
