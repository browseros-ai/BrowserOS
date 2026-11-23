//! No credential a provider echoes back reaches the server's log. Each wire's real Rig decoder
//! reads the shapes a provider can put a credential in (an error event in the stream, a 401 body,
//! a 500 that repeats the start of the request, an event type Rig does not model, a tool call id
//! used twice, a finish reason Rig does not know) while the server's log layer, built with
//! `log_filter` as `app` builds it, captures every line at the default level and at debug.
//!
//! This is the gate on a Rig upgrade: a renamed target or a new log site that writes provider
//! text fails here, not in a person's log file.

use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use serde_json::json;
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

use super::chatgpt::Session;
use super::route::{Clients, Route, Trust, Wire};
use super::{
    Backend, Model, ModelMessage, Models, Reasoning, StepRequest, event_stream, log_filter,
};
use crate::store::{Credentials, ProviderConfig, ProviderKind, Secret, Store};

/// The row's credential, and the value of its custom header. Neither matches the server's own
/// scrubbing patterns, so a pass cannot come from them.
const KEY: &str = "seeded-credential-6f1d2c9e";
const HEADER: &str = "seeded-header-a83b51";
/// The Bedrock row's key id and session token.
const KEY_ID: &str = "SEEDEDKEYIDB29F";
const TOKEN: &str = "seeded-session-token-41c7";

const SECRETS: [&str; 4] = [KEY, HEADER, KEY_ID, TOKEN];

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
    }
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    StreamError,
    Unauthorized,
    EchoedRequest,
    /// An event whose type is the seeded value (the tagged wires: Responses, ChatGPT, Anthropic,
    /// Bedrock).
    UnknownEvent,
    /// Two tool calls with the seeded value as their one id.
    DuplicateCall,
    /// A finish reason that is the seeded value.
    UnknownFinish,
}

const SHAPES: [Shape; 6] = [
    Shape::StreamError,
    Shape::Unauthorized,
    Shape::EchoedRequest,
    Shape::UnknownEvent,
    Shape::DuplicateCall,
    Shape::UnknownFinish,
];
const WIRES: [Wire; 5] = [
    Wire::Chat(ProviderKind::OpenaiCompatible),
    Wire::Responses,
    Wire::ChatGpt,
    Wire::Anthropic,
    Wire::Bedrock,
];

