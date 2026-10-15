//! Local server discovery is published only after bind and removed only while the
//! publication still belongs to this process. Keep the deployed JSON shape free of
//! internal ownership markers; ownership uses the exact bytes this handle published.

use crate::{VERSION, config::Config, identity::Identity};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
pub struct ServerFile {
    path: PathBuf,
    metadata: Value,
    published: Arc<Mutex<Option<Vec<u8>>>>,
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
        tokio::fs::write(&self.path, &bytes).await?;
        if let Ok(mut published) = self.published.lock() {
            *published = Some(bytes);
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
            && tokio::fs::read(&self.path).await.ok().as_ref() == Some(&published)
        {
            let _ = tokio::fs::remove_file(&self.path).await;
        }
    }
}
