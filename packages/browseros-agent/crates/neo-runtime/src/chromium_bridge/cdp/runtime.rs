//! The `Runtime` domain as neo-runtime uses it: calls into a frame's worlds and reading their
//! results, releasing what they returned, the page binding, resuming an auto-attached target,
//! and the execution context, console, exception and binding events.
//!
//! Neo: browseros-cdp's generated params, results and events, made from BrowserOS's pinned
//! protocol, under each method's name; a local type only where its doc says why.

use browseros_cdp::runtime as generated;
use generated::GetPropertiesResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Ignored, OnSession, command, event};

/// `Runtime.consoleAPICalled` and `Runtime.exceptionThrown`, decoded as
/// `browser_representations/console.rs` reads them. Not the generated events: their
/// `RemoteObject` cannot tell `"value": null` from no value, and a console message's text and a
/// page error's message depend on that (`console.rs` `RemoteObject`).
pub(crate) use crate::browser_representations::console::{ConsoleApiCalled, ExceptionThrown};
pub(crate) use generated::{
    AddBindingParams as AddBinding, BindingCalledEvent as BindingCalled, CallArgument,
    CallFunctionOnParams as CallFunctionOn,
    ExecutionContextCreatedEvent as ExecutionContextCreated,
    ExecutionContextDestroyedEvent as ExecutionContextDestroyed,
    ExecutionContextsClearedEvent as ExecutionContextsCleared,
    GetPropertiesParams as GetProperties, ReleaseObjectGroupParams as ReleaseObjectGroup,
    ReleaseObjectParams as ReleaseObject, RemoveBindingParams as RemoveBinding,
};

/// `Runtime.enable`, which Neo sends only for a run that listens to console messages or page
/// errors (or reads their history), or that exposes a binding. Local: it takes no params, and
/// browseros-cdp generates no params type for such a command.
#[derive(Serialize)]
pub(crate) struct Enable {}

/// `Runtime.runIfWaitingForDebugger`. Local: it takes no params ([`Enable`]).
#[derive(Serialize)]
pub(crate) struct RunIfWaitingForDebugger {}

/// The arguments of `Runtime.consoleAPICalled` as the page sent them, kept as JSON for
/// `ConsoleMessage.args()`. Local: re-encoding the generated `RemoteObject` would drop a
/// `"value": null` ([`ConsoleApiCalled`]).
#[derive(Deserialize)]
pub(crate) struct ConsoleArgs {
    #[serde(default)]
    pub(crate) args: Value,
}

command! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession.exposePlaywrightBinding`: `await this._client.send('Runtime.addBinding', { name: PageBinding.kBindingName });`
    AddBinding => "Runtime.addBinding", OnSession, Ignored;
    // Playwright: `server/chromium/crExecutionContext.ts` `CRExecutionContext.evaluateWithArguments`: `const { exceptionDetails, result: remoteObject } = await this._client.send('Runtime.callFunctionOn', {`
    // The reply stays JSON: its readers, the utility world and the main world, decode it after
    // their document check, so an answer that spans a commit reads as stale before it is read.
    CallFunctionOn => "Runtime.callFunctionOn", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Runtime.enable', {}),`
    Enable => "Runtime.enable", OnSession, Ignored;
    // Playwright: `server/chromium/crExecutionContext.ts` `CRExecutionContext.getProperties`: `const response = await this._client.send('Runtime.getProperties', {`
    GetProperties => "Runtime.getProperties", OnSession, GetPropertiesResult;
    // Playwright: `server/chromium/crProtocolHelper.ts` `releaseObject`: `await client.send('Runtime.releaseObject', { objectId }).catch(error => { });`
    ReleaseObject => "Runtime.releaseObject", OnSession, Ignored;
    // Neo: a call's handles live in one object group, and one release frees them all, its `getProperties` children included (probe P1).
    ReleaseObjectGroup => "Runtime.releaseObjectGroup", OnSession, Ignored;
    // Neo: takes the page binding off a session once the run's bindings end, so no later document gets it (`bindings.rs` `remove_binding`); Playwright's bindings last as long as their page.
    RemoveBinding => "Runtime.removeBinding", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Runtime.runIfWaitingForDebugger'));`
    RunIfWaitingForDebugger => "Runtime.runIfWaitingForDebugger", OnSession, Ignored;
}

event! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Runtime.bindingCalled', event => this._onBindingCalled(event)),`
    BindingCalled => "Runtime.bindingCalled";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Runtime.consoleAPICalled', event => this._onConsoleAPI(event)),`
    ConsoleApiCalled => "Runtime.consoleAPICalled";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._onConsoleAPI`: `const values = event.args.map(arg => createHandle(context, arg));`
    ConsoleArgs => "Runtime.consoleAPICalled";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Runtime.exceptionThrown', exception => this._handleException(exception.exceptionDetails)),`
    ExceptionThrown => "Runtime.exceptionThrown";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Runtime.executionContextCreated', event => this._onExecutionContextCreated(event.context)),`
    ExecutionContextCreated => "Runtime.executionContextCreated";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Runtime.executionContextDestroyed', event => this._onExecutionContextDestroyed(event.executionContextId)),`
    ExecutionContextDestroyed => "Runtime.executionContextDestroyed";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Runtime.executionContextsCleared', event => this._onExecutionContextsCleared()),`
    ExecutionContextsCleared => "Runtime.executionContextsCleared";
}
