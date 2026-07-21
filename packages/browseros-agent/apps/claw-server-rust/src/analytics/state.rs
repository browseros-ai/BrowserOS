use serde::Serialize;
use serde_json::Value;
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::fs;
use uuid::Uuid;

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
    pub(crate) distinct_id: String,
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
            let fresh = AnalyticsState {
                distinct_id: Uuid::new_v4().to_string(),
                enabled: true,
            };
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
    AnalyticsState {
        distinct_id: Uuid::new_v4().to_string(),
        enabled: false,
    }
}

fn parse_state(raw: &str) -> Option<AnalyticsState> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let object = value.as_object()?;
    let distinct_id = object.get("distinctId")?.as_str()?.to_string();
    if distinct_id.is_empty() {
        return None;
    }
    Some(AnalyticsState {
        distinct_id,
        enabled: !matches!(object.get("enabled"), Some(Value::Bool(false))),
    })
}

pub(crate) async fn persist_state(path: &Path, state: &AnalyticsState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_file_name(format!(
        "{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let mut raw = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    raw.push('\n');
    fs::write(&tmp, raw).await?;
    fs::rename(tmp, path).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn missing_state_mints_and_persists_the_two_field_format() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let path = state_path(directory.path());
        let state = load_or_create_state(&path).await;
        Uuid::parse_str(&state.distinct_id)?;
        assert!(state.enabled);

        let raw = fs::read_to_string(path).await?;
        assert!(raw.ends_with('\n'));
        let value: Value = serde_json::from_str(&raw)?;
        assert_eq!(value.as_object().map(serde_json::Map::len), Some(2));
        assert_eq!(value["distinctId"], state.distinct_id);
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

    #[test]
    fn parser_preserves_the_historical_opt_out_default() {
        assert_eq!(
            parse_state(r#"{"distinctId":"stable"}"#),
            Some(AnalyticsState {
                distinct_id: "stable".to_string(),
                enabled: true,
            })
        );
        assert_eq!(
            parse_state(r#"{"distinctId":"stable","enabled":false}"#).map(|state| state.enabled),
            Some(false)
        );
        assert_eq!(parse_state(r#"{"enabled":true}"#), None);
    }
}
