//! Playwright: `server/fetch.ts` `APIRequestContext.fetch`
//! Playwright: `server/fetch.ts` `APIRequestContext._sendRequestWithRetries`
//! Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`
//! Playwright: `server/formData.ts` `MultipartFormData`
//!
//! Neo: the HTTP side of an API request context (`page.request`, `context.request`,
//! `playwright.request.newContext()`): one request as Playwright's server makes it, from Rust
//! rather than Node. Playwright's own algorithm runs on top of `reqwest`: its headers, its body
//! encodings, its redirects (followed here, never by the client), its cookies per hop, its one
//! Basic-auth resend and its retries on a reset connection. The caller races the call's
//! deadline and the context's disposal around [`fetch`].

#![expect(
    dead_code,
    reason = "the request contexts' wiring (dispatch, the SDK) calls it in C2 stage 3"
)]

use std::collections::hash_map::RandomState;
use std::error::Error as _;
use std::hash::BuildHasher;
use std::io::{self, ErrorKind, Read};
use std::net::SocketAddr;
use std::slice;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::read::{GzDecoder, ZlibDecoder};
use futures_util::future::BoxFuture;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use tokio::time;
use url::Url;

use crate::cookie_store::{self, CookieStore, NetworkCookie};

/// Playwright: `server/fetch.ts` `redirectStatus`: `const redirectStatus = [301, 302, 303, 307, 308];`
const REDIRECT_STATUS: [u16; 5] = [301, 302, 303, 307, 308];

/// Playwright: `server/fetch.ts` `APIRequestContext.fetch`: `let maxRedirects = params.maxRedirects ?? (defaults.maxRedirects ?? 20);`
const DEFAULT_MAX_REDIRECTS: i64 = 20;

/// Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: `reject(new Error('Max redirect count exceeded'));`
const MAX_REDIRECTS: &str = "Max redirect count exceeded";

/// Playwright: `server/fetch.ts` `serializePostData`: `assert((params.postData ? 1 : 0) + (params.jsonData ? 1 : 0) + (params.formData ? 1 : 0) + (params.multipartData ? 1 : 0) <= 1, `Only one of 'data', 'form' or 'multipart' can be specified`);`
const ONLY_ONE_BODY: &str = "Only one of 'data', 'form' or 'multipart' can be specified";

/// Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: `reject(new Error(`uri requested responds with an invalid redirect URL: ${locationHeaderValue}`));`
const INVALID_REDIRECT: &str = "uri requested responds with an invalid redirect URL: ";

/// Deviation: `server/fetch.ts` `APIRequestContext.fetch`: `'accept-encoding': 'gzip,deflate,br',`
/// Neo decodes gzip and deflate (`flate2`), and has no brotli decoder: it does not offer `br`.
const ACCEPT_ENCODING: &str = "gzip,deflate";

/// Playwright: `server/fetch.ts` `APIRequestContext._sendRequestWithRetries`: `let backoff = 250;`
const FIRST_BACKOFF: Duration = Duration::from_millis(250);

/// Playwright: `server/fetch.ts` `APIRequestContext.fetch`: `if (text.length > 1000)`
const RESPONSE_TEXT_LIMIT: usize = 1000;

/// Playwright: `server/fetch.ts` `HeadersObject`: headers by name in the order they were first
/// set; a name matches in any case and keeps the casing it was first set with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Headers(pub(crate) Vec<(String, String)>);

/// Playwright: `server/fetch.ts` `setHeader`
impl Headers {
    /// `setHeader`: replaces the value of a name already set, unless `keep_existing`.
    pub(crate) fn set(&mut self, name: &str, value: &str, keep_existing: bool) {
        match self.position(name) {
            None => self.0.push((name.to_string(), value.to_string())),
            Some(index) if !keep_existing => self.0[index].1 = value.to_string(),
            Some(_) => {}
        }
    }

    /// `getHeader`
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.position(name).map(|index| self.0[index].1.as_str())
    }

    /// `removeHeader`
    pub(crate) fn remove(&mut self, name: &str) {
        if let Some(index) = self.position(name) {
            self.0.remove(index);
        }
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.0
            .iter()
            .position(|(existing, _)| existing.eq_ignore_ascii_case(name))
    }
}

/// Playwright: `server/channels.ts` `HttpCredentials`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HttpCredentials {
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) origin: Option<String>,
    /// `send: 'always'`, where `'unauthorized'` (the default) waits for a 401.
    pub(crate) send_always: bool,
}

/// Playwright: `server/fetch.ts` `FetchRequestOptions`: what a request context gives each of
/// its requests (`_defaultOptions`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Defaults {
    pub(crate) user_agent: String,
    pub(crate) extra_http_headers: Vec<(String, String)>,
    pub(crate) base_url: Option<String>,
    pub(crate) http_credentials: Option<HttpCredentials>,
    pub(crate) ignore_https_errors: bool,
    pub(crate) max_redirects: Option<i64>,
    pub(crate) fail_on_status_code: bool,
}

/// Playwright: `server/channels.ts` `FormField`
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FormField {
    Value {
        name: String,
        value: String,
    },
    File {
        name: String,
        file_name: String,
        mime_type: Option<String>,
        buffer: Vec<u8>,
    },
}

/// Playwright: `server/channels.ts` `APIRequestContextFetchParams`: `postData` (`raw`),
/// `jsonData`, `formData` and `multipartData`, of which a request has at most one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Body {
    pub(crate) raw: Option<Vec<u8>>,
    pub(crate) json: Option<String>,
    pub(crate) form: Option<Vec<(String, String)>>,
    pub(crate) multipart: Option<Vec<FormField>>,
}

/// Playwright: `server/channels.ts` `APIRequestContextFetchParams`
#[derive(Clone, Debug, Default)]
pub(crate) struct FetchParams {
    pub(crate) url: String,
    pub(crate) encoded_params: Option<String>,
    pub(crate) params: Vec<(String, String)>,
    pub(crate) method: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Body,
    pub(crate) fail_on_status_code: Option<bool>,
    pub(crate) ignore_https_errors: Option<bool>,
    pub(crate) max_redirects: Option<i64>,
    pub(crate) max_retries: Option<u32>,
}

