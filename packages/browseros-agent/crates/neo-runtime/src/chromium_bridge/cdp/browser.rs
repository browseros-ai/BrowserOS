//! The `Browser` domain as neo-runtime uses it: BrowserOS's tab strip and windows, which back a
//! run's pages and their context (`browser_representations/tabs.rs`), the browser's version, a
//! context's permissions and downloads, a page's window, and the download events.
//!
//! Neo: browseros-cdp's generated params under each method's name where its surface has them;
//! local structs of only the fields Neo sends where it does not; replies and events of only the
//! fields Neo reads, as leniently as Neo reads them.

use browseros_cdp::browser as generated;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Ignored, OnBrowser, OnSession, command, event};

pub(crate) use generated::{
    ActivateTabParams as ActivateTab, CreateTabParams as CreateTab,
    CreateWindowParams as CreateWindow, GetTabInfoParams as GetTabInfo, GetTabsParams as GetTabs,
};

/// `Browser.cancelDownload` of a download of the run's context.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelDownload {
    pub(crate) guid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) browser_context_id: Option<String>,
}

/// `Browser.getTabInfo` asked only whether the tab exists, so any answer will do: BrowserOS
/// answers a closed tab with an error (`getters::CLOSED_TAB_MESSAGE`), and `Runtime::connect`
/// quotes any other answer.
#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct TabExists(pub(crate) GetTabInfo);

/// `Browser.getVersion`, which takes no params. Its reply stays JSON: `page.rs` reads the user
/// agent alone and `browser_methods.rs` the product too, each failing on what it needs.
#[derive(Serialize)]
pub(crate) struct GetVersion {}

/// `Browser.getWindowForTarget` of the session's own target, which takes no params. Its reply
/// stays JSON: `emulation.rs` reads `windowId` only when it is a number.
#[derive(Serialize)]
pub(crate) struct GetWindowForTarget {}

/// `Browser.grantPermissions`: `origin` absent for every origin (Playwright's `'*'`), and the
/// run's context when BrowserOS names one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrantPermissions {
    pub(crate) permissions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) browser_context_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResetPermissions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) browser_context_id: Option<String>,
}

/// `Browser.setDownloadBehavior`, as the download reports' setting sends it
/// (`browser_representations/downloads.rs` `DOWNLOAD_EVENTS`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetDownloadBehavior {
    pub(crate) behavior: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) events_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) browser_context_id: Option<String>,
}

/// `Browser.setWindowBounds` of a page's window, as Playwright's `FrameSession.setWindowBounds`
/// resizes it: the size alone.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetWindowBounds {
    pub(crate) window_id: i64,
    pub(crate) bounds: WindowSize,
}

#[derive(Serialize)]
pub(crate) struct WindowSize {
    pub(crate) width: i64,
    pub(crate) height: i64,
}

/// BrowserOS's tab-strip record. `liveBrowserContextId` is routable; the older
/// `browserContextId` field is only a human-readable profile label.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TabInfo {
    pub(crate) target_id: String,
    pub(crate) url: String,
    pub(crate) window_id: Option<i64>,
    pub(crate) live_browser_context_id: Option<String>,
    #[serde(default)]
    pub(crate) is_active: bool,
}

/// The tab of a `Browser.createTab` or `Browser.getTabInfo` reply.
#[derive(Deserialize)]
pub(crate) struct TabReply {
    pub(crate) tab: TabInfo,
}

#[derive(Deserialize)]
pub(crate) struct TabsReply {
    pub(crate) tabs: Vec<TabInfo>,
}

#[derive(Deserialize)]
pub(crate) struct WindowReply {
    pub(crate) window: CreatedWindow,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatedWindow {
    pub(crate) window_id: i64,
}

/// `Browser.downloadProgress`: a download's state, and the file the browser wrote once it
/// completed.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadProgress {
    pub(crate) guid: String,
    pub(crate) state: String,
    #[serde(default)]
    pub(crate) file_path: Option<String>,
}

/// `Browser.downloadWillBegin`: a frame began a download.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadWillBegin {
    pub(crate) frame_id: String,
    pub(crate) guid: String,
    pub(crate) url: String,
    pub(crate) suggested_filename: String,
}

command! {
    // Neo: `page.bringToFront()` also selects the tab in BrowserOS's tab strip, which `Page.bringToFront` alone leaves unchanged there.
    ActivateTab => "Browser.activateTab", OnBrowser, Ignored;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.cancelDownload`: `await this._browser._session.send('Browser.cancelDownload', {`
    CancelDownload => "Browser.cancelDownload", OnBrowser, Ignored;
    // Neo: `context.newPage()` opens a background tab in the run's window, so the new page joins the user's tab strip.
    CreateTab => "Browser.createTab", OnBrowser, TabReply;
    // Neo: a context whose last window closed gets a new window, since BrowserOS creates a tab only in one.
    CreateWindow => "Browser.createWindow", OnBrowser, WindowReply;
    // Neo: a page's window and context, which bound the run's pages (`tabs.rs` `Scope`).
    GetTabInfo => "Browser.getTabInfo", OnBrowser, TabReply;
    // Platform: BrowserOS answers `Browser.getTabInfo` for a closed tab with "No target with given id", the only signal that tells a closed tab from a failed call.
    TabExists => "Browser.getTabInfo", OnBrowser, Value;
    // Neo: `context.pages()` reads the context's tabs in tab-strip order.
    GetTabs => "Browser.getTabs", OnBrowser, TabsReply;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowser.connect`: `const version = await session.send('Browser.getVersion');`
    GetVersion => "Browser.getVersion", OnBrowser, Value;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `const { windowId } = await this._client.send('Browser.getWindowForTarget');`
    GetWindowForTarget => "Browser.getWindowForTarget", OnSession, Value;
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
