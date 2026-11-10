//! The `Browser` domain as neo-runtime uses it: BrowserOS's tab strip and windows, which back a
//! run's pages and their context (`browser_representations/tabs.rs`), the browser's version, a
//! context's permissions and downloads, a page's window, and the download events.
//!
//! Neo: browseros-cdp's generated params, results and events, made from BrowserOS's pinned
//! protocol, under each method's name; a local type only where its doc says why.

use browseros_cdp::browser as generated;
use generated::{
    CreateTabResult, CreateWindowResult, GetTabInfoResult, GetTabsResult, GetVersionResult,
    GetWindowForTargetResult,
};
use serde::Serialize;
use serde_json::Value;

use super::{Ignored, OnBrowser, OnSession, command, event};

pub(crate) use generated::{
    ActivateTabParams as ActivateTab, Bounds, CancelDownloadParams as CancelDownload,
    CreateTabParams as CreateTab, CreateWindowParams as CreateWindow,
    DownloadProgressEvent as DownloadProgress, DownloadWillBeginEvent as DownloadWillBegin,
    GetTabInfoParams as GetTabInfo, GetTabsParams as GetTabs,
    GetWindowForTargetParams as GetWindowForTarget, GrantPermissionsParams as GrantPermissions,
    ResetPermissionsParams as ResetPermissions, SetDownloadBehaviorParams as SetDownloadBehavior,
    SetWindowBoundsParams as SetWindowBounds, TabInfo,
};

/// `Browser.getTabInfo` asked only whether the tab exists, so any answer will do: BrowserOS
/// answers a closed tab with an error (`getters::CLOSED_TAB_MESSAGE`), and `Runtime::connect`
/// quotes any other answer. Local: a second declaration of the method needs a type of its own.
#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct TabExists(pub(crate) GetTabInfo);

/// `Browser.getVersion`. Local: it takes no params, and browseros-cdp generates no params type
/// for such a command.
#[derive(Serialize)]
pub(crate) struct GetVersion {}

command! {
    // Neo: `page.bringToFront()` also selects the tab in BrowserOS's tab strip, which `Page.bringToFront` alone leaves unchanged there.
    ActivateTab => "Browser.activateTab", OnBrowser, Ignored;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.cancelDownload`: `await this._browser._session.send('Browser.cancelDownload', {`
    CancelDownload => "Browser.cancelDownload", OnBrowser, Ignored;
    // Neo: `context.newPage()` opens a background tab in the run's window, so the new page joins the user's tab strip.
    CreateTab => "Browser.createTab", OnBrowser, CreateTabResult;
    // Neo: a context whose last window closed gets a new window, since BrowserOS creates a tab only in one.
    CreateWindow => "Browser.createWindow", OnBrowser, CreateWindowResult;
    // Neo: a page's window and context, which bound the run's pages (`tabs.rs` `Scope`).
    GetTabInfo => "Browser.getTabInfo", OnBrowser, GetTabInfoResult;
    // Platform: BrowserOS answers `Browser.getTabInfo` for a closed tab with "No target with given id", the only signal that tells a closed tab from a failed call.
    TabExists => "Browser.getTabInfo", OnBrowser, Value;
    // Neo: `context.pages()` reads the context's tabs in tab-strip order.
    GetTabs => "Browser.getTabs", OnBrowser, GetTabsResult;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowser.connect`: `const version = await session.send('Browser.getVersion');`
    GetVersion => "Browser.getVersion", OnBrowser, GetVersionResult;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `const { windowId } = await this._client.send('Browser.getWindowForTarget');`
    GetWindowForTarget => "Browser.getWindowForTarget", OnSession, GetWindowForTargetResult;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doGrantPermissions`: `await this._browser._session.send('Browser.grantPermissions', { origin: origin === '*' ? undefined : origin, browserContextId: this._browserContextId, permissions: filtered });`
    GrantPermissions => "Browser.grantPermissions", OnBrowser, Ignored;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doClearPermissions`: `await this._browser._session.send('Browser.resetPermissions', { browserContextId: this._browserContextId });`
    ResetPermissions => "Browser.resetPermissions", OnBrowser, Ignored;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.initialize`: `promises.push(this._browser._session.send('Browser.setDownloadBehavior', {`
    SetDownloadBehavior => "Browser.setDownloadBehavior", OnBrowser, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession.setWindowBounds`: `return await this._client.send('Browser.setWindowBounds', {`
    SetWindowBounds => "Browser.setWindowBounds", OnSession, Ignored;
}

event! {
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowser.constructor`: `this._session.on('Browser.downloadProgress', this._onDownloadProgress.bind(this));`
    DownloadProgress => "Browser.downloadProgress";
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowser.constructor`: `this._session.on('Browser.downloadWillBegin', this._onDownloadWillBegin.bind(this));`
    DownloadWillBegin => "Browser.downloadWillBegin";
}