/// Playwright: `server/channels.ts` `APIResponse`: the last hop's response, with its body
/// decoded, the log the call made (`fetchLog`), and when it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fetched {
    pub(crate) url: String,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    /// Deviation: `server/fetch.ts` `APIRequestContext._sendRequest`: `headers: toHeadersArray(response.rawHeaders),`
    /// The `http` crate keeps neither a header name's case nor the order across names: names
    /// come lowercased, each name's values together.
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) server_addr: Option<SocketAddr>,
    pub(crate) timing: Timing,
    pub(crate) log: Vec<String>,
}

/// Playwright: `server/channels.ts` `ResourceTiming`: milliseconds, `start_time` since the epoch
/// and the others since it; what reqwest does not tell stays -1 (Playwright's missing value).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Timing {
    pub(crate) start_time: f64,
    pub(crate) response_start: f64,
    pub(crate) response_end: f64,
}

/// Neo: `Timing`'s floats compare as values.
impl Eq for Timing {}

/// Neo: where a request context's cookies live: its own jar (`newContext()`), or its browser
/// context's (`context.request`).
pub(crate) trait Jar: Send + Sync {
    /// The cookies a request to `url` is sent, in jar order.
    fn cookies<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Vec<NetworkCookie>>;

    /// Stores a response's cookies; a cookie the jar refuses is dropped, as Playwright's
    /// `addCookies` retry swallows it.
    fn add(&self, cookies: Vec<NetworkCookie>) -> BoxFuture<'_, ()>;
}

/// Playwright: `server/fetch.ts` `GlobalAPIRequestContext`: `this._cookieStore = new CookieStore();`
#[derive(Debug, Default)]
pub(crate) struct OwnJar(Mutex<CookieStore>);

/// Playwright: `server/fetch.ts` `GlobalAPIRequestContext.cookies`
impl Jar for OwnJar {
    fn cookies<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Vec<NetworkCookie>> {
        let all = self
            .0
            .lock()
            .map(|mut jar| jar.all_cookies(now_seconds()))
            .unwrap_or_default();
        let matching = cookie_store::filter_cookies(&all, slice::from_ref(url))
            .into_iter()
            .cloned()
            .collect();
        Box::pin(async move { matching })
    }

    fn add(&self, cookies: Vec<NetworkCookie>) -> BoxFuture<'_, ()> {
        if let Ok(mut jar) = self.0.lock() {
            jar.add_cookies(cookies, now_seconds());
        }
        Box::pin(async {})
    }
}

/// Neo: a request context's two HTTP clients, built on first use: one that checks certificates,
/// and one for `ignoreHTTPSErrors`. Dropping them closes their pooled connections.
#[derive(Debug, Default)]
pub(crate) struct Clients {
    strict: OnceLock<reqwest::Client>,
    insecure: OnceLock<reqwest::Client>,
}

impl Clients {
    fn get(&self, insecure: bool) -> Result<&reqwest::Client, String> {
        let slot = if insecure {
            &self.insecure
        } else {
            &self.strict
        };
        if let Some(client) = slot.get() {
            return Ok(client);
        }
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .http1_only()
            .no_proxy()
            .danger_accept_invalid_certs(insecure)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(slot.get_or_init(|| client))
    }
}

/// Neo: one hop's method, headers, redirect budget and certificate check.
#[derive(Clone, Debug)]
struct SendOptions {
    method: String,
    headers: Headers,
    /// -1: a redirect is the response (Playwright's `maxRedirects: 0`).
    max_redirects: i64,
    insecure: bool,
}

/// Neo: where a redirect leads, and what its hop sends.
#[derive(Debug)]
struct Hop {
    url: Url,
    options: SendOptions,
    post_data: Option<Vec<u8>>,
}

/// Neo: a hop that failed, and whether it was the connection reset Playwright retries.
#[derive(Debug)]
struct SendError {
    message: String,
    reset: bool,
}

impl From<String> for SendError {
    fn from(message: String) -> Self {
        Self {
            message,
            reset: false,
        }
    }
}

/// Playwright: `server/fetch.ts` `APIRequestContext.fetch`: the request `params` asks for, with
/// the context's `defaults`, its cookies in `jar`, through `clients`.
///
/// # Errors
///
/// Playwright's: a URL that does not parse, two bodies, more redirects than allowed, a refused
/// connection, a status `failOnStatusCode` fails on.
pub(crate) async fn fetch(
    clients: &Clients,
    jar: &dyn Jar,
    defaults: &Defaults,
    params: FetchParams,
) -> Result<Fetched, String> {
    let mut headers = Headers::default();
    headers.set("user-agent", &defaults.user_agent, false);
    headers.set("accept", "*/*", false);
    headers.set("accept-encoding", ACCEPT_ENCODING, false);
    for (name, value) in &defaults.extra_http_headers {
        headers.set(name, value, false);
    }
    for (name, value) in &params.headers {
        headers.set(name, value, false);
    }
    let url = request_url(defaults.base_url.as_deref(), &params)?;
    let credentials = credentials_for(defaults.http_credentials.as_ref(), &url);
    if let Some(credentials) = credentials.filter(|credentials| credentials.send_always) {
        set_basic_authorization(&mut headers, credentials);
    }
    // Playwright: `server/fetch.ts` `APIRequestContext.fetch`: `const method = params.method?.toUpperCase() || 'GET';`
    let method = params
        .method
        .as_deref()
        .filter(|method| !method.is_empty())
        .map_or_else(|| "GET".to_string(), str::to_uppercase);
    let max_redirects = params
        .max_redirects
        .or(defaults.max_redirects)
        .unwrap_or(DEFAULT_MAX_REDIRECTS);
    let options = SendOptions {
        method,
        headers,
        max_redirects: if max_redirects == 0 {
            -1
        } else {
            max_redirects
        },
        insecure: params.ignore_https_errors.unwrap_or(false) || defaults.ignore_https_errors,
    };
    let mut options = options;
    let post_data = serialize_post_data(&params.body, &mut options.headers)?;
    if let Some(post_data) = &post_data {
        options
            .headers
            .set("content-length", &post_data.len().to_string(), false);
    }
    let fetched = send_with_retries(
        clients,
        jar,
        defaults,
        url,
        options,
        post_data,
        params.max_retries.unwrap_or(0),
    )
    .await?;
    let fail_on_status_code = params
        .fail_on_status_code
        .unwrap_or(defaults.fail_on_status_code);
    if fail_on_status_code && !(200..400).contains(&fetched.status) {
        return Err(status_failure(&fetched));
    }
    Ok(fetched)
}

