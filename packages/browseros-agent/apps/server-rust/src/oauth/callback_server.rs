//! The PKCE callback listener on `127.0.0.1:1455` (`/auth/callback`, `/cancel`): bound lazily
//! when a flow starts, evicting a previous holder via `GET /cancel`, stopped when no PKCE flow is
//! pending. A separate listener, deliberately outside `http::ROUTES` and the contract.

#[derive(Debug, Default)]
pub struct CallbackServer {
    _private: (),
}

impl CallbackServer {
    pub const PORT: u16 = 1455;

    pub fn new() -> Self {
        Self::default()
    }

    pub async fn stop(&self) {}
}
