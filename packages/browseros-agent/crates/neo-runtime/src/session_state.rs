//! Neo: a run's session state, Playwright's `BrowserContext` cookies, storage state, extra HTTP
//! headers, geolocation, offline mode, HTTP credentials and permissions, on the user's own
//! browser profile, which is the run's context. What each touches, and for how long:
//!
//! ```text
//!  cookies, addCookies, clearCookies         the profile's cookies (Storage.*Cookies): browser data,
//!                                            kept after the run, as the user's own would be
//!  storageState, setStorageState             cookies and each origin's storage: browser data, kept
//!  setExtraHTTPHeaders, setOffline,          the run's sessions (Network.*, Emulation.*): a session
//!  setGeolocation, setHTTPCredentials        keeps them until it detaches, so they end with the run
//!  grantPermissions, clearPermissions        the context's permission overrides (Browser.*Permissions):
//!                                            reset when the last run that granted lets go
//! ```
//!
//! `clearCookies` matches its filter in the SDK, whose `RegExp` is the script's own
//! (neo-sdk `browser-context.ts`), and expires the matches through `addCookies`, as Playwright's
//! server does: the user's other cookies are never touched.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use browseros_cdp::{CdpClient, SessionId};
use neo_runtime_spec::serve::{
    BrowserContextSetGeolocationParamsGeolocation, BrowserContextStorageStateParams,
    BrowserContextStorageStateResult, NameValue, NetworkCookie, NetworkCookieSameSite,
    OriginStorage, SetNetworkCookie,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;
use tracing::debug;
use url::{Origin, Url};

use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::run::PageId;
use crate::tabs::{self, Tabs};

/// Playwright: `server/network.ts` `kMaxCookieExpiresDateInSeconds`: `export const kMaxCookieExpiresDateInSeconds = 253402300799;`
const MAX_COOKIE_EXPIRES_SECONDS: f64 = 253_402_300_799.0;

/// Playwright: `server/network.ts` `rewriteCookies`: `assert(c.url || (c.domain && c.path), 'Cookie should have a url or a domain/path pair');`
const COOKIE_NEEDS_URL_OR_DOMAIN: &str = "Cookie should have a url or a domain/path pair";

/// Playwright: `server/network.ts` `rewriteCookies`: `assert(!(c.url && c.domain), 'Cookie should have either url or domain');`
const COOKIE_URL_OR_DOMAIN: &str = "Cookie should have either url or domain";

/// Playwright: `server/network.ts` `rewriteCookies`: `assert(!(c.url && c.path), 'Cookie should have either url or path');`
const COOKIE_URL_OR_PATH: &str = "Cookie should have either url or path";

/// Playwright: `server/network.ts` `rewriteCookies`: `'Cookie should have a valid expires, only -1 or a positive number for the unix timestamp in seconds is allowed');`
const COOKIE_EXPIRES: &str = "Cookie should have a valid expires, only -1 or a positive number for the unix timestamp in seconds is allowed";

/// Neo: what `cookies(urls)` and `addCookies` throw for a URL that does not parse; Playwright's
/// server throws Node's `TypeError: Invalid URL`, which its client sees as an Error.
const INVALID_URL: &str = "Invalid URL";

/// CDP's `Network.Cookie`, as `Storage.getCookies` reports it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CdpCookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    expires: f64,
    http_only: bool,
    secure: bool,
    #[serde(default)]
    same_site: Option<NetworkCookieSameSite>,
    #[serde(default)]
    partition_key: Option<CdpPartitionKey>,
}

/// CDP's `Network.CookiePartitionKey`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CdpPartitionKey {
    top_level_site: String,
    has_cross_site_ancestor: bool,
}

#[derive(Deserialize)]
struct GotCookies {
    cookies: Vec<CdpCookie>,
}

/// `params` with the run's browser context, when BrowserOS names it; the default context
/// otherwise, which the browser-level `Storage` and `Browser` calls take without one.
fn in_context(mut params: Value, tabs: &Tabs) -> Value {
    if let Some(context) = tabs.context_id() {
        params["browserContextId"] = json!(context);
    }
    params
}

/// Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doGetCookies`: `const { cookies } = await this._browser._session.send('Storage.getCookies', { browserContextId: this._browserContextId });`
///
/// Every cookie of the run's context, the user's own included (the agent acts for the user), as
/// Playwright's `NetworkCookie`, then those that would go with a request to one of `urls` (all of
/// them for none).
pub(crate) async fn cookies(
    tabs: &Tabs,
    urls: &[String],
    progress: &Progress,
) -> Result<Vec<NetworkCookie>, CallError> {
    let urls = urls
        .iter()
        .map(|url| Url::parse(url).map_err(|_| PlaywrightError::error(INVALID_URL)))
        .collect::<Result<Vec<_>, _>>()?;
    let got = progress
        .race(tabs.cdp().send_typed::<_, GotCookies>(
            "Storage.getCookies",
            in_context(json!({}), tabs),
            None,
        ))
        .await?
        .map_err(|error| tabs::cdp_error(&error))?;
    Ok(got
        .cookies
        .into_iter()
        .map(network_cookie)
        .filter(|cookie| urls.is_empty() || urls.iter().any(|url| matches_url(cookie, url)))
        .collect())
}

/// Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doGetCookies`: `sameSite: sameSite ?? 'Lax',`
fn network_cookie(cookie: CdpCookie) -> NetworkCookie {
    let (partition_key, cr_has_cross_site_ancestor) =
        cookie.partition_key.map_or((None, None), |key| {
            (Some(key.top_level_site), Some(key.has_cross_site_ancestor))
        });
    NetworkCookie {
        name: cookie.name,
        value: cookie.value,
        domain: cookie.domain,
        path: cookie.path,
        expires: cookie.expires,
        http_only: cookie.http_only,
        secure: cookie.secure,
        same_site: cookie.same_site.unwrap_or(NetworkCookieSameSite::Lax),
        partition_key,
        cr_has_cross_site_ancestor,
    }
}

/// Playwright: `server/network.ts` `cookieMatchesURL`: `if (cookie.secure && url.protocol !== 'https:' && !isLocalHostname(url.hostname))`
fn matches_url(cookie: &NetworkCookie, url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    domain_matches(host, &cookie.domain)
        && path_matches(url.path(), &cookie.path)
        && !(cookie.secure && url.scheme() != "https" && !is_local_hostname(host))
}

/// Playwright: `server/network.ts` `domainMatches`: `return ('.' + hostname).endsWith(cookieDomain);`
fn domain_matches(hostname: &str, cookie_domain: &str) -> bool {
    hostname == cookie_domain
        || (cookie_domain.starts_with('.') && format!(".{hostname}").ends_with(cookie_domain))
}

/// Playwright: `server/network.ts` `pathMatches`: `return cookiePath.endsWith('/') || requestPath[cookiePath.length] === '/';`
fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    request_path == cookie_path
        || (request_path.starts_with(cookie_path)
            && (cookie_path.ends_with('/')
                || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')))
}

/// Playwright: `server/network.ts` `isLocalHostname`: `return hostname === 'localhost' || hostname.endsWith('.localhost') || hostname === '[::1]' || /^127\.\d+\.\d+\.\d+$/.test(hostname);`
fn is_local_hostname(hostname: &str) -> bool {
    hostname == "localhost"
        || hostname.ends_with(".localhost")
        || hostname == "[::1]"
        || hostname.strip_prefix("127.").is_some_and(|rest| {
            let parts: Vec<&str> = rest.split('.').collect();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        })
}

/// Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.addCookies`: `await this._browser._session.send('Storage.setCookies', {`
///
/// Adds or replaces each cookie in the run's context, where it stays: cookies are browser data.
pub(crate) async fn add_cookies(
    tabs: &Tabs,
    cookies: Vec<SetNetworkCookie>,
    progress: &Progress,
) -> Result<(), CallError> {
    let cookies = cookies
        .into_iter()
        .map(|cookie| rewrite_cookie(cookie).map(|cookie| chromium_cookie(&cookie)))
        .collect::<Result<Vec<_>, _>>()?;
    progress
        .race(tabs.cdp().send(
            "Storage.setCookies",
            in_context(json!({ "cookies": cookies }), tabs),
            None,
        ))
        .await?
        .map_err(|error| tabs::cdp_error(&error))?;
    Ok(())
}

/// Playwright: `server/network.ts` `rewriteCookies`: `copy.path = url.pathname.substring(0, url.pathname.lastIndexOf('/') + 1);`
///
/// Playwright's checks of one cookie to add, and a `url` turned into its domain, path and
/// `secure`.
#[expect(
    clippy::float_cmp,
    reason = "Playwright compares expires to 0 and -1 exactly"
)]
fn rewrite_cookie(mut cookie: SetNetworkCookie) -> Result<SetNetworkCookie, PlaywrightError> {
    let has_domain_and_path = cookie
        .domain
        .as_ref()
        .is_some_and(|domain| !domain.is_empty())
        && cookie.path.as_ref().is_some_and(|path| !path.is_empty());
    let url = cookie.url.clone().filter(|url| !url.is_empty());
    if url.is_none() && !has_domain_and_path {
        return Err(PlaywrightError::error(COOKIE_NEEDS_URL_OR_DOMAIN));
    }
    if url.is_some()
        && cookie
            .domain
            .as_ref()
            .is_some_and(|domain| !domain.is_empty())
    {
        return Err(PlaywrightError::error(COOKIE_URL_OR_DOMAIN));
    }
    if url.is_some() && cookie.path.as_ref().is_some_and(|path| !path.is_empty()) {
        return Err(PlaywrightError::error(COOKIE_URL_OR_PATH));
    }
    if let Some(expires) = cookie.expires
        && expires != 0.0
        && ((expires < 0.0 && expires != -1.0) || expires > MAX_COOKIE_EXPIRES_SECONDS)
    {
        return Err(PlaywrightError::error(COOKIE_EXPIRES));
    }
    if let Some(url) = url {
        if url == "about:blank" {
            return Err(PlaywrightError::error(format!(
                "Blank page can not have cookie \"{}\"",
                cookie.name
            )));
        }
        if url.starts_with("data:") {
            return Err(PlaywrightError::error(format!(
                "Data URL page can not have cookie \"{}\"",
                cookie.name
            )));
        }
        let parsed = Url::parse(&url).map_err(|_| PlaywrightError::error(INVALID_URL))?;
        let path = parsed.path();
        cookie.domain = Some(parsed.host_str().unwrap_or_default().to_string());
        cookie.path = Some(path[..=path.rfind('/').unwrap_or(0)].to_string());
        cookie.secure = Some(parsed.scheme() == "https");
    }
    Ok(cookie)
}

/// Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.addCookies`: `hasCrossSiteAncestor: _crHasCrossSiteAncestor ?? true,`
///
/// One cookie as CDP's `Network.CookieParam`.
fn chromium_cookie(cookie: &SetNetworkCookie) -> Value {
    let mut param = json!({ "name": cookie.name, "value": cookie.value });
    let fields = [
        ("url", cookie.url.as_ref().map(|url| json!(url))),
        ("domain", cookie.domain.as_ref().map(|domain| json!(domain))),
        ("path", cookie.path.as_ref().map(|path| json!(path))),
        ("expires", cookie.expires.map(|expires| json!(expires))),
        (
            "httpOnly",
            cookie.http_only.map(|http_only| json!(http_only)),
        ),
        ("secure", cookie.secure.map(|secure| json!(secure))),
        (
            "sameSite",
            cookie
                .same_site
                .as_ref()
                .and_then(|same_site| serde_json::to_value(same_site).ok()),
        ),
    ];
    for (name, value) in fields {
        if let Some(value) = value {
            param[name] = value;
        }
    }
    if let Some(site) = &cookie.partition_key {
        param["partitionKey"] = json!({
            "topLevelSite": site,
            "hasCrossSiteAncestor": cookie.cr_has_cross_site_ancestor.unwrap_or(true),
        });
    }
    param
}

/// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._setOfflineForSession`: `// values of 0 remove any active throttling. crbug.com/456324#c9`
fn network_conditions(offline: bool) -> Value {
    json!({ "offline": offline, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1 })
}

/// The run's session-scoped state: what each of its sessions gets, the tabs' and their
/// out-of-process frames', those attached later included. CDP keeps each on the session that
/// set it until that session detaches, so all of it ends with the run.
#[derive(Default)]
pub(crate) struct Overrides(Mutex<State>);

#[derive(Default)]
struct State {
    /// The context's extra HTTP headers (`context.setExtraHTTPHeaders`).
    headers: Vec<NameValue>,
    /// Each page's own (`page.setExtraHTTPHeaders`), over the context's.
    page_headers: HashMap<PageId, Vec<NameValue>>,
    offline: bool,
    geolocation: Option<BrowserContextSetGeolocationParamsGeolocation>,
}

impl State {
    /// Playwright: `server/chromium/crPage.ts` `CRPage.updateExtraHTTPHeaders`: `const headers = network.mergeHeaders([`
    fn headers_of(&self, page: &PageId) -> Vec<NameValue> {
        merge_headers(&[
            self.headers.as_slice(),
            self.page_headers.get(page).map_or(&[][..], Vec::as_slice),
        ])
    }
}

/// Playwright: `server/network.ts` `mergeHeaders`: `lowerCaseToOriginalCase.set(lower, name);`
///
/// Later lists win, by name in any case; a name keeps the case it was last given in and the
/// place it first came in.
fn merge_headers(lists: &[&[NameValue]]) -> Vec<NameValue> {
    let mut merged: Vec<NameValue> = Vec::new();
    for header in lists.iter().flat_map(|list| list.iter()) {
        match merged
            .iter_mut()
            .find(|kept| kept.name.eq_ignore_ascii_case(&header.name))
        {
            Some(kept) => kept.clone_from(header),
            None => merged.push(header.clone()),
        }
    }
    merged
}

/// Playwright: `server/browserContext.ts` `verifyGeolocation`: `geolocation.accuracy = geolocation.accuracy || 0;`
fn verify_geolocation(
    geolocation: &BrowserContextSetGeolocationParamsGeolocation,
) -> Result<Value, PlaywrightError> {
    let accuracy = geolocation.accuracy.unwrap_or(0.0);
    if !(-180.0..=180.0).contains(&geolocation.longitude) {
        return Err(PlaywrightError::error(
            "geolocation.longitude: precondition -180 <= LONGITUDE <= 180 failed.",
        ));
    }
    if !(-90.0..=90.0).contains(&geolocation.latitude) {
        return Err(PlaywrightError::error(
            "geolocation.latitude: precondition -90 <= LATITUDE <= 90 failed.",
        ));
    }
    if accuracy < 0.0 {
        return Err(PlaywrightError::error(
            "geolocation.accuracy: precondition 0 <= ACCURACY failed.",
        ));
    }
    Ok(json!({
        "latitude": geolocation.latitude,
        "longitude": geolocation.longitude,
        "accuracy": accuracy,
    }))
}

/// CDP's `Network.Headers`: one object, each name once.
fn headers_object(headers: &[NameValue]) -> Value {
    let mut object = Map::new();
    for header in headers {
        object.insert(header.name.clone(), json!(header.value));
    }
    Value::Object(object)
}

/// One CDP session of the run: a tab's own, or one of its out-of-process frames'.
struct Target<'a> {
    cdp: &'a CdpClient,
    session: SessionId,
}

impl Target<'_> {
    async fn send(&self, method: &str, params: Value) -> Result<(), PlaywrightError> {
        self.cdp
            .send(method, params, Some(&self.session))
            .await
            .map(|_| ())
            .map_err(|error| tabs::cdp_error(&error))
    }

    /// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.addSession`: `session.send('Network.enable'),`
    async fn headers(&self, headers: &[NameValue]) -> Result<(), PlaywrightError> {
        self.send("Network.enable", json!({})).await?;
        self.send(
            "Network.setExtraHTTPHeaders",
            json!({ "headers": headers_object(headers) }),
        )
        .await
    }

    async fn offline(&self, offline: bool) -> Result<(), PlaywrightError> {
        self.send("Network.enable", json!({})).await?;
        self.send(
            "Network.emulateNetworkConditions",
            network_conditions(offline),
        )
        .await
    }

    /// Playwright: `server/chromium/crPage.ts` `FrameSession._updateGeolocation`: `await this._client.send('Emulation.setGeolocationOverride', geolocation || {});`
    async fn geolocation(&self, geolocation: Option<&Value>) -> Result<(), PlaywrightError> {
        self.send(
            "Emulation.setGeolocationOverride",
            geolocation.cloned().unwrap_or_else(|| json!({})),
        )
        .await
    }
}