/// Playwright: `server/fetch.ts` `APIRequestContext.fetch`: `throw new Error(`${response.status} ${response.statusText}${responseText}`);`
fn status_failure(fetched: &Fetched) -> String {
    let mut message = format!("{} {}", fetched.status, fetched.status_text);
    if !fetched.body.is_empty() {
        let text = String::from_utf8_lossy(&fetched.body);
        let units: Vec<u16> = text.encode_utf16().collect();
        // Playwright: `server/fetch.ts` `APIRequestContext.fetch`: `text = text.substring(0, 997) + '...';`
        let shown = if units.len() > RESPONSE_TEXT_LIMIT {
            format!(
                "{}...",
                String::from_utf16_lossy(&units[..RESPONSE_TEXT_LIMIT - 3])
            )
        } else {
            text.into_owned()
        };
        message.push_str("\nResponse text:\n");
        message.push_str(&shown);
    }
    message
}

/// Playwright: `server/fetch.ts` `APIRequestContext.fetch`: `const requestUrl = new URL(constructURLBasedOnBaseURL(defaults.baseURL, params.url));`
fn request_url(base_url: Option<&str>, params: &FetchParams) -> Result<Url, String> {
    // Playwright: `isomorphic/urlMatch.ts` `constructURLBasedOnBaseURL`: a base that does not
    // parse, or a URL that does not resolve against it, leaves the URL as given.
    let given = base_url
        .and_then(|base| Url::parse(base).ok())
        .and_then(|base| base.join(&params.url).ok())
        .map_or_else(|| params.url.clone(), |url| url.to_string());
    // Playwright: `server/fetch.ts` `APIRequestContext.fetch`: Node's `new URL` throws `Invalid URL`.
    let mut url = Url::parse(&given).map_err(|_| "Invalid URL".to_string())?;
    if let Some(encoded) = &params.encoded_params {
        let encoded = encoded.strip_prefix('?').unwrap_or(encoded);
        if !encoded.is_empty() {
            let query = match url.query() {
                Some(existing) if !existing.is_empty() => format!("{existing}&{encoded}"),
                _ => encoded.to_string(),
            };
            url.set_query(Some(&query));
        }
    } else if !params.params.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (name, value) in &params.params {
            pairs.append_pair(name, value);
        }
    }
    Ok(url)
}

/// Playwright: `server/browserContext.ts` `BrowserContext._getHttpCredentials`: the credentials
/// for `url`, if the context's have no origin or this one.
fn credentials_for<'a>(
    credentials: Option<&'a HttpCredentials>,
    url: &Url,
) -> Option<&'a HttpCredentials> {
    credentials.filter(|credentials| {
        credentials.origin.as_ref().is_none_or(|origin| {
            origin.to_lowercase() == url.origin().ascii_serialization().to_lowercase()
        })
    })
}

/// Playwright: `server/fetch.ts` `setBasicAuthorizationHeader`: `setHeader(headers, 'authorization', `Basic ${encoded}`);`
fn set_basic_authorization(headers: &mut Headers, credentials: &HttpCredentials) {
    let encoded = STANDARD.encode(format!("{}:{}", credentials.username, credentials.password));
    headers.set("authorization", &format!("Basic {encoded}"), false);
}

/// Playwright: `server/fetch.ts` `serializePostData`: the body bytes, with the content type the
/// headers lack.
fn serialize_post_data(body: &Body, headers: &mut Headers) -> Result<Option<Vec<u8>>, String> {
    let bodies = usize::from(body.raw.is_some())
        + usize::from(body.json.is_some())
        + usize::from(body.form.is_some())
        + usize::from(body.multipart.is_some());
    if bodies > 1 {
        return Err(ONLY_ONE_BODY.to_string());
    }
    if let Some(json) = &body.json {
        headers.set("content-type", "application/json", true);
        return Ok(Some(json.as_bytes().to_vec()));
    }
    if let Some(form) = &body.form {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        headers.set("content-type", "application/x-www-form-urlencoded", true);
        return Ok(Some(encoded.into_bytes()));
    }
    if let Some(fields) = &body.multipart {
        let mut form = MultipartFormData::new();
        for field in fields {
            match field {
                FormField::Value { name, value } => form.add_field(name, value),
                FormField::File {
                    name,
                    file_name,
                    mime_type,
                    buffer,
                } => form.add_file_field(name, file_name, mime_type.as_deref(), buffer),
            }
        }
        headers.set("content-type", &form.content_type_header(), true);
        return Ok(Some(form.finish()));
    }
    if let Some(post_data) = &body.raw {
        headers.set("content-type", "application/octet-stream", true);
        return Ok(Some(post_data.clone()));
    }
    Ok(None)
}

/// Playwright: `server/formData.ts` `MultipartFormData`
struct MultipartFormData {
    boundary: String,
    chunks: Vec<u8>,
}

/// Playwright: `server/formData.ts` `MultipartFormData`
impl MultipartFormData {
    /// `generateUniqueBoundaryString`: `return '----WebKitFormBoundary' + String.fromCharCode(...charCodes);`
    fn new() -> Self {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789AB";
        let seed = RandomState::new().hash_one(SystemTime::now());
        let boundary: String = (0..16)
            .map(|index| {
                let pick = (seed >> ((index % 10) * 6)).wrapping_add(index * 7) % 64;
                char::from(ALPHABET[usize::try_from(pick).unwrap_or_default()])
            })
            .collect();
        Self {
            boundary: format!("----WebKitFormBoundary{boundary}"),
            chunks: Vec::new(),
        }
    }

    fn content_type_header(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }

    fn add_field(&mut self, name: &str, value: &str) {
        self.begin_header(name);
        self.chunks.extend_from_slice(b"\r\n\r\n");
        self.chunks.extend_from_slice(value.as_bytes());
        self.chunks.extend_from_slice(b"\r\n");
    }

