use crate::{
    db::{AuditLog, audit_log::SessionScreenshotRow},
    error::{AppError, AppResult, IoPath},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use std::{path::PathBuf, sync::Arc};
use tokio::fs;

#[derive(Clone)]
pub struct ScreenshotService {
    root: PathBuf,
    audit_log: Arc<AuditLog>,
}

impl ScreenshotService {
    #[must_use]
    pub fn new(root: PathBuf, audit_log: Arc<AuditLog>) -> Self {
        Self { root, audit_log }
    }

    pub async fn list(&self, session_id: &str) -> AppResult<Option<Vec<SessionScreenshotRow>>> {
        self.audit_log.list_session_screenshots(session_id).await
    }

    pub async fn read(&self, session_id: &str, screenshot_id: i64) -> AppResult<Vec<u8>> {
        if !self
            .audit_log
            .session_owns_screenshot(session_id, screenshot_id)
            .await?
        {
            return Err(AppError::not_found("screenshot not found"));
        }
        let session_path = self.path_for(session_id, screenshot_id);
        match fs::read(&session_path).await {
            Ok(bytes) => return Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(AppError::Io {
                    path: Some(session_path),
                    source,
                });
            }
        }
        let legacy_path = self.legacy_path_for(screenshot_id);
        match fs::read(&legacy_path).await {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(AppError::not_found("screenshot not found"))
            }
            Err(source) => Err(AppError::Io {
                path: Some(legacy_path),
                source,
            }),
        }
    }

    pub async fn write(&self, session_id: &str, screenshot_id: i64, bytes: &[u8]) -> AppResult<()> {
        let path = self.path_for(session_id, screenshot_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.with_path(parent)?;
        }
        fs::write(&path, bytes).await.with_path(path)
    }

    #[must_use]
    pub fn path_for(&self, session_id: &str, screenshot_id: i64) -> PathBuf {
        self.root
            .join(safe_session_key(session_id))
            .join(format!("{screenshot_id}.jpg"))
    }

    #[must_use]
    pub fn legacy_path_for(&self, screenshot_id: i64) -> PathBuf {
        self.root.join(format!("{screenshot_id}.jpg"))
    }
}

fn safe_session_key(session_id: &str) -> String {
    format!("s-{}", URL_SAFE_NO_PAD.encode(session_id.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::safe_session_key;

    #[test]
    fn session_storage_key_is_one_path_safe_segment() {
        for session_id in [
            "session-live",
            "../../escape",
            "slashes/and\\backslashes",
            "",
        ] {
            let key = safe_session_key(session_id);
            assert!(key.starts_with("s-"));
            assert!(!key.contains('/'));
            assert!(!key.contains('\\'));
            assert_ne!(key, ".");
            assert_ne!(key, "..");
        }
    }
}