/// Every session of a tab the run attached: its own, then its out-of-process frames'.
fn targets_of(page: &PageSession) -> Vec<Target<'_>> {
    let cdp = page.events.cdp();
    page.events
        .sessions()
        .into_iter()
        .map(|session| Target { cdp, session })
        .collect()
}

impl Overrides {
    /// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.setExtraHTTPHeaders`: `if (!this._extraHTTPHeaders.length && !extraHTTPHeaders.length)`
    ///
    /// The context's extra headers, on every session of every tab the run has, each with its
    /// page's own over them.
    pub(crate) async fn set_context_headers(
        &self,
        tabs: &Tabs,
        headers: Vec<NameValue>,
        progress: &Progress,
    ) -> Result<(), CallError> {
        let mut state = self.0.lock().await;
        state.headers = headers;
        for (entry, page) in tabs.attached() {
            let merged = state.headers_of(&entry.id);
            for target in targets_of(&page) {
                progress.race(target.headers(&merged)).await??;
            }
        }
        Ok(())
    }

    /// One page's own extra headers, over the context's, on every session of that tab.
    pub(crate) async fn set_page_headers(
        &self,
        page: &PageSession,
        id: &PageId,
        headers: Vec<NameValue>,
        progress: &Progress,
    ) -> Result<(), CallError> {
        let mut state = self.0.lock().await;
        state.page_headers.insert(id.clone(), headers);
        let merged = state.headers_of(id);
        for target in targets_of(page) {
            progress.race(target.headers(&merged)).await??;
        }
        Ok(())
    }

    /// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager.setOffline`: `if (offline === this._offline)`
    pub(crate) async fn set_offline(
        &self,
        tabs: &Tabs,
        offline: bool,
        progress: &Progress,
    ) -> Result<(), CallError> {
        let mut state = self.0.lock().await;
        if state.offline == offline {
            return Ok(());
        }
        state.offline = offline;
        for (_, page) in tabs.attached() {
            for target in targets_of(&page) {
                progress.race(target.offline(offline)).await??;
            }
        }
        Ok(())
    }

    /// Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.setGeolocation`: `verifyGeolocation(geolocation);`
    pub(crate) async fn set_geolocation(
        &self,
        tabs: &Tabs,
        geolocation: Option<BrowserContextSetGeolocationParamsGeolocation>,
        progress: &Progress,
    ) -> Result<(), CallError> {
        let checked = geolocation.as_ref().map(verify_geolocation).transpose()?;
        let mut state = self.0.lock().await;
        state.geolocation = geolocation;
        for (_, page) in tabs.attached() {
            for target in targets_of(&page) {
                progress
                    .race(target.geolocation(checked.as_ref()))
                    .await??;
            }
        }
        Ok(())
    }

    /// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._setOfflineForSession`: `if (initial && !this._offline)`
    ///
    /// A session the run attached just now, a tab's or an out-of-process frame's of tab `page`:
    /// it gets what the run set so far, before the call that attached it goes on. A failure is
    /// logged; that session's later requests go without it.
    pub(crate) async fn apply_to_new(&self, cdp: &CdpClient, session: SessionId, page: &PageId) {
        let state = self.0.lock().await;
        let target = Target { cdp, session };
        let headers = state.headers_of(page);
        let geolocation = state
            .geolocation
            .as_ref()
            .and_then(|geolocation| verify_geolocation(geolocation).ok());
        let applied = async {
            if !headers.is_empty() {
                target.headers(&headers).await?;
            }
            if state.offline {
                target.offline(true).await?;
            }
            if let Some(geolocation) = &geolocation {
                target.geolocation(Some(geolocation)).await?;
            }
            Ok::<_, PlaywrightError>(())
        };
        if let Err(error) = applied.await {
            debug!(?error, "a new session did not take the run's session state");
        }
    }
}

/// Deviation: `server/chromium/crNetworkManager.ts` `CRNetworkManager.authenticate`: `this._credentials = credentials;`
/// Playwright answers a server's credentials challenge through `Fetch` request interception,
/// which the run's routes own (pw-net-network B3, building); until that layer can answer
/// `Fetch.authRequired`, Neo refuses `setHTTPCredentials` rather than send none.
const CREDENTIALS_PENDING: &str = "Neo does not serve setHTTPCredentials yet: it answers a server's credentials challenge through request interception, which Neo's routes do not offer yet";

/// `setHTTPCredentials`: refused until the routes' `Fetch` layer answers credential challenges
/// ([`CREDENTIALS_PENDING`]).
pub(crate) fn set_http_credentials() -> Result<(), CallError> {
    Err(PlaywrightError::error(CREDENTIALS_PENDING).into())
}

/// Neo: what `storageState` throws for what Neo does not read yet: `IndexedDB`, OPFS and `WebAuthn`
/// credentials.
const STORAGE_OPTION_PENDING: &str = "Neo reads cookies and localStorage only: storageState({ indexedDB, opfs, credentials }) is not served yet";