    fn add_file_field(
        &mut self,
        name: &str,
        file_name: &str,
        mime_type: Option<&str>,
        buffer: &[u8],
    ) {
        self.begin_header(name);
        self.chunks
            .extend_from_slice(format!("; filename=\"{}\"", escape_name(file_name)).as_bytes());
        let mime_type = mime_type
            .filter(|mime_type| !mime_type.is_empty())
            .unwrap_or_else(|| mime_type_of(file_name));
        self.chunks
            .extend_from_slice(format!("\r\ncontent-type: {mime_type}").as_bytes());
        self.chunks.extend_from_slice(b"\r\n\r\n");
        self.chunks.extend_from_slice(buffer);
        self.chunks.extend_from_slice(b"\r\n");
    }

    fn finish(mut self) -> Vec<u8> {
        self.chunks
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        self.chunks
    }

    fn begin_header(&mut self, name: &str) {
        self.chunks
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        let name = escape_name(&normalize_line_endings(name));
        self.chunks.extend_from_slice(
            format!("content-disposition: form-data; name=\"{name}\"").as_bytes(),
        );
    }
}

/// Playwright: `server/formData.ts` `escapeName`
fn escape_name(name: &str) -> String {
    name.replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace('"', "%22")
}

/// Playwright: `server/formData.ts` `normalizeLineEndings`
fn normalize_line_endings(name: &str) -> String {
    name.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

/// Deviation: `server/formData.ts` `MultipartFormData.addFileField`: `this._chunks.push(Buffer.from(`\r\ncontent-type: ${value.mimeType || mime.getType(value.name) || 'application/octet-stream'}`));`
/// Neo knows the common extensions' types, where Playwright's `mime` package knows them all.
fn mime_type_of(file_name: &str) -> &'static str {
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "csv" => "text/csv",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Playwright: `server/fetch.ts` `APIRequestContext._sendRequestWithRetries`: only a reset
/// connection is retried, `max_retries` times, waiting 250 ms and twice as long each time.
async fn send_with_retries(
    clients: &Clients,
    jar: &dyn Jar,
    defaults: &Defaults,
    url: Url,
    options: SendOptions,
    post_data: Option<Vec<u8>>,
    max_retries: u32,
) -> Result<Fetched, String> {
    let mut log = Vec::new();
    let mut backoff = FIRST_BACKOFF;
    let mut attempt = 0;
    loop {
        match send(
            clients,
            jar,
            defaults,
            url.clone(),
            options.clone(),
            post_data.clone(),
            &mut log,
        )
        .await
        {
            Ok(mut fetched) => {
                fetched.log = log;
                return Ok(fetched);
            }
            Err(error) if max_retries == 0 => return Err(error.message),
            // Playwright: `server/fetch.ts` `APIRequestContext._sendRequestWithRetries`: `throw new Error(`Failed after ${i + 1} attempt(s): ${e}`);`
            Err(error) if attempt == max_retries => {
                return Err(format!(
                    "Failed after {} attempt(s): Error: {}",
                    attempt + 1,
                    error.message
                ));
            }
            Err(error) if !error.reset => return Err(error.message),
            Err(_) => {
                // Playwright: `server/fetch.ts` `APIRequestContext._sendRequestWithRetries`: `const message = `  Received ECONNRESET, will retry after ${backoff}ms.`;`
                log.push(format!(
                    "  Received ECONNRESET, will retry after {}ms.",
                    backoff.as_millis()
                ));
                time::sleep(backoff).await;
                backoff *= 2;
                attempt += 1;
            }
        }
    }
}

/// Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: one request and the hops
/// its redirects and Basic-auth challenge lead to.
async fn send(
    clients: &Clients,
    jar: &dyn Jar,
    defaults: &Defaults,
    mut url: Url,
    mut options: SendOptions,
    mut post_data: Option<Vec<u8>>,
    log: &mut Vec<String>,
) -> Result<Fetched, SendError> {
    loop {
        if options.headers.get("cookie").is_none() {
            let cookies = jar.cookies(&url).await;
            if !cookies.is_empty() {
                let values: Vec<String> = cookies
                    .iter()
                    .map(|cookie| {
                        cookie_store::serialize_cookie_name_value(&cookie.name, &cookie.value)
                    })
                    .collect();
                options.headers.set("cookie", &values.join("; "), false);
            }
        }
        // Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: `fetchLog(`→ ${options.method} ${url.toString()}`);`
        log.push(format!("→ {} {url}", options.method));
        for (name, value) in &options.headers.0 {
            log.push(format!("  {name}: {value}"));
        }
        let started = now_millis();
        let response = request(clients, &url, &options, post_data.as_deref()).await?;
        let response_start = now_millis() - started;
        let status = response.status().as_u16();
        let status_text = response
            .status()
            .canonical_reason()
            .unwrap_or_default()
            .to_string();
        let headers = response_headers(&response);
        log.push(format!("← {status} {status_text}"));
        for (name, value) in &headers {
            log.push(format!("  {name}: {value}"));
        }
        let set_cookies: Vec<&str> = headers
            .iter()
            .filter(|(name, _)| name == "set-cookie")
            .map(|(_, value)| value.as_str())
            .collect();
        let cookies = cookie_store::parse_set_cookie_headers(&url, &set_cookies, now_seconds());
        if !cookies.is_empty() {
            jar.add(cookies).await;
        }
        if REDIRECT_STATUS.contains(&status) && options.max_redirects >= 0 {
            if options.max_redirects == 0 {
                return Err(MAX_REDIRECTS.to_string().into());
            }
            if let Some(hop) = redirect(&url, &options, post_data.take(), status, &headers)? {
                url = hop.url;
                options = hop.options;
                post_data = hop.post_data;
                continue;
            }
        }
        // Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: `if (auth?.trim().startsWith('Basic') && credentials) {`
        if status == 401
            && options.headers.get("authorization").is_none()
            && let Some(credentials) = credentials_for(defaults.http_credentials.as_ref(), &url)
            && header(&headers, "www-authenticate")
                .is_some_and(|auth| auth.trim().starts_with("Basic"))
        {
            set_basic_authorization(&mut options.headers, credentials);
            continue;
        }
        let server_addr = response.remote_addr();
        let encoding = header(&headers, "content-encoding").map(str::to_lowercase);
        let raw = response.bytes().await.map_err(|error| send_error(&error))?;
        let body = decode(&raw, encoding.as_deref())?;
        return Ok(Fetched {
            url: url.to_string(),
            status,
            status_text,
            headers,
            body,
            server_addr,
            timing: Timing {
                start_time: started,
                response_start,
                response_end: now_millis() - started,
            },
            log: Vec::new(),
        });
    }
}

/// Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: where a redirect leads, with
/// the method, body and headers the next hop goes with; `None` for one with no `Location`, which
/// is the response.
fn redirect(
    url: &Url,
    options: &SendOptions,
    mut post_data: Option<Vec<u8>>,
    status: u16,
    headers: &[(String, String)],
) -> Result<Option<Hop>, SendError> {
    let mut next_headers = options.headers.clone();
    next_headers.remove("cookie");
    let mut method = options.method.clone();
    // Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: `// HTTP-redirect fetch step 13 (https://fetch.spec.whatwg.org/#http-redirect-fetch)`
    if ((status == 301 || status == 302) && method == "POST")
        || (status == 303 && method != "GET" && method != "HEAD")
    {
        method = "GET".to_string();
        post_data = None;
        for name in [
            "content-encoding",
            "content-language",
            "content-length",
            "content-location",
            "content-type",
        ] {
            next_headers.remove(name);
        }
    }
    let Some(location) = header(headers, "location").filter(|location| !location.is_empty()) else {
        return Ok(None);
    };
    // Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: `const locationHeaderValue = Buffer.from(response.headers.location ?? '', 'latin1').toString('utf8');`
    let location = latin1_as_utf8(location);
    let next = url
        .join(&location)
        .map_err(|_| SendError::from(format!("{INVALID_REDIRECT}{location}")))?;
    let host = match next.port() {
        Some(port) => format!("{}:{port}", next.host_str().unwrap_or_default()),
        None => next.host_str().unwrap_or_default().to_string(),
    };
    next_headers.set("host", &host, false);
    // Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: `// Drop credentials scoped to the original origin on cross-origin redirects.`
    if next.origin() != url.origin() {
        next_headers.remove("authorization");
    }
    let next_options = SendOptions {
        method,
        headers: next_headers,
        max_redirects: options.max_redirects - 1,
        insecure: options.insecure,
    };
    Ok(Some(Hop {
        url: next,
        options: next_options,
        post_data,
    }))
}

/// Neo: one hop through reqwest: header values as Node encodes them (Latin-1), the body as given.
async fn request(
    clients: &Clients,
    url: &Url,
    options: &SendOptions,
    body: Option<&[u8]>,
) -> Result<reqwest::Response, SendError> {
    let client = clients.get(options.insecure)?;
    let method = reqwest::Method::from_bytes(options.method.as_bytes())
        .map_err(|_| SendError::from(format!("Invalid method: {}", options.method)))?;
    let mut headers = HeaderMap::new();
    for (name, value) in &options.headers.0 {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| SendError::from(format!("Invalid header name: \"{name}\"")))?;
        // Playwright: Node's `ERR_INVALID_CHAR`, for a header value it cannot send as Latin-1.
        let header_value = latin1_bytes(value)
            .and_then(|bytes| HeaderValue::from_bytes(&bytes).ok())
            .ok_or_else(|| {
                SendError::from(format!("Invalid character in header content [\"{name}\"]"))
            })?;
        headers.append(header_name, header_value);
    }
    let mut builder = client.request(method, url.clone()).headers(headers);
    if let Some(body) = body {
        builder = builder.body(body.to_vec());
    }
    builder.send().await.map_err(|error| send_error(&error))
}

