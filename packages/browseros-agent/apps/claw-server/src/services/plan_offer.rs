//! Whether a release build sells ultrafast, from the `ultrafast-plan` PostHog flag.
//!
//! Dev and staging builds always sell it. A release build asks PostHog at startup and on a timer,
//! saves the answer next to `analytics.json`, and closes sales until it has one. The flag only
//! decides selling: billing keeps a computer that already has a licence on its plan whatever
//! the flag says.

use crate::{
    analytics::{AnalyticsService, TelemetryState},
    config::ServerMode,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const FLAG_KEY: &str = "ultrafast-plan";

/// Asked for when analytics consent is off, so the install id never leaves the computer; a
/// global rollout still reaches that install, targeting one person does not.
const SHARED_IDENTITY: &str = "browserclaw-remote-config";

pub const REFRESH_INTERVAL: Duration = Duration::from_secs(10 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const CACHE_FILE: &str = "plan-offer.json";

/// Whether this build puts sales behind the flag: release builds only. `browseros-dev watch
/// --claw` builds with `billing-staging`, and dev mode uses the local gateway.
#[must_use]
pub fn sells_behind_flag(dev_mode: bool) -> bool {
    !cfg!(feature = "billing-staging") && !dev_mode
}

/// The PostHog project the flag is read from: the analytics host and public project key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagSource {
    pub host: String,
    pub project_key: String,
}

/// The source a release build reads the flag from, or `None` when it sells without asking (dev,
/// staging, hosted) or has no PostHog project to ask (sales then stay closed).
#[must_use]
pub fn flag_source(
    mode: ServerMode,
    dev_mode: bool,
    posthog: Option<(String, String)>,
) -> Option<FlagSource> {
    if mode.is_hosted() || !sells_behind_flag(dev_mode) {
        return None;
    }
    let (host, project_key) = posthog?;
    let host = host.trim().trim_end_matches('/').to_owned();
    let project_key = project_key.trim().to_owned();
    (!host.is_empty() && !project_key.is_empty()).then_some(FlagSource { host, project_key })
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Cached {
    offered: bool,
    checked_at: i64,
}

fn cache_path(browserclaw_dir: &Path) -> PathBuf {
    browserclaw_dir.join(CACHE_FILE)
}

/// The last answer this computer saved, if any.
pub async fn load_cached(browserclaw_dir: &Path) -> Option<bool> {
    let raw = tokio::fs::read(cache_path(browserclaw_dir)).await.ok()?;
    serde_json::from_slice::<Cached>(&raw)
        .inspect_err(|error| tracing::warn!(%error, "saved plan offer unreadable"))
        .ok()
        .map(|cached| cached.offered)
}

async fn save_cached(browserclaw_dir: &Path, offered: bool, now_ms: i64) {
    let body = match serde_json::to_vec(&Cached {
        offered,
        checked_at: now_ms,
    }) {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!(%error, "plan offer not saved");
            return;
        }
    };
    if let Err(error) = tokio::fs::write(cache_path(browserclaw_dir), body).await {
        tracing::warn!(%error, "plan offer not saved");
    }
}

#[derive(Debug, Deserialize)]
struct FlagsResponse {
    #[serde(default)]
    flags: std::collections::HashMap<String, FlagDetail>,
    #[serde(default, rename = "errorsWhileComputingFlags")]
    errors_while_computing_flags: bool,
    #[serde(default, rename = "quotaLimited")]
    quota_limited: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct FlagDetail {
    #[serde(default)]
    enabled: bool,
}

/// What PostHog's flag evaluation says about [`FLAG_KEY`], or `None` when it gave no usable
/// answer, so the caller keeps what it had.
fn offered_from(body: &[u8]) -> Option<bool> {
    let response: FlagsResponse = serde_json::from_slice(body)
        .inspect_err(|error| tracing::warn!(%error, "plan offer flags response unreadable"))
        .ok()?;
    if response
        .quota_limited
        .as_ref()
        .is_some_and(|limited| !limited.is_empty())
    {
        return None;
    }
    match response.flags.get(FLAG_KEY) {
        Some(flag) => Some(flag.enabled),
        None if response.errors_while_computing_flags => None,
        // A deleted or switched-off flag is not returned: not on sale.
        None => Some(false),
    }
}

/// Asks PostHog once. `install_id` is sent only while analytics consent is on.
pub async fn fetch_offered(source: &FlagSource, install_id: Option<&str>) -> Option<bool> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .inspect_err(|error| tracing::warn!(%error, "plan offer client unavailable"))
        .ok()?;
    let identity = install_id.unwrap_or(SHARED_IDENTITY);
    let response = client
        .post(format!("{}/flags?v=2", source.host))
        .json(&serde_json::json!({
            "api_key": source.project_key,
            "distinct_id": identity,
            // Server events skip person profiles, so PostHog has no person to match a
            // `distinct_id` condition against unless the request carries it.
            "person_properties": { "distinct_id": identity },
            "flag_keys_to_evaluate": [FLAG_KEY],
            "disable_geoip": true,
        }))
        .send()
        .await
        .inspect_err(|error| tracing::warn!(%error, "plan offer fetch failed"))
        .ok()?;
    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "plan offer fetch rejected");
        return None;
    }
    let body = response
        .bytes()
        .await
        .inspect_err(|error| tracing::warn!(%error, "plan offer body unreadable"))
        .ok()?;
    offered_from(&body)
}

