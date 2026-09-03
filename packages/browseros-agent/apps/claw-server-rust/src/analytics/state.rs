use serde::Serialize;
use serde_json::Value;
use std::{
    fs as std_fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use tokio::fs;

const ANALYTICS_FILE: &str = "analytics.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryState {
    pub distinct_id: String,
    pub enabled: bool,
    pub consent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsState {
    pub(crate) enabled: bool,
}

pub(crate) fn state_path(browserclaw_dir: &Path) -> PathBuf {
    browserclaw_dir.join(ANALYTICS_FILE)
}

pub(crate) async fn load_or_create_state(path: &Path) -> AnalyticsState {
    match fs::read_to_string(path).await {
        Ok(raw) => match parse_state(&raw) {
            Some(state) => state,
            None => {
                tracing::warn!(path = %path.display(), "analytics state corrupt; disabling to preserve consent");
                disabled_ephemeral_state()
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let fresh = AnalyticsState { enabled: true };
            if let Err(error) = persist_state(path, &fresh).await {
                tracing::warn!(%error, "analytics state write failed");
            }
            fresh
        }
        Err(error) => {
            tracing::warn!(%error, "analytics state unreadable; disabling to preserve consent");
            disabled_ephemeral_state()
        }
    }
}

fn disabled_ephemeral_state() -> AnalyticsState {
    AnalyticsState { enabled: false }
}

fn parse_state(raw: &str) -> Option<AnalyticsState> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let object = value.as_object()?;
    let enabled = match object.get("enabled") {
        Some(Value::Bool(enabled)) => *enabled,
        None => true,
        Some(_) => return None,
    };
    Some(AnalyticsState { enabled })
}

pub(crate) async fn persist_state(path: &Path, state: &AnalyticsState) -> io::Result<()> {
    let path = path.to_path_buf();
    let state = state.clone();
    tokio::task::spawn_blocking(move || persist_state_blocking(&path, &state))
        .await
        .map_err(io::Error::other)?
}

fn persist_state_blocking(path: &Path, state: &AnalyticsState) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std_fs::create_dir_all(parent)?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    let mut raw = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
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
    async fn missing_state_persists_consent_only() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let path = state_path(directory.path());
        let state = load_or_create_state(&path).await;
        assert!(state.enabled);

        let raw = fs::read_to_string(path).await?;
        assert!(raw.ends_with('\n'));
        let value: Value = serde_json::from_str(&raw)?;
        assert_eq!(value.as_object().map(serde_json::Map::len), Some(1));
        assert_eq!(value["enabled"], true);
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_or_unreadable_existing_state_fails_closed_without_overwriting()
    -> anyhow::Result<()> {
        let directory = tempdir()?;
        let corrupt_path = state_path(directory.path());
        fs::write(&corrupt_path, "{not json").await?;
        assert!(!load_or_create_state(&corrupt_path).await.enabled);
        assert_eq!(fs::read_to_string(&corrupt_path).await?, "{not json");

        let unreadable_root = directory.path().join("unreadable");
        fs::create_dir_all(state_path(&unreadable_root)).await?;
        assert!(
            !load_or_create_state(&state_path(&unreadable_root))
                .await
                .enabled
        );
        assert!(state_path(&unreadable_root).is_dir());
        Ok(())
    }

    #[tokio::test]
    async fn consent_state_atomically_replaces_an_existing_file() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let path = state_path(directory.path());
        persist_state(&path, &AnalyticsState { enabled: true }).await?;
        persist_state(&path, &AnalyticsState { enabled: false }).await?;

        assert_eq!(
            parse_state(&fs::read_to_string(path).await?),
            Some(AnalyticsState { enabled: false })
        );
        Ok(())
    }

    #[test]
    fn parser_preserves_the_historical_opt_out_default() {
        assert_eq!(
            parse_state(r#"{"distinctId":"stable"}"#),
            Some(AnalyticsState { enabled: true })
        );
        assert_eq!(
            parse_state(r#"{"distinctId":"stable","enabled":false}"#).map(|state| state.enabled),
            Some(false)
        );
        assert_eq!(
            parse_state(r#"{"distinctId":"legacy","enabled":true}"#),
            Some(AnalyticsState { enabled: true })
        );
        assert_eq!(parse_state(r#"{"enabled":"yes"}"#), None);
    }
}
