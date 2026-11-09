//! Context and Page wire handlers. Frame handlers stay in `method_router::serve` and receive
//! their own page's session; these lifecycle calls use the same registry and closure tokens.

use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::CdpError;
use neo_runtime_spec::serve::{
    BrowserContextCall, BrowserContextCookiesParams, BrowserContextCookiesResult,
    BrowserContextNewPageResult, BrowserContextSetNetworkInterceptionPatternsParams,
    BrowserContextStorageStateParams, ConsoleMessagesFilter, NameValue, PageCall, PageChannel,
    PageClearConsoleMessagesParams, PageClearConsoleMessagesResult, PageClearPageErrorsParams,
    PageClearPageErrorsResult, PageConsoleMessagesParams, PageConsoleMessagesResult,
    PageGoBackParams, PageGoBackResult, PageGoForwardParams, PageGoForwardResult,
    PagePageErrorsParams, PagePageErrorsResult, PageRegisterLocatorHandlerParams,
    PageRegisterLocatorHandlerResult, PageReloadParams, PageReloadResult, PageRequestsParams,
    PageRequestsResult, PageResolveLocatorHandlerNoReplyParams,
    PageResolveLocatorHandlerNoReplyResult, PageSetNetworkInterceptionPatternsParams,
    PageSetNetworkInterceptionPatternsResult, PageUnregisterLocatorHandlerParams,
    PageUnregisterLocatorHandlerResult, ResponseChannel,
};
use serde_json::{Value, json};
use tracing::debug;

use crate::browser_representations::cookies_and_storage::{self, StateFile};
use crate::browser_representations::emulation::{self, Size};
use crate::browser_representations::page::{FrameTab, PageSession};
use crate::browser_representations::routes::{self, Patterns};
use crate::browser_representations::tabs::{self, PageEntry, Tabs};
use crate::browser_representations::web_storage;
use crate::chromium_bridge::frames::FrameScope;
use crate::debugger::DEFAULT_CONTEXT;
use crate::method_router::encode_result;
use crate::playwright_methods::browser_methods;
use crate::playwright_methods::click::PageError;
use crate::playwright_methods::content::WriteTurn;
use crate::playwright_methods::highlight;
use crate::playwright_methods::history::{self, HistoryOptions};
use crate::playwright_methods::navigate;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::quickjs_bridge;

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
            // The run's init scripts reach the page's first document before the page is handed
            // out, as a new page's `FrameSession` runs them in Playwright.
            let scripts = progress.init_scripts();
            if scripts.has_scripts() {
                let session = progress.race(tabs.session(entry)).await??;
                progress
                    .race(scripts.show_new(tabs.cdp(), &session, DEFAULT_CONTEXT))
                    .await?;
            }
            let result = BrowserContextNewPageResult {
                page: PageChannel {
                    guid: quickjs_bridge::page_guid(&entry.id),
                },
            };
            let value = serde_json::to_value(result)
                .map_err(|error| PlaywrightError::error(error.to_string()))?;
            // Neo: commit the host creation event after readiness and reply encoding, before the creating call can end.
            created.keep(progress);
            Ok(Some(value))
        }
        BrowserContextCall::Close(_) => {
            browser_methods::disconnect(tabs, progress).await?;
            Ok(None)
        }
        // Playwright: `server/dispatchers/browserContextDispatcher.ts` `BrowserContextDispatcher.pause`: `// Debugger will take care of this.`
        // `setTestIdAttributeName`: `Dispatcher::call` applied it before this call started.
        BrowserContextCall::Pause(_) | BrowserContextCall::SetTestIdAttributeName(_) => Ok(None),
        BrowserContextCall::SetNetworkInterceptionPatterns(params) => {
            context_patterns(tabs, params, progress).await
        }
        BrowserContextCall::Cookies(params) => {
            let cookies = cookies_and_storage::cookies(tabs, &params.urls, progress).await?;
            encode_result::<BrowserContextCookiesParams>(&BrowserContextCookiesResult { cookies })
        }
        BrowserContextCall::AddCookies(params) => {
            cookies_and_storage::add_cookies(tabs, params.cookies, progress).await?;
            Ok(None)
        }
        BrowserContextCall::SetExtraHTTPHeaders(params) => {
            tabs.overrides()
                .set_context_headers(tabs, params.headers, progress)
                .await?;
            Ok(None)
        }
        BrowserContextCall::SetOffline(params) => {
            tabs.overrides()
                .set_offline(tabs, params.offline, progress)
                .await?;
            Ok(None)
        }
        BrowserContextCall::SetGeolocation(params) => {
            tabs.overrides()
                .set_geolocation(tabs, params.geolocation, progress)
                .await?;
            Ok(None)
        }
        BrowserContextCall::GrantPermissions(params) => {
            tabs.permissions()
                .grant(
                    tabs,
                    tabs.run_id(),
                    params.origin.as_deref(),
                    &params.permissions,
                    progress,
                )
                .await?;
            Ok(None)
        }
        BrowserContextCall::ClearPermissions(_) => {
            tabs.permissions()
                .clear(tabs, tabs.run_id(), progress)
                .await?;
            Ok(None)
        }
        BrowserContextCall::SetHTTPCredentials(_) => {
            cookies_and_storage::set_http_credentials()?;
            Ok(None)
        }
        BrowserContextCall::StorageState(params) => {
            state_file(tabs, StateFile::Export(params), None, progress).await
        }
        BrowserContextCall::SetStorageState(params) => {
            state_file(tabs, StateFile::Import(params), None, progress).await
        }
    }
}

