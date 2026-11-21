//! The one trust rule (design section 4.3), applied by one middleware to every contract route.
//! The system routes are outside it because the browser's own C++ calls them, and the private
//! MCP endpoint applies its own, stricter rule in `browser`.
//!
//! There are no tokens, cookies or accounts. What keeps a web page out:
//!
//! 1. The listener is `127.0.0.1` only (in `app`).
//! 2. `Host` must name `127.0.0.1` or `localhost`, which stops a page that points its own domain
//!    at loopback (DNS rebinding).
//! 3. A request that carries `Origin` must carry an allowed one: the extension's, or one listed
//!    in `BROWSEROS_TRUSTED_ORIGINS`.
//! 4. A request with no `Origin` may only read (`GET`, `HEAD`, `OPTIONS`): a page cannot read such
//!    a response, and a local program could read the SQLite file anyway.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::ApiError;
use crate::app::Launch;

/// The extension's fixed origin, from its packaged key.
pub const EXTENSION_ORIGIN: &str = "chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk";

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
