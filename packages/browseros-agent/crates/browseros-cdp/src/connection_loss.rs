use std::fmt;

/// Why the client's socket to the browser went down, as the reader or the keepalive saw it.
///
/// A request the loss cut off fails with the bare [`crate::CdpError::ConnectionLost`], which
/// every consumer matches on; the cause lives here instead, on
/// [`crate::CdpClient::last_connection_loss`], so adding it changed no consumer's match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionLoss {
    /// The browser sent a close frame. One without a status reads as 1005, the code RFC 6455
    /// reserves for "no status received".
    Closed { code: u16, reason: String },
    /// The socket ended without a close frame.
    Ended,
    /// Reading the socket failed, in the WebSocket library's words: an I/O error, or a reset
    /// without the closing handshake.
    Read(String),
    /// The keepalive's `Browser.getVersion` and ping went unanswered for `keepalive_timeout`.
    KeepaliveTimeout,
}

impl fmt::Display for ConnectionLoss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed { code, reason } if reason.is_empty() => {
                write!(f, "the browser closed the socket with code {code}")
            }
            Self::Closed { code, reason } => {
                write!(
                    f,
                    "the browser closed the socket with code {code}: {reason}"
                )
            }
            Self::Ended => f.write_str("the socket ended without a close frame"),
            Self::Read(error) => write!(f, "reading the socket failed: {error}"),
            Self::KeepaliveTimeout => f.write_str("the keepalive went unanswered"),
        }
    }
}