/// Playwright: `client/browserContext.ts` `BrowserContext.storageState`: `if (options.path) {`
///
/// `context.storageState` and `context.setStorageState`, with the `path` they write or read in
/// the run's artifact directory (`artifacts.rs`), checked before anything else: `storageState`
/// writes none for an empty one, as Playwright's `if (options.path)` does; `setStorageState`
/// takes any string as a file to read.
pub(crate) async fn state_file(
    tabs: &Tabs,
    file: StateFile,
    path: Option<String>,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let artifacts = progress.artifacts();
    match file {
        StateFile::Export(params) => {
            let path = path
                .filter(|path| !path.is_empty())
                .map(|path| artifacts.check("path", &path))
                .transpose()?;
            let state = cookies_and_storage::storage_state(tabs, &params, progress).await?;
            if let Some(path) = &path {
                cookies_and_storage::write_state(artifacts, path, &state).await?;
            }
            encode_result::<BrowserContextStorageStateParams>(&state)
        }
        StateFile::Import(params) => {
            let state = match path {
                Some(path) => Some(cookies_and_storage::read_state(artifacts, &path).await?),
                None => params.storage_state,
            };
            cookies_and_storage::set_storage_state(tabs, state, progress).await?;
            Ok(None)
        }
    }
}

/// `page.setExtraHTTPHeaders`: the page's own, over the context's (`cookies_and_storage.rs`).
async fn set_page_headers(
    tabs: &Tabs,
    entry: &Arc<PageEntry>,
    headers: Vec<NameValue>,
    progress: &Progress,
) -> Result<(), CallError> {
    // (platform) The page's own headers go over its context's: the run's own context's.
    if entry.context.get().is_some() {
        return Err(browser_methods::created_state("setExtraHTTPHeaders").into());
    }
    let page = progress.race(tabs.session(entry)).await??;
    tabs.overrides()
        .set_page_headers(&page, &entry.id, headers, progress)
        .await
}

/// Neo: `method_router::route` hands every `Page.screenshot` and `Page.pdf` to `capture.rs`, so one
/// reaching the tab-lifecycle handler is a routing bug, reported as such.
const CAPTURE_MISROUTED: &str = "neo-runtime: a capture reached the tab-lifecycle handler";

