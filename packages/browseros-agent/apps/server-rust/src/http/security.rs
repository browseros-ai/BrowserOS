//! Request trust: the trusted-origin guard (every route), `require_trusted_app` (Auth::TrustedApp
//! routes), the browser-fetch rejection on `/mcp`, and CORS without PUT. `is_trusted_app_request`
//! is ported verbatim from TS, quirks included (Referer loopback check, `[::1]` Host failing,
//! no-Origin mutating requests rejected). Implemented by P3.

/// `isTrustedAppRequest`, computed once per request and handed to handlers that decide
/// conditionally (`Auth::TrustedAppInHandler`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RequestTrust {
    pub trusted_app: bool,
}
