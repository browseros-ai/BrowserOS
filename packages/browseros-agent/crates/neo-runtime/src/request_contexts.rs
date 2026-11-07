//! Playwright: `server/fetch.ts` `APIRequestContext`
//! Playwright: `server/fetch.ts` `BrowserContextAPIRequestContext`
//! Playwright: `server/fetch.ts` `GlobalAPIRequestContext`
//!
//! Neo: the run's API request contexts, `request-context@<id>`: the browser context's
//! (`context.request`, which `page.request` is), created at its first call, and one per
//! `playwright.request.newContext()`. A context sends its requests through `api_request.rs`
//! and keeps each response's body by `fetchUid` until the response or the context is
//! disposed. The run's end disposes every context: what is in flight aborts, and their
//! connections close with their clients.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use neo_runtime_spec::Binary;
use neo_runtime_spec::serve::{
    APIRequestContextCall, APIRequestContextChannel, APIRequestContextCookiesParams,
    APIRequestContextCookiesResult, APIRequestContextFetchLogParams,
    APIRequestContextFetchLogResult, APIRequestContextFetchParams,
    APIRequestContextFetchResponseBodyParams, APIRequestContextFetchResponseBodyResult,
    APIRequestContextFetchResult, APIRequestContextStorageStateParams,
    APIRequestContextStorageStateResult, APIResponse, BrowserContextStorageStateParams,
    HttpCredentialsSend, NameValue, NetworkCookie, NetworkCookieSameSite, OriginStorage,
    PlaywrightCall, PlaywrightNewRequestParams, PlaywrightNewRequestResult, RemoteAddr,
    ResourceTiming, SetNetworkCookie, SetNetworkCookieSameSite, SetOriginStorage,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::api_request::{
    self, Body, BrowserJar, Clients, Defaults, FetchParams, Fetched, FormField, HttpCredentials,
    Jar, OwnJar,
};
use crate::dispatch::encode_result;
use crate::progress::{CallError, ErrorName, PlaywrightError, Progress};
use crate::session_state;
use crate::tabs::{self, Tabs};

/// Neo: a request context's guid prefix: Playwright's own type name (`super(parent,
/// 'request-context')`), as `request@` and `response@` are the network log's.
pub(crate) const REQUEST_CONTEXT_PREFIX: &str = "request-context@";

/// Neo: the browser context's request context, `request-context@default`, as the context is
/// `context@default`.
pub(crate) const BROWSER_CONTEXT_ID: &str = "default";

/// Neo: the guid of the script's `playwright` object, the receiver of `newRequest`.
pub(crate) const PLAYWRIGHT_GUID: &str = "playwright@default";

/// Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: `throw new TargetClosedError(this._closeReason || 'Request context disposed.');`
const DISPOSED: &str = "Request context disposed.";

/// Neo: a stored body crosses to the realm whole when `body()` asks, so it stays under the
/// binary result cap (`artifacts.rs`, 16 MiB).
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Deviation: `server/fetch.ts` `GlobalAPIRequestContext.constructor`: `proxy: options.proxy,`
/// Playwright sends a context's requests through its proxy. Neo has no proxy client yet: it
/// refuses the option rather than ignore it.
const PROXY_PENDING: &str = "Neo does not support a proxy for API request contexts yet";

/// Deviation: `server/fetch.ts` `GlobalAPIRequestContext.constructor`: `this._options = {`
/// Playwright presents a context's client certificates. Neo has none yet: it refuses the
/// option rather than ignore it.
const CLIENT_CERTIFICATES_PENDING: &str =
    "Neo does not support client certificates for API request contexts yet";

/// The run's request contexts, by id (after `request-context@`).
#[derive(Default)]
pub(crate) struct RequestContexts {
    registry: Mutex<Registry>,
}

#[derive(Default)]
struct Registry {
    contexts: HashMap<String, Arc<RequestContext>>,
    next: u64,
}

/// Playwright: `server/fetch.ts` `APIRequestContext`
struct RequestContext {
    /// `None`: the browser context's, whose cookies are the browser's (`BrowserJar`).
    jar: Option<OwnJar>,
    /// `newContext({ storageState })`'s origins, which its `storageState()` gives back.
    origins: Vec<OriginStorage>,
    defaults: Defaults,
    /// Taken at dispose, which closes the clients' pooled connections.
    clients: Mutex<Option<Arc<Clients>>>,
    /// Playwright: `server/fetch.ts` `APIRequestContext.fetchResponses`
    /// Bodies and logs.
    responses: Mutex<HashMap<String, Stored>>,
    next_uid: Mutex<u64>,
    disposed: CancellationToken,
    reason: Mutex<Option<String>>,
}

struct Stored {
    body: Vec<u8>,
    log: Vec<String>,
}

/// Neo: the registry the run's calls find their context in.
impl RequestContexts {
    /// Disposes every context of the run, as its end does: in-flight requests abort, bodies
    /// go, clients close.
    pub(crate) fn close_all(&self) {
        let contexts: Vec<Arc<RequestContext>> = self
            .registry
            .lock()
            .map(|mut registry| {
                registry
                    .contexts
                    .drain()
                    .map(|(_, context)| context)
                    .collect()
            })
            .unwrap_or_default();
        for context in contexts {
            context.dispose(None);
        }
    }

    fn get(&self, id: &str) -> Option<Arc<RequestContext>> {
        self.registry
            .lock()
            .ok()
            .and_then(|registry| registry.contexts.get(id).cloned())
    }

    fn insert(&self, id: Option<&str>, context: RequestContext) -> String {
        let Ok(mut registry) = self.registry.lock() else {
            return String::new();
        };
        let id = id.map_or_else(
            || {
                registry.next += 1;
                registry.next.to_string()
            },
            str::to_string,
        );
        registry.contexts.insert(id.clone(), Arc::new(context));
        id
    }
}

/// Playwright: `server/fetch.ts` `APIRequestContext`
impl RequestContext {
    fn new(jar: Option<OwnJar>, origins: Vec<OriginStorage>, defaults: Defaults) -> Self {
        Self {
            jar,
            origins,
            defaults,
            clients: Mutex::new(Some(Arc::default())),
            responses: Mutex::default(),
            next_uid: Mutex::new(0),
            disposed: CancellationToken::new(),
            reason: Mutex::default(),
        }
    }

    /// Playwright: `server/fetch.ts` `APIRequestContext._disposeImpl`
    fn dispose(&self, reason: Option<String>) {
        if let Ok(mut stored) = self.reason.lock()
            && stored.is_none()
        {
            *stored = reason;
        }
        self.disposed.cancel();
        if let Ok(mut responses) = self.responses.lock() {
            responses.clear();
        }
        let _closed = self
            .clients
            .lock()
            .ok()
            .and_then(|mut clients| clients.take());
    }

    fn closed_error(&self) -> CallError {
        let message = self
            .reason
            .lock()
            .ok()
            .and_then(|reason| reason.clone())
            .unwrap_or_else(|| DISPOSED.to_string());
        PlaywrightError {
            name: ErrorName::TargetClosedError,
            message,
        }
        .into()
    }

    /// Playwright: `server/fetch.ts` `APIRequestContext._storeResponseBody`
    fn store(&self, body: Vec<u8>, log: Vec<String>) -> String {
        let uid = self.next_uid.lock().map_or(0, |mut next| {
            *next += 1;
            *next
        });
        let uid = format!("fetch@{uid}");
        if let Ok(mut responses) = self.responses.lock() {
            responses.insert(uid.clone(), Stored { body, log });
        }
        uid
    }
}

/// Neo: serves one call on `request-context@<id>`.
///
/// # Errors
///
/// Playwright's: a disposed context, the request's failures (`api_request.rs`), a refused
/// cookie; `TargetClosedError` for a context the run never had.
pub(crate) async fn serve(
    tabs: &Tabs,
    id: &str,
    call: APIRequestContextCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let context = match tabs.request_contexts().get(id) {
        Some(context) => context,
        None if id == BROWSER_CONTEXT_ID => browser_context(tabs, progress).await?,
        None => return Err(PlaywrightError::target_closed().into()),
    };
    match call {
        APIRequestContextCall::Fetch(params) => fetch(tabs, &context, params, progress).await,
        APIRequestContextCall::FetchResponseBody(params) => {
            let binary = context.responses.lock().ok().and_then(|responses| {
                responses
                    .get(&params.fetch_uid)
                    .map(|stored| Binary(stored.body.clone()))
            });
            encode_result::<APIRequestContextFetchResponseBodyParams>(
                &APIRequestContextFetchResponseBodyResult { binary },
            )
        }
        APIRequestContextCall::FetchLog(params) => {
            let log = context
                .responses
                .lock()
                .ok()
                .and_then(|responses| {
                    responses
                        .get(&params.fetch_uid)
                        .map(|stored| stored.log.clone())
                })
                .unwrap_or_default();
            encode_result::<APIRequestContextFetchLogParams>(&APIRequestContextFetchLogResult {
                log,
            })
        }
        APIRequestContextCall::DisposeAPIResponse(params) => {
            if let Ok(mut responses) = context.responses.lock() {
                responses.remove(&params.fetch_uid);
            }
            Ok(None)
        }
        // Playwright: `server/fetch.ts` `BrowserContextAPIRequestContext.dispose`
        // A browser
        // context's request context lives as long as its context; disposing it lets its bodies go.
        APIRequestContextCall::Dispose(params) => {
            context.dispose(params.reason);
            Ok(None)
        }
        APIRequestContextCall::StorageState(params) => {
            storage_state(tabs, &context, &params, progress).await
        }
        APIRequestContextCall::Cookies(params) => cookies(tabs, &context, &params, progress).await,
        APIRequestContextCall::AddCookies(params) => {
            if let Some(jar) = &context.jar {
                jar.add_all(
                    params
                        .cookies
                        .into_iter()
                        .map(|cookie| session_state::rewrite_cookie(cookie).map(network_cookie))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            } else {
                session_state::add_cookies(tabs, params.cookies, progress).await?;
            }
            Ok(None)
        }
    }
}

/// Playwright: `server/fetch.ts` `BrowserContextAPIRequestContext`
/// The browser context's,
/// created at its first call. Its requests go with the browser's user agent
/// (`Browser.getVersion`) and the context's extra HTTP headers.
async fn browser_context(
    tabs: &Tabs,
    progress: &Progress,
) -> Result<Arc<RequestContext>, CallError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Version {
        user_agent: String,
    }
    let version: Version = progress
        .race(tabs.cdp().send_typed("Browser.getVersion", json!({}), None))
        .await?
        .map_err(|error| tabs::cdp_error(&error))?;
    let defaults = Defaults {
        user_agent: version.user_agent,
        ..Defaults::default()
    };
    tabs.request_contexts().insert(
        Some(BROWSER_CONTEXT_ID),
        RequestContext::new(None, Vec::new(), defaults),
    );
    tabs.request_contexts()
        .get(BROWSER_CONTEXT_ID)
        .ok_or_else(|| PlaywrightError::target_closed().into())
}

/// Playwright: `server/fetch.ts` `APIRequestContext.fetch`
async fn fetch(
    tabs: &Tabs,
    context: &RequestContext,
    params: APIRequestContextFetchParams,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let Some(clients) = context
        .clients
        .lock()
        .ok()
        .and_then(|clients| clients.clone())
    else {
        return Err(context.closed_error());
    };
    if context.disposed.is_cancelled() {
        return Err(context.closed_error());
    }
    let mut defaults = context.defaults.clone();
    if context.jar.is_none() {
        // Playwright: `server/fetch.ts` `BrowserContextAPIRequestContext._defaultOptions`
        // The context's headers, as they are now.
        defaults.extra_http_headers = pairs(tabs.overrides().context_headers().await);
    }
    let browser_jar = BrowserJar { tabs };
    let jar: &dyn Jar = match &context.jar {
        Some(jar) => jar,
        None => &browser_jar,
    };
    let request = fetch_params(params);
    let sent = tokio::select! {
        sent = progress.race(api_request::fetch(&clients, jar, &defaults, request, progress)) => sent?,
        () = context.disposed.cancelled() => return Err(context.closed_error()),
    };
    let fetched = sent.map_err(PlaywrightError::error)?;
    if fetched.body.len() > MAX_BODY_BYTES {
        return Err(body_too_large(fetched.body.len()));
    }
    let response = api_response(&fetched, String::new());
    let Fetched { body, log, .. } = fetched;
    let fetch_uid = context.store(body, log);
    encode_result::<APIRequestContextFetchParams>(&APIRequestContextFetchResult {
        response: APIResponse {
            fetch_uid,
            ..response
        },
    })
}

/// Deviation: `server/fetch.ts` `APIRequestContext.fetch`: `const fetchUid = this._storeResponseBody(body);`
/// Playwright keeps a body of any size in Node. Neo's realm takes a binary result of at most
/// 16 MiB, so a larger body fails its fetch, with its size.
fn body_too_large(bytes: usize) -> CallError {
    PlaywrightError::error(format!(
        "Neo returns API response bodies of at most 16 MiB; this one has {bytes} bytes"
    ))
    .into()
}

/// Neo: the wire's fetch params as `api_request.rs` takes them.
fn fetch_params(params: APIRequestContextFetchParams) -> FetchParams {
    FetchParams {
        url: params.url,
        encoded_params: params.encoded_params,
        params: params.params.map(pairs).unwrap_or_default(),
        method: params.method,
        headers: params.headers.map(pairs).unwrap_or_default(),
        body: Body {
            raw: params.post_data.map(|binary| binary.0),
            json: params.json_data,
            form: params.form_data.map(pairs),
            multipart: params.multipart_data.map(|fields| {
                fields
                    .into_iter()
                    .filter_map(|field| match (field.file, field.value) {
                        (Some(file), _) => Some(FormField::File {
                            name: field.name,
                            file_name: file.name,
                            mime_type: file.mime_type,
                            buffer: file.buffer.0,
                        }),
                        (None, Some(value)) => Some(FormField::Value {
                            name: field.name,
                            value,
                        }),
                        (None, None) => None,
                    })
                    .collect()
            }),
        },
        fail_on_status_code: params.fail_on_status_code,
        ignore_https_errors: params.ignore_https_errors,
        max_redirects: params.max_redirects,
        max_retries: params
            .max_retries
            .map(|retries| u32::try_from(retries).unwrap_or(0)),
    }
}

/// Neo: the protocol's `APIResponse`: the last hop's response; `fetchUid` filled
/// once its body is stored.
fn api_response(fetched: &Fetched, fetch_uid: String) -> APIResponse {
    APIResponse {
        fetch_uid,
        url: fetched.url.clone(),
        status: i64::from(fetched.status),
        status_text: fetched.status_text.clone(),
        headers: fetched
            .headers
            .iter()
            .map(|(name, value)| NameValue {
                name: name.clone(),
                value: value.clone(),
            })
            .collect(),
        security_details: None,
        server_addr: fetched.server_addr.map(|address| RemoteAddr {
            ip_address: address.ip().to_string(),
            port: i64::from(address.port()),
        }),
        // Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`
        // What the socket does not tell stays -1.
        timing: Some(ResourceTiming {
            start_time: fetched.timing.start_time,
            domain_lookup_start: -1.0,
            domain_lookup_end: -1.0,
            connect_start: -1.0,
            secure_connection_start: -1.0,
            connect_end: -1.0,
            request_start: -1.0,
            response_start: fetched.timing.response_start,
        }),
        response_end_timing: Some(fetched.timing.response_end),
    }
}

/// Playwright: `server/fetch.ts` `GlobalAPIRequestContext.storageState`
///
/// And a browser context's request context, whose state is its context's (`session_state.rs`).
async fn storage_state(
    tabs: &Tabs,
    context: &RequestContext,
    params: &APIRequestContextStorageStateParams,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let result = if let Some(jar) = &context.jar {
        APIRequestContextStorageStateResult {
            cookies: jar.all(),
            origins: context
                .origins
                .iter()
                .map(|origin| OriginStorage {
                    indexed_db: if params.indexed_db == Some(true) {
                        origin.indexed_db.clone()
                    } else {
                        Some(Vec::new())
                    },
                    opfs: if params.opfs == Some(true) {
                        origin.opfs.clone()
                    } else {
                        None
                    },
                    ..origin.clone()
                })
                .collect(),
        }
    } else {
        let state = session_state::storage_state(
            tabs,
            &BrowserContextStorageStateParams {
                indexed_db: params.indexed_db,
                opfs: params.opfs,
                credentials: None,
            },
            progress,
        )
        .await?;
        APIRequestContextStorageStateResult {
            cookies: state.cookies,
            origins: state.origins,
        }
    };
    encode_result::<APIRequestContextStorageStateParams>(&result)
}

/// Playwright: `server/fetch.ts` `APIRequestContext.cookies`
/// The cookies sent to any of
/// `urls`, all for none.
async fn cookies(
    tabs: &Tabs,
    context: &RequestContext,
    params: &APIRequestContextCookiesParams,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let cookies = match &context.jar {
        Some(jar) => {
            let urls = params
                .urls
                .iter()
                .map(|url| {
                    url::Url::parse(url)
                        .map_err(|_| PlaywrightError::error(session_state::INVALID_URL))
                })
                .collect::<Result<Vec<_>, _>>()?;
            jar.all()
                .into_iter()
                .filter(|cookie| {
                    urls.is_empty()
                        || urls
                            .iter()
                            .any(|url| session_state::matches_url(cookie, url))
                })
                .collect()
        }
        None => session_state::cookies(tabs, &params.urls, progress).await?,
    };
    encode_result::<APIRequestContextCookiesParams>(&APIRequestContextCookiesResult { cookies })
}

/// Playwright: `server/dispatchers/playwrightDispatcher.ts` `PlaywrightDispatcher.newRequest`
/// A `newContext()`'s own context.
///
/// # Errors
///
/// Neo's refusals of `proxy` and `clientCertificates` (Deviations).
pub(crate) fn new_request(tabs: &Tabs, call: PlaywrightCall) -> Result<Option<Value>, CallError> {
    let PlaywrightCall::NewRequest(params) = call;
    if params.proxy.is_some() {
        return Err(PlaywrightError::error(PROXY_PENDING).into());
    }
    if params
        .client_certificates
        .as_ref()
        .is_some_and(|certificates| !certificates.is_empty())
    {
        return Err(PlaywrightError::error(CLIENT_CERTIFICATES_PENDING).into());
    }
    let jar = OwnJar::default();
    let mut origins = Vec::new();
    if let Some(state) = params.storage_state.clone() {
        // Playwright: `server/fetch.ts` `GlobalAPIRequestContext.constructor`: `this._cookieStore.addCookies(options.storageState.cookies || []);`
        jar.add_all(state.cookies.unwrap_or_default());
        origins = state
            .origins
            .unwrap_or_default()
            .into_iter()
            .map(origin_storage)
            .collect();
    }
    let id = tabs.request_contexts().insert(
        None,
        RequestContext::new(Some(jar), origins, defaults_of(params)),
    );
    encode_result::<PlaywrightNewRequestParams>(&PlaywrightNewRequestResult {
        request: APIRequestContextChannel {
            guid: format!("{REQUEST_CONTEXT_PREFIX}{id}"),
        },
    })
}

/// Playwright: `server/fetch.ts` `GlobalAPIRequestContext.constructor`
/// What its requests go
/// with. Deviation: Playwright's default user agent names Node (`getUserAgent()`); Neo's names
/// Neo.
fn defaults_of(params: PlaywrightNewRequestParams) -> Defaults {
    Defaults {
        user_agent: params
            .user_agent
            .unwrap_or_else(|| format!("Playwright/{} (Neo)", env!("CARGO_PKG_VERSION"))),
        extra_http_headers: params.extra_http_headers.map(pairs).unwrap_or_default(),
        base_url: params.base_url,
        http_credentials: params
            .http_credentials
            .unwrap_or_default()
            .into_iter()
            .map(|credentials| HttpCredentials {
                username: credentials.username,
                password: credentials.password,
                origin: credentials.origin,
                send_always: credentials.send == Some(HttpCredentialsSend::Always),
            })
            .collect(),
        ignore_https_errors: params.ignore_https_errors.unwrap_or(false),
        max_redirects: params.max_redirects,
        fail_on_status_code: params.fail_on_status_code.unwrap_or(false),
    }
}

/// Neo: an origin of `newContext({ storageState })`, as `storageState()` gives it back.
fn origin_storage(origin: SetOriginStorage) -> OriginStorage {
    OriginStorage {
        origin: origin.origin,
        local_storage: origin.local_storage,
        indexed_db: Some(origin.indexed_db.unwrap_or_default()),
        opfs: origin.opfs,
    }
}

/// Neo: a cookie `addCookies` checked (`rewriteCookies`), as a jar stores it, with
/// Playwright's defaults.
fn network_cookie(cookie: SetNetworkCookie) -> NetworkCookie {
    NetworkCookie {
        name: cookie.name,
        value: cookie.value,
        domain: cookie.domain.unwrap_or_default(),
        path: cookie.path.unwrap_or_default(),
        expires: cookie.expires.unwrap_or(-1.0),
        http_only: cookie.http_only.unwrap_or(false),
        secure: cookie.secure.unwrap_or(false),
        same_site: match cookie.same_site {
            Some(SetNetworkCookieSameSite::Strict) => NetworkCookieSameSite::Strict,
            Some(SetNetworkCookieSameSite::None) => NetworkCookieSameSite::None,
            Some(SetNetworkCookieSameSite::Lax) | None => NetworkCookieSameSite::Lax,
        },
        partition_key: cookie.partition_key,
        cr_has_cross_site_ancestor: cookie.cr_has_cross_site_ancestor,
    }
}

/// Neo: name-value pairs as `api_request.rs` takes them.
fn pairs(values: Vec<NameValue>) -> Vec<(String, String)> {
    values
        .into_iter()
        .map(|value| (value.name, value.value))
        .collect()
}
