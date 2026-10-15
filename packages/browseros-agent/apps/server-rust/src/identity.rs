//! Who this server is: the BrowserOS id (keys OAuth tokens and Klavis), the installation id
//! (`installation.json`, created race-safely like `installation-id.ts`), and the sidecar's
//! client id. Owned by P3.

use crate::{config::Config, paths::BrowserosPaths};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub browseros_id: String,
    pub install_id: Option<String>,
    pub client_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("failed to resolve installation identity: {0}")]
pub struct IdentityError(pub String);

impl Identity {
    /// S0 stub: identity from the sidecar only; P3 reads/creates `installation.json`.
    pub async fn load(config: &Config, paths: &BrowserosPaths) -> Result<Self, IdentityError> {
        let _ = paths;
        Ok(Self {
            browseros_id: config.instance.install_id.clone().unwrap_or_default(),
            install_id: config.instance.install_id.clone(),
            client_id: config.instance.client_id.clone(),
        })
    }
}
