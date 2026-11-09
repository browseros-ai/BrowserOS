//! The `Network` domain as neo-runtime uses it: turning a session's network reports on, a
//! context's extra headers and offline mode, request interception's cache, response bodies,
//! and the request events the network log and the page watcher read
//! (`browser_representations/network.rs`, `page.rs`).
//!
//! Neo: local structs, since browseros-cdp's surface has no `Network` domain. Event payloads
//! hold the fields the network log reads, as optional as it reads them; replies stay JSON where
//! their readers decode them with their own errors.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{Ignored, OnSession, command, event};

/// `Network.emulateNetworkConditions`: offline or not, and no throttling (latency 0, throughput
/// -1), as Playwright's `CRNetworkManager._setOfflineForSession` sends it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmulateNetworkConditions {
    pub(crate) offline: bool,
    pub(crate) latency: i64,
    pub(crate) download_throughput: i64,
    pub(crate) upload_throughput: i64,
}

#[derive(Serialize)]
pub(crate) struct Enable {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetResponseBody {
    pub(crate) request_id: String,
}

/// `Network.loadNetworkResource`: an empty body read again, as Playwright's
/// `createResponseBodyCallback` refetches it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadNetworkResource {
    pub(crate) url: String,
    pub(crate) frame_id: String,
    pub(crate) options: LoadNetworkResourceOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadNetworkResourceOptions {
    pub(crate) disable_cache: bool,
    pub(crate) include_credentials: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetCacheDisabled {
    pub(crate) cache_disabled: bool,
}

/// `Network.setExtraHTTPHeaders`: CDP's `Network.Headers`, one object, each name once.
#[derive(Serialize)]
pub(crate) struct SetExtraHttpHeaders {
    pub(crate) headers: Map<String, Value>,
}

/// `Network.loadingFailed`: a request failed. `canceled` tells a navigation the frame dropped.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadingFailed {
    pub(crate) request_id: String,
    pub(crate) timestamp: f64,
    #[serde(default)]
    pub(crate) error_text: Option<String>,
    #[serde(default)]
    pub(crate) blocked_reason: Option<String>,
    #[serde(default)]
    pub(crate) canceled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadingFinished {
    pub(crate) request_id: String,
    pub(crate) timestamp: f64,
    pub(crate) encoded_data_length: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestServedFromCache {
    pub(crate) request_id: String,
}

/// `Network.requestWillBeSent`: a request, or a redirect's next hop with the response that
/// redirected it.
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestWillBeSentExtraInfo {
    pub(crate) request_id: String,
    #[serde(default)]
    pub(crate) headers: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResponseReceived {
    pub(crate) request_id: String,
    pub(crate) response: ResponsePayload,
    #[serde(default)]
    pub(crate) has_extra_info: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResponseReceivedExtraInfo {
    pub(crate) request_id: String,
    #[serde(default)]
    pub(crate) headers: Map<String, Value>,
    #[serde(default)]
    pub(crate) headers_text: Option<String>,
}

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
    GetResponseBody => "Network.getResponseBody", OnSession, Value;
    // Playwright: `server/chromium/crNetworkManager.ts` `InterceptableRequest.createResponseBodyCallback`: `const resource = await session.send('Network.loadNetworkResource', { url: request.url(), frameId: request.serviceWorker() ? undefined : request.frame()!._id, options: { disableCache: false, includeCredentials: true } });`
    LoadNetworkResource => "Network.loadNetworkResource", OnSession, Value;
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
