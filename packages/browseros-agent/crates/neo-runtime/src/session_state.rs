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
use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use browseros_cdp::{CdpClient, CdpError, SessionId};
use neo_runtime_spec::serve::{
    BrowserContextSetGeolocationParamsGeolocation, BrowserContextSetStorageStateParamsStorageState,
    BrowserContextStorageStateParams, BrowserContextStorageStateResult, NameValue, NetworkCookie,
    NetworkCookieSameSite, OriginStorage, SetNetworkCookie, SetOriginStorage,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;
use tracing::debug;
use url::{Origin, Url};

use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::run::PageId;
use crate::tabs::{self, PageEntry, Tabs};

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
/// set it until that session detaches; [`Overrides::release`] takes it back off before then,
/// when the script closes the context or the run ends (`Tabs::release_run_scoped`).
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
    /// Every session any of it went to, with what, noted before the send: what
    /// [`Overrides::release`] takes back, a session still attaching included.
    applied: HashMap<SessionId, Applied>,
    /// Set by [`Overrides::release`]: from then on no session gets any of it.
    released: bool,
}

/// What one session was sent of the run's session state.
#[derive(Default, Clone, Copy)]
struct Applied {
    headers: bool,
    offline: bool,
    geolocation: bool,
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
    /// Playwright: `server/chromium/crPage.ts` `CRPage._forAllFrameSessions`: `if (isSessionClosedError(e))`
    ///
    /// A session that went away (its frame detached, its tab closed) has nothing left to set,
    /// so a send to it is a no-op, as Playwright's broadcast to a closed frame's session is.
    async fn send(&self, method: &str, params: Value) -> Result<(), PlaywrightError> {
        match self.cdp.send(method, params, Some(&self.session)).await {
            Ok(_) | Err(CdpError::SessionGone) => Ok(()),
            Err(error) => Err(tabs::cdp_error(&error)),
        }
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

/// The run's attached tabs that are still open. A closed one is no page of the context any
/// more, so a setting for every page passes it by, as Playwright's goes over `pages()`.
fn open_tabs(tabs: &Tabs) -> impl Iterator<Item = (Arc<PageEntry>, Arc<PageSession>)> {
    tabs.attached()
        .into_iter()
        .filter(|(entry, _)| !entry.closed.is_cancelled())
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
        if state.released {
            return Err(PlaywrightError::target_closed().into());
        }
        state.headers = headers;
        for (entry, page) in open_tabs(tabs) {
            let merged = state.headers_of(&entry.id);
            for target in targets_of(&page) {
                state
                    .applied
                    .entry(target.session.clone())
                    .or_default()
                    .headers = true;
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
        if state.released {
            return Err(PlaywrightError::target_closed().into());
        }
        state.page_headers.insert(id.clone(), headers);
        let merged = state.headers_of(id);
        for target in targets_of(page) {
            state
                .applied
                .entry(target.session.clone())
                .or_default()
                .headers = true;
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
        if state.released {
            return Err(PlaywrightError::target_closed().into());
        }
        if state.offline == offline {
            return Ok(());
        }
        state.offline = offline;
        for (_, page) in open_tabs(tabs) {
            for target in targets_of(&page) {
                state
                    .applied
                    .entry(target.session.clone())
                    .or_default()
                    .offline = true;
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
        if state.released {
            return Err(PlaywrightError::target_closed().into());
        }
        state.geolocation = geolocation;
        // The tab's own session only: its frames, out-of-process ones too, see the tab's
        // override, and a frame session that detaches would take the tab's with it (measured).
        for (_, page) in open_tabs(tabs) {
            if let Some(own) = targets_of(&page).into_iter().next() {
                state
                    .applied
                    .entry(own.session.clone())
                    .or_default()
                    .geolocation = true;
                progress.race(own.geolocation(checked.as_ref())).await??;
            }
        }
        Ok(())
    }

    /// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._setOfflineForSession`: `if (initial && !this._offline)`
    ///
    /// A session the run attached just now, a tab's or (`frame`) an out-of-process frame's of tab
    /// `page`: it gets what the run set so far, before the call that attached it goes on, a
    /// frame's session all but the geolocation, which the tab's own carries for every frame. A
    /// failure is logged; that session's later requests go without it.
    ///
    /// Deviation: `server/chromium/crBrowser.ts` `CRBrowser.connect`: `await session.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true });`
    /// Playwright holds each new page and out-of-process frame paused until its session has the
    /// context's settings. Neo attaches one when the run first reaches it, so what it requested
    /// before then went without the run's extra headers, offline mode and geolocation: a popup's
    /// document, an out-of-process frame's own fetches as it loaded (measured). A frame's
    /// document request carries them, as the tab's session sends it.
    pub(crate) async fn apply_to_new(
        &self,
        cdp: &CdpClient,
        session: SessionId,
        page: &PageId,
        frame: bool,
    ) {
        let mut state = self.0.lock().await;
        if state.released {
            return;
        }
        let headers = state.headers_of(page);
        let geolocation = state
            .geolocation
            .as_ref()
            .filter(|_| !frame)
            .and_then(|geolocation| verify_geolocation(geolocation).ok());
        let offline = state.offline;
        if !headers.is_empty() || offline || geolocation.is_some() {
            let noted = state.applied.entry(session.clone()).or_default();
            noted.headers |= !headers.is_empty();
            noted.offline |= offline;
            noted.geolocation |= geolocation.is_some();
        }
        let target = Target { cdp, session };
        let applied = async {
            if !headers.is_empty() {
                target.headers(&headers).await?;
            }
            if offline {
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

    /// Takes the run's session state back off every session it went to, the tabs the run
    /// leaves open included: their extra headers, offline mode and geolocation are the
    /// browser's own again, as when Playwright's sessions detach with its connection. From then
    /// on no session gets any of it: an attach still running sets nothing
    /// ([`Self::apply_to_new`]), nor does a call still running. Idempotent. A session that went
    /// away is passed by (`Target::send`); another failure is logged, and that session keeps
    /// the state until it detaches at the run's end.
    pub(crate) async fn release(&self, cdp: &CdpClient) {
        let mut state = self.0.lock().await;
        let applied = mem::take(&mut state.applied);
        *state = State {
            released: true,
            ..State::default()
        };
        for (session, applied) in applied {
            let target = Target { cdp, session };
            let reset = async {
                if applied.headers {
                    target.headers(&[]).await?;
                }
                if applied.offline {
                    target.offline(false).await?;
                }
                if applied.geolocation {
                    target
                        .send("Emulation.clearGeolocationOverride", json!({}))
                        .await?;
                }
                Ok::<_, PlaywrightError>(())
            };
            if let Err(error) = reset.await {
                debug!(?error, "a session kept the run's session state");
            }
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

/// Neo: what `setStorageState` throws for an origin the run has no page open on: Neo writes an
/// origin's storage only through such a page.
const NO_PAGE_FOR_ORIGIN: &str = "Neo writes an origin's storage only through a page of the run open on it: open one first. No page is open on";

/// Neo: what `setStorageState` throws for what Neo does not write yet: `IndexedDB`, OPFS and
/// `WebAuthn` credentials.
const SET_STORAGE_PENDING: &str = "Neo writes cookies and localStorage only: a state with indexedDB, opfs or credentials is not served yet";

/// Deviation: `server/browserContext.ts` `BrowserContext.setStorageState`: `await progress.race(this.doClearCookies());`
/// Playwright first clears the browser's whole HTTP cache, every cookie and the storage of each
/// origin it saw, then writes the state through a page of its own. Neo's context is the user's
/// browser, where that would sign the user out of every site, so Neo only adds the state's cookies
/// and replaces the listed origins' `localStorage`, each through the run's open page on it.
///
/// Checks the whole state before it writes anything: an origin with no open page, or storage Neo
/// does not write yet, refuses the call unchanged.
pub(crate) async fn set_storage_state(
    tabs: &Tabs,
    state: Option<BrowserContextSetStorageStateParamsStorageState>,
    progress: &Progress,
) -> Result<(), CallError> {
    let Some(state) = state else {
        return Ok(());
    };
    let origins = state.origins.unwrap_or_default();
    let unwritable = |origin: &SetOriginStorage| {
        origin
            .indexed_db
            .as_ref()
            .is_some_and(|databases| !databases.is_empty())
            || origin
                .opfs
                .as_ref()
                .is_some_and(|entries| !entries.is_empty())
    };
    if origins.iter().any(unwritable)
        || state
            .credentials
            .as_ref()
            .is_some_and(|credentials| !credentials.is_empty())
    {
        return Err(PlaywrightError::error(SET_STORAGE_PENDING).into());
    }
    let mut pages = Vec::with_capacity(origins.len());
    for origin in &origins {
        let Some(page) = progress.race(open_page_of(tabs, &origin.origin)).await?? else {
            return Err(
                PlaywrightError::error(format!("{NO_PAGE_FOR_ORIGIN} {}", origin.origin)).into(),
            );
        };
        pages.push(page);
    }
    if let Some(cookies) = state.cookies.filter(|cookies| !cookies.is_empty()) {
        add_cookies(tabs, cookies, progress).await?;
    }
    for (origin, page) in origins.iter().zip(&pages) {
        progress
            .race(replace_local_storage(
                page,
                &origin.origin,
                &origin.local_storage,
            ))
            .await??;
    }
    Ok(())
}

/// The run's open page whose main frame is on `origin`, if it has one.
async fn open_page_of(
    tabs: &Tabs,
    origin: &str,
) -> Result<Option<Arc<PageSession>>, PlaywrightError> {
    for (entry, page) in open_tabs(tabs) {
        let Some(url) = tabs.url(&entry).await? else {
            continue;
        };
        if origin_of(&url).as_deref() == Some(origin) {
            return Ok(Some(page));
        }
    }
    Ok(None)
}

/// `url`'s origin, `None` for an opaque one (`about:blank`, `data:`).
fn origin_of(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .map(|url| url.origin())
        .filter(Origin::is_tuple)
        .map(|origin| origin.ascii_serialization())
}

/// Playwright: `server/browserContext.ts` `BrowserContext.setStorageState`: `return script.restore(${JSON.stringify(newOrigins.get(origin))});`
///
/// `origin`'s `localStorage` becomes `items`, its earlier items gone, over the session of the
/// page open on it.
async fn replace_local_storage(
    page: &PageSession,
    origin: &str,
    items: &[NameValue],
) -> Result<(), PlaywrightError> {
    let cdp = page.events.cdp();
    let session = page.events.sessions().into_iter().next();
    let storage = json!({ "securityOrigin": origin, "isLocalStorage": true });
    let send = |method: &'static str, params: Value| {
        let session = session.clone();
        async move {
            cdp.send(method, params, session.as_ref())
                .await
                .map(|_| ())
                .map_err(|error| tabs::cdp_error(&error))
        }
    };
    send("DOMStorage.enable", json!({})).await?;
    send("DOMStorage.clear", json!({ "storageId": storage })).await?;
    for item in items {
        send(
            "DOMStorage.setDOMStorageItem",
            json!({ "storageId": storage, "key": item.name, "value": item.value }),
        )
        .await?;
    }
    Ok(())
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
        let Some(origin) = origin_of(&url) else {
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

/// Neo: what `clearPermissions` throws when its reset task ended before it finished (a panic, the
/// `Runtime` shutting down).
const RESET_STOPPED: &str = "Neo's permission reset stopped before it finished";

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
        // A grant that waited here while the context closed or the run ended goes no further:
        // the run let go of its grants meanwhile (`Tabs::release_run_scoped`).
        progress.check()?;
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
    /// nothing changes nothing, so the user's browser keeps every other client's overrides. The
    /// call may end before the reset does (cancelled, timed out); the reset still finishes.
    pub(crate) async fn clear(
        &self,
        tabs: &Tabs,
        run: u64,
        progress: &Progress,
    ) -> Result<(), CallError> {
        progress
            .race(self.let_go(tabs.cdp().clone(), run))
            .await?
            .map_err(|error| PlaywrightError::error(format!("{RESET_STOPPED}: {error}")))??;
        Ok(())
    }

    /// The run's end: its grants go, as [`Self::clear`]'s do, before the run's outcome returns.
    pub(crate) async fn release(&self, cdp: &CdpClient, run: u64) {
        if let Err(error) = self.let_go(cdp.clone(), run).await {
            debug!(?error, "the run's permission reset stopped");
        }
    }

    /// [`Self::release`] without waiting, for a run whose future was dropped.
    pub(crate) fn release_later(&self, cdp: CdpClient, run: u64) {
        drop(self.let_go(cdp, run));
    }

    /// Run `run`'s grants leave the registry and the browser is reset to the other runs', as one
    /// task on the `Runtime`'s cleanup tracker. Whoever waits may stop waiting (a cancelled call,
    /// a dropped run); the task never stops halfway, which would leave the browser between the
    /// reset and the other runs' grants, or holding this run's once its record is gone. A
    /// failure is logged.
    fn let_go(&self, cdp: CdpClient, run: u64) -> JoinHandle<Result<(), PlaywrightError>> {
        let grants = Arc::clone(&self.grants);
        self.cleanup.spawn(async move {
            let mut grants = grants.lock().await;
            let Some(held) = grants.remove(&run) else {
                return Ok(());
            };
            let reset = reset_context(&cdp, &grants, held.context.as_deref()).await;
            if let Err(error) = &reset {
                debug!(?error, "the run's permission grants were not reset");
            }
            reset
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::runtime::Handle;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::deadline::Deadline;
    use crate::fake_browser::{FakeBrowser, PAGE, Replies};
    use crate::page::TabInputs;
    use crate::progress::{CallEntry, CallStart, RunScope};
    use crate::screenshot::TabScreenshots;
    use crate::switch::Switches;

    fn call_progress() -> Progress {
        progress_ending_with(CancellationToken::new())
    }

    /// A call's progress whose run ends once `cancel` is.
    fn progress_ending_with(cancel: CancellationToken) -> Progress {
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
            Deadline::new(Duration::from_mins(1), cancel),
            TaskTracker::new(),
        );
        Progress::new(entry, 0.0, run, CancellationToken::new())
    }

    /// The permission commands the browser got: each method with its origin and permissions.
    fn permission_commands(browser: &FakeBrowser) -> Vec<Value> {
        browser
            .exchanges()
            .into_iter()
            .filter(|exchange| exchange.method().ends_with("Permissions"))
            .map(|exchange| {
                let params = &exchange.command["params"];
                json!([exchange.method(), params["origin"], params["permissions"]])
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tests_that_each_runs_grants_add_up_and_reset_once_no_run_holds_one() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let (tabs, _) = Tabs::open(
            browser.cdp().await,
            Handle::current(),
            TabInputs::default(),
            TabScreenshots::default(),
            Switches::default(),
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
        assert_eq!(
            permission_commands(&browser),
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

    /// Review C1 P2: run 2's `clearPermissions`, cancelled as it starts, still resets the
    /// context and grants run 1's again, on the `Runtime`'s cleanup tracker; no cancellation
    /// point leaves the browser without run 1's grant or with run 2's.
    #[tokio::test(flavor = "multi_thread")]
    async fn tests_that_a_cancelled_clear_still_resets_and_grants_the_other_runs_again() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let cleanup = TaskTracker::new();
        let (tabs, _) = Tabs::open(
            browser.cdp().await,
            Handle::current(),
            TabInputs::default(),
            TabScreenshots::default(),
            Switches::default(),
            Permissions::new(cleanup.clone()),
            TaskTracker::new(),
            &PageId::new(PAGE.to_string()),
        )
        .await
        .unwrap_or_else(|error| panic!("open: {error:?}"));
        let permissions = tabs.permissions().clone();
        let progress = call_progress();
        let ok = |result: Result<(), CallError>| result.unwrap_or_else(|error| panic!("{error:?}"));
        for (run, name) in [(1, "geolocation"), (2, "notifications")] {
            ok(permissions
                .grant(
                    &tabs,
                    run,
                    Some("http://a.test"),
                    &[name.to_string()],
                    &progress,
                )
                .await);
        }
        let cancel = CancellationToken::new();
        cancel.cancel();
        let cleared = permissions
            .clear(&tabs, 2, &progress_ending_with(cancel))
            .await;
        assert!(
            matches!(cleared, Err(CallError::RunEnded(_))),
            "the call ends with its run: {cleared:?}"
        );
        permissions.release(tabs.cdp(), 2).await;
        cleanup.close();
        cleanup.wait().await;
        assert_eq!(
            permission_commands(&browser)[2..],
            [
                json!(["Browser.resetPermissions", null, null]),
                json!(["Browser.grantPermissions", "http://a.test", ["geolocation"]]),
            ],
            "the reset and run 1's grant again, run 2's gone"
        );
    }

    /// Review delta D1: a released run's session state comes off every session it went to,
    /// one that attached as a frame's included (headers and offline there, geolocation on the
    /// tab's own only), and goes on no session after, however late it attaches; a second
    /// release sends nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn tests_that_released_overrides_come_off_every_session_and_go_on_no_new_one() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let cdp = browser.cdp().await;
        let overrides = Overrides::default();
        {
            let mut state = overrides.0.lock().await;
            state.headers = vec![NameValue {
                name: "x-scope".to_string(),
                value: "owned".to_string(),
            }];
            state.offline = true;
            state.geolocation = Some(BrowserContextSetGeolocationParamsGeolocation {
                latitude: 10.0,
                longitude: 20.0,
                accuracy: None,
            });
        }
        let page = PageId::new(PAGE.to_string());
        let session = |name: &str| SessionId::from(name.to_string());
        overrides
            .apply_to_new(&cdp, session("tab"), &page, false)
            .await;
        overrides
            .apply_to_new(&cdp, session("frame"), &page, true)
            .await;
        let before = browser.exchanges().len();
        overrides.release(&cdp).await;
        let released = browser.exchanges().len();
        overrides.release(&cdp).await;
        overrides
            .apply_to_new(&cdp, session("late"), &page, false)
            .await;
        let exchanges = browser.exchanges();
        let mut sent: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for exchange in &exchanges[before..released] {
            sent.entry(
                exchange.command["sessionId"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            )
            .or_default()
            .push(json!([exchange.method(), exchange.command["params"]]));
        }
        let offline = json!([
            "Network.emulateNetworkConditions",
            network_conditions(false)
        ]);
        let no_headers = json!(["Network.setExtraHTTPHeaders", { "headers": {} }]);
        let enable = json!(["Network.enable", {}]);
        assert_eq!(
            sent,
            BTreeMap::from([
                (
                    "frame".to_string(),
                    vec![
                        enable.clone(),
                        no_headers.clone(),
                        enable.clone(),
                        offline.clone()
                    ]
                ),
                (
                    "tab".to_string(),
                    vec![
                        enable.clone(),
                        no_headers,
                        enable,
                        offline,
                        json!(["Emulation.clearGeolocationOverride", {}]),
                    ]
                ),
            ]),
            "each session gets back what it was given"
        );
        assert_eq!(
            exchanges.len(),
            released,
            "a second release and a late attach send nothing"
        );
    }

    /// A grant that reaches the registry after the context closed sends nothing: the run let go
    /// of its grants meanwhile (`Tabs::release_run_scoped`).
    #[tokio::test(flavor = "multi_thread")]
    async fn tests_that_a_grant_after_the_context_closed_sends_nothing() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let (tabs, _) = Tabs::open(
            browser.cdp().await,
            Handle::current(),
            TabInputs::default(),
            TabScreenshots::default(),
            Switches::default(),
            Permissions::default(),
            TaskTracker::new(),
            &PageId::new(PAGE.to_string()),
        )
        .await
        .unwrap_or_else(|error| panic!("open: {error:?}"));
        let progress = call_progress();
        progress.stop_the_runs_calls();
        let granted = tabs
            .permissions()
            .grant(
                &tabs,
                1,
                Some("http://a.test"),
                &["geolocation".to_string()],
                &progress,
            )
            .await;
        assert!(granted.is_err(), "{granted:?}");
        assert!(
            permission_commands(&browser).is_empty(),
            "nothing reached the browser"
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