/// Neo: a response's headers, values as Latin-1 text, as Node reads them.
fn response_headers(response: &reqwest::Response) -> Vec<(String, String)> {
    response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value
                    .as_bytes()
                    .iter()
                    .map(|byte| char::from(*byte))
                    .collect(),
            )
        })
        .collect()
}

/// Neo: the first value of a header, by lowercase name.
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(existing, _)| existing == name)
        .map(|(_, value)| value.as_str())
}

/// Playwright: `server/fetch.ts` `APIRequestContext._sendRequest`: `const encoding = response.headers['content-encoding']?.toLowerCase();`
/// gzip and deflate decode, as Node's zlib does with a sync flush, so a truncated stream gives
/// what it has; an empty body or another encoding is the body as it came.
fn decode(raw: &[u8], encoding: Option<&str>) -> Result<Vec<u8>, SendError> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut decoded = Vec::new();
    let result = match encoding {
        Some("gzip" | "x-gzip") => GzDecoder::new(raw).read_to_end(&mut decoded),
        Some("deflate") => ZlibDecoder::new(raw).read_to_end(&mut decoded),
        _ => return Ok(raw.to_vec()),
    };
    match result {
        Ok(_) => Ok(decoded),
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => Ok(decoded),
        Err(error) => Err(error.to_string().into()),
    }
}

/// Neo: a reqwest failure, as the error Node's request gives, and whether Playwright retries it
/// (Node's `ECONNRESET`: a reset, or a connection closed before its response).
fn send_error(error: &reqwest::Error) -> SendError {
    let mut source = error.source();
    let mut reset = false;
    let mut refused = false;
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<io::Error>() {
            reset |= io.kind() == ErrorKind::ConnectionReset;
            refused |= io.kind() == ErrorKind::ConnectionRefused;
        }
        reset |= cause
            .to_string()
            .contains("connection closed before message completed");
        source = cause.source();
    }
    let message = if refused {
        let target = error
            .url()
            .map(|url| {
                format!(
                    "{}:{}",
                    url.host_str().unwrap_or_default(),
                    url.port_or_known_default().unwrap_or_default()
                )
            })
            .unwrap_or_default();
        format!("connect ECONNREFUSED {target}")
    } else if reset {
        "read ECONNRESET".to_string()
    } else {
        error.to_string()
    };
    SendError { message, reset }
}

/// Neo: Node's Latin-1 encoding of a header value; `None` past U+00FF.
fn latin1_bytes(value: &str) -> Option<Vec<u8>> {
    value
        .chars()
        .map(|char| u8::try_from(u32::from(char)).ok())
        .collect()
}

/// Neo: Latin-1 text whose bytes are UTF-8, decoded leniently.
fn latin1_as_utf8(value: &str) -> String {
    match latin1_bytes(value) {
        Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        None => value.to_string(),
    }
}

fn now_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

