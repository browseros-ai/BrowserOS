use std::{sync::Mutex, time::Instant};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::types::{
    CaptchaEvent, CaptchaEventName, CaptchaKind, ChallengeState, ProviderName, SolveMethod,
    SolveOutcome, SolveReason, SolveStatus,
};

#[derive(Clone)]
pub struct CaptchaEventBus {
    sender: broadcast::Sender<CaptchaEvent>,
}

impl Default for CaptchaEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptchaEventBus {
    pub fn new() -> Self {
        Self {
            sender: broadcast::channel(64).0,
        }
    }
    pub fn subscribe(&self) -> broadcast::Receiver<CaptchaEvent> {
        self.sender.subscribe()
    }
    pub fn emit(&self, event: CaptchaEvent) {
        tracing::info!(target: "captcha", event = %json!(event.event), request_id = %event.request_id, page = event.page, at_ms = event.at_ms, data = %log_data(&event), "CAPTCHA event");
        let _ = self.sender.send(event);
    }
}

pub struct EventLog {
    bus: CaptchaEventBus,
    request_id: String,
    page: u32,
    started_at: Instant,
    events: Mutex<Vec<CaptchaEvent>>,
}

impl EventLog {
    pub fn new(bus: CaptchaEventBus, request_id: String, page: u32) -> Self {
        Self {
            bus,
            request_id,
            page,
            started_at: Instant::now(),
            events: Mutex::new(Vec::new()),
        }
    }
    pub fn emit(&self, event: CaptchaEventName, data: Value) {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let event = CaptchaEvent {
            event,
            request_id: self.request_id.clone(),
            page: self.page,
            at_ms: self
                .started_at
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            data,
        };
        events.push(event.clone());
        self.bus.emit(event);
    }
    pub fn events(&self) -> Vec<CaptchaEvent> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

fn typed<T: DeserializeOwned + Serialize>(data: &Value, key: &str) -> Value {
    data.get(key)
        .and_then(|value| serde_json::from_value::<T>(value.clone()).ok())
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or(Value::Null)
}

fn log_data(event: &CaptchaEvent) -> Value {
    let data = &event.data;
    let mut fields = json!({"type": typed::<CaptchaKind>(data, "type")});
    match event.event {
        CaptchaEventName::Detected => {
            fields["state"] = typed::<ChallengeState>(data, "state");
            fields["complete"] = typed::<bool>(data, "complete");
            fields["challenges"] = typed::<u32>(data, "challenges");
        }
        CaptchaEventName::Solving => {
            fields["method"] = typed::<SolveMethod>(data, "method");
            fields["attempt"] = typed::<u32>(data, "attempt");
            if data.get("round").is_some() {
                fields["round"] = typed::<u32>(data, "round");
            }
        }
        CaptchaEventName::Solved | CaptchaEventName::TokenApplied => {
            fields["method"] = typed::<SolveMethod>(data, "method");
            fields["durationMs"] = typed::<u64>(data, "durationMs");
            if event.event == CaptchaEventName::TokenApplied {
                fields["callbackRan"] = typed::<bool>(data, "callbackRan");
            }
        }
        CaptchaEventName::Failed => {
            fields["status"] = typed::<SolveStatus>(data, "status");
            fields["reason"] = typed::<SolveReason>(data, "reason");
        }
        CaptchaEventName::Handoff => {
            fields["reason"] = typed::<SolveReason>(data, "reason");
            fields["humanHelp"] = typed::<bool>(data, "humanHelp");
        }
    }
    fields
}

pub fn log_outcome(outcome: &SolveOutcome, host: Option<&str>, provider: Option<ProviderName>) {
    let host = host.and_then(|host| {
        url::Url::parse(host)
            .ok()
            .filter(|url| url.has_host())
            .or_else(|| url::Url::parse(&format!("https://{host}")).ok())
            .and_then(|url| url.host_str().map(str::to_owned))
    });
    tracing::info!(target: "captcha", event = "captcha.outcome", request_id = %outcome.request_id,
        page = outcome.page, r#type = %json!(outcome.kind), host = host.as_deref(), method = %json!(outcome.method),
        attempts = ?outcome.attempts, latency_ms = outcome.duration_ms, status = %json!(outcome.status),
        reason = %json!(outcome.reason), paid_tasks = outcome.paid_tasks, cost_usd = outcome.cost_usd,
        provider = %json!(provider), "CAPTCHA solve finished");
}

#[cfg(test)]
mod tests {
    use super::super::types::{Attempts, CaptchaKind, SolveMethod, SolveReason, SolveStatus};
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[test]
    fn tests_that_token_applied_logs_callback_state_without_secrets() {
        for callback_ran in [false, true] {
            let event = CaptchaEvent {
                event: CaptchaEventName::TokenApplied,
                request_id: "cap_test".into(),
                page: 42,
                at_ms: 10,
                data: json!({"type":"turnstile","method":"token","durationMs":10,
                    "callbackRan":callback_ran,"token":"secret-token"}),
            };
            assert_eq!(
                log_data(&event),
                json!({"type":"turnstile","method":"token",
                "durationMs":10,"callbackRan":callback_ran})
            );
        }
    }