type Answer = (StatusCode, &'static str, Vec<u8>);

/// What a provider on `wire` answers with for `shape`, or `None` where the wire has no such
/// shape (Chat Completions events carry no type).
fn reply(wire: Wire, shape: Shape) -> Option<Answer> {
    match shape {
        Shape::StreamError | Shape::Unauthorized | Shape::EchoedRequest => Some(echo(wire, shape)),
        Shape::UnknownEvent => unknown_event(wire),
        Shape::DuplicateCall => Some(duplicate_call(wire)),
        Shape::UnknownFinish => Some(unknown_finish(wire)),
    }
}

fn sse(events: &[serde_json::Value]) -> Answer {
    let mut body = String::new();
    for event in events {
        body.push_str(&format!("data: {event}\n\n"));
    }
    (StatusCode::OK, "text/event-stream", body.into_bytes())
}

fn converse(frames: Vec<Vec<u8>>) -> Answer {
    (
        StatusCode::OK,
        "application/vnd.amazon.eventstream",
        frames.concat(),
    )
}

fn unknown_event(wire: Wire) -> Option<Answer> {
    Some(match wire {
        Wire::Chat(_) => return None,
        Wire::Responses | Wire::ChatGpt => sse(&[
            json!({"type": KEY, "detail": HEADER}),
            json!({"type":"response.output_text.delta","delta":"ok"}),
            json!({"type":"response.completed","response":{"status":"completed","output":[]}}),
        ]),
        Wire::Anthropic => sse(&[
            json!({"type":"message_start","message":{"role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}),
            json!({"type": KEY, "detail": HEADER}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}),
            json!({"type":"message_stop"}),
        ]),
        Wire::Bedrock => converse(vec![
            event_stream::event("messageStart", &json!({"role": "assistant"})),
            event_stream::event(KEY, &json!({"detail": HEADER})),
            event_stream::event(
                "contentBlockDelta",
                &json!({"contentBlockIndex": 0, "delta": {"text": "ok"}}),
            ),
            event_stream::event("contentBlockStop", &json!({"contentBlockIndex": 0})),
            event_stream::event("messageStop", &json!({"stopReason": "end_turn"})),
            event_stream::event(
                "metadata",
                &json!({"usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}, "metrics": {"latencyMs": 1}}),
            ),
        ]),
    })
}

fn duplicate_call(wire: Wire) -> Answer {
    match wire {
        Wire::Chat(_) => sse(&[
            json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[
                {"index":0,"id":KEY,"type":"function","function":{"name":"run","arguments":"{}"}},
                {"index":1,"id":KEY,"type":"function","function":{"name":"run","arguments":"{}"}}
            ]},"finish_reason":"tool_calls"}]}),
        ]),
        Wire::Responses | Wire::ChatGpt => {
            let call = |id: &str| json!({"type":"function_call","id":id,"call_id":KEY,"name":"run","arguments":"{}","status":"completed"});
            sse(&[
                json!({"type":"response.output_item.added","output_index":0,"item":call("fc_1")}),
                json!({"type":"response.output_item.done","output_index":0,"item":call("fc_1")}),
                json!({"type":"response.output_item.added","output_index":1,"item":call("fc_2")}),
                json!({"type":"response.output_item.done","output_index":1,"item":call("fc_2")}),
                json!({"type":"response.completed","response":{"status":"completed","output":[call("fc_1"),call("fc_2")]}}),
            ])
        }
        Wire::Anthropic => sse(&[
            json!({"type":"message_start","message":{"role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":KEY,"name":"run","input":{}}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":KEY,"name":"run","input":{}}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":1}}),
            json!({"type":"message_stop"}),
        ]),
        Wire::Bedrock => {
            let call = |index: i32| {
                [
                    event_stream::event(
                        "contentBlockStart",
                        &json!({"contentBlockIndex": index, "start": {"toolUse": {"toolUseId": KEY, "name": "run"}}}),
                    ),
                    event_stream::event(
                        "contentBlockDelta",
                        &json!({"contentBlockIndex": index, "delta": {"toolUse": {"input": "{}"}}}),
                    ),
                    event_stream::event("contentBlockStop", &json!({"contentBlockIndex": index})),
                ]
            };
            let mut frames = vec![event_stream::event(
                "messageStart",
                &json!({"role": "assistant"}),
            )];
            frames.extend(call(0));
            frames.extend(call(1));
            frames.push(event_stream::event(
                "messageStop",
                &json!({"stopReason": "tool_use"}),
            ));
            frames.push(event_stream::event("metadata", &json!({"usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}, "metrics": {"latencyMs": 1}})));
            converse(frames)
        }
    }
}

fn unknown_finish(wire: Wire) -> Answer {
    match wire {
        Wire::Chat(_) => {
            sse(&[json!({"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":KEY}]})])
        }
        Wire::Responses | Wire::ChatGpt => sse(&[
            json!({"type":"response.output_text.delta","delta":"ok"}),
            json!({"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":KEY},"output":[]}}),
        ]),
        Wire::Anthropic => sse(&[
            json!({"type":"message_start","message":{"role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":KEY},"usage":{"output_tokens":1}}),
            json!({"type":"message_stop"}),
        ]),
        Wire::Bedrock => converse(vec![
            event_stream::event("messageStart", &json!({"role": "assistant"})),
            event_stream::event(
                "contentBlockDelta",
                &json!({"contentBlockIndex": 0, "delta": {"text": "ok"}}),
            ),
            event_stream::event("contentBlockStop", &json!({"contentBlockIndex": 0})),
            event_stream::event("messageStop", &json!({"stopReason": KEY})),
            event_stream::event(
                "metadata",
                &json!({"usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}, "metrics": {"latencyMs": 1}}),
            ),
        ]),
    }
}

/// The shapes that echo a credential in an error.
fn echo(wire: Wire, shape: Shape) -> Answer {
    let echoed = format!(
        "POST /v1 HTTP/1.1\r\nauthorization: Bearer {KEY}\r\nx-api-key: {KEY}\r\n\
         x-amz-security-token: {TOKEN}\r\nauthorization: AWS4-HMAC-SHA256 Credential={KEY_ID}/\r\n\
         x-fixture: {HEADER}\r\n"
    );
    let message = format!("the key {KEY} and {HEADER} from {KEY_ID} with {TOKEN} were refused");
    let sse = |events: &[serde_json::Value]| {
        let mut body = String::new();
        for event in events {
            body.push_str(&format!("data: {event}\n\n"));
        }
        (StatusCode::OK, "text/event-stream", body.into_bytes())
    };
    match (wire, shape) {
        (Wire::Bedrock, Shape::StreamError) => (
            StatusCode::OK,
            "application/vnd.amazon.eventstream",
            [
                event_stream::event("messageStart", &json!({"role": "assistant"})),
                event_stream::exception("throttlingException", &json!({"message": message})),
            ]
            .concat(),
        ),
        (Wire::Bedrock, Shape::Unauthorized) => (
            StatusCode::FORBIDDEN,
            "application/json",
            json!({"message": message}).to_string().into_bytes(),
        ),
        (Wire::Chat(_), Shape::StreamError) => sse(&[
            json!({"choices":[{"index":0,"delta":{"content":"partial"}}]}),
            json!({"error":{"code":502,"message":message}}),
        ]),
        (Wire::Responses, Shape::StreamError) => sse(&[
            json!({"type":"error","code":"server_error","message":message}),
        ]),
        (Wire::ChatGpt, Shape::StreamError) => sse(&[
            json!({"type":"response.failed","response":{"status":"failed","error":{"code":"server_error","message":message}}}),
        ]),
        (Wire::Anthropic, Shape::StreamError) => sse(&[
            json!({"type":"error","error":{"type":"overloaded_error","message":message}}),
        ]),
        (Wire::Anthropic, Shape::Unauthorized) => (
            StatusCode::UNAUTHORIZED,
            "application/json",
            json!({"type":"error","error":{"type":"authentication_error","message":message}})
                .to_string()
                .into_bytes(),
        ),
        (_, Shape::Unauthorized) => (
            StatusCode::UNAUTHORIZED,
            "application/json",
            json!({"error":{"message":message,"type":"invalid_request_error","code":"invalid_api_key"}})
                .to_string()
                .into_bytes(),
        ),
        // `reply` sends only the three echo shapes here; this is the 500 that repeats the start
        // of the request.
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "text/plain",
            echoed.into_bytes(),
        ),
    }
}

