//! The `Fetch` domain as neo-runtime uses it: pausing a session's requests for the run's routes,
//! each paused request's one answer, and a fetched response's body
//! (`browser_representations/routes.rs`, `page.rs`).
//!
//! Neo: browseros-cdp's generated params and results, made from BrowserOS's pinned protocol,
//! under each method's name; a local type only where its doc says why.

use browseros_cdp::fetch as generated;
use generated::GetResponseBodyResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Ignored, OnSession, command, event};

pub(crate) use generated::{
    ContinueRequestParams as ContinueRequest, EnableParams as Enable,
    FailRequestParams as FailRequest, FulfillRequestParams as FulfillRequest,
    GetResponseBodyParams as GetResponseBody, HeaderEntry, RequestPattern,
};

/// `Fetch.disable`. Local: it takes no params, and browseros-cdp generates no params type for
/// such a command.
#[derive(Serialize)]
pub(crate) struct Disable {}

/// `Fetch.requestPaused`, as JSON: the routes read the paused request's fields as they need
/// them. Local: the generated event's `Network.Request` holds its headers in browseros-cdp's
/// `Network.Headers`, an empty struct that would drop every header.
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
    GetResponseBody => "Fetch.getResponseBody", OnSession, GetResponseBodyResult;
}

event! {
    // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `eventsHelper.addEventListener(session, 'Fetch.requestPaused', this._onRequestPaused.bind(this, sessionInfo)),`
    RequestPaused => "Fetch.requestPaused";
}
