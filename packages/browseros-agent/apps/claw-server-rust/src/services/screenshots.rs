use crate::error::{AppError, AppResult, IoPath};
use std::path::PathBuf;
use tokio::fs;

#[derive(Clone)]
pub struct ScreenshotService {
    root: PathBuf,
}

impl ScreenshotService {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    #[must_use]
    pub fn path_for(&self, dispatch_id: &str) -> PathBuf {
        self.root.join(format!("{dispatch_id}.jpg"))
    }

    pub async fn read(&self, dispatch_id: &str) -> AppResult<Vec<u8>> {
        let path = self.path_for(dispatch_id);
        match fs::read(&path).await {
            Ok(bytes) => Ok(bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Err(AppError::not_found("not found"))
            }
            Err(source) => Err(AppError::Io {
                path: Some(path),
                source,
            }),
        }
    }

    pub async fn write(&self, dispatch_id: &str, bytes: &[u8]) -> AppResult<()> {
        let path = self.path_for(dispatch_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.with_path(parent)?;
        }
        fs::write(&path, bytes).await.with_path(path)
    }
}