/// A loopback provider that answers every request with `reply`.
async fn provider(
    reply: (StatusCode, &'static str, Vec<u8>),
) -> anyhow::Result<(String, tokio::task::JoinHandle<()>)> {
    let app = axum::Router::new()
        .fallback(
            |State(reply): State<Arc<(StatusCode, &'static str, Vec<u8>)>>,
             _headers: HeaderMap,
             _body: Bytes| async move {
                let (status, kind, body) = reply.as_ref().clone();
                (status, [(header::CONTENT_TYPE, kind)], body)
            },
        )
        .with_state(Arc::new(reply));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((base, server))
}

fn row(kind: ProviderKind, base: &str) -> ProviderConfig {
    ProviderConfig {
        id: "fixture".into(),
        kind,
        model: "fixture-model".into(),
        base_url: Some(format!("{base}/v1")),
        headers: vec![("x-fixture".into(), HEADER.into())],
        context_window: 8192,
        supports_images: false,
        reasoning_effort: None,
        reasoning_summary: None,
        resource_name: None,
        credentials: Credentials::ApiKey(Secret::new(KEY)),
    }
}

fn wrap(route: Route, kind: ProviderKind) -> Model {
    Model {
        route: Backend::Rig(Box::new(route)),
        name: "fixture-model".into(),
        window: 8192,
        supports_images: false,
        kind: Some(kind),
        reasoning: Reasoning::default(),
    }
}

/// The model for `wire`, resolved the way a turn resolves it, pointed at `base`.
async fn model(models: &Models, wire: Wire, base: &str) -> anyhow::Result<Model> {
    Ok(match wire {
        Wire::Chat(kind) => models.resolve(row(kind, base), Uuid::new_v4()).await?,
        Wire::Responses => {
            models
                .resolve(row(ProviderKind::Openai, base), Uuid::new_v4())
                .await?
        }
        Wire::Anthropic => {
            models
                .resolve(row(ProviderKind::Anthropic, base), Uuid::new_v4())
                .await?
        }
        Wire::ChatGpt => {
            let session = Session {
                url: url::Url::parse(&format!("{base}/backend-api/codex/responses"))?,
                access: Secret::new(KEY),
                account_id: Some(HEADER.into()),
            };
            let route = Route::chatgpt(
                &row(ProviderKind::ChatgptPro, base),
                session,
                &Clients::new()?,
            )?;
            wrap(route, ProviderKind::ChatgptPro)
        }
        Wire::Bedrock => {
            let mut row = row(ProviderKind::Bedrock, base);
            row.model = "anthropic.claude-sonnet-4-5-v1:0".into();
            row.credentials = Credentials::Aws {
                access_key_id: Secret::new(KEY_ID),
                secret_access_key: Secret::new(KEY),
                session_token: Some(Secret::new(TOKEN)),
                region: "us-east-1".into(),
            };
            let (model, secrets, retry_after) = super::bedrock::model_at(&row, Some(base))?;
            wrap(
                Route {
                    model,
                    wire: Wire::Bedrock,
                    url: None,
                    secrets,
                    trust: Trust::System,
                    retry_after: Some(retry_after),
                },
                ProviderKind::Bedrock,
            )
        }
    })
}

/// Runs every wire and shape with a subscriber whose layer uses `filter`, and returns what it
/// logged.
async fn logged(filter: tracing_subscriber::EnvFilter, shapes: &[Shape]) -> anyhow::Result<String> {
    logged_on(filter, &WIRES, shapes).await
}

/// The HTTP wires: every wire but Bedrock, whose SDK logs its signed request at trace.
const HTTP_WIRES: [Wire; 4] = [
    Wire::Chat(ProviderKind::OpenaiCompatible),
    Wire::Responses,
    Wire::ChatGpt,
    Wire::Anthropic,
];

async fn logged_on(
    filter: tracing_subscriber::EnvFilter,
    wires: &[Wire],
    shapes: &[Shape],
) -> anyhow::Result<String> {
    let capture = Capture::default();
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(capture.clone())
        .with_filter(filter);
    let _default = tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
    let dir = tempfile::tempdir()?;
    let models = Models::new(Store::open(dir.path()).await?, None);
    let messages = [ModelMessage::User {
        text: "Hello".into(),
        images: Vec::new(),
    }];
    let request = StepRequest {
        system: "Help",
        messages: &messages,
        tools: &[],
        effort: None,
    };
    for &wire in wires {
        for &shape in shapes {
            let Some(answer) = reply(wire, shape) else {
                continue;
            };
            let (base, server) = provider(answer).await?;
            let model = model(&models, wire, &base).await?;
            let result = model.step(&request, &mut Vec::new()).await;
            server.abort();
            // A provider-sent type, id or finish reason need not fail the step.
            let Err(error) = result else {
                continue;
            };
            // The card is scrubbed of what this route sent; the Bedrock values are not secrets
            // of an HTTP route, which never had them.
            let sent: &[&str] = match wire {
                Wire::Bedrock => &SECRETS,
                _ => &[KEY, HEADER],
            };
            for secret in sent {
                let details = error.details.as_deref().unwrap_or_default();
                anyhow::ensure!(
                    !error.message.contains(secret) && !details.contains(secret),
                    "the card for {shape:?} on {} shows {secret}",
                    wire_name(wire)
                );
            }
        }
    }
    Ok(capture.text())
}

fn wire_name(wire: Wire) -> &'static str {
    match wire {
        Wire::Chat(_) => "Chat Completions",
        Wire::Responses => "Responses",
        Wire::ChatGpt => "ChatGPT",
        Wire::Anthropic => "Anthropic Messages",
        Wire::Bedrock => "Bedrock",
    }
}

#[tokio::test]
async fn no_echoed_credential_reaches_the_log_at_info_or_debug() -> anyhow::Result<()> {
    for spec in [None, Some("debug")] {
        let text = logged(log_filter(spec), &SHAPES).await?;
        assert!(!text.is_empty(), "the layer captured nothing at {spec:?}");
        for secret in SECRETS {
            let leaked: Vec<_> = text.lines().filter(|line| line.contains(secret)).collect();
            assert!(
                leaked.is_empty(),
                "{secret} reached the log at {spec:?}:\n{}",
                leaked.join("\n")
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn the_capture_sees_each_leak_the_filter_turns_off() -> anyhow::Result<()> {
    // Without the server's filter, each of these reaches the log through some wire's decoder:
    // Rig logs a stream error's message, an unmodelled event's type, a reused call id and an
    // unknown finish reason as the provider sent them.
    for shape in [
        Shape::StreamError,
        Shape::UnknownEvent,
        Shape::DuplicateCall,
        Shape::UnknownFinish,
    ] {
        let unfiltered = logged(tracing_subscriber::EnvFilter::new("debug"), &[shape]).await?;
        assert!(
            unfiltered.lines().any(|line| line.contains(KEY)),
            "without the filter, {shape:?} logs the seeded value"
        );
    }
    let opted_in = logged(
        log_filter(Some(
            "info,rig_core::providers::openai::wire::chat::finish=warn",
        )),
        &[Shape::StreamError],
    )
    .await?;
    assert!(
        opted_in.lines().any(|line| line.contains(KEY)),
        "a developer who names a target gets its lines back"
    );
    Ok(())
}

#[test]
fn every_rig_crate_in_the_lockfile_and_every_target_it_names_is_turned_off() -> anyhow::Result<()> {
    let muted = |target: &str| {
        // A filter target matches every target it is a prefix of (tracing-subscriber's rule).
        super::UNSCRUBBED_LOG_TARGETS
            .iter()
            .any(|off| target.starts_with(off))
    };
    let lockfile =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))?;
    let rig: Vec<String> = lockfile
        .lines()
        .filter_map(|line| line.strip_prefix("name = \""))
        .filter_map(|name| name.strip_suffix('"'))
        .filter(|name| name.starts_with("rig-"))
        .map(|name| name.replace('-', "_"))
        .collect();
    assert!(!rig.is_empty(), "the lockfile names no rig crate");
    for name in &rig {
        assert!(
            muted(name),
            "{name} is in Cargo.lock but its logs are not turned off (UNSCRUBBED_LOG_TARGETS)"
        );
    }

    // Rig also names targets of its own outside its module paths, `rig::streaming` among them.
    // Read every one its locked sources name, so a new one after an upgrade fails here.
    let metadata = std::process::Command::new(env!("CARGO"))
        // The host's graph, as tests/feature_guard.rs reads it: a cache filled by a host build
        // holds no other platform's sources.
        .args(["metadata", "--format-version", "1", "--locked", "--offline"])
        .args(["--filter-platform", "host-tuple"])
        .arg("--manifest-path")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .output()?;
    anyhow::ensure!(
        metadata.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout)?;
    let named = regex::Regex::new(r#"(?:\btarget\s*[:=]\s*|\bemit!\()"([^"]+)""#)?;
    let mut targets = std::collections::BTreeSet::new();
    let mut sources = vec![];
    for package in metadata["packages"].as_array().into_iter().flatten() {
        if package["name"]
            .as_str()
            .is_some_and(|name| name.starts_with("rig-"))
            && let Some(manifest) = package["manifest_path"].as_str()
            && let Some(root) = std::path::Path::new(manifest).parent()
        {
            sources.push(root.join("src"));
        }
    }
    while let Some(path) = sources.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(&path)? {
                sources.push(entry?.path());
            }
            continue;
        }
        // Rig's own tests name targets that never run in this build.
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let test_only = name.ends_with("tests.rs")
            || path
                .components()
                .any(|part| matches!(part.as_os_str().to_str(), Some("tests" | "test_utils")));
        if !name.ends_with(".rs") || test_only {
            continue;
        }
        for line in std::fs::read_to_string(&path)?.lines() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            targets.extend(named.captures_iter(line).map(|found| found[1].to_owned()));
        }
    }
    for expected in ["rig::completions", "rig::streaming"] {
        assert!(
            targets.contains(expected),
            "the scan of Rig's sources did not find {expected}"
        );
    }
    for target in &targets {
        assert!(
            muted(target),
            "Rig logs under {target} and it is not turned off (UNSCRUBBED_LOG_TARGETS)"
        );
    }
    Ok(())
}

#[tokio::test]
async fn rig_writes_no_raw_json_at_trace() -> anyhow::Result<()> {
    // At trace, `rig::completions` and `rig::streaming` carry a request's and a reply's JSON. The
    // AWS SDK's own trace output is out of scope (see `log_filter`), so the HTTP wires are run.
    let rig_lines = |text: &str| {
        text.lines()
            .filter(|line| line.contains(" rig::completions:") || line.contains(" rig::streaming:"))
            .count()
    };
    let only_crates = tracing_subscriber::EnvFilter::new(
        "trace,rig_core=off,rig_bedrock=off,rig_http=off,rig_reqwest=off",
    );
    let unfiltered = logged_on(only_crates, &HTTP_WIRES, &[Shape::UnknownFinish]).await?;
    assert!(
        rig_lines(&unfiltered) > 0,
        "muting Rig's crates alone leaves its raw JSON in the log"
    );
    let filtered = logged_on(
        log_filter(Some("trace")),
        &HTTP_WIRES,
        &[Shape::UnknownFinish],
    )
    .await?;
    assert_eq!(rig_lines(&filtered), 0, "{filtered}");
    Ok(())
}
