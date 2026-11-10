//! The `Network` domain as neo-runtime uses it: turning a session's network reports on, a
//! context's extra headers and offline mode, request interception's cache, response bodies,
//! and the request events the network log and the page watcher read
//! (`browser_representations/network.rs`, `page.rs`).
//!
//! Neo: browseros-cdp's generated params, results and events, made from BrowserOS's pinned
//! protocol, under each method's name, except where headers cross: browseros-cdp generates
//! `Network.Headers`, a free-form object, as an empty struct that would drop every header, so the
//! commands and events that carry headers are local, holding them as JSON.

use browseros_cdp::network as generated;
use generated::{GetResponseBodyResult, LoadNetworkResourceResult};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{Ignored, OnSession, command, event};

pub(crate) use generated::{
    CookiePartitionKey, EmulateNetworkConditionsParams as EmulateNetworkConditions,
    EnableParams as Enable, GetResponseBodyParams as GetResponseBody, LoadNetworkResourceOptions,
    LoadNetworkResourceParams as LoadNetworkResource, LoadingFailedEvent as LoadingFailed,
    LoadingFinishedEvent as LoadingFinished, RequestServedFromCacheEvent as RequestServedFromCache,
    SetCacheDisabledParams as SetCacheDisabled,
};

/// `Network.setExtraHTTPHeaders`: CDP's `Network.Headers`, one object, each name once. Local:
/// headers cross ([module docs](self)).
#[derive(Serialize)]
pub(crate) struct SetExtraHttpHeaders {
    pub(crate) headers: Map<String, Value>,
}

/// `Network.requestWillBeSent`: a request, or a redirect's next hop with the response that
/// redirected it. Local: headers cross ([module docs](self)).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestWillBeSent {
    pub(crate) request_id: String,
    pub(crate) loader_id: String,
    #[serde(rename = "type", default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) frame_id: Option<String>,
    pub(crate) request: RequestPayload,
    pub(crate) timestamp: f64,
    pub(crate) wall_time: f64,
    #[serde(default)]
    pub(crate) redirect_response: Option<ResponsePayload>,
    #[serde(default)]
    pub(crate) redirect_has_extra_info: bool,
}

/// CDP's `Network.Request`, as far as the network log reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestPayload {
    pub(crate) url: String,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) headers: Map<String, Value>,
    #[serde(default)]
    pub(crate) post_data_entries: Option<Vec<PostDataEntry>>,
}

#[derive(Deserialize)]
pub(crate) struct PostDataEntry {
    #[serde(default)]
    pub(crate) bytes: Option<String>,
}

/// `Network.requestWillBeSentExtraInfo`: the headers the request actually went out with.
/// Local: headers cross ([module docs](self)).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestWillBeSentExtraInfo {
    pub(crate) request_id: String,
    #[serde(default)]
    pub(crate) headers: Map<String, Value>,
}

/// `Network.responseReceived`. Local: headers cross ([module docs](self)).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResponseReceived {
    pub(crate) request_id: String,
    pub(crate) response: ResponsePayload,
    #[serde(default)]
    pub(crate) has_extra_info: bool,
}

/// `Network.responseReceivedExtraInfo`: the headers the response actually came with. Local:
/// headers cross ([module docs](self)).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResponseReceivedExtraInfo {
    pub(crate) request_id: String,
    #[serde(default)]
    pub(crate) headers: Map<String, Value>,
    #[serde(default)]
    pub(crate) headers_text: Option<String>,
}

/// CDP's `Network.Response`, as far as the network log reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResponsePayload {
    pub(crate) status: i64,
    #[serde(default)]
    pub(crate) status_text: String,
    #[serde(default)]
    pub(crate) headers: Map<String, Value>,
    #[serde(default)]
    pub(crate) timing: Option<TimingPayload>,
    #[serde(default)]
    pub(crate) from_service_worker: bool,
    #[serde(default)]
    pub(crate) protocol: Option<String>,
    #[serde(rename = "remoteIPAddress", default)]
    pub(crate) remote_ip_address: Option<String>,
    #[serde(default)]
    pub(crate) remote_port: Option<i64>,
    #[serde(default)]
    pub(crate) security_details: Option<SecurityPayload>,
}

