//! Publishes the live claw-server URL to `<browserclaw_dir>/runtime.json` so
//! external discovery (the Codex and Claude Desktop plugins) can read the
//! current port without probing or scanning. Best-effort: a failed write logs
//! a warning and never blocks startup.

use std::path::Path;

use serde_json::json;
use tokio::fs;
use tracing::warn;

const RUNTIME_FILE: &str = "runtime.json";

/// Write `{ "url": <url> }` to `<dir>/runtime.json`, replacing any existing file
/// in place. Errors are logged and swallowed so this best-effort disk write can
/// never fail boot.
pub async fn write(dir: &Path, url: &str) {
    if let Err(err) = try_write(dir, url).await {
        warn!(
            error = %err,
            path = %dir.join(RUNTIME_FILE).display(),
            "runtime file write failed",
        );
    }
}

async fn try_write(dir: &Path, url: &str) -> std::io::Result<()> {
    fs::create_dir_all(dir).await?;
    let path = dir.join(RUNTIME_FILE);
    let mut payload = serde_json::to_string_pretty(&json!({ "url": url }))
        .unwrap_or_else(|_| format!("{{\n  \"url\": \"{url}\"\n}}"));
    payload.push('\n');
    // Write in place (create or truncate) rather than write-a-temp-then-rename.
    // The rename could fail on Windows when the destination was momentarily
    // locked, leaving a stale URL (#2721); a single portable overwrite avoids
    // that whole class, and a torn read of this tiny startup-written file is
    // both vanishingly unlikely and recoverable by re-reading.
    fs::write(&path, &payload).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_url() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let dir = root.path();

        write(dir, "http://127.0.0.1:9200").await;

        let raw = fs::read_to_string(dir.join("runtime.json")).await?;
        // Byte-for-byte identical to the archived TS writer's contract:
        // JSON.stringify({ url }, null, 2) + "\n".
        assert_eq!(raw, "{\n  \"url\": \"http://127.0.0.1:9200\"\n}\n");
        // No temp file is created any more.
        assert!(!dir.join("runtime.json.tmp").exists());

        Ok(())
    }

    #[tokio::test]
    async fn missing_parent_is_created() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let dir = root.path().join("state");

        write(&dir, "http://127.0.0.1:9201").await;

        assert!(dir.join("runtime.json").exists());
        Ok(())
    }

    #[tokio::test]
    async fn replaces_existing_runtime_file() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let dir = root.path();

        write(dir, "http://127.0.0.1:9200").await;
        write(dir, "http://127.0.0.1:9201").await;

        let raw = fs::read_to_string(dir.join("runtime.json")).await?;
        assert_eq!(raw, "{\n  \"url\": \"http://127.0.0.1:9201\"\n}\n");
        Ok(())
    }
}
