//! The `Page` domain as neo-runtime uses it: a tab's domains and lifecycle, navigation and its
//! history, the utility world, dialogs, file choosers, init scripts, screenshots and PDFs, and
//! the tab's own close and focus; and the frame, lifecycle, dialog and file chooser events the
//! page watcher and its consumers read.
//!
//! Neo: browseros-cdp's generated params, results and events, made from BrowserOS's pinned
//! protocol, under each method's name; a local type only where its doc says why.

use browseros_cdp::page as generated;
use generated::{
    AddScriptToEvaluateOnNewDocumentResult, CaptureScreenshotResult, CreateIsolatedWorldResult,
    GetLayoutMetricsResult, GetNavigationHistoryResult, NavigateResult, PrintToPDFResult,
};
use serde::Serialize;
use serde_json::Value;

use super::{Ignored, OnSession, command, event};

pub(crate) use generated::{
    AddScriptToEvaluateOnNewDocumentParams as AddScriptToEvaluateOnNewDocument,
    CaptureScreenshotParams as CaptureScreenshot, CreateIsolatedWorldParams as CreateIsolatedWorld,
    EnableParams as Enable, FileChooserOpenedEvent as FileChooserOpened,
    FrameAttachedEvent as FrameAttached, FrameDetachedEvent as FrameDetached,
    FrameNavigatedEvent as FrameNavigated,
    FrameRequestedNavigationEvent as FrameRequestedNavigation,
    FrameStoppedLoadingEvent as FrameStoppedLoading,
    HandleJavaScriptDialogParams as HandleJavaScriptDialog,
    JavascriptDialogClosedEvent as JavascriptDialogClosed,
    JavascriptDialogOpeningEvent as JavascriptDialogOpening, LifecycleEventEvent as LifecycleEvent,
    NavigateParams as Navigate, NavigateToHistoryEntryParams as NavigateToHistoryEntry,
    NavigatedWithinDocumentEvent as NavigatedWithinDocument, PrintToPDFParams as PrintToPdf,
    ReloadParams as Reload,
    RemoveScriptToEvaluateOnNewDocumentParams as RemoveScriptToEvaluateOnNewDocument,
    SetBypassCSPParams as SetBypassCsp,
    SetInterceptFileChooserDialogParams as SetInterceptFileChooserDialog,
    SetLifecycleEventsEnabledParams as SetLifecycleEventsEnabled, Viewport,
};

/// `Page.bringToFront`. Local: it takes no params, and browseros-cdp generates no params type
/// for such a command.
#[derive(Serialize)]
pub(crate) struct BringToFront {}

/// `Page.close`. Local: it takes no params ([`BringToFront`]).
#[derive(Serialize)]
pub(crate) struct Close {}

/// `Page.disable`. Local: it takes no params ([`BringToFront`]).
#[derive(Serialize)]
pub(crate) struct Disable {}

/// `Page.getFrameTree`. Local: it takes no params ([`BringToFront`]). Its reply stays JSON: its
/// three readers (`frames.rs` `local_tree`, `page.rs`'s watcher seed, the frame model's seed)
/// walk the tree as they need it.
#[derive(Serialize)]
pub(crate) struct GetFrameTree {}

/// `Page.getLayoutMetrics`. Local: it takes no params ([`BringToFront`]).
#[derive(Serialize)]
pub(crate) struct GetLayoutMetrics {}

/// `Page.getNavigationHistory`. Local: it takes no params ([`BringToFront`]).
#[derive(Serialize)]
pub(crate) struct GetNavigationHistory {}

command! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._evaluateOnNewDocument`: `const { identifier } = await this._client.send('Page.addScriptToEvaluateOnNewDocument', { source: initScript.source, worldName, runImmediately });`
    AddScriptToEvaluateOnNewDocument => "Page.addScriptToEvaluateOnNewDocument", OnSession, AddScriptToEvaluateOnNewDocumentResult;
    // Playwright: `server/chromium/crPage.ts` `CRPage.bringToFront`: `await this._mainFrameSession._client.send('Page.bringToFront');`
    BringToFront => "Page.bringToFront", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `CRPage.takeScreenshot`: `const result = await progress.race(this._mainFrameSession._client.send('Page.captureScreenshot', { format, quality, clip, captureBeyondViewport: !fitsViewport }));`
    CaptureScreenshot => "Page.captureScreenshot", OnSession, CaptureScreenshotResult;
    // Playwright: `server/chromium/crPage.ts` `CRPage.closePage`: `await this._mainFrameSession._client.send('Page.close');`
    Close => "Page.close", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client._sendMayFail('Page.createIsolatedWorld', {`
    CreateIsolatedWorld => "Page.createIsolatedWorld", OnSession, CreateIsolatedWorldResult;
    // Neo: a released run's init scripts stop running on a session's next documents (`init_scripts.rs` `suspend_scripts`); Playwright removes them one by one, and its sessions end with the page.
    Disable => "Page.disable", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Page.enable'),`
    Enable => "Page.enable", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Page.getFrameTree').then(({ frameTree }) => {`
    GetFrameTree => "Page.getFrameTree", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `CRPage.takeScreenshot`: `const { visualViewport, contentSize, cssContentSize } = await progress.race(this._mainFrameSession._client.send('Page.getLayoutMetrics'));`
    GetLayoutMetrics => "Page.getLayoutMetrics", OnSession, GetLayoutMetricsResult;
    // Playwright: `server/chromium/crPage.ts` `CRPage._go`: `const history = await this._mainFrameSession._client.send('Page.getNavigationHistory');`
    GetNavigationHistory => "Page.getNavigationHistory", OnSession, GetNavigationHistoryResult;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._onDialog`: `await this._client.send('Page.handleJavaScriptDialog', { accept, promptText });`
    HandleJavaScriptDialog => "Page.handleJavaScriptDialog", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._navigate`: `const response = await this._client.send('Page.navigate', { url, referrer, frameId: frame._id, referrerPolicy: 'unsafeUrl' });`
    Navigate => "Page.navigate", OnSession, NavigateResult;
    // Playwright: `server/chromium/crPage.ts` `CRPage._go`: `await this._mainFrameSession._client.send('Page.navigateToHistoryEntry', { entryId: entry.id });`
    NavigateToHistoryEntry => "Page.navigateToHistoryEntry", OnSession, Ignored;
    // Playwright: `server/chromium/crPdf.ts` `CRPDF.generate`: `const result = await this._client.send('Page.printToPDF', {`
    PrintToPdf => "Page.printToPDF", OnSession, PrintToPDFResult;
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
