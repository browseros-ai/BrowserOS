//! The two files billing keeps, both owner-only under `<data dir>/billing/`: the install's id
//! and secret, and the last verified licence. The licence key itself is never written.

use super::{licence::SignedLicence, secret::Secret};
use crate::{
    error::{AppError, AppResult},
    storage::JsonStore,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const INSTALL_FILE: &str = "billing/install.json";
pub const LICENCE_FILE: &str = "billing/licence.json";

/// This computer as the billing gateway knows it.
#[derive(Clone, Debug)]
pub struct Install {
    pub id: String,
    pub secret: Secret,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallFile {
    install_id: String,
    install_secret: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LicenceFile {
    #[serde(flatten)]
    signed: SignedLicence,
    /// Whether this computer is on a licence key, which a licence whose plan has ended cannot
    /// say by itself.
    #[serde(default)]
    on_key: bool,
}

#[derive(Clone, Debug)]
pub struct BillingStore {
    json: JsonStore,
}

impl BillingStore {
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            json: JsonStore::new(data_dir),
        }
    }

    pub async fn load_install(&self) -> Option<Install> {
        let file: InstallFile = self.read(INSTALL_FILE).await?;
        Some(Install {
            id: file.install_id,
            secret: Secret::new(file.install_secret),
        })
    }

    pub async fn save_install(&self, install: &Install) -> AppResult<()> {
        self.json
            .write_json_private(
                INSTALL_FILE,
                &InstallFile {
                    install_id: install.id.clone(),
                    install_secret: install.secret.expose().to_string(),
                },
            )
            .await
    }

    pub async fn load_licence(&self) -> Option<(SignedLicence, bool)> {
        let file: LicenceFile = self.read(LICENCE_FILE).await?;
        Some((file.signed, file.on_key))
    }

    pub async fn save_licence(&self, signed: &SignedLicence, on_key: bool) -> AppResult<()> {
        self.json
            .write_json_private(
                LICENCE_FILE,
                &LicenceFile {
                    signed: signed.clone(),
                    on_key,
                },
            )
            .await
    }

    /// Removes both files, for an install the gateway no longer knows.
    pub async fn forget(&self) -> AppResult<()> {
        self.json.remove_file(LICENCE_FILE).await?;
        self.json.remove_file(INSTALL_FILE).await?;
        Ok(())
    }

    /// A missing or unreadable file reads as none; the next save replaces it.
    async fn read<T: serde::de::DeserializeOwned>(&self, path: &str) -> Option<T> {
        match self.json.read_json(path).await {
            Ok(value) => Some(value),
            Err(AppError::StorageNotFound(_)) => None,
            // The parse error is not logged: serde can quote the value it choked on, which
            // here could be the install secret.
            Err(_) => {
                tracing::warn!(file = path, "billing file unreadable; ignoring it");
                None
            }
        }
    }
}