/// Deviation: `server/browserContext.ts` `BrowserContext.setStorageState`: `await progress.race(this.doClearCookies());`
/// Playwright first clears the browser's whole HTTP cache and every cookie, and the storage of
/// each origin it saw, then writes the state. Neo's context is the user's browser, where that
/// would sign the user out of every site and lose their sites' data, so Neo refuses it.
const SET_STORAGE_STATE_REFUSED: &str = "Neo does not set a storage state on the user's browser: it clears every cookie and each visited origin's storage first, which would sign the user out of every site";

/// `setStorageState`: refused on the user's browser ([`SET_STORAGE_STATE_REFUSED`]).
pub(crate) fn set_storage_state() -> Result<(), CallError> {
    Err(PlaywrightError::error(SET_STORAGE_STATE_REFUSED).into())
}

/// Deviation: `server/browserContext.ts` `BrowserContext.storageState`: `// If there are still origins to save, create a blank page to iterate over origins.`
/// Playwright reads an origin it saw but has no page on by visiting it in a new tab, every
/// request answered by a stub. That needs request interception, which Neo's routes do not offer
/// yet, so Neo reads only the origins of the run's open pages.
///
/// Every cookie of the run's context, and the `localStorage` of each origin the run's pages
/// navigated to whose page is still open there, read in place (`DOMStorage`): Playwright's
/// state, as far as Neo can read it without loading a page.
pub(crate) async fn storage_state(
    tabs: &Tabs,
    params: &BrowserContextStorageStateParams,
    progress: &Progress,
) -> Result<BrowserContextStorageStateResult, CallError> {
    if [params.indexed_db, params.opfs, params.credentials].contains(&Some(true)) {
        return Err(PlaywrightError::error(STORAGE_OPTION_PENDING).into());
    }
    let cookies = cookies(tabs, &[], progress).await?;
    let attached = tabs.attached();
    let mut visited: Vec<String> = Vec::new();
    for (_, page) in &attached {
        for origin in page.events.visited_origins() {
            if !visited.contains(&origin) {
                visited.push(origin);
            }
        }
    }
    let mut origins: Vec<OriginStorage> = Vec::new();
    for (entry, page) in &attached {
        let Some(url) = progress.race(tabs.url(entry)).await?? else {
            continue;
        };
        let Some(origin) = Url::parse(&url)
            .ok()
            .map(|url| url.origin())
            .filter(Origin::is_tuple)
            .map(|origin| origin.ascii_serialization())
        else {
            continue;
        };
        if !visited.contains(&origin) || origins.iter().any(|kept| kept.origin == origin) {
            continue;
        }
        let local_storage = progress.race(local_storage(page, &origin)).await??;
        // Playwright: `server/browserContext.ts` `BrowserContext.storageState`: `const hasStorage = (storage: SerializedStorage) => !!(storage.localStorage.length || storage.indexedDB?.length || storage.opfs?.length);`
        if !local_storage.is_empty() {
            origins.push(OriginStorage {
                origin,
                local_storage,
                indexed_db: None,
                opfs: None,
            });
        }
    }
    Ok(BrowserContextStorageStateResult {
        cookies,
        origins,
        credentials: None,
    })
}

#[derive(Deserialize)]
struct StorageItems {
    entries: Vec<(String, String)>,
}

/// `origin`'s `localStorage`, read over the tab's own session, where it has a frame.
async fn local_storage(
    page: &PageSession,
    origin: &str,
) -> Result<Vec<NameValue>, PlaywrightError> {
    let cdp = page.events.cdp();
    let session = page.events.sessions().into_iter().next();
    let send = |method: &'static str, params: Value| {
        let session = session.clone();
        async move {
            cdp.send(method, params, session.as_ref())
                .await
                .map_err(|error| tabs::cdp_error(&error))
        }
    };
    send("DOMStorage.enable", json!({})).await?;
    let items = send(
        "DOMStorage.getDOMStorageItems",
        json!({ "storageId": { "securityOrigin": origin, "isLocalStorage": true } }),
    )
    .await?;
    let StorageItems { entries } =
        serde_json::from_value(items).map_err(|error| PlaywrightError::error(error.to_string()))?;
    Ok(entries
        .into_iter()
        .map(|(name, value)| NameValue { name, value })
        .collect())
}

/// Neo: each run's id among the process's runs, for the permission grants it holds.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

/// A new run's id ([`Permissions`]).
pub(crate) fn next_run() -> u64 {
    NEXT_RUN.fetch_add(1, Ordering::Relaxed)
}

/// Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doGrantPermissions`: `['local-network-access', ['localNetworkAccess', 'localNetwork', 'loopbackNetwork']],`
///
/// A web permission's CDP `Browser.PermissionType`s, or `None` for a name Playwright does not
/// know. `fallback`: the mapping Playwright retries with on a browser that refuses the first.
fn protocol_permission(name: &str, fallback: bool) -> Option<&'static [&'static str]> {
    Some(match name {
        "geolocation" => &["geolocation"],
        "midi" => &["midi"],
        "notifications" => &["notifications"],
        "camera" => &["videoCapture"],
        "microphone" => &["audioCapture"],
        "background-sync" => &["backgroundSync"],
        "ambient-light-sensor" | "accelerometer" | "gyroscope" | "magnetometer" => &["sensors"],
        "clipboard-read" => &["clipboardReadWrite"],
        "clipboard-write" => &["clipboardSanitizedWrite"],
        "payment-handler" => &["paymentHandler"],
        "midi-sysex" => &["midiSysex"],
        "storage-access" => &["storageAccess"],
        "local-fonts" => &["localFonts"],
        "local-network-access" if fallback => &["localNetworkAccess"],
        "local-network-access" => &["localNetworkAccess", "localNetwork", "loopbackNetwork"],
        "screen-wake-lock" => &["wakeLockScreen"],
        _ => return None,
    })
}

/// Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doGrantPermissions`: `throw new Error('Unknown permission: ' + permission);`
const UNKNOWN_PERMISSION: &str = "Unknown permission: ";

/// Playwright: `server/browserContext.ts` `BrowserContext.grantPermissions`: `resolvedOrigin = url.origin;`
///
/// The origin a grant names (`*` for every origin), as Playwright resolves it.
fn grant_origin(origin: Option<&str>) -> Result<String, PlaywrightError> {
    let Some(origin) = origin else {
        return Ok("*".to_string());
    };
    let url = Url::parse(origin).map_err(|_| PlaywrightError::error(INVALID_URL))?;
    Ok(url.origin().ascii_serialization())
}

/// The runs' permission grants, the `Runtime`'s: on the user's profile a grant is an override of
/// its browser context, which every run and client of the browser shares, so Neo keeps each
/// run's own and gives the browser their union, origin by origin. A run's grants go with its
/// `clearPermissions` and with its end; once no run of the context holds one, its overrides are
/// reset, back to the user's own settings.
#[derive(Clone)]
pub(crate) struct Permissions {
    /// By run id: the earliest run first, so the browser gets each origin's list in that order.
    grants: Arc<Mutex<BTreeMap<u64, RunGrants>>>,
    /// The `Runtime`'s cleanup tracker, for a run dropped before it let go.
    cleanup: TaskTracker,
}

impl Default for Permissions {
    fn default() -> Self {
        Self::new(TaskTracker::new())
    }
}

/// One run's grants: its browser context, and by origin the web permissions it granted there.
#[derive(Default)]
struct RunGrants {
    context: Option<String>,
    origins: BTreeMap<String, Vec<String>>,
}

/// Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doGrantPermissions`: `// Old stable browsers dislike the new permission name, so we use the fallback mapping.`
///
/// `Browser.grantPermissions` for every web permission the context's runs granted on `origin`.
async fn grant_union(
    cdp: &CdpClient,
    grants: &BTreeMap<u64, RunGrants>,
    context: Option<&str>,
    origin: &str,
) -> Result<(), PlaywrightError> {
    let mut names: Vec<&str> = Vec::new();
    for run in grants
        .values()
        .filter(|run| run.context.as_deref() == context)
    {
        for name in run.origins.get(origin).into_iter().flatten() {
            if !names.contains(&name.as_str()) {
                names.push(name);
            }
        }
    }
    let grant = |fallback: bool| {
        let mut permissions: Vec<&str> = Vec::new();
        for name in &names {
            for permission in protocol_permission(name, fallback).unwrap_or(&[]) {
                if !permissions.contains(permission) {
                    permissions.push(permission);
                }
            }
        }
        let mut params = json!({ "permissions": permissions });
        if origin != "*" {
            params["origin"] = json!(origin);
        }
        if let Some(context) = context {
            params["browserContextId"] = json!(context);
        }
        params
    };
    match cdp
        .send("Browser.grantPermissions", grant(false), None)
        .await
    {
        Ok(_) => Ok(()),
        Err(_) => cdp
            .send("Browser.grantPermissions", grant(true), None)
            .await
            .map(|_| ())
            .map_err(|error| tabs::cdp_error(&error)),
    }
}

/// Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doClearPermissions`: `await this._browser._session.send('Browser.resetPermissions', { browserContextId: this._browserContextId });`
///
/// Resets the context's overrides, then grants again what its other runs hold.
async fn reset_context(
    cdp: &CdpClient,
    grants: &BTreeMap<u64, RunGrants>,
    context: Option<&str>,
) -> Result<(), PlaywrightError> {
    let mut params = json!({});
    if let Some(context) = context {
        params["browserContextId"] = json!(context);
    }
    cdp.send("Browser.resetPermissions", params, None)
        .await
        .map_err(|error| tabs::cdp_error(&error))?;
    let origins: Vec<String> = grants
        .values()
        .filter(|run| run.context.as_deref() == context)
        .flat_map(|run| run.origins.keys().cloned())
        .collect();
    let mut granted: Vec<String> = Vec::new();
    for origin in origins {
        if !granted.contains(&origin) {
            grant_union(cdp, grants, context, &origin).await?;
            granted.push(origin);
        }
    }
    Ok(())
}

impl Permissions {
    pub(crate) fn new(cleanup: TaskTracker) -> Self {
        Self {
            grants: Arc::default(),
            cleanup,
        }
    }

