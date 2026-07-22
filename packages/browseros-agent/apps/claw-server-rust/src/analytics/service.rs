use super::{
    AnalyticsSink,
    events::{self, EventDefinition},
    state::{AnalyticsState, TelemetryState, load_or_create_state, persist_state, state_path},
};
use crate::error::{AppError, AppResult};
use posthog_rs::{Client, ClientOptionsBuilder, Event};
use serde_json::Value;
use std::{
    env,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use tokio::sync::Mutex;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

const DEFAULT_POSTHOG_HOST: &str = "https://us.i.posthog.com";
const BUILD_POSTHOG_KEY: Option<&str> = option_env!("CLAW_POSTHOG_KEY");
const BUILD_POSTHOG_HOST: Option<&str> = option_env!("CLAW_POSTHOG_HOST");
const REQUEST_TIMEOUT_SECONDS: u64 = 2;
const SHUTDOWN_TIMEOUT_MILLISECONDS: u64 = 2_000;
const MAX_QUEUE_SIZE: usize = 256;

const SERVER_VERSION: &str = "server_version";
const OS_PLATFORM: &str = "os_platform";
const PROCESS_PERSON_PROFILE: &str = "$process_person_profile";
const GEOIP_DISABLE: &str = "$geoip_disable";
const IS_SERVER: &str = "$is_server";

#[derive(Debug, Clone)]
struct AnalyticsConfig {
    project_key: Option<String>,
    host: String,
    environment_enabled: bool,
}

impl AnalyticsConfig {
    fn from_env() -> Self {
        let runtime_project_key = env::var("CLAW_POSTHOG_KEY").ok();
        let runtime_host = env::var("CLAW_POSTHOG_HOST").ok();
        let project_key = configured_value(runtime_project_key.as_deref(), BUILD_POSTHOG_KEY);
        let host = configured_value(runtime_host.as_deref(), BUILD_POSTHOG_HOST)
            .unwrap_or_else(|| DEFAULT_POSTHOG_HOST.to_string());
        let environment_enabled = env::var("CLAW_ANALYTICS_ENABLED").ok().is_none_or(|value| {
            !matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false")
        });
        Self {
            project_key,
            host,
            environment_enabled,
        }
    }

    fn is_configured(&self) -> bool {
        self.project_key.is_some() && self.environment_enabled
    }
}

/**
 * Shipped sidecars inherit Chromium's environment, so release builds embed the public PostHog
 * project key. A runtime value takes precedence for local builds and controlled deployments.
 */
fn configured_value(runtime: Option<&str>, compiled: Option<&str>) -> Option<String> {
    runtime
        .and_then(non_empty_value)
        .or_else(|| compiled.and_then(non_empty_value))
}

fn non_empty_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[derive(Clone)]
struct ActiveClient {
    client: Arc<Client>,
    distinct_id: String,
}

pub struct AnalyticsService {
    path: PathBuf,
    config: AnalyticsConfig,
    state: Mutex<AnalyticsState>,
    active: RwLock<Option<ActiveClient>>,
    #[cfg(test)]
    shutdown_calls: AtomicUsize,
}

impl AnalyticsService {
    pub async fn new(browserclaw_dir: impl AsRef<Path>) -> AppResult<Self> {
        Self::new_with_config(browserclaw_dir.as_ref(), AnalyticsConfig::from_env()).await
    }

    async fn new_with_config(browserclaw_dir: &Path, config: AnalyticsConfig) -> AppResult<Self> {
        let path = state_path(browserclaw_dir);
        let state = load_or_create_state(&path).await;
        let active = if state.enabled && config.is_configured() {
            Some(build_client(&config, &state.distinct_id).await?)
        } else {
            None
        };
        Ok(Self {
            path,
            config,
            state: Mutex::new(state),
            active: RwLock::new(active),
            #[cfg(test)]
            shutdown_calls: AtomicUsize::new(0),
        })
    }

    #[cfg(test)]
    pub(crate) async fn new_for_test(
        browserclaw_dir: &Path,
        project_key: Option<&str>,
        host: String,
        environment_enabled: bool,
    ) -> AppResult<Self> {
        Self::new_with_config(
            browserclaw_dir,
            AnalyticsConfig {
                project_key: project_key.map(str::to_string),
                host,
                environment_enabled,
            },
        )
        .await
    }

    #[cfg(test)]
    pub(crate) fn shutdown_calls_for_testing(&self) -> usize {
        self.shutdown_calls.load(Ordering::SeqCst)
    }

    /// Returns the persisted identity, effective delivery state, and raw consent choice.
    pub async fn get_state(&self) -> TelemetryState {
        let state = self.state.lock().await;
        self.telemetry_state(&state)
    }

    /**
     * Serializes consent changes. Revocation removes the delivery client before disk I/O so a
     * failed opt-out is reported to the caller while capture stays disabled for this process.
     */
    pub async fn set_consent(&self, consent: bool) -> AppResult<TelemetryState> {
        let mut state = self.state.lock().await;
        let next = AnalyticsState {
            distinct_id: state.distinct_id.clone(),
            enabled: consent,
        };
        let previous = if consent { None } else { self.take_active() };
        if let Err(source) = persist_state(&self.path, &next).await {
            if !consent {
                *state = next;
            }
            drop(state);
            if let Some(previous) = previous {
                previous.client.shutdown().await;
            }
            return Err(AppError::Io {
                path: Some(self.path.clone()),
                source,
            });
        }
        *state = next;

        if !consent || !self.config.is_configured() {
            if let Some(previous) = previous {
                previous.client.shutdown().await;
            }
        } else if self.active_client().is_none() {
            match build_client(&self.config, &state.distinct_id).await {
                Ok(client) => self.replace_active(Some(client)),
                Err(error) => {
                    tracing::error!(%error, "analytics client initialization failed");
                }
            }
        }

        Ok(self.telemetry_state(&state))
    }

    pub fn capture(&self, definition: EventDefinition, properties: Value) {
        let active = self
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(active) = active.as_ref() else {
            return;
        };
        let Some(Value::Object(properties)) = definition.sanitize(&properties) else {
            return;
        };

        let mut event = Event::new(definition.name(), active.distinct_id.as_str());
        for (key, value) in properties {
            if event.insert_prop(key, value).is_err() {
                return;
            }
        }
        for (key, value) in [
            (
                SERVER_VERSION,
                Value::String(env!("CARGO_PKG_VERSION").to_string()),
            ),
            (
                OS_PLATFORM,
                Value::String(events::platform_token().to_string()),
            ),
            (PROCESS_PERSON_PROFILE, Value::Bool(false)),
            (GEOIP_DISABLE, Value::Bool(true)),
            (IS_SERVER, Value::Bool(true)),
        ] {
            if event.insert_prop(key, value).is_err() {
                return;
            }
        }
        active.client.capture(event);
    }

    pub async fn shutdown(&self) {
        #[cfg(test)]
        self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(active) = self.take_active() {
            active.client.shutdown().await;
        }
    }

    fn telemetry_state(&self, state: &AnalyticsState) -> TelemetryState {
        TelemetryState {
            distinct_id: state.distinct_id.clone(),
            enabled: state.enabled && self.config.is_configured() && self.active_client().is_some(),
            consent: state.enabled,
        }
    }

    fn active_client(&self) -> Option<ActiveClient> {
        self.active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn take_active(&self) -> Option<ActiveClient> {
        self.active
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn replace_active(&self, active: Option<ActiveClient>) {
        *self
            .active
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = active;
    }
}

impl AnalyticsSink for AnalyticsService {
    fn capture(&self, event: EventDefinition, properties: Value) {
        Self::capture(self, event, properties);
    }
}

async fn build_client(config: &AnalyticsConfig, distinct_id: &str) -> AppResult<ActiveClient> {
    let project_key = config.project_key.as_ref().ok_or_else(|| {
        AppError::Internal("analytics client requested without a project key".to_string())
    })?;
    let options = ClientOptionsBuilder::default()
        .api_key(project_key.clone())
        .host(config.host.clone())
        .request_timeout_seconds(REQUEST_TIMEOUT_SECONDS)
        .disable_geoip(true)
        .is_server(true)
        .flush_at(1usize)
        .max_queue_size(MAX_QUEUE_SIZE)
        .shutdown_timeout_ms(SHUTDOWN_TIMEOUT_MILLISECONDS)
        .before_send(final_allowlist)
        .build()
        .map_err(|error| AppError::Internal(format!("invalid analytics configuration: {error}")))?;
    Ok(ActiveClient {
        client: Arc::new(posthog_rs::client(options).await),
        distinct_id: distinct_id.to_string(),
    })
}

fn final_allowlist(mut event: Event) -> Option<Event> {
    let definition = events::by_wire_name(event.event_name())?;
    if !definition.required_values_are_normalized(event.properties())
        || event.properties().get(PROCESS_PERSON_PROFILE) != Some(&Value::Bool(false))
        || event.properties().get(GEOIP_DISABLE) != Some(&Value::Bool(true))
        || event.properties().get(IS_SERVER) != Some(&Value::Bool(true))
    {
        return None;
    }

    let remove = event
        .properties()
        .keys()
        .filter(|key| {
            !definition.allows_property(key)
                && !matches!(
                    key.as_str(),
                    SERVER_VERSION
                        | OS_PLATFORM
                        | PROCESS_PERSON_PROFILE
                        | GEOIP_DISABLE
                        | IS_SERVER
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    for key in remove {
        event.remove_prop(&key);
    }
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::events::{AGENT_SESSION_STARTED, SERVER_STARTED};
    use axum::{Router, body::Bytes, routing::any};
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle, time::Duration};

    async fn local_endpoint()
    -> anyhow::Result<(String, mpsc::UnboundedReceiver<Value>, JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (sender, receiver) = mpsc::unbounded_channel();
        let app = Router::new().fallback(any(move |body: Bytes| {
            let sender = sender.clone();
            async move {
                if let Ok(value) = serde_json::from_slice(&body) {
                    let _ = sender.send(value);
                }
                "ok"
            }
        }));
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((format!("http://{address}"), receiver, task))
    }

    fn test_config(host: String, enabled: bool) -> AnalyticsConfig {
        AnalyticsConfig {
            project_key: Some("test-project-key".to_string()),
            host,
            environment_enabled: enabled,
        }
    }

    #[test]
    fn runtime_analytics_config_overrides_compiled_defaults() {
        assert_eq!(
            configured_value(Some(" runtime-key "), Some("compiled-key")),
            Some("runtime-key".to_string())
        );
        assert_eq!(
            configured_value(None, Some(" compiled-key ")),
            Some("compiled-key".to_string())
        );
        assert_eq!(configured_value(Some("  "), Some("  ")), None);
    }

    #[tokio::test]
    async fn wire_payload_is_personless_and_exactly_allowlisted() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let stable_id = "2e087632-1f4e-4ee7-b8bb-cf8ad53e91a8";
        persist_state(
            &state_path(directory.path()),
            &AnalyticsState {
                distinct_id: stable_id.to_string(),
                enabled: true,
            },
        )
        .await?;
        let (host, mut requests, endpoint) = local_endpoint().await?;
        let service =
            AnalyticsService::new_with_config(directory.path(), test_config(host, true)).await?;

        service.capture(
            AGENT_SESSION_STARTED,
            json!({ "client_name": "Claude Code", "url": "https://private.example" }),
        );
        service.shutdown().await;

        let body = tokio::time::timeout(Duration::from_secs(2), requests.recv())
            .await?
            .ok_or_else(|| anyhow::anyhow!("local endpoint closed before capture"))?;
        endpoint.abort();
        let event = &body["batch"][0];
        assert_eq!(event["event"], AGENT_SESSION_STARTED.name());
        assert_eq!(event["distinct_id"], stable_id);
        assert_eq!(
            event["properties"],
            json!({
                "client_name": "claude-code",
                "server_version": env!("CARGO_PKG_VERSION"),
                "os_platform": events::platform_token(),
                "$process_person_profile": false,
                "$geoip_disable": true,
                "$is_server": true
            })
        );
        Ok(())
    }

    #[test]
    fn final_hook_drops_unknown_or_identified_events_and_strips_sdk_context() -> anyhow::Result<()>
    {
        let mut valid = Event::new(SERVER_STARTED.name(), "stable-id");
        for (key, value) in [
            (SERVER_VERSION, json!("1")),
            (OS_PLATFORM, json!("linux")),
            (PROCESS_PERSON_PROFILE, json!(false)),
            (GEOIP_DISABLE, json!(true)),
            (IS_SERVER, json!(true)),
            ("$os_version", json!("private")),
            ("$lib", json!("posthog-rs")),
            ("unexpected", json!("private")),
        ] {
            valid.insert_prop(key, value)?;
        }
        let filtered = final_allowlist(valid)
            .ok_or_else(|| anyhow::anyhow!("catalog event was unexpectedly dropped"))?;
        assert_eq!(filtered.properties().len(), 5);
        assert!(!filtered.properties().contains_key("$os_version"));
        assert!(!filtered.properties().contains_key("$lib"));
        assert!(!filtered.properties().contains_key("unexpected"));

        let mut identified = Event::new(SERVER_STARTED.name(), "stable-id");
        identified.insert_prop(PROCESS_PERSON_PROFILE, true)?;
        assert!(final_allowlist(identified).is_none());

        let mut unknown = Event::new("unknown", "stable-id");
        unknown.insert_prop(PROCESS_PERSON_PROFILE, false)?;
        assert!(final_allowlist(unknown).is_none());
        Ok(())
    }

    #[tokio::test]
    async fn gates_and_runtime_consent_control_client_lifetime() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let (host, mut requests, endpoint) = local_endpoint().await?;
        let service =
            AnalyticsService::new_with_config(directory.path(), test_config(host, true)).await?;
        let original = service.get_state().await;
        assert!(original.enabled);

        let disabled = service.set_consent(false).await?;
        assert!(!disabled.enabled);
        assert!(!disabled.consent);
        service.capture(SERVER_STARTED, json!({}));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), requests.recv())
                .await
                .is_err()
        );

        let enabled = service.set_consent(true).await?;
        assert!(enabled.enabled);
        assert!(enabled.consent);
        assert_eq!(enabled.distinct_id, original.distinct_id);
        service.capture(SERVER_STARTED, json!({}));
        service.shutdown().await;
        assert!(
            tokio::time::timeout(Duration::from_secs(2), requests.recv())
                .await?
                .is_some()
        );
        service.shutdown().await;
        endpoint.abort();

        let no_key = AnalyticsConfig {
            project_key: None,
            host: DEFAULT_POSTHOG_HOST.to_string(),
            environment_enabled: true,
        };
        let gated = AnalyticsService::new_with_config(directory.path(), no_key).await?;
        let state = gated.get_state().await;
        assert!(!state.enabled);
        assert!(state.consent);

        let env_off = AnalyticsService::new_with_config(
            directory.path(),
            test_config(DEFAULT_POSTHOG_HOST.to_string(), false),
        )
        .await?;
        assert!(!env_off.get_state().await.enabled);
        Ok(())
    }

    #[tokio::test]
    async fn failed_opt_out_returns_an_error_and_keeps_delivery_disabled() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let (host, mut requests, endpoint) = local_endpoint().await?;
        let mut service =
            AnalyticsService::new_with_config(directory.path(), test_config(host, true)).await?;
        let non_directory = directory.path().join("not-a-directory");
        tokio::fs::write(&non_directory, "block writes below this path").await?;
        service.path = non_directory.join("analytics.json");

        assert!(service.set_consent(false).await.is_err());
        let state = service.get_state().await;
        assert!(!state.enabled);
        assert!(!state.consent);
        service.capture(SERVER_STARTED, json!({}));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), requests.recv())
                .await
                .is_err()
        );

        endpoint.abort();
        Ok(())
    }

    #[tokio::test]
    async fn unresponsive_delivery_cannot_hang_shutdown_past_the_request_and_drain_budgets()
    -> anyhow::Result<()> {
        let directory = tempdir()?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let host = format!("http://{}", listener.local_addr()?);
        let endpoint = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let _stream = stream;
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });
        let service =
            AnalyticsService::new_with_config(directory.path(), test_config(host, true)).await?;
        service.capture(SERVER_STARTED, json!({}));

        tokio::time::timeout(Duration::from_secs(5), service.shutdown()).await?;
        endpoint.abort();
        Ok(())
    }
}