fn misrouted_capture() -> CallError {
    PlaywrightError::error(CAPTURE_MISROUTED).into()
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
            return clear_console(tabs, entry, true, progress).await;
        }
        call @ (PageCall::RegisterLocatorHandler(_)
        | PageCall::ResolveLocatorHandlerNoReply(_)
        | PageCall::UnregisterLocatorHandler(_)) => {
            return locator_handler_call(tabs, entry, call, progress).await;
        }
        PageCall::ClearPageErrors(_) => return clear_console(tabs, entry, false, progress).await,
        // Playwright's server keeps a page's last requests from its first moment; Neo keeps
        // what the run's network log recorded, which starts with the session's
        // `Network.enable`: the first goto, history step or navigation wait, a network event
        // listener, or this read, as Chromium replays no earlier request.
        PageCall::Requests(_) => return requests(tabs, entry, progress).await,
        // The page's routes (`routes.rs`): its sessions pause every request while one exists.
        PageCall::SetNetworkInterceptionPatterns(params) => {
            return set_page_patterns(tabs, entry, params, progress).await;
        }
        // `method_router::route` hands these to [`storage`], in their page's storage order.
        PageCall::WebStorageItems(_)
        | PageCall::WebStorageGetItem(_)
        | PageCall::WebStorageSetItem(_)
        | PageCall::WebStorageRemoveItem(_)
        | PageCall::WebStorageClear(_) => {
            return Err(PlaywrightError::error(STORAGE_MISROUTED).into());
        }
        PageCall::Close(_) => close(tabs, entry, progress).await?,
        call @ (PageCall::SetViewportSize(_)
        | PageCall::EmulateMedia(_)
        | PageCall::RequestGC(_)) => emulated(tabs, entry, call, progress).await?,
        PageCall::SetExtraHTTPHeaders(params) => {
            set_page_headers(tabs, entry, params.headers, progress).await?;
        }
        PageCall::RunBeforeUnload(_) => run_before_unload(tabs, entry, progress).await?,
        PageCall::Screenshot(_) | PageCall::Pdf(_) => return Err(misrouted_capture()),
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
        PageCall::BringToFront(_) => bring_to_front(tabs, entry, progress).await?,
        PageCall::HideHighlight(_) => {
            let page = progress.race(tabs.session(entry)).await??;
            highlight::hide_all(&page, progress).await?;
        }
    }
    Ok(None)
}

/// `Page.bringToFront`: the tab's own `Page.bringToFront`, then its tab-strip selection.
async fn bring_to_front(
    tabs: &Tabs,
    entry: &Arc<PageEntry>,
    progress: &Progress,
) -> Result<(), CallError> {
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
    Ok(())
}

