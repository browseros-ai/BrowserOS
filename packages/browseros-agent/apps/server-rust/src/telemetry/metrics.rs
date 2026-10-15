//! PostHog sink for the TS server's `browseros.server.*` events. Calls never await the
//! network; the SDK owns a bounded send queue, and noisy MCP/tool events are aggregated in
//! wall-clock-aligned 30-minute buckets. No install id or build-time key means no client.

use posthog_rs::{Client, ClientOptionsBuilder, Event};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

const BUILD_KEY: Option<&str> = option_env!("POSTHOG_API_KEY");
const PREFIX: &str = "browseros.server.";
const INTERVAL_MS: u64 = 30 * 60 * 1000;
const SAMPLE_RATE: f64 = 0.2;
const MAX_TOOL_NAMES: usize = 200;

/// Stable process identity. These fields override event properties so callers cannot
/// accidentally attribute a server event to a different installation or product.
#[derive(Debug, Clone, Default)]
pub struct MetricsConfig {
    pub install_id: Option<String>,
    pub browseros_version: Option<String>,
    pub chromium_version: Option<String>,
    pub server_version: Option<String>,
    pub properties: BTreeMap<String, Value>,
}

#[derive(Clone, Default)]
pub struct Metrics {
    active: Option<Arc<Active>>,
}

struct Active {
    client: Client,
    install_id: String,
    config: MetricsConfig,
    rollup: Mutex<Rollup>,
    stop: CancellationToken,
}

impl Drop for Active {
    fn drop(&mut self) {
        // The timer holds only a Weak handle, so dropping the last owner cancels it. A
        // background telemetry task must never extend the server's own lifetime.
        self.stop.cancel();
    }
}

impl Metrics {
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Initializes once after installation identity is resolved. Keys are compiled from the
    /// same env name as Bun's INLINED_ENV; Chromium need not export them at runtime.
    pub async fn initialize(config: MetricsConfig) -> Self {
        Self::initialize_with(config, BUILD_KEY, "https://us.i.posthog.com").await
    }

    async fn initialize_with(config: MetricsConfig, key: Option<&str>, host: &str) -> Self {
        let Some(install_id) = config.install_id.as_deref().filter(|id| !id.is_empty()) else {
            return Self::disabled();
        };
        let Some(key) = key.filter(|key| !key.is_empty()) else {
            return Self::disabled();
        };
        let options = ClientOptionsBuilder::default()
            .api_key(key.to_owned())
            .host(host.to_owned())
            .request_timeout_seconds(2u64)
            .max_queue_size(256usize)
            .shutdown_timeout_ms(2000u64)
            .build();
        let Ok(options) = options else {
            tracing::warn!("invalid PostHog configuration; metrics disabled");
            return Self::disabled();
        };
        let active = Arc::new(Active {
            client: posthog_rs::client(options).await,
            install_id: install_id.to_owned(),
            config,
            rollup: Mutex::new(Rollup::default()),
            stop: CancellationToken::new(),
        });
        let weak = Arc::downgrade(&active);
        let stop = active.stop.clone();
        tokio::spawn(async move {
            loop {
                let delay = INTERVAL_MS - now_ms() % INTERVAL_MS;
                tokio::select! {
                    () = stop.cancelled() => break,
                    () = tokio::time::sleep(Duration::from_millis(delay)) => {}
                }
                let Some(active) = weak.upgrade() else { break };
                active.flush_rollup();
            }
        });
        Self {
            active: Some(active),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| !active.stop.is_cancelled())
    }

    pub fn capture(&self, event: &str, properties: Value) {
        self.capture_sampled(event, properties, SAMPLE_RATE);
    }

    /// Startup can request sampling=1. Aggregated events are always counted before sampling,
    /// matching TS; sampling an execution before counting it would under-report usage.
    pub fn capture_sampled(&self, event: &str, properties: Value, sampling: f64) {
        let Some(active) = &self.active else { return };
        if active.stop.is_cancelled() {
            return;
        }
        if matches!(event, "mcp.request" | "tool_executed") {
            active
                .rollup
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .record(event, &properties, now_ms());
            return;
        }
        let sampling = if sampling.is_finite() {
            sampling.clamp(0.0, 1.0)
        } else {
            SAMPLE_RATE
        };
        if sampling <= 0.0 || sampling < 1.0 && rand::random::<f64>() >= sampling {
            return;
        }
        let mut properties = properties.as_object().cloned().unwrap_or_default();
        if sampling < 1.0 {
            properties.insert("sample_rate".to_owned(), json!(sampling));
        }
        active.send(event, Value::Object(properties));
    }

