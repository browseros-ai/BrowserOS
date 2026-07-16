use serde::Serialize;
use serde_json::Value;
use std::{
    env, io,
    path::{Path, PathBuf},
};
use tokio::{fs, sync::Mutex};
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
struct AnalyticsState {
    distinct_id: String,
    enabled: bool,
}

pub struct TelemetryService {
    path: PathBuf,
    analytics_configured: bool,
    state: Mutex<Option<AnalyticsState>>,
}

impl TelemetryService {
    /// Creates persisted telemetry state using the TS-compatible environment gates.
    #[must_use]
    pub fn new(claw_dir: impl Into<PathBuf>) -> Self {
        let posthog_key_configured = env::var("CLAW_POSTHOG_KEY")
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        let env_enabled = env::var("CLAW_ANALYTICS_ENABLED").ok().is_none_or(|value| {
            !matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false")
        });
        Self {
            path: claw_dir.into().join(ANALYTICS_FILE),
            analytics_configured: posthog_key_configured && env_enabled,
            state: Mutex::new(None),
        }
    }

    /// Returns the persisted anonymous identity, effective state, and raw consent choice.
    pub async fn get_state(&self) -> TelemetryState {
        let mut state = self.state.lock().await;
        if let Some(current) = state.as_ref() {
            return telemetry_state(current, self.analytics_configured);
        }
        let current = load_or_create_state(&self.path).await;
        let response = telemetry_state(&current, self.analytics_configured);
        *state = Some(current);
        response
    }

    /// Persists a consent change before applying it to the in-memory state.
    pub async fn set_consent(&self, consent: bool) -> TelemetryState {
        let mut state = self.state.lock().await;
        if state.is_none() {
            *state = Some(load_or_create_state(&self.path).await);
        }
        let distinct_id = state
            .as_ref()
            .map(|current| current.distinct_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let next = AnalyticsState {
            distinct_id,
            enabled: consent,
        };
        if let Err(error) = persist_state(&self.path, &next).await {
            tracing::error!(%consent, %error, "analytics consent write failed; choice may not survive a restart");
        }
        *state = Some(next.clone());
        telemetry_state(&next, self.analytics_configured)
    }
}

fn telemetry_state(state: &AnalyticsState, analytics_configured: bool) -> TelemetryState {
    TelemetryState {
        distinct_id: state.distinct_id.clone(),
        enabled: state.enabled && analytics_configured,
        consent: state.enabled,
    }
}

async fn load_or_create_state(path: &Path) -> AnalyticsState {
    match fs::read_to_string(path).await {
        Ok(raw) => match parse_state(&raw) {
            Some(state) => state,
            None => {
                tracing::warn!(path = %path.display(), "analytics state corrupt; disabling to preserve consent");
                AnalyticsState {
                    distinct_id: Uuid::new_v4().to_string(),
                    enabled: false,
                }
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
            AnalyticsState {
                distinct_id: Uuid::new_v4().to_string(),
                enabled: false,
            }
        }
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

async fn persist_state(path: &Path, state: &AnalyticsState) -> io::Result<()> {
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