/// The locator handler commands of `entry`'s session (`locator_handlers.rs`).
async fn locator_handler_call(
    tabs: &Tabs,
    entry: &Arc<PageEntry>,
    call: PageCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let page = progress.race(tabs.session(entry)).await??;
    match call {
        PageCall::RegisterLocatorHandler(params) => {
            let uid = page
                .locator_handlers
                .register(params.selector, params.no_wait_after.unwrap_or(false))?;
            encode_result::<PageRegisterLocatorHandlerParams>(&PageRegisterLocatorHandlerResult {
                uid,
            })
        }
        PageCall::ResolveLocatorHandlerNoReply(params) => {
            page.locator_handlers
                .resolve(params.uid, params.remove.unwrap_or(false))?;
            encode_result::<PageResolveLocatorHandlerNoReplyParams>(
                &PageResolveLocatorHandlerNoReplyResult {},
            )
        }
        PageCall::UnregisterLocatorHandler(params) => {
            page.locator_handlers.unregister(params.uid)?;
            encode_result::<PageUnregisterLocatorHandlerParams>(
                &PageUnregisterLocatorHandlerResult {},
            )
        }
        _ => Err(PlaywrightError::error(
            "neo-runtime: `locator_handler_call` serves the locator handler commands only",
        )
        .into()),
    }
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

/// The page's emulation calls and `requestGC`, on its run session (`emulation.rs`).
async fn emulated(
    tabs: &Tabs,
    entry: &PageEntry,
    call: PageCall,
    progress: &Progress,
) -> Result<(), CallError> {
    let page = progress.race(tabs.session(entry)).await??;
    match call {
        PageCall::SetViewportSize(params) => {
            let size = Size {
                width: params.viewport_size.width,
                height: params.viewport_size.height,
            };
            page.emulation
                .set_viewport_size(&page, size, progress)
                .await
        }
        PageCall::EmulateMedia(params) => {
            page.emulation.emulate_media(&page, &params, progress).await
        }
        PageCall::RequestGC(_) => emulation::request_gc(&page, progress).await,
        _ => Err(PlaywrightError::error("neo-runtime: not an emulation call").into()),
    }
}

/// Neo: `page.route`'s patterns (`setNetworkInterceptionPatterns`): which of the page's requests
/// pause for its handlers (`routes.rs`).
async fn set_page_patterns(
    tabs: &Tabs,
    entry: &PageEntry,
    params: PageSetNetworkInterceptionPatternsParams,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let patterns = Patterns::new(params.patterns.into_iter().map(|pattern| {
        (
            pattern.glob,
            pattern.regex_source,
            pattern.regex_flags,
            pattern.url_pattern.is_some(),
        )
    }))?;
    let page = progress.race(tabs.session(entry)).await??;
    routes::set_page_patterns(&page, patterns, progress).await?;
    encode_result::<PageSetNetworkInterceptionPatternsParams>(
        &PageSetNetworkInterceptionPatternsResult {},
    )
}

/// Neo: `context.route`'s patterns (`setNetworkInterceptionPatterns`): which requests of every
/// tab of the run pause for the context's handlers (`routes.rs`).
async fn context_patterns(
    tabs: &Tabs,
    params: BrowserContextSetNetworkInterceptionPatternsParams,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let patterns = Patterns::new(params.patterns.into_iter().map(|pattern| {
        (
            pattern.glob,
            pattern.regex_source,
            pattern.regex_flags,
            pattern.url_pattern.is_some(),
        )
    }))?;
    routes::set_context_patterns(tabs, patterns, progress).await?;
    Ok(Some(json!({})))
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

/// `Page.clearConsoleMessages` (`messages`) or `Page.clearPageErrors`: the page's kept console
/// messages, or its page errors, go.
async fn clear_console(
    tabs: &Tabs,
    entry: &Arc<PageEntry>,
    messages: bool,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let console = &console_page(tabs, entry, progress).await?.events.console;
    if messages {
        console.clear_messages();
        encode_result::<PageClearConsoleMessagesParams>(&PageClearConsoleMessagesResult {})
    } else {
        console.clear_errors();
        encode_result::<PageClearPageErrorsParams>(&PageClearPageErrorsResult {})
    }
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

/// Neo: a storage call reaching the tab-lifecycle handler, which `method_router::route` never does.
const STORAGE_MISROUTED: &str = "neo-runtime: a storage call reached the tab-lifecycle handler";

/// A `page.localStorage` / `page.sessionStorage` call on `entry`'s main frame, in call order:
/// after the session's one shared preparation, it waits for every earlier storage call of the
/// page to finish, so no call overtakes an earlier one, as Playwright's server sends them in
/// order. Its place (`turn`) was taken as it arrived (`method_router.rs` `Inner::ordered`).
pub(crate) async fn storage(
    tabs: &Tabs,
    entry: &Arc<PageEntry>,
    call: PageCall,
    turn: Option<WriteTurn>,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let page = prepared_page(tabs, entry, progress).await?;
    let _turn = match turn {
        Some(turn) => Some(progress.race(turn.ready()).await?),
        None => None,
    };
    // The call belongs to the document it starts in. A commit that replaces that document ends
    // it with Playwright's destroyed-context error and hands its turn on, as Playwright's
    // utility-world evaluation rejects once its context is gone: a later call never waits for
    // an obsolete document's reply.
    let document = page.main_document();
    let serving = web_storage::serve(Arc::clone(&page), call, progress);
    tokio::select! {
        biased;
        result = serving => result,
        () = page.main_document_replaced(document) => {
            Err(navigate::page_error(PageError::NotConnected).into())
        }
    }
}

/// `entry`'s session with its watcher running (`PageSession::prepare`), as a call on its frames
/// needs it.
async fn prepared_page(
    tabs: &Tabs,
    entry: &PageEntry,
    progress: &Progress,
) -> Result<Arc<PageSession>, CallError> {
    let page = progress.race(tabs.session(entry)).await??;
    let prepared = progress.race(page.prepare()).await?;
    checked(tabs, entry, progress, prepared).await?;
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
