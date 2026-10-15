//! OAuth for the three subscription providers (ChatGPT Pro via PKCE + the `127.0.0.1:1455`
//! callback listener; GitHub Copilot and Qwen Code via device codes), token persistence and
//! single-flight refresh. `chat` pulls credentials from here at turn start.

mod callback_server;
mod providers;
mod token_manager;

pub(crate) use token_manager::expires_at;

pub use callback_server::CallbackServer;
pub use providers::{OAuthProvider, ProviderSpec};
pub use token_manager::{
    OAuthCredentials, OAuthError, OAuthStatus, StartFlow, TokenInput, TokenManager,
};