    /// Playwright: `server/browserContext.ts` `BrowserContext.grantPermissions`: `permissions.forEach(p => existing.add(p));`
    ///
    /// Run `run`'s grant of `permissions` on `origin` (every origin for none), added to what it
    /// granted there before; the browser gets every run's on that origin.
    pub(crate) async fn grant(
        &self,
        tabs: &Tabs,
        run: u64,
        origin: Option<&str>,
        permissions: &[String],
        progress: &Progress,
    ) -> Result<(), CallError> {
        if let Some(unknown) = permissions
            .iter()
            .find(|name| protocol_permission(name, false).is_none())
        {
            return Err(PlaywrightError::error(format!("{UNKNOWN_PERMISSION}{unknown}")).into());
        }
        let origin = grant_origin(origin)?;
        let context = tabs.context_id().map(str::to_string);
        let mut grants = self.grants.lock().await;
        let held = grants.entry(run).or_default();
        held.context.clone_from(&context);
        let list = held.origins.entry(origin.clone()).or_default();
        for name in permissions {
            if !list.contains(name) {
                list.push(name.clone());
            }
        }
        progress
            .race(grant_union(
                tabs.cdp(),
                &grants,
                context.as_deref(),
                &origin,
            ))
            .await??;
        Ok(())
    }

    /// Playwright: `server/browserContext.ts` `BrowserContext.clearPermissions`: `this._permissions.clear();`
    ///
    /// Run `run`'s `clearPermissions`: its grants go; the other runs' stay. A run that granted
    /// nothing changes nothing, so the user's browser keeps every other client's overrides.
    pub(crate) async fn clear(
        &self,
        tabs: &Tabs,
        run: u64,
        progress: &Progress,
    ) -> Result<(), CallError> {
        let mut grants = self.grants.lock().await;
        let Some(held) = grants.remove(&run) else {
            return Ok(());
        };
        progress
            .race(reset_context(tabs.cdp(), &grants, held.context.as_deref()))
            .await??;
        Ok(())
    }

    /// The run's end: its grants go, as [`Self::clear`]; a failure is logged.
    pub(crate) async fn release(&self, cdp: &CdpClient, run: u64) {
        let mut grants = self.grants.lock().await;
        let Some(held) = grants.remove(&run) else {
            return;
        };
        if let Err(error) = reset_context(cdp, &grants, held.context.as_deref()).await {
            debug!(?error, "the run's permission grants were not reset");
        }
    }

    /// [`Self::release`] on the `Runtime`'s cleanup tracker, for a run whose future was dropped.
    pub(crate) fn release_later(&self, cdp: CdpClient, run: u64) {
        let permissions = self.clone();
        self.cleanup
            .spawn(async move { permissions.release(&cdp, run).await });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::runtime::Handle;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::deadline::Deadline;
    use crate::discovery::Discovery;
    use crate::fake_browser::{FakeBrowser, PAGE, Replies};
    use crate::page::TabInputs;
    use crate::progress::{CallEntry, CallStart, RunScope};

    fn call_progress() -> Progress {
        let entry = Arc::new(CallEntry::new(CallStart {
            call_id: 1,
            guid: "context@default".to_string(),
            method: "grantPermissions".to_string(),
            title: None,
            params: json!({}),
            location: None,
            input: false,
        }));
        let run = RunScope::new(
            Deadline::new(Duration::from_mins(1), CancellationToken::new()),
            TaskTracker::new(),
        );
        Progress::new(entry, 0.0, run, CancellationToken::new())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tests_that_each_runs_grants_add_up_and_reset_once_no_run_holds_one() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let (tabs, _) = Tabs::open(
            browser.cdp().await,
            Handle::current(),
            TabInputs::default(),
            Discovery::default(),
            Permissions::default(),
            TaskTracker::new(),
            &PageId::new(PAGE.to_string()),
        )
        .await
        .unwrap_or_else(|error| panic!("open: {error:?}"));
        let permissions = tabs.permissions().clone();
        let progress = call_progress();
        let names = |names: &[&str]| {
            names
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>()
        };
        let unknown = permissions
            .grant(&tabs, 1, None, &names(&["telepathy"]), &progress)
            .await;
        assert!(unknown.is_err(), "an unknown permission, before any grant");
        let ok = |result: Result<(), CallError>| result.unwrap_or_else(|error| panic!("{error:?}"));
        ok(permissions
            .grant(
                &tabs,
                1,
                Some("http://a.test/page"),
                &names(&["geolocation"]),
                &progress,
            )
            .await);
        ok(permissions
            .grant(
                &tabs,
                2,
                Some("http://a.test"),
                &names(&["notifications"]),
                &progress,
            )
            .await);
        ok(permissions.clear(&tabs, 3, &progress).await);
        ok(permissions.clear(&tabs, 1, &progress).await);
        permissions.release(tabs.cdp(), 2).await;
        let sent: Vec<Value> = browser
            .exchanges()
            .into_iter()
            .filter(|exchange| exchange.method().ends_with("Permissions"))
            .map(|exchange| {
                let params = &exchange.command["params"];
                json!([exchange.method(), params["origin"], params["permissions"]])
            })
            .collect();
        assert_eq!(
            sent,
            [
                json!(["Browser.grantPermissions", "http://a.test", ["geolocation"]]),
                json!([
                    "Browser.grantPermissions",
                    "http://a.test",
                    ["geolocation", "notifications"]
                ]),
                json!(["Browser.resetPermissions", null, null]),
                json!([
                    "Browser.grantPermissions",
                    "http://a.test",
                    ["notifications"]
                ]),
                json!(["Browser.resetPermissions", null, null]),
            ],
            "a run with no grant clears nothing; the others' grants stay until the last goes"
        );
    }

    fn cookie(domain: &str, path: &str, secure: bool) -> NetworkCookie {
        NetworkCookie {
            name: "a".to_string(),
            value: "1".to_string(),
            domain: domain.to_string(),
            path: path.to_string(),
            expires: -1.0,
            http_only: false,
            secure,
            same_site: NetworkCookieSameSite::Lax,
            partition_key: None,
            cr_has_cross_site_ancestor: None,
        }
    }

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap_or_else(|error| panic!("{text}: {error}"))
    }

