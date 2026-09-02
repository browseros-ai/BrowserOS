use serde::{Deserialize, Serialize};
use std::{
    fs as std_fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use uuid::Uuid;

const INSTALLATION_FILE: &str = "installation.json";

#[derive(Debug, Deserialize, Serialize)]
struct InstallationFile {
    install_id: String,
}

pub(crate) fn installation_path(browserclaw_dir: &Path) -> PathBuf {
    browserclaw_dir.join(INSTALLATION_FILE)
}

/**
 * Loads BrowserClaw's product-wide identity without ever repairing a malformed file.
 * Chromium may race this sidecar during startup, so the blocking publisher uses a hard link to
 * make destination creation exclusive and adopts the winner's UUID when another process wins.
 */
pub(crate) async fn load_or_create_installation_id(browserclaw_dir: &Path) -> Option<String> {
    let browserclaw_dir = browserclaw_dir.to_path_buf();
    match tokio::task::spawn_blocking(move || load_or_create_blocking(&browserclaw_dir)).await {
        Ok(Ok(install_id)) => Some(install_id),
        Ok(Err(error)) => {
            tracing::warn!(%error, "installation identity unavailable; analytics disabled");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "installation identity worker failed; analytics disabled");
            None
        }
    }
}

fn load_or_create_blocking(browserclaw_dir: &Path) -> io::Result<String> {
    let path = installation_path(browserclaw_dir);
    match read_installation_id(&path) {
        Ok(install_id) => return Ok(install_id),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    std_fs::create_dir_all(browserclaw_dir)?;
    let candidate_id = Uuid::new_v4().to_string();
    let installation = InstallationFile {
        install_id: candidate_id.clone(),
    };
    let mut raw = serde_json::to_string_pretty(&installation).map_err(io::Error::other)?;
    raw.push('\n');

    let mut temporary = NamedTempFile::new_in(browserclaw_dir)?;
    temporary.write_all(raw.as_bytes())?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    let temporary_path = temporary.into_temp_path();

    match std_fs::hard_link(&temporary_path, &path) {
        Ok(()) => Ok(candidate_id),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => read_installation_id(&path),
        Err(error) => Err(error),
    }
}

fn read_installation_id(path: &Path) -> io::Result<String> {
    let raw = std_fs::read_to_string(path)?;
    let installation: InstallationFile = serde_json::from_str(&raw)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let parsed = Uuid::parse_str(&installation.install_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if parsed.hyphenated().to_string() != installation.install_id.to_ascii_lowercase() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "install_id must use canonical UUID syntax",
        ));
    }
    Ok(installation.install_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tempfile::tempdir;

    #[tokio::test]
    async fn missing_identity_creates_and_reuses_the_one_field_file() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let first = load_or_create_installation_id(directory.path())
            .await
            .ok_or_else(|| anyhow::anyhow!("missing installation identity"))?;
        let second = load_or_create_installation_id(directory.path())
            .await
            .ok_or_else(|| anyhow::anyhow!("missing installation identity"))?;

        assert_eq!(second, first);
        let raw = std_fs::read_to_string(installation_path(directory.path()))?;
        assert!(raw.ends_with('\n'));
        let value: Value = serde_json::from_str(&raw)?;
        assert_eq!(value.as_object().map(serde_json::Map::len), Some(1));
        assert_eq!(value["install_id"], first);
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_creators_converge_on_one_identity() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let mut tasks = Vec::new();
        for _ in 0..12 {
            let root = directory.path().to_path_buf();
            tasks.push(tokio::spawn(async move {
                load_or_create_installation_id(&root).await
            }));
        }

        let mut ids = Vec::new();
        for task in tasks {
            ids.push(
                task.await?
                    .ok_or_else(|| anyhow::anyhow!("missing installation identity"))?,
            );
        }
        assert!(ids.iter().all(|install_id| install_id == &ids[0]));
        Ok(())
    }

    #[tokio::test]
    async fn malformed_identity_is_preserved_and_disables_analytics() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let path = installation_path(directory.path());
        std_fs::write(&path, "{not json")?;

        assert_eq!(load_or_create_installation_id(directory.path()).await, None);
        assert_eq!(std_fs::read_to_string(path)?, "{not json");
        Ok(())
    }
}
