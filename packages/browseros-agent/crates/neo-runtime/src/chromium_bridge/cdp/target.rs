//! The `Target` domain as neo-runtime uses it: a run's own sessions on tabs and out-of-process
//! frames, the targets Chromium auto-attaches under them, created browser contexts and their
//! pages, closing a tab, discovery of new tabs, and the events that report targets coming,
//! changing and going.
//!
//! Neo: browseros-cdp's generated params under each method's name where its surface has them;
//! local structs of only the fields Neo sends where it does not; replies and events of only the
//! fields Neo reads, as leniently as Neo reads them.

use browseros_cdp::target as generated;
use generated::AttachToTargetResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Ignored, OnBrowser, OnSession, command, event};

pub(crate) use generated::{
    AttachToTargetParams as AttachToTarget, DetachFromTargetParams as DetachFromTarget,
    FilterEntry, GetTargetsParams as GetTargets, SetAutoAttachParams as SetAutoAttach,
    SetDiscoverTargetsParams as SetDiscoverTargets,
};

/// `Target.closeTarget`. Its `success` reply is never read: the caller checks the tab is gone.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloseTarget {
    pub(crate) target_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateBrowserContext {
    pub(crate) dispose_on_detach: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatedBrowserContext {
    pub(crate) browser_context_id: String,
}

/// `Target.createTarget`: a page of a created browser context.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTarget {
    pub(crate) url: String,
    pub(crate) browser_context_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatedTarget {
    pub(crate) target_id: String,
}

/// `Target.detachFromTarget` sent on the parent session of a target Chromium auto-attached
/// there: the child session belongs to that parent, not to the browser target.
#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct DetachChild(pub(crate) DetachFromTarget);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DisposeBrowserContext {
    pub(crate) browser_context_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetTargetInfo {
    pub(crate) target_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TargetInfoReply {
    pub(crate) target_info: TargetInfo,
}

/// What `Target.targetCreated`, `Target.targetInfoChanged`, `Target.getTargetInfo` and
/// `Target.getTargets` report of a target.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TargetInfo {
    pub(crate) target_id: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) url: String,
    /// The target whose frame opened this one, set even for `noopener` (measured).
    #[serde(default)]
    pub(crate) opener_id: Option<String>,
    #[serde(default)]
    pub(crate) browser_context_id: Option<String>,
}

/// `Target.attachedToTarget`, on the parent session of a target Chromium auto-attached:
/// the child's session, what kind of target it is, and whether it waits to be resumed.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachedToTarget {
    pub(crate) session_id: String,
    #[serde(default)]
    pub(crate) target_info: AttachedTargetInfo,
    #[serde(default)]
    pub(crate) waiting_for_debugger: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachedTargetInfo {
    #[serde(default)]
    pub(crate) target_id: Option<String>,
    #[serde(rename = "type", default)]
    pub(crate) kind: Option<String>,
}

/// `Target.detachedFromTarget`. On the root session it names the detached session in its
/// params; `targetId` is deprecated, so CDP may name the session alone.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DetachedFromTarget {
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) target_id: Option<String>,
}

/// `Target.targetCreated`, reported to the browser target while discovery is on.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TargetCreated {
    pub(crate) target_info: TargetInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TargetDestroyed {
    pub(crate) target_id: String,
}

/// `Target.targetInfoChanged`: a target's URL or opener changed, its first commit included.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TargetInfoChanged {
    pub(crate) target_info: TargetInfo,
}

command! {
    // Playwright: `server/chromium/crConnection.ts` `CDPSession.attachToTarget`: `const { sessionId } = await this._send('Target.attachToTarget', { targetId, flatten: true });`
    AttachToTarget => "Target.attachToTarget", OnBrowser, AttachToTargetResult;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowser._closePage`: `await this._session.send('Target.closeTarget', { targetId: crPage._targetId });`
    CloseTarget => "Target.closeTarget", OnBrowser, Ignored;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowser.doCreateNewContext`: `const { browserContextId } = await this._session.send('Target.createBrowserContext', {`
    CreateBrowserContext => "Target.createBrowserContext", OnBrowser, CreatedBrowserContext;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doCreateNewPage`: `const { targetId } = await this._browser._session.send('Target.createTarget', { url: 'about:blank', browserContextId: this._browserContextId });`
    CreateTarget => "Target.createTarget", OnBrowser, CreatedTarget;
    // Playwright: `server/chromium/crConnection.ts` `CRSession.detach`: `await this._parentSession.send('Target.detachFromTarget', { sessionId: this._sessionId });`
    DetachFromTarget => "Target.detachFromTarget", OnBrowser, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._onAttachedToTarget`: `parentSession._sendMayFail('Target.detachFromTarget', { sessionId: event.sessionId });`
    DetachChild => "Target.detachFromTarget", OnSession, Ignored;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doClose`: `await this._browser._session.send('Target.disposeBrowserContext', { browserContextId: this._browserContextId });`
    DisposeBrowserContext => "Target.disposeBrowserContext", OnBrowser, Ignored;
    // Neo: a popup's opener, and whether a reported target is a page, which only the browser's own record of the target tells (`tabs.rs`).
    GetTargetInfo => "Target.getTargetInfo", OnBrowser, TargetInfoReply;
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
