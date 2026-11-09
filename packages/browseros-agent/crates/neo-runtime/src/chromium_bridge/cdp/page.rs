//! The `Page` domain as neo-runtime uses it: a tab's domains and lifecycle, navigation and its
//! history, the utility world, dialogs, file choosers, init scripts, screenshots and PDFs, and
//! the tab's own close and focus; and the frame, lifecycle, dialog and file chooser events the
//! page watcher and its consumers read.
//!
//! Neo: browseros-cdp's generated params under each method's name where its surface has them;
//! local structs of only the fields Neo sends where it does not; replies and events of only the
//! fields Neo reads, as leniently as Neo reads them. CDP marks most event fields required, but a
//! strict decode would drop an event that lacks one, and with it navigation tracking.

use browseros_cdp::page as generated;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Ignored, OnSession, command, event};

pub(crate) use generated::{
    CaptureScreenshotParams as CaptureScreenshot, EnableParams as Enable,
    HandleJavaScriptDialogParams as HandleJavaScriptDialog, NavigateParams as Navigate,
    PrintToPDFParams as PrintToPdf, ReloadParams as Reload, Viewport,
};

/// `Page.addScriptToEvaluateOnNewDocument` in the page's main world. Its reply stays JSON:
/// `init_scripts.rs` reads its `identifier` only when it is a string.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddScriptToEvaluateOnNewDocument {
    pub(crate) source: String,
    pub(crate) run_immediately: bool,
}

#[derive(Serialize)]
pub(crate) struct BringToFront {}

#[derive(Serialize)]
pub(crate) struct Close {}

/// Neo's utility world in a frame (`playwright_injected_methods.rs`). Its reply stays JSON:
/// a reply without an `executionContextId` is quoted whole in the call's error.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateIsolatedWorld<'a> {
    pub(crate) frame_id: &'a str,
    pub(crate) world_name: &'a str,
    /// Platform: CDP spells the field `grantUniveralAccess` (`protocol.json`); a corrected spelling is ignored silently.
    #[serde(rename = "grantUniveralAccess")]
    pub(crate) grant_universal_access: bool,
}

#[derive(Serialize)]
pub(crate) struct Disable {}

/// `Page.getFrameTree`, which takes no params. Its reply stays JSON: the frame model reads
/// the whole tree as it walks it (`frames.rs` `local_tree`).
#[derive(Serialize)]
pub(crate) struct GetFrameTree {}

#[derive(Serialize)]
pub(crate) struct GetLayoutMetrics {}

/// `Page.getNavigationHistory`, which takes no params. Its reply stays JSON: `page.url()` and
/// the history steps each decode the entries they read, with their own errors.
#[derive(Serialize)]
pub(crate) struct GetNavigationHistory {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigateToHistoryEntry {
    pub(crate) entry_id: i64,
}

#[derive(Serialize)]
pub(crate) struct RemoveScriptToEvaluateOnNewDocument {
    pub(crate) identifier: String,
}

#[derive(Serialize)]
pub(crate) struct SetBypassCsp {
    pub(crate) enabled: bool,
}

#[derive(Serialize)]
pub(crate) struct SetInterceptFileChooserDialog {
    pub(crate) enabled: bool,
}

#[derive(Serialize)]
pub(crate) struct SetLifecycleEventsEnabled {
    pub(crate) enabled: bool,
}

/// `Page.fileChooserOpened` while file choosers are intercepted: the input's frame and node,
/// and whether it takes several files.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileChooserOpened {
    pub(crate) frame_id: String,
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) backend_node_id: Option<i64>,
}

/// `Page.frameAttached`: a child frame and its parent. Chromium always names the parent; a
/// reader that needs it checks.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameAttached {
    pub(crate) frame_id: String,
    #[serde(default)]
    pub(crate) parent_frame_id: Option<String>,
}

/// `Page.frameDetached`: `reason` is `remove`, or `swap` when the frame's document moved to
/// another process.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameDetached {
    pub(crate) frame_id: String,
    #[serde(default)]
    pub(crate) reason: Option<String>,
}

/// `Page.frameNavigated`: a frame's commit. `type` and the frame's other fields may be absent.
#[derive(Deserialize)]
pub(crate) struct FrameNavigated {
    pub(crate) frame: NavigatedFrame,
    #[serde(rename = "type", default)]
    pub(crate) kind: Option<String>,
}

/// The frame of `Page.frameNavigated`. `parentId` is absent for a main frame.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigatedFrame {
    pub(crate) id: String,
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) url_fragment: Option<String>,
    #[serde(default)]
    pub(crate) loader_id: Option<String>,
    #[serde(default)]
    pub(crate) parent_id: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
}

/// `Page.frameRequestedNavigation`: a frame asked to navigate (a link, a form, a script).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameRequestedNavigation {
    pub(crate) frame_id: String,
    pub(crate) url: String,
    pub(crate) disposition: String,
}

/// `Page.frameStoppedLoading`: a frame stopped loading, whether or not it committed.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameStoppedLoading {
    pub(crate) frame_id: String,
}

/// `Page.javascriptDialogClosed`. Nothing of it is read.
#[derive(Deserialize)]
pub(crate) struct JavascriptDialogClosed {}

/// `Page.javascriptDialogOpening`: the dialog's type, its message and a prompt's default.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JavascriptDialogOpening {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) default_prompt: Option<String>,
}

/// `Page.lifecycleEvent`: a document reached a lifecycle point (`init`, `DOMContentLoaded`,
/// `load`, …).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LifecycleEvent {
    pub(crate) frame_id: String,
    pub(crate) loader_id: String,
    pub(crate) name: String,
}

