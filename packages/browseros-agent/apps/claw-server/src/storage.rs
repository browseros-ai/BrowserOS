use crate::error::{AppError, AppResult, IoPath};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tokio::fs;

#[derive(Debug, Clone)]
pub struct JsonStore {
    root: Arc<PathBuf>,
}

impl JsonStore {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: Arc::new(root),
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.as_ref()
    }

    pub async fn ensure_dir(&self, rel_dir: &str) -> AppResult<()> {
        let abs = self.resolve(rel_dir)?;
        fs::create_dir_all(&abs).await.with_path(abs)
    }

    pub async fn read_json<T>(&self, rel_path: &str) -> AppResult<T>
    where
        T: DeserializeOwned,
    {
        let abs = self.resolve(rel_path)?;
        let raw = match fs::read_to_string(&abs).await {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(AppError::StorageNotFound(rel_path.to_string()));
            }
            Err(source) => {
                return Err(AppError::Io {
                    path: Some(abs),
                    source,
                });
            }
        };
        serde_json::from_str(&raw).map_err(|source| AppError::StorageCorrupt {
            path: rel_path.to_string(),
            source,
        })
    }

    pub async fn write_json<T>(&self, rel_path: &str, value: &T) -> AppResult<()>
    where
        T: Serialize,
    {
        let abs = self.resolve(rel_path)?;
        let parent = abs
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| AppError::InvalidStoragePath(rel_path.to_string()))?;
        fs::create_dir_all(&parent).await.with_path(parent)?;
        let tmp = abs.with_extension(tmp_extension(abs.extension()));
        let body = serde_json::to_string_pretty(value)?;
        fs::write(&tmp, body).await.with_path(tmp.clone())?;
        fs::rename(&tmp, &abs).await.with_path(abs)
    }

    /// Writes a file only its owner can read.
    ///
    /// For a credential. On unix the temporary file is created with mode 0600, so the
    /// contents are never readable by others, not even between the write and the rename,
    /// which carries the mode over to the final path.
    pub async fn write_json_private<T>(&self, rel_path: &str, value: &T) -> AppResult<()>
    where
        T: Serialize,
    {
        use tokio::io::AsyncWriteExt;

        let abs = self.resolve(rel_path)?;
        let parent = abs
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| AppError::InvalidStoragePath(rel_path.to_string()))?;
        fs::create_dir_all(&parent).await.with_path(parent)?;
        let tmp = abs.with_extension(tmp_extension(abs.extension()));
        let body = serde_json::to_string_pretty(value)?;
        // A leftover from an interrupted write keeps whatever mode it was created with, and
        // the mode below only applies to a file this call creates.
        match fs::remove_file(&tmp).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(AppError::Io {
                    path: Some(tmp),
                    source,
                });
            }
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&tmp).await.with_path(tmp.clone())?;
        file.write_all(body.as_bytes())
            .await
            .with_path(tmp.clone())?;
        file.flush().await.with_path(tmp.clone())?;
        drop(file);
        fs::rename(&tmp, &abs).await.with_path(abs)
    }

    pub async fn remove_file(&self, rel_path: &str) -> AppResult<bool> {
        let abs = self.resolve(rel_path)?;
        match fs::remove_file(&abs).await {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(AppError::Io {
                path: Some(abs),
                source,
            }),
        }
    }

    pub async fn file_exists(&self, rel_path: &str) -> AppResult<bool> {
        let abs = self.resolve(rel_path)?;
        match fs::metadata(&abs).await {
            Ok(meta) => Ok(meta.is_file()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(AppError::Io {
                path: Some(abs),
                source,
            }),
        }
    }

    pub async fn list_files(&self, rel_dir: &str, extension: &str) -> AppResult<Vec<String>> {
        let abs = self.resolve(rel_dir)?;
        let mut out = Vec::new();
        let mut entries = match fs::read_dir(&abs).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(source) => {
                return Err(AppError::Io {
                    path: Some(abs),
                    source,
                });
            }
        };
        while let Some(entry) = entries.next_entry().await.with_path(&abs)? {
            let file_type = entry.file_type().await.with_path(entry.path())?;
            if file_type.is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(extension) {
                    out.push(name);
                }
            }
        }
        Ok(out)
    }

    pub fn resolve(&self, rel_path: &str) -> AppResult<PathBuf> {
        guard_relative_path(rel_path)?;
        Ok(self.root.join(rel_path))
    }
}

fn guard_relative_path(rel_path: &str) -> AppResult<()> {
    let path = Path::new(rel_path);
    if path.is_absolute() {
        return Err(AppError::InvalidStoragePath(rel_path.to_string()));
    }
    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(AppError::InvalidStoragePath(rel_path.to_string()));
        }
    }
    Ok(())
}

fn tmp_extension(existing: Option<&OsStr>) -> String {
    match existing.and_then(OsStr::to_str) {
        Some(ext) if !ext.is_empty() => format!("{ext}.tmp"),
        _ => "tmp".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::JsonStore;
    use serde::{Deserialize, Serialize};
    use tempfile::tempdir;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct Row {
        id: String,
    }

    #[tokio::test]
    async fn atomic_json_roundtrip() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = JsonStore::new(dir.path().join("claw-server"));
        store
            .write_json(
                "agents/a.json",
                &Row {
                    id: "a".to_string(),
                },
            )
            .await?;
        let row: Row = store.read_json("agents/a.json").await?;
        assert_eq!(
            row,
            Row {
                id: "a".to_string()
            }
        );
        assert!(store.file_exists("agents/a.json").await?);
        assert_eq!(store.list_files("agents", ".json").await?, vec!["a.json"]);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn private_writes_are_owner_only_even_over_a_wide_leftover() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir()?;
        let store = JsonStore::new(dir.path().to_path_buf());
        let leftover = dir.path().join("secret.json.tmp");
        tokio::fs::write(&leftover, "{}").await?;
        tokio::fs::set_permissions(&leftover, std::fs::Permissions::from_mode(0o644)).await?;
        store
            .write_json_private(
                "secret.json",
                &Row {
                    id: "s".to_string(),
                },
            )
            .await?;
        let mode = tokio::fs::metadata(dir.path().join("secret.json"))
            .await?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
        assert!(!leftover.exists());
        let row: Row = store.read_json("secret.json").await?;
        assert_eq!(row.id, "s");
        Ok(())
    }

    #[tokio::test]
    async fn rejects_escape_paths() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = JsonStore::new(dir.path().to_path_buf());
        assert!(
            store
                .write_json(
                    "../x.json",
                    &Row {
                        id: "x".to_string()
                    }
                )
                .await
                .is_err()
        );
        Ok(())
    }
}