/// CDP's `Network.ResourceTiming`, as far as the network log reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimingPayload {
    pub(crate) request_time: f64,
    pub(crate) dns_start: f64,
    pub(crate) dns_end: f64,
    pub(crate) connect_start: f64,
    pub(crate) ssl_start: f64,
    pub(crate) connect_end: f64,
    pub(crate) send_start: f64,
    pub(crate) receive_headers_end: f64,
}

/// CDP's `Network.SecurityDetails`, as far as the network log reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SecurityPayload {
    #[serde(default)]
    pub(crate) protocol: Option<String>,
    #[serde(default)]
    pub(crate) subject_name: Option<String>,
    #[serde(default)]
    pub(crate) issuer: Option<String>,
    #[serde(default)]
    pub(crate) valid_from: Option<f64>,
    #[serde(default)]
    pub(crate) valid_to: Option<f64>,
}

command! {
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._setOfflineForSession`: `await info.session.send('Network.emulateNetworkConditions', {`
    EmulateNetworkConditions => "Network.emulateNetworkConditions", OnSession, Ignored;
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `session.send('Network.enable'),`
    Enable => "Network.enable", OnSession, Ignored;
    // Playwright: `server/chromium/crNetworkManager.ts` `InterceptableRequest.createResponseBodyCallback`: `const response = await session.send('Network.getResponseBody', { requestId: interceptable._requestId });`
    GetResponseBody => "Network.getResponseBody", OnSession, GetResponseBodyResult;
    // Playwright: `server/chromium/crNetworkManager.ts` `InterceptableRequest.createResponseBodyCallback`: `const resource = await session.send('Network.loadNetworkResource', { url: request.url(), frameId: request.serviceWorker() ? undefined : request.frame()!._id, options: { disableCache: false, includeCredentials: true } });`
    LoadNetworkResource => "Network.loadNetworkResource", OnSession, LoadNetworkResourceResult;
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._updateProtocolRequestInterceptionForSession`: `const cachePromise = info.session.send('Network.setCacheDisabled', { cacheDisabled: enabled });`
    SetCacheDisabled => "Network.setCacheDisabled", OnSession, Ignored;
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._setExtraHTTPHeadersForSession`: `await info.session.send('Network.setExtraHTTPHeaders', { headers: headersArrayToObject(this._extraHTTPHeaders, false /* lowerCase */) });`
    SetExtraHttpHeaders => "Network.setExtraHTTPHeaders", OnSession, Ignored;
}

event! {
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `eventsHelper.addEventListener(session, 'Network.loadingFailed', this._onLoadingFailed.bind(this, sessionInfo)),`
    LoadingFailed => "Network.loadingFailed";
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `eventsHelper.addEventListener(session, 'Network.loadingFinished', this._onLoadingFinished.bind(this, sessionInfo)),`
    LoadingFinished => "Network.loadingFinished";
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `eventsHelper.addEventListener(session, 'Network.requestServedFromCache', this._onRequestServedFromCache.bind(this)),`
    RequestServedFromCache => "Network.requestServedFromCache";
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `eventsHelper.addEventListener(session, 'Network.requestWillBeSent', this._onRequestWillBeSent.bind(this, sessionInfo)),`
    RequestWillBeSent => "Network.requestWillBeSent";
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `eventsHelper.addEventListener(session, 'Network.requestWillBeSentExtraInfo', this._onRequestWillBeSentExtraInfo.bind(this)),`
    RequestWillBeSentExtraInfo => "Network.requestWillBeSentExtraInfo";
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `eventsHelper.addEventListener(session, 'Network.responseReceived', this._onResponseReceived.bind(this, sessionInfo)),`
    ResponseReceived => "Network.responseReceived";
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `eventsHelper.addEventListener(session, 'Network.responseReceivedExtraInfo', this._onResponseReceivedExtraInfo.bind(this)),`
    ResponseReceivedExtraInfo => "Network.responseReceivedExtraInfo";
}
