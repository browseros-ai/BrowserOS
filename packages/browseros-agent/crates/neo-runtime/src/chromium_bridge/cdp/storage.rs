//! The `Storage` domain as neo-runtime uses it: the run's context's cookies, read and added
//! browser-wide (`browser_representations/cookies_and_storage.rs`).
//!
//! Neo: local structs, since browseros-cdp's surface has no `Storage` domain. Cookies go out as
//! the JSON `cookies_and_storage.rs` builds, and come back as JSON it decodes.

use serde::Serialize;
use serde_json::Value;

use super::{Ignored, OnBrowser, command};

/// `Storage.getCookies` of the run's context, the default one when BrowserOS names none.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetCookies {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) browser_context_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetCookies {
    pub(crate) cookies: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) browser_context_id: Option<String>,
}

command! {
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doGetCookies`: `const { cookies } = await this._browser._session.send('Storage.getCookies', { browserContextId: this._browserContextId });`
    GetCookies => "Storage.getCookies", OnBrowser, Value;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.addCookies`: `await this._browser._session.send('Storage.setCookies', {`
    SetCookies => "Storage.setCookies", OnBrowser, Ignored;
}
