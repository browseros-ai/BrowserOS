//! OAuth for the three subscription providers (ChatGPT Pro via PKCE + the `127.0.0.1:1455`
//! callback listener; GitHub Copilot and Qwen Code via device codes), token persistence and
//! single-flight refresh. `chat` pulls credentials from here at turn start. Implemented by P10.

mod callback_server;
mod providers;
mod token_manager;

pub use callback_server::CallbackServer;
pub use providers::{OAuthProvider, ProviderSpec};
pub use token_manager::{
    OAuthCredentials, OAuthError, OAuthStatus, StartFlow, TokenInput, TokenManager,
};