/// `Page.navigatedWithinDocument`: a same-document commit. Without a `url` it changes nothing.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigatedWithinDocument {
    pub(crate) frame_id: String,
    #[serde(default)]
    pub(crate) url: Option<String>,
}

command! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._evaluateOnNewDocument`: `const { identifier } = await this._client.send('Page.addScriptToEvaluateOnNewDocument', { source: initScript.source, worldName, runImmediately });`
    AddScriptToEvaluateOnNewDocument => "Page.addScriptToEvaluateOnNewDocument", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `CRPage.bringToFront`: `await this._mainFrameSession._client.send('Page.bringToFront');`
    BringToFront => "Page.bringToFront", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `CRPage.takeScreenshot`: `const result = await progress.race(this._mainFrameSession._client.send('Page.captureScreenshot', { format, quality, clip, captureBeyondViewport: !fitsViewport }));`
    CaptureScreenshot => "Page.captureScreenshot", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `CRPage.closePage`: `await this._mainFrameSession._client.send('Page.close');`
    Close => "Page.close", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client._sendMayFail('Page.createIsolatedWorld', {`
    CreateIsolatedWorld<'_> => "Page.createIsolatedWorld", OnSession, Value;
    // Neo: a released run's init scripts stop running on a session's next documents (`init_scripts.rs` `suspend_scripts`); Playwright removes them one by one, and its sessions end with the page.
    Disable => "Page.disable", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Page.enable'),`
    Enable => "Page.enable", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Page.getFrameTree').then(({ frameTree }) => {`
    GetFrameTree => "Page.getFrameTree", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `CRPage.takeScreenshot`: `const { visualViewport, contentSize, cssContentSize } = await progress.race(this._mainFrameSession._client.send('Page.getLayoutMetrics'));`
    GetLayoutMetrics => "Page.getLayoutMetrics", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `CRPage._go`: `const history = await this._mainFrameSession._client.send('Page.getNavigationHistory');`
    GetNavigationHistory => "Page.getNavigationHistory", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._onDialog`: `await this._client.send('Page.handleJavaScriptDialog', { accept, promptText });`
    HandleJavaScriptDialog => "Page.handleJavaScriptDialog", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._navigate`: `const response = await this._client.send('Page.navigate', { url, referrer, frameId: frame._id, referrerPolicy: 'unsafeUrl' });`
    Navigate => "Page.navigate", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `CRPage._go`: `await this._mainFrameSession._client.send('Page.navigateToHistoryEntry', { entryId: entry.id });`
    NavigateToHistoryEntry => "Page.navigateToHistoryEntry", OnSession, Ignored;
    // Playwright: `server/chromium/crPdf.ts` `CRPDF.generate`: `const result = await this._client.send('Page.printToPDF', {`
    PrintToPdf => "Page.printToPDF", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `CRPage.reload`: `await this._mainFrameSession._client.send('Page.reload');`
    Reload => "Page.reload", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._removeEvaluatesOnNewDocument`: `await Promise.all(ids.map(identifier => this._client.send('Page.removeScriptToEvaluateOnNewDocument', { identifier }).catch(() => {}))); // target can be closed`
    RemoveScriptToEvaluateOnNewDocument => "Page.removeScriptToEvaluateOnNewDocument", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Page.setBypassCSP', { enabled: true }));`
    SetBypassCsp => "Page.setBypassCSP", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._updateFileChooserInterception`: `await this._client.send('Page.setInterceptFileChooserDialog', { enabled }).catch(() => {}); // target can be closed.`
    SetInterceptFileChooserDialog => "Page.setInterceptFileChooserDialog", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `lifecycleEventsEnabled = this._client.send('Page.setLifecycleEventsEnabled', { enabled: true }),`
    SetLifecycleEventsEnabled => "Page.setLifecycleEventsEnabled", OnSession, Ignored;
}

event! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Page.fileChooserOpened', event => this._onFileChooserOpened(event)),`
    FileChooserOpened => "Page.fileChooserOpened";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Page.frameAttached', event => this._onFrameAttached(event.frameId, event.parentFrameId)),`
    FrameAttached => "Page.frameAttached";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Page.frameDetached', event => this._onFrameDetached(event.frameId, event.reason)),`
    FrameDetached => "Page.frameDetached";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Page.frameNavigated', event => this._onFrameNavigated(event.frame, false)),`
    FrameNavigated => "Page.frameNavigated";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Page.frameRequestedNavigation', event => this._onFrameRequestedNavigation(event)),`
    FrameRequestedNavigation => "Page.frameRequestedNavigation";
    // Neo: a stop ends a requested navigation that never commits (a 204, a download), and marks a document restored from bfcache loaded, since Chromium replays no lifecycle for it.
    FrameStoppedLoading => "Page.frameStoppedLoading";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Page.javascriptDialogClosed', () => this._onDialogClosed()),`
    JavascriptDialogClosed => "Page.javascriptDialogClosed";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Page.javascriptDialogOpening', event => this._onDialog(event)),`
    JavascriptDialogOpening => "Page.javascriptDialogOpening";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._eventListeners.push(eventsHelper.addEventListener(this._client, 'Page.lifecycleEvent', event => this._onLifecycleEvent(event)));`
    LifecycleEvent => "Page.lifecycleEvent";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Page.navigatedWithinDocument', event => this._onFrameNavigatedWithinDocument(event.frameId, event.url)),`
    NavigatedWithinDocument => "Page.navigatedWithinDocument";
}
