//! THE SEAM for "Sign in with ChatGPT": everything about it lives in this file. Nothing outside
//! it knows a client id, the callback port (1455) or a token, so moving to OpenAI's official
//! sign-in flow replaces the inside of this file and keeps `SignIn`'s three methods and
//! `Model::step` (design section 3.6). It is also the only writer of `store::Tokens`. It knows
//! nothing about the Responses wire: `endpoint()` hands `model::responses` a URL, headers and a
//! dialect, and the wire is spoken there (CORRECTIONS-2 section 2).

use super::responses::ResponsesEndpoint;
use crate::store::Store;
use crate::turn::ChatError;

/// What `http` sees of the ChatGPT sign-in: three calls and no tokens.
pub struct SignIn {
    #[expect(dead_code, reason = "read once chunk `chatgpt` writes the sign-in")]
    store: Store,
    install_id: Option<String>,
}

/// Why a sign-in cannot start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInError {
    /// Port 1455 is held by a program that would not let go.
    PortBusy,
    /// No readable install id: tokens are keyed by it, so they could not be stored.
    NoInstallId,
    /// Anything else that stopped the flow; the text is one sentence for the extension.
    Failed(String),
}

/// Whether tokens are stored for this install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInStatus {
    pub authenticated: bool,
    pub email: Option<String>,
}

impl SignIn {
    pub(super) fn new(store: Store, install_id: Option<String>) -> Self {
        Self { store, install_id }
    }

    /// Binds `127.0.0.1:1455` if it is not already bound and returns the authorize URL.
    pub async fn start(&self) -> Result<url::Url, SignInError> {
        if self.install_id.is_none() {
            return Err(SignInError::NoInstallId);
        }
        Err(SignInError::Failed(
            "Sign in with ChatGPT is not implemented in this build of the server.".into(),
        ))
    }

    pub async fn status(&self) -> SignInStatus {
        SignInStatus {
            authenticated: false,
            email: None,
        }
    }

    pub async fn sign_out(&self) {}

    /// Refreshes the token if it is within five minutes of expiry, then returns the Codex
    /// endpoint: the URL, `Authorization: Bearer`, `ChatGPT-Account-Id`, `originator: browseros`,
    /// `OpenAI-Beta: responses=experimental`, and `ResponsesDialect::ChatGptCodex`.
    #[expect(
        dead_code,
        reason = "called once chunk `models` writes the ChatGPT arm of the dispatch"
    )]
    pub(super) async fn endpoint(&self) -> Result<ResponsesEndpoint, ChatError> {
        Err(ChatError::not_implemented("Sign in with ChatGPT"))
    }
}