    /// Call before the Tokio runtime exits. Clones share the same stop token, so no caller can
    /// enqueue events after shutdown starts. The final rollup is drained before the SDK queue.
    pub async fn shutdown(&self) {
        if let Some(active) = &self.active {
            active.stop.cancel();
            active.flush_rollup();
            active.client.shutdown().await;
        }
    }
}

impl Active {
    fn send(&self, name: &str, event_properties: Value) {
        let mut properties: serde_json::Map<_, _> =
            self.config.properties.clone().into_iter().collect();
        // Build defaults first, then event properties, then protected identity fields (TS order).
        if let Value::Object(event_properties) = event_properties {
            properties.extend(event_properties);
        }
        properties.insert("install_id".to_owned(), json!(self.install_id));
        properties.insert("product".to_owned(), json!("browseros"));
        properties.insert("surface".to_owned(), json!("server"));
        properties.insert("$process_person_profile".to_owned(), json!(false));
        for (name, value) in [
            ("browseros_version", &self.config.browseros_version),
            ("chromium_version", &self.config.chromium_version),
            ("server_version", &self.config.server_version),
        ] {
            if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
                properties.insert(name.to_owned(), json!(value));
            }
        }
        let mut event = Event::new(format!("{PREFIX}{name}"), self.install_id.clone());
        for (name, value) in properties {
            if event.insert_prop(name, value).is_err() {
                return;
            }
        }
        self.client.capture(event);
    }

    fn flush_rollup(&self) {
        let bucket = self
            .rollup
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(now_ms());
        if let Some(properties) = bucket {
            self.send("usage_rollup", properties);
        }
    }
}

/// The buffer contains counts only, never scripts, prompts or complete tool arguments.
#[derive(Default)]
struct Rollup {
    start: Option<u64>,
    mcp: u64,
    tools: u64,
    failed: u64,
    sources: BTreeMap<String, u64>,
    names: BTreeMap<String, u64>,
}