    #[test]
    fn tests_that_a_cookie_goes_with_the_urls_playwright_sends_it_to() {
        let cases = [
            (cookie("a.test", "/", false), "http://a.test/x", true),
            (cookie("a.test", "/", false), "http://b.a.test/", false),
            (cookie(".a.test", "/", false), "http://b.a.test/", true),
            (cookie(".a.test", "/", false), "http://a.test/", true),
            (cookie("a.test", "/dir", false), "http://a.test/dir/x", true),
            (cookie("a.test", "/dir", false), "http://a.test/dirx", false),
            (
                cookie("a.test", "/dir/", false),
                "http://a.test/dir/x",
                true,
            ),
            (cookie("a.test", "/", true), "http://a.test/", false),
            (cookie("a.test", "/", true), "https://a.test/", true),
            (
                cookie("localhost", "/", true),
                "http://localhost:3000/",
                true,
            ),
            (
                cookie("127.0.0.1", "/", true),
                "http://127.0.0.1:3000/",
                true,
            ),
        ];
        for (cookie, to, expected) in cases {
            assert_eq!(
                matches_url(&cookie, &url(to)),
                expected,
                "{} {} → {to}",
                cookie.domain,
                cookie.path
            );
        }
    }

    #[test]
    fn tests_that_a_cookie_to_add_is_checked_and_its_url_rewritten_as_playwright_does() {
        let set = |url: Option<&str>,
                   domain: Option<&str>,
                   path: Option<&str>,
                   expires: Option<f64>| SetNetworkCookie {
            name: "a".to_string(),
            value: "1".to_string(),
            url: url.map(str::to_string),
            domain: domain.map(str::to_string),
            path: path.map(str::to_string),
            expires,
            http_only: None,
            secure: None,
            same_site: None,
            partition_key: None,
            cr_has_cross_site_ancestor: None,
        };
        let rewritten = rewrite_cookie(set(Some("https://a.test/dir/page"), None, None, None))
            .unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(
            (
                rewritten.domain.as_deref(),
                rewritten.path.as_deref(),
                rewritten.secure
            ),
            (Some("a.test"), Some("/dir/"), Some(true))
        );
        let refused =
            |cookie: SetNetworkCookie| rewrite_cookie(cookie).err().map(|error| error.message);
        assert_eq!(
            refused(set(None, None, None, None)).as_deref(),
            Some(COOKIE_NEEDS_URL_OR_DOMAIN)
        );
        assert_eq!(
            refused(set(Some("http://a.test/"), Some("a.test"), None, None)).as_deref(),
            Some(COOKIE_URL_OR_DOMAIN)
        );
        assert_eq!(
            refused(set(Some("http://a.test/"), None, Some("/"), None)).as_deref(),
            Some(COOKIE_URL_OR_PATH)
        );
        assert_eq!(
            refused(set(None, Some("a.test"), Some("/"), Some(-2.0))).as_deref(),
            Some(COOKIE_EXPIRES)
        );
        assert_eq!(
            refused(set(None, Some("a.test"), Some("/"), Some(1e12))).as_deref(),
            Some(COOKIE_EXPIRES)
        );
        assert!(
            refused(set(Some("about:blank"), None, None, None))
                .is_some_and(|message| message == "Blank page can not have cookie \"a\"")
        );
        assert!(
            refused(set(Some("data:text/plain,x"), None, None, None))
                .is_some_and(|message| message == "Data URL page can not have cookie \"a\"")
        );
        assert!(
            rewrite_cookie(set(None, Some("a.test"), Some("/"), Some(0.0))).is_ok(),
            "expires 0 removes it"
        );
    }

    #[test]
    fn tests_that_a_partitioned_cookie_keeps_its_partition_both_ways() {
        let got: CdpCookie = serde_json::from_value(json!({
            "name": "p", "value": "1", "domain": "a.test", "path": "/", "expires": -1,
            "httpOnly": false, "secure": true, "sameSite": "None",
            "partitionKey": { "topLevelSite": "https://top.test", "hasCrossSiteAncestor": false },
        }))
        .unwrap_or_else(|error| panic!("{error}"));
        let cookie = network_cookie(got);
        assert_eq!(
            (
                cookie.partition_key.as_deref(),
                cookie.cr_has_cross_site_ancestor,
                &cookie.same_site
            ),
            (
                Some("https://top.test"),
                Some(false),
                &NetworkCookieSameSite::None
            )
        );
        let lax: CdpCookie = serde_json::from_value(json!({
            "name": "l", "value": "1", "domain": "a.test", "path": "/", "expires": -1,
            "httpOnly": false, "secure": false,
        }))
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            network_cookie(lax).same_site,
            NetworkCookieSameSite::Lax,
            "Chromium omits None"
        );
        let set = SetNetworkCookie {
            name: "p".to_string(),
            value: "1".to_string(),
            url: None,
            domain: Some("a.test".to_string()),
            path: Some("/".to_string()),
            expires: None,
            http_only: None,
            secure: Some(true),
            same_site: None,
            partition_key: Some("https://top.test".to_string()),
            cr_has_cross_site_ancestor: None,
        };
        assert_eq!(
            chromium_cookie(&set)["partitionKey"],
            json!({ "topLevelSite": "https://top.test", "hasCrossSiteAncestor": true })
        );
    }
}
