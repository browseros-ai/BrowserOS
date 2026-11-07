//! Context and Page wire handlers. Frame handlers stay in `dispatch::serve` and receive
//! their own page's session; these lifecycle calls use the same registry and closure tokens.

use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::CdpError;
use neo_runtime_spec::serve::{
    BrowserContextCall, BrowserContextNewPageResult, ConsoleMessagesFilter, PageCall, PageChannel,
    PageClearConsoleMessagesParams, PageClearConsoleMessagesResult, PageClearPageErrorsParams,
    PageClearPageErrorsResult, PageConsoleMessagesParams, PageConsoleMessagesResult,
    PageGoBackParams, PageGoBackResult, PageGoForwardParams, PageGoForwardResult,
    PagePageErrorsParams, PagePageErrorsResult, PageReloadParams, PageReloadResult,
    PageRequestsParams, PageRequestsResult, ResponseChannel,
};
use serde_json::{Value, json};
use tracing::debug;

use crate::bridge;
use crate::dispatch::encode_result;
use crate::frames::FrameScope;
use crate::highlight;
use crate::history::{self, HistoryOptions};
use crate::page::{FrameTab, PageSession};
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
        BrowserContextCall::Close(_) => {
            close_context(tabs, progress).await?;
            Ok(None)
        }
    }
}

/// Playwright's close of a connectOverCDP default context, which only disconnects: every tab
/// stays open (measured on BrowserOS neo, a2-mapping-evidence.md §2). The run lets go of what it
/// holds in the browser: a `beforeunload` dialog a listener holds is dismissed, so its tab stays,
/// every subscription ends with target discovery, and every other call of the run stops, as
/// Playwright's calls stop with its connection. The SDK closes the run's pages and context as the
/// script sees them; the reason stays there.
async fn close_context(tabs: &Tabs, progress: &Progress) -> Result<(), CallError> {
    for (_, session) in tabs.attached() {
        if session.events.dialogs.take_open_beforeunload() {
            let dismissed = progress
                .race(session.send("Page.handleJavaScriptDialog", json!({ "accept": false })))
                .await?;
            if let Err(error) = dismissed {
                debug!(%error, "dismissing a beforeunload dialog at the context's close failed");
            }
        }
    }
    tabs.events().end_subscriptions();
    // The script's promises for them have rejected (the SDK); this stops the calls themselves
    // before the close resolves, so none writes input to a tab the run let go of.
    progress.stop_the_runs_calls();
    Ok(())
}

pub(crate) async fn page(
    tabs: &Tabs,
    entry: &Arc<PageEntry>,
    call: PageCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match call {
        PageCall::Reload(params) => {
            let response = history_step(tabs, entry, HistoryOptions::reload(&params), progress);
            return encode_result::<PageReloadParams>(&PageReloadResult {
                response: response.await?,
            });
        }
        PageCall::GoBack(params) => {
            let response = history_step(tabs, entry, HistoryOptions::back(&params), progress);
            return encode_result::<PageGoBackParams>(&PageGoBackResult {
                response: response.await?,
            });
        }
        PageCall::GoForward(params) => {
            let response = history_step(tabs, entry, HistoryOptions::forward(&params), progress);
            return encode_result::<PageGoForwardParams>(&PageGoForwardResult {
                response: response.await?,
            });
        }
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
        // Playwright's server keeps a page's last requests from its first moment; Neo keeps
        // what the run's network log recorded, which starts with the session's
        // `Network.enable`: the first goto, history step or navigation wait, a network event
        // listener, or this read, as Chromium replays no earlier request.
        PageCall::Requests(_) => return requests(tabs, entry, progress).await,
        PageCall::Close(_) => close(tabs, entry, progress).await?,
        PageCall::RunBeforeUnload(_) => run_before_unload(tabs, entry, progress).await?,
        // The dispatcher serves these over the tab's input devices (`devices.rs`).
        PageCall::KeyboardDown(_)
        | PageCall::KeyboardUp(_)
        | PageCall::KeyboardInsertText(_)
        | PageCall::KeyboardType(_)
        | PageCall::KeyboardPress(_)
        | PageCall::MouseMove(_)
        | PageCall::MouseDown(_)
        | PageCall::MouseUp(_)
        | PageCall::MouseClick(_)
        | PageCall::MouseWheel(_)
        | PageCall::TouchscreenTap(_) => {
            return Err(PlaywrightError::error(
                "neo-runtime: a device call reached the tab-lifecycle handler",
            )
            .into());
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
        PageCall::HideHighlight(_) => {
            let page = progress.race(tabs.session(entry)).await??;
            highlight::hide_all(&page, progress).await?;
        }
    }
    Ok(None)
}

/// `Page.reload`, `goBack` or `goForward` on the main frame of `entry`'s session: the new
/// document's response, or `None` where Playwright's answer is `null`. An option Neo refuses
/// (`networkidle`) fails before the session is touched.
async fn history_step(
    tabs: &Tabs,
    entry: &Arc<PageEntry>,
    options: HistoryOptions,
    progress: &Progress,
) -> Result<Option<ResponseChannel>, CallError> {
    let page = progress.race(tabs.session(entry)).await??;
    let main = FrameTab {
        page,
        scope: FrameScope::Main,
    };
    history::navigate(&main, &options, progress).await
}

/// `page.requests()`: what the session's network log recorded, after making sure it records.
async fn requests(
    tabs: &Tabs,
    entry: &PageEntry,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let page = progress.race(tabs.session(entry)).await??;
    let enabled = progress.race(page.enable_network()).await?;
    checked(tabs, entry, progress, enabled).await?;
    let requests = page
        .network
        .requests()
        .map_err(|error| PlaywrightError::error(error.to_string()))?;
    encode_result::<PageRequestsParams>(&PageRequestsResult { requests })
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

/// Playwright 26137face server/page.ts:852-861 and chromium/crPage.ts:264-270: Page.close runs
/// beforeunload; errors are logged and closure is not awaited.
async fn run_before_unload(
    tabs: &Tabs,
    entry: &Arc<PageEntry>,
    progress: &Progress,
) -> Result<(), CallError> {
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
    Ok(())
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