fn now_seconds() -> f64 {
    now_millis() / 1000.0
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::sync::{Arc, MutexGuard};

    use flate2::Compression;
    use flate2::write::{GzEncoder, ZlibEncoder};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    /// One request the scripted server saw.
    #[derive(Clone, Debug, Default)]
    struct Seen {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(existing, _)| existing.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }
    }

    /// What the scripted server answers: raw response bytes, or a dropped connection.
    enum Answer {
        Bytes(Vec<u8>),
        Hangup,
    }

    fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Answer {
        let mut bytes = format!(
            "HTTP/1.1 {status}\r\nconnection: close\r\ncontent-length: {}\r\n",
            body.len()
        )
        .into_bytes();
        for (name, value) in headers {
            bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(body);
        Answer::Bytes(bytes)
    }

    /// A scripted HTTP/1.1 server on 127.0.0.1: `answer` gets each request and the count of
    /// those before it; every connection closes after its answer.
    async fn serve(
        answer: impl Fn(&Seen, usize) -> Answer + Send + Sync + 'static,
    ) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("bind: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("address: {error}"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let request = read_request(&mut socket).await;
                let count = log.lock().map(|mut log: MutexGuard<'_, Vec<Seen>>| {
                    log.push(request.clone());
                    log.len() - 1
                });
                match answer(&request, count.unwrap_or_default()) {
                    Answer::Bytes(bytes) => {
                        let _written = socket.write_all(&bytes).await;
                        let _flushed = socket.shutdown().await;
                    }
                    Answer::Hangup => drop(socket),
                }
            }
        });
        (format!("http://{address}"), seen)
    }

    async fn read_request(socket: &mut TcpStream) -> Seen {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break end;
            }
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => return Seen::default(),
                Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            }
        };
        let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
        let mut lines = head.split("\r\n");
        let mut request_line = lines.next().unwrap_or_default().split(' ');
        let method = request_line.next().unwrap_or_default().to_string();
        let path = request_line.next().unwrap_or_default().to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
            .collect();
        let length: usize = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.parse().ok())
            .unwrap_or_default();
        let mut body = buffer[header_end + 4..].to_vec();
        while body.len() < length {
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(read) => body.extend_from_slice(&chunk[..read]),
            }
        }
        Seen {
            method,
            path,
            headers,
            body,
        }
    }

    fn defaults() -> Defaults {
        Defaults {
            user_agent: "neo-test".into(),
            ..Defaults::default()
        }
    }

    fn params(url: &str) -> FetchParams {
        FetchParams {
            url: url.to_string(),
            ..FetchParams::default()
        }
    }

    async fn run(params: FetchParams) -> Result<Fetched, String> {
        fetch(&Clients::default(), &OwnJar::default(), &defaults(), params).await
    }

    fn seen(log: &Arc<Mutex<Vec<Seen>>>) -> Vec<Seen> {
        log.lock().map(|log| log.clone()).unwrap_or_default()
    }

    #[test]
    fn tests_that_headers_match_any_case_and_keep_their_first_casing() {
        let mut headers = Headers::default();
        headers.set("X-Foo", "1", false);
        headers.set("x-foo", "2", false);
        headers.set("X-FOO", "3", true);
        assert_eq!(headers.0, [("X-Foo".to_string(), "2".to_string())]);
        assert_eq!(headers.get("x-FOO"), Some("2"));
        headers.remove("x-foo");
        assert!(headers.0.is_empty());
    }

    #[tokio::test]
    async fn tests_that_a_get_sends_playwrights_headers_and_reads_the_response() {
        let (origin, log) = serve(|_, _| {
            response(
                "200 OK",
                &[
                    ("x-answer", "yes"),
                    ("set-cookie", "a=1"),
                    ("set-cookie", "b=2"),
                ],
                b"hello",
            )
        })
        .await;
        let fetched = run(FetchParams {
            headers: vec![("X-Mine".into(), "mine".into())],
            ..params(&format!("{origin}/path?q=1"))
        })
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            (
                fetched.status,
                fetched.status_text.as_str(),
                fetched.body.as_slice()
            ),
            (200, "OK", b"hello".as_slice())
        );
        assert_eq!(fetched.url, format!("{origin}/path?q=1"));
        assert_eq!(header(&fetched.headers, "x-answer"), Some("yes"));
        let set_cookie: Vec<&str> = fetched
            .headers
            .iter()
            .filter(|(name, _)| name == "set-cookie")
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(set_cookie, ["a=1", "b=2"], "each Set-Cookie value apart");
        assert!(
            fetched
                .server_addr
                .is_some_and(|address| address.ip().is_loopback())
        );
        assert!(
            fetched.timing.start_time > 0.0
                && fetched.timing.response_end >= fetched.timing.response_start
        );
        let request = &seen(&log)[0];
        assert_eq!(
            (request.method.as_str(), request.path.as_str()),
            ("GET", "/path?q=1")
        );
        assert_eq!(request.header("user-agent"), Some("neo-test"));
        assert_eq!(request.header("accept"), Some("*/*"));
        assert_eq!(request.header("accept-encoding"), Some(ACCEPT_ENCODING));
        assert_eq!(request.header("x-mine"), Some("mine"));
        assert!(
            fetched.log.iter().any(|line| line.starts_with("→ GET "))
                && fetched.log.iter().any(|line| line == "← 200 OK")
        );
    }

    #[tokio::test]
    async fn tests_that_redirects_change_method_and_body_as_fetch_says() {
        let (origin, log) = serve(|request, _| match request.path.as_str() {
            "/302" => response("302 Found", &[("location", "/end")], b""),
            "/303" => response("303 See Other", &[("location", "/end")], b""),
            "/307" => response("307 Temporary Redirect", &[("location", "/end")], b""),
            _ => response("200 OK", &[], b"end"),
        })
        .await;
        let post = |path: &str, method: &str| FetchParams {
            method: Some(method.to_string()),
            body: Body {
                json: Some("{\"a\":1}".into()),
                ..Body::default()
            },
            ..params(&format!("{origin}{path}"))
        };
        for (path, method, next_method, keeps_body) in [
            ("/302", "post", "GET", false),
            ("/302", "PUT", "PUT", true),
            ("/303", "PUT", "GET", false),
            ("/307", "POST", "POST", true),
        ] {
            let before = seen(&log).len();
            let fetched = run(post(path, method))
                .await
                .unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(fetched.url, format!("{origin}/end"));
            let hops = seen(&log);
            let (first, next) = (&hops[before], &hops[before + 1]);
            assert_eq!(first.header("content-length"), Some("7"));
            assert_eq!(next.method, next_method, "{path} {method}");
            assert_eq!(next.body.is_empty(), !keeps_body, "{path} {method}: body");
            assert_eq!(
                next.header("content-type").is_some(),
                keeps_body,
                "{path} {method}: content-type"
            );
            assert_eq!(next.header("host"), origin.strip_prefix("http://"));
        }
    }

    #[tokio::test]
    async fn tests_that_too_many_redirects_fail_and_zero_returns_the_redirect() {
        let (origin, _log) = serve(|request, _| match request.path.as_str() {
            "/a" => response("302 Found", &[("location", "/b")], b""),
            "/b" => response("302 Found", &[("location", "/c")], b""),
            "/bad" => response("302 Found", &[("location", "http://[")], b""),
            _ => response("200 OK", &[], b"c"),
        })
        .await;
        let limited = |max_redirects| FetchParams {
            max_redirects: Some(max_redirects),
            ..params(&format!("{origin}/a"))
        };
        assert_eq!(run(limited(1)).await.err().as_deref(), Some(MAX_REDIRECTS));
        assert_eq!(
            run(limited(2)).await.map(|fetched| fetched.body).ok(),
            Some(b"c".to_vec())
        );
        let zero = run(limited(0))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            (zero.status, header(&zero.headers, "location")),
            (302, Some("/b"))
        );
        assert_eq!(
            run(params(&format!("{origin}/bad"))).await.err(),
            Some(format!("{INVALID_REDIRECT}http://["))
        );
    }

    #[tokio::test]
    async fn tests_that_cookies_are_stored_per_hop_and_sent_on_the_next() {
        let (origin, log) = serve(|request, _| match request.path.as_str() {
            "/login" => response(
                "302 Found",
                &[("location", "/home"), ("set-cookie", "session=1; Path=/")],
                b"",
            ),
            _ => response("200 OK", &[("set-cookie", "seen=yes")], b"home"),
        })
        .await;
        let jar = OwnJar::default();
        fetch(
            &Clients::default(),
            &jar,
            &defaults(),
            params(&format!("{origin}/login")),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        let hops = seen(&log);
        assert_eq!(hops[0].header("cookie"), None);
        assert_eq!(
            hops[1].header("cookie"),
            Some("session=1"),
            "stored before the redirect"
        );
        let base = Url::parse(&origin).unwrap_or_else(|error| panic!("{error}"));
        let names: Vec<String> = jar
            .cookies(&base)
            .await
            .into_iter()
            .map(|cookie| cookie.name)
            .collect();
        assert_eq!(names, ["session", "seen"]);
        // A request's own Cookie header goes as it is, on the first hop only.
        fetch(
            &Clients::default(),
            &jar,
            &defaults(),
            FetchParams {
                headers: vec![("Cookie".into(), "mine=1".into())],
                ..params(&format!("{origin}/login"))
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        let hops = seen(&log);
        assert_eq!(hops[2].header("cookie"), Some("mine=1"));
        assert_eq!(hops[3].header("cookie"), Some("session=1; seen=yes"));
    }

    #[tokio::test]
    async fn tests_that_gzip_and_deflate_bodies_decode() {
        fn gzip(data: &[u8]) -> Vec<u8> {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            let _written = encoder.write_all(data);
            encoder.finish().unwrap_or_default()
        }
        fn zlib(data: &[u8]) -> Vec<u8> {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            let _written = encoder.write_all(data);
            encoder.finish().unwrap_or_default()
        }
        let (origin, _log) = serve(|request, _| match request.path.as_str() {
            "/gzip" => response("200 OK", &[("content-encoding", "GZIP")], &gzip(b"zipped")),
            "/deflate" => response(
                "200 OK",
                &[("content-encoding", "deflate")],
                &zlib(b"deflated"),
            ),
            "/both" => response("200 OK", &[("content-encoding", "gzip, br")], b"raw"),
            _ => response("200 OK", &[("content-encoding", "gzip")], b""),
        })
        .await;
        let body = |path: &str| {
            let url = format!("{origin}{path}");
            async move {
                run(params(&url))
                    .await
                    .map_or_else(|error| panic!("{error}"), |fetched| fetched.body)
            }
        };
        assert_eq!(body("/gzip").await, b"zipped");
        assert_eq!(body("/deflate").await, b"deflated");
        assert_eq!(
            body("/both").await,
            b"raw",
            "only an exact encoding decodes"
        );
        assert!(
            body("/empty").await.is_empty(),
            "an empty body skips the decoder"
        );
        let truncated = gzip(b"truncated body");
        assert!(
            decode(&truncated[..truncated.len() - 4], Some("gzip")).is_ok(),
            "a truncated stream gives what it has"
        );
    }

    #[tokio::test]
    async fn tests_that_a_reset_is_retried_then_reported() {
        let (origin, log) = serve(|_, count| {
            if count < 2 {
                Answer::Hangup
            } else {
                response("200 OK", &[], b"third")
            }
        })
        .await;
        let fetched = run(FetchParams {
            max_retries: Some(3),
            ..params(&origin)
        })
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(fetched.body, b"third");
        assert_eq!(seen(&log).len(), 3);
        assert_eq!(
            fetched
                .log
                .iter()
                .filter(|line| line.contains("Received ECONNRESET, will retry after"))
                .count(),
            2
        );
        let (always, _) = serve(|_, _| Answer::Hangup).await;
        assert_eq!(
            run(params(&always)).await.err().as_deref(),
            Some("read ECONNRESET")
        );
        assert_eq!(
            run(FetchParams {
                max_retries: Some(1),
                ..params(&always)
            })
            .await
            .err()
            .as_deref(),
            Some("Failed after 2 attempt(s): Error: read ECONNRESET")
        );
    }

    #[tokio::test]
    async fn tests_that_basic_auth_resends_once_on_a_challenge() {
        let (origin, log) = serve(|request, _| match request.header("authorization") {
            Some(auth) if auth == format!("Basic {}", STANDARD.encode("user:pass")) => {
                response("200 OK", &[], b"in")
            }
            _ => response(
                "401 Unauthorized",
                &[("www-authenticate", " Basic realm=\"x\"")],
                b"no",
            ),
        })
        .await;
        let with = |origin_filter: Option<String>, send_always: bool| Defaults {
            http_credentials: Some(HttpCredentials {
                username: "user".into(),
                password: "pass".into(),
                origin: origin_filter,
                send_always,
            }),
            ..defaults()
        };
        let clients = Clients::default();
        let jar = OwnJar::default();
        let fetched = fetch(&clients, &jar, &with(None, false), params(&origin))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            (fetched.status, fetched.body.as_slice()),
            (200, b"in".as_slice())
        );
        assert_eq!(seen(&log).len(), 2, "one challenge, one resend");
        let elsewhere = fetch(
            &clients,
            &jar,
            &with(Some("http://other.test".into()), false),
            params(&origin),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            elsewhere.status, 401,
            "credentials for another origin are not sent"
        );
        let before = seen(&log).len();
        let upfront = fetch(&clients, &jar, &with(None, true), params(&origin))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            (upfront.status, seen(&log).len() - before),
            (200, 1),
            "send: always needs no challenge"
        );
    }

    #[tokio::test]
    async fn tests_that_fail_on_status_code_reports_the_status_and_text() {
        let long = "x".repeat(1200);
        let (origin, _log) = serve(move |request, _| match request.path.as_str() {
            "/long" => response("500 Internal Server Error", &[], long.as_bytes()),
            "/empty" => response("404 Not Found", &[], b""),
            _ => response("404 Not Found", &[], b"File not found"),
        })
        .await;
        let failing = |path: &str| FetchParams {
            fail_on_status_code: Some(true),
            ..params(&format!("{origin}{path}"))
        };
        assert_eq!(
            run(failing("/missing")).await.err().as_deref(),
            Some("404 Not Found\nResponse text:\nFile not found")
        );
        assert_eq!(
            run(failing("/empty")).await.err().as_deref(),
            Some("404 Not Found")
        );
        let long_error = run(failing("/long")).await.err().unwrap_or_default();
        assert!(
            long_error.ends_with(&format!("{}...", "x".repeat(997)))
                && !long_error.contains(&"x".repeat(998))
        );
        assert_eq!(
            run(params(&format!("{origin}/missing")))
                .await
                .map(|fetched| fetched.status)
                .ok(),
            Some(404),
            "off by default"
        );
    }

    #[tokio::test]
    async fn tests_that_bodies_serialize_as_playwrights_do() {
        let (origin, log) = serve(|_, _| response("200 OK", &[], b"")).await;
        let send = |body: Body, headers: Vec<(String, String)>| FetchParams {
            method: Some("POST".into()),
            body,
            headers,
            ..params(&origin)
        };
        let last = |log: &Arc<Mutex<Vec<Seen>>>| seen(log).pop().unwrap_or_default();
        run(send(
            Body {
                form: Some(vec![("a b".into(), "c&d".into()), ("e".into(), "ж".into())]),
                ..Body::default()
            },
            vec![],
        ))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        let form = last(&log);
        assert_eq!(
            form.header("content-type"),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(String::from_utf8_lossy(&form.body), "a+b=c%26d&e=%D0%B6");
        run(send(
            Body {
                raw: Some(vec![1, 2, 3]),
                ..Body::default()
            },
            vec![("Content-Type".into(), "text/mine".into())],
        ))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        let raw = last(&log);
        assert_eq!(
            (
                raw.header("content-type"),
                raw.body.as_slice(),
                raw.header("content-length")
            ),
            (Some("text/mine"), [1, 2, 3].as_slice(), Some("3")),
            "a given content type stays"
        );
        run(send(
            Body {
                multipart: Some(vec![
                    FormField::Value {
                        name: "first".into(),
                        value: "John".into(),
                    },
                    FormField::File {
                        name: "file".into(),
                        file_name: "f\"1\".js".into(),
                        mime_type: None,
                        buffer: b"var x;".to_vec(),
                    },
                ]),
                ..Body::default()
            },
            vec![],
        ))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        let multipart = last(&log);
        let content_type = multipart
            .header("content-type")
            .unwrap_or_default()
            .to_string();
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap_or_default();
        assert!(
            boundary.starts_with("----WebKitFormBoundary") && boundary.len() == 38,
            "{content_type}"
        );
        assert_eq!(
            String::from_utf8_lossy(&multipart.body),
            format!(
                "--{boundary}\r\ncontent-disposition: form-data; name=\"first\"\r\n\r\nJohn\r\n--{boundary}\r\ncontent-disposition: form-data; name=\"file\"; filename=\"f%221%22.js\"\r\ncontent-type: text/javascript\r\n\r\nvar x;\r\n--{boundary}--\r\n"
            )
        );
        assert_eq!(
            run(send(
                Body {
                    json: Some("1".into()),
                    raw: Some(vec![1]),
                    ..Body::default()
                },
                vec![]
            ))
            .await
            .err()
            .as_deref(),
            Some(ONLY_ONE_BODY)
        );
    }

    #[test]
    fn tests_that_params_and_base_url_build_the_request_url() {
        let built = |base: Option<&str>, params: FetchParams| {
            request_url(base, &params).map(|url| url.to_string())
        };
        assert_eq!(
            built(Some("http://a.test/x/"), params("y")).as_deref(),
            Ok("http://a.test/x/y")
        );
        assert_eq!(
            built(Some("not a url"), params("http://b.test/")).as_deref(),
            Ok("http://b.test/"),
            "a base that does not parse is ignored"
        );
        assert_eq!(
            built(None, params("/relative")).err().as_deref(),
            Some("Invalid URL")
        );
        assert_eq!(
            built(
                None,
                FetchParams {
                    encoded_params: Some("?a=1&b=%20".into()),
                    ..params("http://a.test/p?q=0")
                }
            )
            .as_deref(),
            Ok("http://a.test/p?q=0&a=1&b=%20")
        );
        assert_eq!(
            built(
                None,
                FetchParams {
                    params: vec![("ключ".into(), "a b".into())],
                    ..params("http://a.test/p")
                }
            )
            .as_deref(),
            Ok("http://a.test/p?%D0%BA%D0%BB%D1%8E%D1%87=a+b")
        );
    }
}
