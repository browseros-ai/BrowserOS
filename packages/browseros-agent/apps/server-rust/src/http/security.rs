//! Hono-compatible ingress policy: CORS runs first, then the exact origin allowlist,
//! then prefix auth. Socket trust is never inferred from forwarding headers; handlers
//! receive the computed decision for routes whose auth depends on the request body.

use super::errors;
use crate::{app::AppState, config::EnvConfig};
use axum::{
    extract::{ConnectInfo, FromRequestParts, Request, State},
    middleware::Next,
    response::Response,
};
use futures_util::StreamExt;
use http::{HeaderMap, HeaderValue, Method, StatusCode, header, request::Parts};
use std::{
    convert::Infallible,
    net::{IpAddr, SocketAddr},
};
use url::Url;

/// Cached ingress evidence used by handlers with body-dependent trusted-app rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RequestTrust {
    pub trusted_app: bool,
}

impl<S: Send + Sync> FromRequestParts<S> for RequestTrust {
    type Rejection = Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<Self>().copied().unwrap_or_default())
    }
}

/// Normalized once per app, matching TS's process-cached allowlist. Request Origin
/// values are compared verbatim: only configuration entries are normalized.
#[derive(Clone)]
pub struct OriginPolicy(Vec<String>);
impl OriginPolicy {
    pub fn new(env: &EnvConfig) -> Self {
        let mut origins = vec!["chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk".to_owned()];
        for configured in &env.trusted_origins {
            if let Some(origin) = normalize_origin(configured.trim()) {
                origins.push(origin);
            } else {
                tracing::warn!(
                    value = configured,
                    "Ignoring invalid BROWSEROS_TRUSTED_ORIGINS entry"
                );
            }
        }
        Self(origins)
    }
    pub fn allows(&self, origin: &str) -> bool {
        self.0.iter().any(|allowed| allowed == origin)
    }
}

fn normalize_origin(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    if !url.has_host() {
        return None;
    }
    let origin = if matches!(url.scheme(), "chrome-extension" | "moz-extension") {
        format!(
            "{}://{}",
            url.scheme(),
            &url[url::Position::BeforeHost..url::Position::AfterPort]
        )
    } else {
        url.origin().ascii_serialization()
    };
    (origin != "null").then_some(origin)
}

fn value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

pub fn is_trusted_app_request(
    method: &Method,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> bool {
    let loopback = peer.is_some_and(|peer| match peer.ip() {
        IpAddr::V4(ip) => ip == std::net::Ipv4Addr::LOCALHOST,
        IpAddr::V6(ip) => {
            ip == std::net::Ipv6Addr::LOCALHOST
                || ip.to_ipv4_mapped() == Some(std::net::Ipv4Addr::LOCALHOST)
        }
    });
    // TS deliberately splits at the first colon, so even [::1]:port fails here.
    let host = value(headers, "host").and_then(|host| host.split(':').next());
    if !loopback || !matches!(host, Some("127.0.0.1" | "localhost")) {
        return false;
    }
    if let Some(referer) = value(headers, "referer").filter(|v| !v.is_empty()) {
        let Ok(url) = Url::parse(referer) else {
            return false;
        };
        if !matches!(url.host_str(), Some("localhost" | "127.0.0.1")) {
            return false;
        }
    }
    match value(headers, "origin").filter(|v| !v.is_empty()) {
        Some(origin) => is_trusted_app_origin(origin),
        None => matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS),
    }
}

pub fn is_trusted_app_origin(origin: &str) -> bool {
    let Ok(url) = Url::parse(origin) else {
        return false;
    };
    matches!(url.scheme(), "chrome-extension" | "moz-extension")
        || (matches!(url.scheme(), "http" | "https")
            && matches!(
                url.host_str(),
                Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
            ))
}

/// Paths only the trusted app may call, as prefixes so unknown descendants are 403 like Hono's
/// `app.use('/agents/*', …)` rather than 404. [`super::Auth::TrustedApp`] in the route table
/// documents the same set; the test below keeps the two in agreement.
fn protected_prefix(path: &str) -> bool {
    [
        "/acpx/probe",
        "/agents",
        "/conversations",
        "/providers",
        "/test-provider",
        "/refine-prompt",
        "/scheduled-jobs",
        "/scheduled-job-runs",
    ]
    .iter()
    .any(|prefix| {
        path == *prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// One middleware owns ordering including fallback paths. A route-only layer would
/// miss auth on unknown protected descendants and return 404 where Hono returns 403.
pub async fn ingress(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let policy = &state.origins;
    let origin = request.headers().get(header::ORIGIN).cloned();
    let allowed = origin
        .as_ref()
        .and_then(|origin| origin.to_str().ok())
        .is_some_and(|origin| policy.allows(origin));
    let preflight = request.method() == Method::OPTIONS;
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    let trust = RequestTrust {
        trusted_app: is_trusted_app_request(request.method(), request.headers(), peer),
    };
    request.extensions_mut().insert(trust);
    let mut response = if preflight {
        Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(axum::body::Body::empty())
            .unwrap_or_default()
    } else if origin.is_some() && !allowed {
        errors::forbidden_origin()
    } else if protected_prefix(request.uri().path()) && !trust.trusted_app {
        errors::error_string(StatusCode::FORBIDDEN, "Forbidden")
    } else if (request.uri().path() == "/mcp" || request.uri().path().starts_with("/mcp/"))
        && request.headers().contains_key("sec-fetch-site")
    {
        errors::forbidden_browser_request()
    } else {
        let activity = (request.method() == Method::POST && request.uri().path() == "/mcp")
            .then(|| state.activity.reserve());
        let response = next.run(request).await;
        if let Some(activity) = activity {
            let (parts, body) = response.into_parts();
            // rmcp may execute while its response stream is being polled. Moving
            // the guard into the body covers that async handoff and cancellation.
            let stream = futures_util::stream::unfold(
                (body.into_data_stream(), activity),
                |(mut stream, guard)| async move {
                    stream.next().await.map(|chunk| (chunk, (stream, guard)))
                },
            );
            Response::from_parts(parts, axum::body::Body::from_stream(stream))
        } else {
            response
        }
    };
    let headers = response.headers_mut();
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    if allowed && let Some(origin) = origin {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    if preflight {
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET,POST,DELETE,OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Content-Type,Authorization,Accept"),
        );
        headers.append(
            header::VARY,
            HeaderValue::from_static("Access-Control-Request-Headers"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::protected_prefix;
    use crate::http::{Auth, ROUTES};

    /// `Route.auth` is what a reader consults; `protected_prefix` is what runs. A
    /// `TrustedApp` route outside the prefix list would be open to any origin, and an
    /// `Origin` or handler-decided route inside it would be refused before its handler runs.
    #[test]
    fn route_auth_column_matches_the_ingress_prefix_guard() {
        let drift: Vec<_> = ROUTES
            .iter()
            .filter(|route| protected_prefix(route.path) != (route.auth == Auth::TrustedApp))
            .map(|route| format!("{} {} ({:?})", route.method, route.path, route.auth))
            .collect();
        assert!(drift.is_empty(), "route auth drifted from protected_prefix: {drift:?}");
    }
}
