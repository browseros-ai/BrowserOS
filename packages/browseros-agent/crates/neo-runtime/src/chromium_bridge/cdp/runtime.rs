//! The `Runtime` domain as neo-runtime uses it: calls into a frame's worlds and reading their
//! results, releasing what they returned, the page binding, resuming an auto-attached target,
//! and the execution context, console, exception and binding events.
//!
//! Neo: browseros-cdp's generated params under each method's name where its surface has them;
//! local structs where it does not. Replies stay JSON where their readers decode them after a
//! document check or with their own errors.

use browseros_cdp::runtime as generated;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Ignored, OnSession, command, event};

pub(crate) use crate::browser_representations::console::{ConsoleApiCalled, ExceptionThrown};
pub(crate) use generated::{
    GetPropertiesParams as GetProperties, ReleaseObjectGroupParams as ReleaseObjectGroup,
    ReleaseObjectParams as ReleaseObject,
};

/// `Runtime.callFunctionOn` of a built neo-playwright-injected function, on an object or in an
/// execution context. Local rather than browseros-cdp's generated params: the declaration is
/// borrowed, so a caller names its built const right at the field, where the page JavaScript
/// guard (`tests/page_js_guard.rs`) reads it. Each argument is the JSON CDP takes,
/// `{"value": …}` or `{"objectId": …}`.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CallFunctionOn<'a> {
    pub(crate) function_declaration: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) object_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) execution_context_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unique_context_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) arguments: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) return_by_value: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) await_promise: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) user_gesture: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) object_group: Option<&'a str>,
}

/// `Runtime.addBinding` of the page binding Playwright's `exposeBinding` calls through.
#[derive(Serialize)]
pub(crate) struct AddBinding {
    pub(crate) name: String,
}

/// `Runtime.enable`, which Neo sends only for a run that listens to console messages or page
/// errors (or reads their history), or that exposes a binding.
#[derive(Serialize)]
pub(crate) struct Enable {}

#[derive(Serialize)]
pub(crate) struct RemoveBinding {
    pub(crate) name: String,
}

#[derive(Serialize)]
pub(crate) struct RunIfWaitingForDebugger {}

/// `Runtime.bindingCalled`: the binding's name, the calling context and the call's payload.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BindingCalled {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) execution_context_id: Option<i64>,
    #[serde(default)]
    pub(crate) payload: Option<String>,
}

/// The arguments of `Runtime.consoleAPICalled` as the page sent them, kept as JSON for
/// `ConsoleMessage.args()`; [`ConsoleApiCalled`] is the same event, decoded.
#[derive(Deserialize)]
pub(crate) struct ConsoleArgs {
    #[serde(default)]
    pub(crate) args: Value,
}

/// `Runtime.executionContextCreated`: the context's id, and the frame and world its `auxData`
/// names.
#[derive(Deserialize)]
pub(crate) struct ExecutionContextCreated {
    #[serde(default)]
    pub(crate) context: ExecutionContextDescription,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecutionContextDescription {
    #[serde(default)]
    pub(crate) id: Option<i64>,
    #[serde(default)]
    pub(crate) unique_id: Option<String>,
    #[serde(default)]
    pub(crate) aux_data: AuxData,
}

/// What Chromium's `auxData` says of a context: its frame, and whether it is the frame's page
/// world.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuxData {
    #[serde(default)]
    pub(crate) frame_id: Option<String>,
    #[serde(default)]
    pub(crate) is_default: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecutionContextDestroyed {
    #[serde(default)]
    pub(crate) execution_context_id: Option<i64>,
}

/// `Runtime.executionContextsCleared`. Nothing of it is read.
#[derive(Deserialize)]
pub(crate) struct ExecutionContextsCleared {}

command! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession.exposePlaywrightBinding`: `await this._client.send('Runtime.addBinding', { name: PageBinding.kBindingName });`
    AddBinding => "Runtime.addBinding", OnSession, Ignored;
    // Playwright: `server/chromium/crExecutionContext.ts` `CRExecutionContext.evaluateWithArguments`: `const { exceptionDetails, result: remoteObject } = await this._client.send('Runtime.callFunctionOn', {`
    // The reply stays JSON: a world decodes it after its document check, so an answer that
    // spans a commit reads as stale before it is read at all.
    CallFunctionOn<'_> => "Runtime.callFunctionOn", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Runtime.enable', {}),`
    Enable => "Runtime.enable", OnSession, Ignored;
    // Playwright: `server/chromium/crExecutionContext.ts` `CRExecutionContext.getProperties`: `const response = await this._client.send('Runtime.getProperties', {`
    GetProperties => "Runtime.getProperties", OnSession, Value;
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