impl Rollup {
    fn record(&mut self, event: &str, properties: &Value, now: u64) {
        self.start.get_or_insert(now / INTERVAL_MS * INTERVAL_MS);
        if event == "mcp.request" {
            self.mcp += 1;
            return;
        }
        self.tools += 1;
        if properties.get("success") == Some(&Value::Bool(false)) {
            self.failed += 1;
        }
        *self
            .sources
            .entry(
                properties["source"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_owned(),
            )
            .or_default() += 1;
        if let Some(name) = properties["tool_name"]
            .as_str()
            .filter(|name| !name.is_empty())
        {
            let mut name: String = name.replace('.', "_").chars().take(80).collect();
            if !self.names.contains_key(&name) && self.names.len() >= MAX_TOOL_NAMES {
                name = "__other__".to_owned();
            }
            *self.names.entry(name).or_default() += 1;
        }
    }

    fn drain(&mut self, end: u64) -> Option<Value> {
        let bucket = std::mem::take(self);
        Some(json!({
            "interval_seconds": INTERVAL_MS / 1000,
            "period_start_ts": timestamp(bucket.start?), "period_end_ts": timestamp(end),
            "mcp_requests_count": bucket.mcp, "tool_executions_count": bucket.tools,
            "tool_executions_failed": bucket.failed,
            "tool_executions_by_source": bucket.sources, "tool_executions_by_tool": bucket.names,
        }))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn timestamp(milliseconds: u64) -> String {
    let Ok(format) = time::format_description::parse_borrowed::<2>(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z",
    ) else {
        return String::new();
    };
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000)
        .ok()
        .and_then(|time| time.format(&format).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::State};

    #[test]
    fn rollup_matches_ts_counts_alignment_and_cardinality() {
        let mut rollup = Rollup::default();
        assert_eq!(rollup.drain(INTERVAL_MS), None);
        rollup.record("mcp.request", &json!({}), INTERVAL_MS + 17);
        rollup.record(
            "tool_executed",
            &json!({"tool_name":"browser.click","source":"mcp","success":false}),
            INTERVAL_MS + 18,
        );
        for index in 0..MAX_TOOL_NAMES + 5 {
            rollup.record(
                "tool_executed",
                &json!({"tool_name":format!("tool.{index}")}),
                INTERVAL_MS + 19,
            );
        }
        let bucket = rollup.drain(INTERVAL_MS * 2).unwrap_or_default();
        assert_eq!(bucket["period_start_ts"], "1970-01-01T00:30:00.000Z");
        assert_eq!(bucket["period_end_ts"], "1970-01-01T01:00:00.000Z");
        assert_eq!(bucket["interval_seconds"], 1800);
        assert_eq!(bucket["mcp_requests_count"], 1);
        assert_eq!(bucket["tool_executions_count"], 206);
        assert_eq!(bucket["tool_executions_failed"], 1);
        assert_eq!(
            bucket["tool_executions_by_source"],
            json!({"mcp":1,"unknown":205})
        );
        assert_eq!(bucket["tool_executions_by_tool"]["browser_click"], 1);
        assert_eq!(bucket["tool_executions_by_tool"]["__other__"], 6);
        assert_eq!(
            bucket["tool_executions_by_tool"]
                .as_object()
                .map(|names| names.len()),
            Some(201)
        );
        assert_eq!(rollup.drain(INTERVAL_MS * 3), None);
    }

    #[tokio::test]
    async fn identity_and_build_key_gate_client_creation() {
        let config = MetricsConfig::default();
        assert!(
            !Metrics::initialize_with(config, Some("key"), "invalid")
                .await
                .is_enabled()
        );
        let config = MetricsConfig {
            install_id: Some("install".to_owned()),
            ..Default::default()
        };
        assert!(
            !Metrics::initialize_with(config, None, "invalid")
                .await
                .is_enabled()
        );
    }

    #[tokio::test]
    async fn posthog_wire_preserves_identity_names_and_unsampled_rollup() -> anyhow::Result<()> {
        type Captures = Arc<Mutex<Vec<Value>>>;
        async fn batch(State(seen): State<Captures>, Json(body): Json<Value>) -> Json<Value> {
            seen.lock().unwrap_or_else(|e| e.into_inner()).push(body);
            Json(json!({"status":1}))
        }
        let seen: Captures = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let host = format!("http://{}", listener.local_addr()?);
        let router = Router::new().fallback(batch).with_state(seen.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let metrics = Metrics::initialize_with(
            MetricsConfig {
                install_id: Some("install".to_owned()),
                server_version: Some("1.2.3".to_owned()),
                properties: BTreeMap::from([("os_platform".to_owned(), json!("darwin"))]),
                ..Default::default()
            },
            Some("test-key"),
            &host,
        )
        .await;
        assert!(metrics.is_enabled());
        metrics.capture_sampled("http_server.started", json!({"install_id":"spoof", "product":"spoof", "surface":"spoof", "$process_person_profile":true, "server_version":"spoof"}), 1.0);
        metrics.capture_sampled("filtered", json!({}), 0.0);
        // Aggregation precedes sampling, even at sampling=0.
        metrics.capture_sampled(
            "tool_executed",
            json!({"tool_name":"run","source":"mcp","success":true}),
            0.0,
        );
        metrics.capture_sampled("mcp.request", json!({}), 0.0);
        metrics.shutdown().await;
        assert!(!metrics.is_enabled());
        metrics.capture_sampled("after_shutdown", json!({}), 1.0);
        task.abort();
        let requests = seen.lock().unwrap_or_else(|e| e.into_inner());
        let events: Vec<_> = requests
            .iter()
            .flat_map(|body| body["batch"].as_array().into_iter().flatten())
            .collect();
        assert_eq!(events.len(), 2, "{requests:?}");
        for event in &events {
            assert_eq!(event["distinct_id"], "install");
            assert_eq!(event["properties"]["install_id"], "install");
            assert_eq!(event["properties"]["product"], "browseros");
            assert_eq!(event["properties"]["surface"], "server");
            assert_eq!(event["properties"]["server_version"], "1.2.3");
            assert_eq!(event["properties"]["$process_person_profile"], false);
            assert_eq!(event["properties"]["os_platform"], "darwin");
        }
        assert_eq!(events[0]["event"], "browseros.server.http_server.started");
        assert_eq!(events[1]["event"], "browseros.server.usage_rollup");
        assert_eq!(events[1]["properties"]["tool_executions_count"], 1);
        assert_eq!(events[1]["properties"]["mcp_requests_count"], 1);
        Ok(())
    }
}
