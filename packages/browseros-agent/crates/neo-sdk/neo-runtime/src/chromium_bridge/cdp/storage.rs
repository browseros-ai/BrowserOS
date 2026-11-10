//! The `Storage` domain as neo-runtime uses it: the run's context's cookies, read and added
//! browser-wide (`browser_representations/cookies_and_storage.rs`).
//!
//! Neo: browseros-cdp's generated params and results, made from BrowserOS's pinned protocol,
//! under each method's name.

use browseros_cdp::storage as generated;
use generated::GetCookiesResult;

use super::{Ignored, OnBrowser, command};

pub(crate) use browseros_cdp::network::{Cookie, CookieParam};
pub(crate) use generated::{GetCookiesParams as GetCookies, SetCookiesParams as SetCookies};

command! {
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doGetCookies`: `const { cookies } = await this._browser._session.send('Storage.getCookies', { browserContextId: this._browserContextId });`
    GetCookies => "Storage.getCookies", OnBrowser, GetCookiesResult;
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.addCookies`: `await this._browser._session.send('Storage.setCookies', {`
    SetCookies => "Storage.setCookies", OnBrowser, Ignored;
}
