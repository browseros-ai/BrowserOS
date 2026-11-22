//! The trust rules (design section 4.3). One middleware, `guard`, covers every contract route.
//! The system routes get the narrower `loopback_only` and `supervisor_only`, because the browser's
//! own C++ calls them with no `Origin`. The private MCP endpoint applies its own rule in `browser`.
//!
//! There are no tokens, cookies or accounts. What keeps a web page out:
//!
//! 1. The listener is `127.0.0.1` only (in `app`).
//! 2. `Host` must name `127.0.0.1` or `localhost`, on every route. This stops a page that points
//!    its own domain at loopback (DNS rebinding).
//! 3. A request that carries `Origin` must carry an allowed one: the extension's, or one listed
//!    in `BROWSEROS_TRUSTED_ORIGINS`.
//! 4. A request with no `Origin` may only read (`GET`, `HEAD`, `OPTIONS`): a page cannot read such
//!    a response, and a local program could read the SQLite file anyway. The one `GET` that
//!    changes state, sign-in start, also refuses what `page_can_send_without_origin` describes.
//! 5. Shutdown serves the browser itself and native callers only (`supervisor_only`): nothing a
//!    web page can make the browser send reaches it.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::ApiError;
use crate::app::Launch;

/// The extension's fixed origin, from its packaged key.
pub const EXTENSION_ORIGIN: &str = "chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk";

// Fetch metadata. Chromium sends these to every potentially trustworthy URL, loopback included.
const SEC_FETCH_SITE: &str = "sec-fetch-site";
const SEC_FETCH_MODE: &str = "sec-fetch-mode";

/// Origins from `BROWSEROS_TRUSTED_ORIGINS` (comma separated), normalised to what a browser
/// sends. An entry that is not an origin is dropped with a warning.
pub fn parse_trusted_origins(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let normalized = normalize_origin(entry);
            if normalized.is_none() {
                tracing::warn!(entry, "ignoring an invalid BROWSEROS_TRUSTED_ORIGINS entry");
            }
            normalized
        })
        .collect()
}

/// `scheme://host[:port]` of a URL. Extension URLs have an opaque origin in the URL standard, so
/// they are rebuilt from their scheme and host, as the browser spells them in `Origin`.
fn normalize_origin(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    let host = url.host_str().filter(|host| !host.is_empty())?;
    Some(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

/// Refuses an untrusted request with `403 {"error":"Forbidden"}`, answers a trusted preflight
/// itself, and echoes `Access-Control-Allow-Origin` only for an allowed origin.
pub(super) async fn guard(
    State(launch): State<Arc<Launch>>,
    request: Request,
    next: Next,
) -> Response {
    if !loopback_host(request.headers()) {
        return ApiError::forbidden().into_response();
    }
    let origin = request.headers().get(header::ORIGIN).cloned();
    match origin {
        None if is_read(request.method()) => next.run(request).await,
        None => ApiError::forbidden().into_response(),
        Some(origin) if allowed(&origin, &launch.trusted_origins) => {
            if request.method() == Method::OPTIONS {
                let mut response = StatusCode::NO_CONTENT.into_response();
                add_cors(response.headers_mut(), origin);
                return response;
            }
            let mut response = next.run(request).await;
            add_cors(response.headers_mut(), origin);
            response
        }
        Some(_) => ApiError::forbidden().into_response(),
    }
}

/// The rule for the system routes the browser polls: `Host` only. Health and status must answer
/// the browser's C++, which sends no `Origin`, and they hold nothing but liveness, update
/// readiness and the version.
pub(super) async fn loopback_only(request: Request, next: Next) -> Response {
    if !loopback_host(request.headers()) {
        return ApiError::forbidden().into_response();
    }
    next.run(request).await
}

/// Shutdown's rule: a loopback `Host`, no `Origin`, and a `Sec-Fetch-Site` that is absent or
/// `none`. Every `POST` a web page makes carries `Origin`, and its `Sec-Fetch-Site` names the
/// page's relation to the server (`cross-site`, `same-site`, `same-origin`). `none` means the
/// browser itself started the request: BrowserOS's C++ supervisor sends it (measured on its
/// health poll), as would the older supervisor that calls shutdown and builds its request the
/// same way. Refusing that supervisor would not keep the server up: it answers any status but
/// 200 with SIGKILL. `curl` sends no fetch metadata. Without this rule any web page could stop
/// the server, and a shipped browser does not start it again after a clean exit.
///
/// The private MCP endpoint refuses every `Sec-Fetch-Site`, `none` included: only our own coding
/// agent calls it, and that is not a browser.
pub(super) async fn supervisor_only(request: Request, next: Next) -> Response {
    let headers = request.headers();
    let from_page = headers.contains_key(header::ORIGIN)
        || headers
            .get(SEC_FETCH_SITE)
            .is_some_and(|site| site.as_bytes() != b"none");
    if !loopback_host(headers) || from_page {
        return ApiError::forbidden().into_response();
    }
    next.run(request).await
}

/// True for a request a web page can make the browser send without `Origin`: an image, a link, a
/// form `GET`, a no-cors fetch. All of them carry a `Sec-Fetch-Mode` other than `cors`. A page's
/// CORS-mode request always carries `Origin`, which `guard` checks, so a `GET` that changes state
/// can refuse these and still serve the extension's `fetch` (mode `cors`) and native callers
/// (no `Sec-Fetch-Mode` at all).
pub(super) fn page_can_send_without_origin(headers: &HeaderMap) -> bool {
    headers
        .get(SEC_FETCH_MODE)
        .is_some_and(|mode| mode.as_bytes() != b"cors")
}

fn is_read(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn loopback_host(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let name = match host.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    };
    name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1"
}

fn allowed(origin: &HeaderValue, trusted: &[String]) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    origin == EXTENSION_ORIGIN || trusted.iter().any(|entry| entry == origin)
}

fn add_cors(headers: &mut HeaderMap, origin: HeaderValue) {
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Accept"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_origins_are_normalised_and_bad_entries_dropped() {
        assert_eq!(
            parse_trusted_origins(
                " http://localhost:5173/path , not a url, chrome-extension://abc/x "
            ),
            vec![
                "http://localhost:5173".to_owned(),
                "chrome-extension://abc".to_owned()
            ]
        );
    }

    #[test]
    fn only_loopback_names_pass_the_host_check() {
        let host = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, HeaderValue::from_static(value));
            loopback_host(&headers)
        };
        assert!(host("127.0.0.1:9200"));
        assert!(host("localhost"));
        assert!(!host("evil.example:9200"));
        assert!(!host("127.0.0.1.evil.example"));
        assert!(!loopback_host(&HeaderMap::new()));
    }
}