/// The install id to ask with: only while analytics consent is on.
#[must_use]
pub fn install_id_to_send(telemetry: &TelemetryState) -> Option<&str> {
    (telemetry.consent && !telemetry.distinct_id.is_empty())
        .then_some(telemetry.distinct_id.as_str())
}

/// Asks once and saves a usable answer; an unusable one returns `None` and saves nothing.
pub async fn refresh_once(
    source: &FlagSource,
    install_id: Option<&str>,
    browserclaw_dir: &Path,
    now_ms: i64,
) -> Option<bool> {
    let offered = fetch_offered(source, install_id).await?;
    save_cached(browserclaw_dir, offered, now_ms).await;
    Some(offered)
}

/// Re-reads the flag on a timer until the server shuts down, starting straight away, and hands
/// each usable answer to `apply`.
pub fn spawn_refresh_loop(
    apply: impl Fn(bool) + Send + 'static,
    analytics: Arc<AnalyticsService>,
    source: FlagSource,
    browserclaw_dir: PathBuf,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = ticker.tick() => {
                    let telemetry = analytics.get_state().await;
                    let answer = refresh_once(
                        &source,
                        install_id_to_send(&telemetry),
                        &browserclaw_dir,
                        crate::clock::now_epoch_ms(),
                    )
                    .await;
                    if let Some(offered) = answer {
                        apply(offered);
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(value: &serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap_or_default()
    }

    #[test]
    fn the_flag_decides_only_release_builds() {
        assert!(!sells_behind_flag(true));
        assert_eq!(sells_behind_flag(false), !cfg!(feature = "billing-staging"));
    }

    #[test]
    fn a_release_build_reads_the_flag_from_the_analytics_project() {
        let posthog = Some((
            "https://us.i.posthog.com/".to_owned(),
            " phc_test ".to_owned(),
        ));
        let source = flag_source(ServerMode::Desktop, false, posthog.clone());
        if cfg!(feature = "billing-staging") {
            assert!(source.is_none());
        } else {
            assert_eq!(
                source,
                Some(FlagSource {
                    host: "https://us.i.posthog.com".to_owned(),
                    project_key: "phc_test".to_owned(),
                })
            );
        }
        assert!(flag_source(ServerMode::Desktop, true, posthog.clone()).is_none());
        assert!(flag_source(ServerMode::Desktop, false, None).is_none());
        assert!(
            flag_source(
                ServerMode::Desktop,
                false,
                Some((String::new(), "phc".into()))
            )
            .is_none()
        );
    }

    #[test]
    fn the_answer_is_read_from_the_flag() {
        let on =
            body(&json!({"flags": {"ultrafast-plan": {"key": "ultrafast-plan", "enabled": true}}}));
        let off = body(
            &json!({"flags": {"ultrafast-plan": {"key": "ultrafast-plan", "enabled": false}}}),
        );
        assert_eq!(offered_from(&on), Some(true));
        assert_eq!(offered_from(&off), Some(false));
    }

    /// A PostHog stand-in that answers `answer` and keeps every request body it was sent.
    async fn posthog(
        status: axum::http::StatusCode,
        answer: serde_json::Value,
    ) -> anyhow::Result<(FlagSource, Arc<std::sync::Mutex<Vec<serde_json::Value>>>)> {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = seen.clone();
        let app = axum::Router::new().route(
            "/flags",
            axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                let kept = kept.clone();
                let answer = answer.clone();
                async move {
                    if let Ok(mut kept) = kept.lock() {
                        kept.push(body);
                    }
                    (status, axum::Json(answer))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((
            FlagSource {
                host: format!("http://{address}"),
                project_key: "phc_test".to_owned(),
            },
            seen,
        ))
    }

    fn sent(seen: &std::sync::Mutex<Vec<serde_json::Value>>) -> Vec<serde_json::Value> {
        seen.lock().map(|seen| seen.clone()).unwrap_or_default()
    }

    #[tokio::test]
    async fn an_answer_is_saved_and_read_back_on_the_next_start() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (source, seen) = posthog(
            axum::http::StatusCode::OK,
            json!({"flags": {"ultrafast-plan": {"key": "ultrafast-plan", "enabled": true}}}),
        )
        .await?;
        assert_eq!(load_cached(dir.path()).await, None);
        let install = "5d0c6a1e-3b7f-4c2a-9e81-2f64b0a7c913";
        assert_eq!(
            refresh_once(&source, Some(install), dir.path(), 1_000).await,
            Some(true)
        );
        assert_eq!(load_cached(dir.path()).await, Some(true));
        let request = sent(&seen).pop().unwrap_or_default();
        assert_eq!(request["api_key"], "phc_test");
        assert_eq!(request["distinct_id"], install);
        assert_eq!(request["person_properties"]["distinct_id"], install);
        assert_eq!(request["flag_keys_to_evaluate"], json!(["ultrafast-plan"]));
        Ok(())
    }

    #[tokio::test]
    async fn without_consent_the_install_id_is_never_sent() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (source, seen) = posthog(axum::http::StatusCode::OK, json!({"flags": {}})).await?;
        let opted_out = TelemetryState {
            distinct_id: "5d0c6a1e-3b7f-4c2a-9e81-2f64b0a7c913".to_owned(),
            enabled: false,
            consent: false,
        };
        assert_eq!(install_id_to_send(&opted_out), None);
        assert_eq!(
            refresh_once(&source, install_id_to_send(&opted_out), dir.path(), 1).await,
            Some(false)
        );
        let request = sent(&seen).pop().unwrap_or_default();
        assert_eq!(request["distinct_id"], SHARED_IDENTITY);
        assert!(!request.to_string().contains(&opted_out.distinct_id));
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_answer_keeps_the_saved_one() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (on, _) = posthog(
            axum::http::StatusCode::OK,
            json!({"flags": {"ultrafast-plan": {"key": "ultrafast-plan", "enabled": true}}}),
        )
        .await?;
        refresh_once(&on, None, dir.path(), 1).await;
        let (down, _) = posthog(axum::http::StatusCode::SERVICE_UNAVAILABLE, json!({})).await?;
        assert_eq!(refresh_once(&down, None, dir.path(), 2).await, None);
        let unreachable = FlagSource {
            host: "http://127.0.0.1:9".to_owned(),
            project_key: "phc_test".to_owned(),
        };
        assert_eq!(refresh_once(&unreachable, None, dir.path(), 3).await, None);
        assert_eq!(load_cached(dir.path()).await, Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn an_unreadable_saved_answer_counts_as_none() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        tokio::fs::write(dir.path().join(CACHE_FILE), b"not json").await?;
        assert_eq!(load_cached(dir.path()).await, None);
        Ok(())
    }

    #[test]
    fn a_missing_flag_is_not_on_sale_unless_posthog_had_trouble() {
        assert_eq!(offered_from(&body(&json!({"flags": {}}))), Some(false));
        let trouble = body(&json!({"flags": {}, "errorsWhileComputingFlags": true}));
        assert_eq!(offered_from(&trouble), None);
        let limited = body(&json!({"flags": {}, "quotaLimited": ["feature_flags"]}));
        assert_eq!(offered_from(&limited), None);
        assert_eq!(offered_from(b"not json"), None);
    }
}