    #[test]
    fn tests_that_events_are_stamped_kept_in_order_and_broadcast() -> anyhow::Result<()> {
        let bus = CaptchaEventBus::new();
        let mut receiver = bus.subscribe();
        let log = EventLog::new(bus, "cap_test".into(), 42);
        let payloads = [
            (
                CaptchaEventName::Detected,
                "captcha.detected",
                json!({"type":"recaptcha_v2", "state":"active", "complete":true, "challenges":1}),
            ),
            (
                CaptchaEventName::Solving,
                "captcha.solving",
                json!({"type":"recaptcha_v2", "method":"tiles", "attempt":1, "round":2}),
            ),
            (
                CaptchaEventName::Solved,
                "captcha.solved",
                json!({"type":"recaptcha_v2", "method":"tiles", "durationMs":123}),
            ),
            (
                CaptchaEventName::Failed,
                "captcha.failed",
                json!({"type":"recaptcha_v2", "status":"failed", "reason":"provider_error"}),
            ),
            (
                CaptchaEventName::Handoff,
                "captcha.handoff",
                json!({"type":"recaptcha_v2", "reason":"no_provider", "humanHelp":true}),
            ),
        ];
        for (event, _, data) in &payloads {
            log.emit(*event, data.clone());
        }
        let events = log.events();
        assert_eq!(events.len(), payloads.len());
        for (event, (_, name, data)) in events.iter().zip(payloads) {
            assert_eq!(receiver.try_recv()?, *event);
            assert_eq!(
                serde_json::to_value(event)?,
                json!({"event":name, "requestId":"cap_test", "page":42, "atMs":event.at_ms, "data":data})
            );
        }
        assert!(events.windows(2).all(|pair| pair[0].at_ms <= pair[1].at_ms));
        Ok(())
    }

    #[test]
    fn tests_that_event_logs_work_without_subscribers() {
        let log = EventLog::new(CaptchaEventBus::default(), "cap_test".into(), 42);
        log.emit(CaptchaEventName::Solving, json!({"method":"checkbox"}));
        assert_eq!(log.events().len(), 1);
    }

    #[derive(Clone, Default)]
    struct LogWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogWriter {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn tests_that_tracing_records_contract_fields_without_sensitive_data() -> anyhow::Result<()> {
        let writer = LogWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(writer.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let bus = CaptchaEventBus::new();
        bus.emit(CaptchaEvent {
            event: CaptchaEventName::Solving, request_id: "cap_test".into(), page: 42, at_ms: 10,
            data: json!({"type":"recaptcha_v2", "method":"tiles", "attempt":1, "round":2, "token":"secret-token", "url":"https://example.com/private?q=secret", "image":"secret-image"}),
        });
        log_outcome(
            &SolveOutcome {
                status: SolveStatus::Solved,
                kind: Some(CaptchaKind::RecaptchaV2),
                method: SolveMethod::Tiles,
                reason: SolveReason::TilesAccepted,
                request_id: "cap_test".into(),
                page: 42,
                duration_ms: 123,
                attempts: Attempts {
                    checkbox: 1,
                    tile_rounds: 2,
                    token: 0,
                },
                paid_tasks: 2,
                cost_usd: 0.002,
                message: "secret-token https://example.com/private?q=secret secret-image".into(),
                events: Vec::new(),
            },
            Some("https://user:password@example.com/private?q=secret"),
            Some(ProviderName::Capsolver),
        );
        let output = String::from_utf8(
            writer
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        )?;
        assert_eq!(output.lines().count(), 2, "{output}");
        for field in [
            "captcha:",
            "captcha.solving",
            "request_id=cap_test",
            "page=42",
            "at_ms=10",
            "recaptcha_v2",
            "example.com",
            "tiles_accepted",
            "capsolver",
            "paid_tasks=2",
            "cost_usd=0.002",
        ] {
            assert!(output.contains(field), "missing {field}: {output}");
        }
        for secret in [
            "secret-token",
            "secret-image",
            "https://",
            "/private",
            "q=secret",
            "password",
        ] {
            assert!(!output.contains(secret), "leaked {secret}: {output}");
        }
        Ok(())
    }
}
