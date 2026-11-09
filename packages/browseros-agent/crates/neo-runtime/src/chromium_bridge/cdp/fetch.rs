//! The `Fetch` domain as neo-runtime uses it: pausing a session's requests for the run's routes,
//! each paused request's one answer, and a fetched response's body
//! (`browser_representations/routes.rs`, `page.rs`).
//!
//! Neo: local structs, since browseros-cdp's surface has no `Fetch` domain. A paused request is
//! read as JSON, as `routes.rs` reads it.

use neo_runtime_spec::Binary;
use neo_runtime_spec::serve::NameValue;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Ignored, OnSession, command, event};

/// `Fetch.continueRequest`: the paused request goes on, with whatever a route overrides, and
/// paused again at its response when a route fetches it.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContinueRequest {
    pub(crate) request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) headers: Option<Vec<NameValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) post_data: Option<Binary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) intercept_response: Option<bool>,
}

#[derive(Serialize)]
pub(crate) struct Disable {}

/// `Fetch.enable`: every request paused at its request stage.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Enable {
    pub(crate) handle_auth_requests: bool,
    pub(crate) patterns: Vec<RequestPattern>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestPattern {
    pub(crate) url_pattern: &'static str,
    pub(crate) request_stage: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FailRequest {
    pub(crate) request_id: String,
    pub(crate) error_reason: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FulfillRequest {
    pub(crate) request_id: String,
    pub(crate) response_code: i64,
    pub(crate) response_phrase: String,
    pub(crate) response_headers: Vec<NameValue>,
    /// Base64.
    pub(crate) body: String,
}

/// `Fetch.getResponseBody` of a response paused at its response stage. Its reply stays JSON.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetResponseBody {
    pub(crate) request_id: String,
}

/// `Fetch.requestPaused`, as JSON: the routes read the paused request's fields as they need
/// them, and keep a fetched response's pause to continue its redirect.
#[derive(Deserialize)]
#[serde(transparent)]
pub(crate) struct RequestPaused(pub(crate) Value);

command! {
    // Playwright: `server/chromium/crNetworkManager.ts` `RouteImpl.continue`: `await this._session.send('Fetch.continueRequest', this._alreadyContinuedParams);`
    ContinueRequest => "Fetch.continueRequest", OnSession, Ignored;
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._updateProtocolRequestInterceptionForSession`: `fetchPromise = info.session.send('Fetch.disable');`
    Disable => "Fetch.disable", OnSession, Ignored;
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._updateProtocolRequestInterceptionForSession`: `fetchPromise = info.session.send('Fetch.enable', { handleAuthRequests: true, patterns: [{ urlPattern: '*', requestStage: 'Request' }] });`
    Enable => "Fetch.enable", OnSession, Ignored;
    // Playwright: `server/chromium/crNetworkManager.ts` `RouteImpl.abort`: `await this._session.send('Fetch.failRequest', {`
    FailRequest => "Fetch.failRequest", OnSession, Ignored;
    // Playwright: `server/chromium/crNetworkManager.ts` `RouteImpl.fulfill`: `await this._session.send('Fetch.fulfillRequest', {`
    FulfillRequest => "Fetch.fulfillRequest", OnSession, Ignored;
    // Neo: `route.fetch()` reads the response where it paused, base64 (`routes.rs` `response_body`); Playwright's fetch goes through its own HTTP client instead.
    GetResponseBody => "Fetch.getResponseBody", OnSession, Value;
}

event! {
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `eventsHelper.addEventListener(session, 'Fetch.requestPaused', this._onRequestPaused.bind(this, sessionInfo)),`
    RequestPaused => "Fetch.requestPaused";
}
