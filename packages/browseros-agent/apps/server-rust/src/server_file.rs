//! Local server discovery is published only after bind and removed only while the
//! publication still belongs to this process. Keep the deployed JSON shape free of
//! internal ownership markers; ownership uses file identity and the published bytes.

use crate::{VERSION, config::Config, identity::Identity};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone)]
struct Publication {
    bytes: Vec<u8>,
    metadata: std::fs::Metadata,
}

/// All clones share one publication token so cleanup cannot remove a replacement.
#[derive(Debug, Clone)]
pub struct ServerFile {
    path: PathBuf,
    metadata: Value,
    published: Arc<Mutex<Option<Publication>>>,
}
impl ServerFile {
    pub fn new(browseros_dir: &Path) -> Self {
        Self {
            path: browseros_dir.join("server.json"),
            metadata: json!({}),
            published: Arc::new(Mutex::new(None)),
        }
    }
    pub fn with_config(config: &Config, identity: &Identity) -> Self {
        let mut publication = Self::new(&config.browseros_dir);
        let mut metadata = json!({"cdp_port":config.cdp_port,"server_version":VERSION,"browseros_id":identity.browseros_id});
        if let Some(version) = &config.instance.browseros_version {
            metadata["browseros_version"] = json!(version);
        }
        if let Some(version) = &config.instance.chromium_version {
            metadata["chromium_version"] = json!(version);
        }
        publication.metadata = metadata;
        publication
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub async fn write(&self, port: u16) -> std::io::Result<()> {
        let mut metadata = self.metadata.clone();
        metadata["server_port"] = json!(port);
        metadata["url"] = json!(format!("http://127.0.0.1:{port}"));
        let mut bytes = serde_json::to_vec_pretty(&metadata).map_err(std::io::Error::other)?;
        bytes.push(b'\n');
        // Rename a complete, uniquely owned inode into place. A restarted server
        // may publish identical JSON on the same port; bytes alone cannot tell it
        // apart from the old process during late shutdown cleanup.
        let temporary = self.path.with_file_name(format!(
            ".server.json.{}.{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let result = async {
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .await?;
            file.write_all(&bytes).await?;
            file.flush().await?;
            let metadata = file.metadata().await?;
            drop(file);
            tokio::fs::rename(&temporary, &self.path).await?;
            Ok::<_, std::io::Error>(metadata)
        }
        .await;
        let _ = tokio::fs::remove_file(&temporary).await;
        let metadata = result?;
        if let Ok(mut published) = self.published.lock() {
            *published = Some(Publication { bytes, metadata });
        }
        Ok(())
    }
    pub async fn remove(&self) {
        let published = self
            .published
            .lock()
            .ok()
            .and_then(|mut value| value.take());
        if let Some(published) = published
            && tokio::fs::read(&self.path).await.ok().as_ref() == Some(&published.bytes)
            && let Ok(current) = tokio::fs::symlink_metadata(&self.path).await
            && same_publication(&current, &published.metadata)
        {
            let _ = tokio::fs::remove_file(&self.path).await;
        }
    }
}

fn same_publication(current: &std::fs::Metadata, published: &std::fs::Metadata) -> bool {
    if current.file_type().is_symlink()
        || current.len() != published.len()
        || current.modified().ok() != published.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        current.dev() == published.dev() && current.ino() == published.ino()
    }
    #[cfg(not(unix))]
    {
        current.created().ok() == published.created().ok()
    }
}
