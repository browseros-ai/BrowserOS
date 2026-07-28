//! First-launch marker: a one-shot flag persisted as a small JSON file in the
//! BrowserClaw dir, recording that the first-launch harness auto-connect sweep
//! has already run. Mirrors `audit_settings`: atomic write, corrupt/unreadable
//! reads as not-done so a lost or corrupt marker re-runs the sweep once, which
//! is idempotent (already-linked harnesses are skipped).

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    path::Path,
};
use tempfile::NamedTempFile;

const FIRST_RUN_CONNECT_FILE: &str = "first-run-connect.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirstRunState {
    first_run_connect_done: bool,
}

/// Whether the first-launch harness auto-connect sweep has already run. A
/// missing, corrupt, or unreadable marker reads as `false`.
pub async fn is_first_run_connect_done(browserclaw_dir: &Path) -> bool {
    let path = browserclaw_dir.join(FIRST_RUN_CONNECT_FILE);
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => serde_json::from_str::<FirstRunState>(&raw)
            .map(|state| state.first_run_connect_done)
            .unwrap_or_else(|error| {
                tracing::warn!(path = %path.display(), %error, "first-run marker corrupt; treating as not done");
                false
            }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!(%error, "first-run marker unreadable; treating as not done");
            false
        }
    }
}

/// Records that the first-launch harness auto-connect sweep has run so it never
/// sweeps again; a later manual disconnect is then preserved.
pub async fn mark_first_run_connect_done(browserclaw_dir: &Path) -> AppResult<()> {
    let path = browserclaw_dir.join(FIRST_RUN_CONNECT_FILE);
    persist(&path).await.map_err(|source| AppError::Io {
        path: Some(path.clone()),
        source,
    })
}

async fn persist(path: &Path) -> io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || persist_blocking(&path))
        .await
        .map_err(io::Error::other)?
}

fn persist_blocking(path: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    let mut raw = serde_json::to_string_pretty(&FirstRunState {
        first_run_connect_done: true,
    })
    .map_err(io::Error::other)?;
    raw.push('\n');
    tmp.write_all(raw.as_bytes())?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map(|_| ()).map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn missing_marker_reads_as_not_done() -> anyhow::Result<()> {
        let dir = tempdir()?;
        assert!(!is_first_run_connect_done(dir.path()).await);
        Ok(())
    }

    #[tokio::test]
    async fn mark_then_read_is_done_and_survives_reopen() -> anyhow::Result<()> {
        let dir = tempdir()?;
        mark_first_run_connect_done(dir.path()).await?;
        assert!(is_first_run_connect_done(dir.path()).await);
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_marker_reads_as_not_done() -> anyhow::Result<()> {
        let dir = tempdir()?;
        tokio::fs::write(dir.path().join(FIRST_RUN_CONNECT_FILE), "{not json").await?;
        assert!(!is_first_run_connect_done(dir.path()).await);
        Ok(())
    }
}
