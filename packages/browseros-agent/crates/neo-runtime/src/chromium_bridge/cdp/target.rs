//! The `Target` domain as neo-runtime uses it: a run's own sessions on tabs and out-of-process
//! frames, the targets Chromium auto-attaches under them, created browser contexts and their
//! pages, closing a tab, discovery of new tabs, and the events that report targets coming,
//! changing and going.
//!
//! Neo: browseros-cdp's generated params, results and events, made from BrowserOS's pinned
//! protocol, under each method's name; a local type only where its doc says why.

use browseros_cdp::target as generated;
use generated::{
    AttachToTargetResult, CreateBrowserContextResult, CreateTargetResult, GetTargetInfoResult,
};
use serde::Serialize;
use serde_json::Value;

use super::{Ignored, OnBrowser, OnSession, command, event};

pub(crate) use generated::{
    AttachToTargetParams as AttachToTarget, AttachedToTargetEvent as AttachedToTarget,
    CloseTargetParams as CloseTarget, CreateBrowserContextParams as CreateBrowserContext,
    CreateTargetParams as CreateTarget, DetachFromTargetParams as DetachFromTarget,
    DetachedFromTargetEvent as DetachedFromTarget,
    DisposeBrowserContextParams as DisposeBrowserContext, FilterEntry,
    GetTargetInfoParams as GetTargetInfo, GetTargetsParams as GetTargets,
    SetAutoAttachParams as SetAutoAttach, SetDiscoverTargetsParams as SetDiscoverTargets,
    TargetCreatedEvent as TargetCreated, TargetDestroyedEvent as TargetDestroyed, TargetInfo,
    TargetInfoChangedEvent as TargetInfoChanged,
};

/// `Target.detachFromTarget` sent on the parent session of a target Chromium auto-attached
/// there: the child session belongs to that parent, not to the browser target. Local: a second
/// declaration of the method needs a type of its own.
#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct DetachChild(pub(crate) DetachFromTarget);

command! {
    // Playwright: `server/chromium/crConnection.ts` `CDPSession.attachToTarget`: `const { sessionId } = await this._send('Target.attachToTarget', { targetId, flatten: true });`
    AttachToTarget => "Target.attachToTarget", OnBrowser, AttachToTargetResult;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowser._closePage`: `await this._session.send('Target.closeTarget', { targetId: crPage._targetId });`
    CloseTarget => "Target.closeTarget", OnBrowser, Ignored;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowser.doCreateNewContext`: `const { browserContextId } = await this._session.send('Target.createBrowserContext', {`
    CreateBrowserContext => "Target.createBrowserContext", OnBrowser, CreateBrowserContextResult;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doCreateNewPage`: `const { targetId } = await this._browser._session.send('Target.createTarget', { url: 'about:blank', browserContextId: this._browserContextId });`
    CreateTarget => "Target.createTarget", OnBrowser, CreateTargetResult;
    // Playwright: `server/chromium/crConnection.ts` `CRSession.detach`: `await this._parentSession.send('Target.detachFromTarget', { sessionId: this._sessionId });`
    DetachFromTarget => "Target.detachFromTarget", OnBrowser, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._onAttachedToTarget`: `parentSession._sendMayFail('Target.detachFromTarget', { sessionId: event.sessionId });`
    DetachChild => "Target.detachFromTarget", OnSession, Ignored;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doClose`: `await this._browser._session.send('Target.disposeBrowserContext', { browserContextId: this._browserContextId });`
    DisposeBrowserContext => "Target.disposeBrowserContext", OnBrowser, Ignored;
    // Neo: a popup's opener, and whether a reported target is a page, which only the browser's own record of the target tells (`tabs.rs`).
    GetTargetInfo => "Target.getTargetInfo", OnBrowser, GetTargetInfoResult;
    // Neo: the out-of-process frames under a tab, which `Page.getFrameTree` leaves out, the baseline of discovery's page targets, and a created context's pages; three readers, so the reply stays JSON.
    GetTargets => "Target.getTargets", OnBrowser, Value;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }),`
    SetAutoAttach => "Target.setAutoAttach", OnSession, Ignored;
    // Neo: discovery reports the run's new tabs and popups, which Neo's attach-on-demand sessions would not see; Playwright auto-attaches every page instead.
    SetDiscoverTargets => "Target.setDiscoverTargets", OnBrowser, Ignored;
}

event! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addBrowserListeners`: `eventsHelper.addEventListener(this._client, 'Target.attachedToTarget', event => this._onAttachedToTarget(this._client, event)),`
    AttachedToTarget => "Target.attachedToTarget";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addBrowserListeners`: `eventsHelper.addEventListener(this._client, 'Target.detachedFromTarget', event => this._onDetachedFromTarget(this._client, event)),`
    DetachedFromTarget => "Target.detachedFromTarget";
    // Neo: discovery's report of a new page target, which Neo turns into the run's `page` and `popup` events once it commits (`discovery.rs`).
    TargetCreated => "Target.targetCreated";
    // Neo: a page target discovery reported that closed before its first commit (`discovery.rs`).
    TargetDestroyed => "Target.targetDestroyed";
    // Neo: a page target's first commit or its opener, as discovery reports them (`discovery.rs`).
    TargetInfoChanged => "Target.targetInfoChanged";
}
