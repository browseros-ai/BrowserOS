//! Canonical installation identity is shared with Chromium, OAuth and Klavis.
//! Publish complete files without clobbering another starter; damaged durable state
//! stays untouched and only this process falls back to an ephemeral functional ID.

use crate::{config::Config, paths::BrowserosPaths};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

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
    pub async fn load(config: &Config, paths: &BrowserosPaths) -> Result<Self, IdentityError> {
        let install_id = match load_or_create(paths).await {
            Ok(id) => Some(id),
            Err(error) => {
                tracing::error!(%error, "Installation identity unavailable; telemetry correlation disabled");
                None
            }
        };
        Ok(Self {
            browseros_id: install_id
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            install_id,
            client_id: config.instance.client_id.clone(),
        })
    }
}

async fn read_identity(path: &std::path::Path) -> std::io::Result<String> {
    let raw = tokio::fs::read(path).await?;
    let value: serde_json::Value = serde_json::from_slice(&raw).map_err(std::io::Error::other)?;
    let id = value
        .get("install_id")
        .and_then(|id| id.as_str())
        .filter(|id| {
            // Installation IDs use the older shape-only regex, not Zod's version/variant check.
            id.len() == 36
                && id.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit()
                    }
                })
        })
        .ok_or_else(|| std::io::Error::other("Invalid installation identity file"))?;
    Ok(id.to_owned())
}

async fn load_or_create(paths: &BrowserosPaths) -> std::io::Result<String> {
    match read_identity(&paths.installation_file).await {
        Ok(id) => return Ok(id),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    tokio::fs::create_dir_all(&paths.browseros_dir).await?;
    let candidate = Uuid::new_v4().to_string();
    let temporary = paths.browseros_dir.join(format!(
        ".installation.json.{}.{}.tmp",
        std::process::id(),
        Uuid::new_v4()
    ));
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary).await?;
    let result = async {
        file.write_all(format!("{{\n  \"install_id\": \"{candidate}\"\n}}\n").as_bytes())
            .await?;
        file.sync_all().await?;
        drop(file);
        // A hard link is atomic and fails if a concurrent Chromium/server won.
        match tokio::fs::hard_link(&temporary, &paths.installation_file).await {
            Ok(()) => Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                read_identity(&paths.installation_file).await
            }
            Err(error) => Err(error),
        }
    }
    .await;
    let _ = tokio::fs::remove_file(&temporary).await;
    result
}
