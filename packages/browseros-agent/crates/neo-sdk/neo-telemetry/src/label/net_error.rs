//! Why a navigation failed on the network, as a label.

/// A navigation's network failure: the `error.type` of `neo_navigation_errors_total`. The
/// Chromium net errors a browsing agent meets, each spelled as Chromium's `net::ERR_<NAME>`
/// lowercased without its prefix; every other one is [`NetError::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NetError {
    /// `net::ERR_ABORTED`: the load was stopped (a `javascript:` URL ends this way too).
    Aborted,
    /// `net::ERR_ADDRESS_UNREACHABLE`.
    AddressUnreachable,
    /// `net::ERR_BLOCKED_BY_CLIENT`: an extension or policy in the browser blocked it.
    BlockedByClient,
    /// `net::ERR_BLOCKED_BY_RESPONSE`: the response's own headers forbade the load.
    BlockedByResponse,
    /// `net::ERR_CERT_AUTHORITY_INVALID`.
    CertAuthorityInvalid,
    /// `net::ERR_CERT_COMMON_NAME_INVALID`.
    CertCommonNameInvalid,
    /// `net::ERR_CERT_DATE_INVALID`.
    CertDateInvalid,
    /// `net::ERR_CONNECTION_CLOSED`.
    ConnectionClosed,
    /// `net::ERR_CONNECTION_REFUSED`.
    ConnectionRefused,
    /// `net::ERR_CONNECTION_RESET`.
    ConnectionReset,
    /// `net::ERR_CONNECTION_TIMED_OUT`.
    ConnectionTimedOut,
    /// `net::ERR_EMPTY_RESPONSE`.
    EmptyResponse,
    /// `net::ERR_FAILED`.
    Failed,
    /// `net::ERR_HTTP2_PROTOCOL_ERROR`.
    Http2ProtocolError,
    /// `net::ERR_INTERNET_DISCONNECTED`.
    InternetDisconnected,
    /// `net::ERR_NAME_NOT_RESOLVED`: the host has no address.
    NameNotResolved,
    /// `net::ERR_SSL_PROTOCOL_ERROR`.
    SslProtocolError,
    /// `net::ERR_TIMED_OUT`.
    TimedOut,
    /// `net::ERR_TOO_MANY_REDIRECTS`.
    TooManyRedirects,
    /// `net::ERR_UNSAFE_PORT`.
    UnsafePort,
    /// Any other net error, or an error page the browser committed without naming one:
    /// `error.type`'s registered fallback, `_OTHER`.
    Other,
}

impl NetError {
    /// Every value, in wire order.
    pub const ALL: [Self; 21] = [
        Self::Aborted,
        Self::AddressUnreachable,
        Self::BlockedByClient,
        Self::BlockedByResponse,
        Self::CertAuthorityInvalid,
        Self::CertCommonNameInvalid,
        Self::CertDateInvalid,
        Self::ConnectionClosed,
        Self::ConnectionRefused,
        Self::ConnectionReset,
        Self::ConnectionTimedOut,
        Self::EmptyResponse,
        Self::Failed,
        Self::Http2ProtocolError,
        Self::InternetDisconnected,
        Self::NameNotResolved,
        Self::SslProtocolError,
        Self::TimedOut,
        Self::TooManyRedirects,
        Self::UnsafePort,
        Self::Other,
    ];

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aborted => "aborted",
            Self::AddressUnreachable => "address_unreachable",
            Self::BlockedByClient => "blocked_by_client",
            Self::BlockedByResponse => "blocked_by_response",
            Self::CertAuthorityInvalid => "cert_authority_invalid",
            Self::CertCommonNameInvalid => "cert_common_name_invalid",
            Self::CertDateInvalid => "cert_date_invalid",
            Self::ConnectionClosed => "connection_closed",
            Self::ConnectionRefused => "connection_refused",
            Self::ConnectionReset => "connection_reset",
            Self::ConnectionTimedOut => "connection_timed_out",
            Self::EmptyResponse => "empty_response",
            Self::Failed => "failed",
            Self::Http2ProtocolError => "http2_protocol_error",
            Self::InternetDisconnected => "internet_disconnected",
            Self::NameNotResolved => "name_not_resolved",
            Self::SslProtocolError => "ssl_protocol_error",
            Self::TimedOut => "timed_out",
            Self::TooManyRedirects => "too_many_redirects",
            Self::UnsafePort => "unsafe_port",
            Self::Other => "_OTHER",
        }
    }

    /// The label of Chromium's error text, such as `net::ERR_NAME_NOT_RESOLVED`: its own value
    /// when listed, else [`NetError::Other`]. Matched whole, so no text a page chose can pick a
    /// value.
    #[must_use]
    pub fn of_chromium(text: &str) -> Self {
        text.strip_prefix("net::ERR_")
            .and_then(|name| {
                Self::ALL
                    .into_iter()
                    .find(|error| error.as_str().eq_ignore_ascii_case(name))
            })
            .unwrap_or(Self::Other)
    }
}
