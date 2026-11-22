//! No credential a provider echoes back reaches the server's log. Each wire's real Rig decoder
//! reads the error shapes a provider can echo a credential in (an error event in the stream, a
//! 401 body, a 500 that repeats the start of the request) while the server's log layer, built
//! with `log_filter` as `app` builds it, captures every line at the default level and at debug.
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
use super::route::{Clients, Route, Wire};
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
}

const SHAPES: [Shape; 3] = [
    Shape::StreamError,
    Shape::Unauthorized,
    Shape::EchoedRequest,
];
const WIRES: [Wire; 5] = [
    Wire::Chat(ProviderKind::OpenaiCompatible),
    Wire::Responses,
    Wire::ChatGpt,
    Wire::Anthropic,
    Wire::Bedrock,
];

/// What a provider on `wire` answers with for `shape`, echoing the seeded values.
fn reply(wire: Wire, shape: Shape) -> (StatusCode, &'static str, Vec<u8>) {
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
        (_, Shape::EchoedRequest) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "text/plain",
            echoed.into_bytes(),
        ),
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
            let (model, secrets) = super::bedrock::model_at(&row, Some(base))?;
            wrap(
                Route {
                    model,
                    wire: Wire::Bedrock,
                    url: None,
                    secrets,
                },
                ProviderKind::Bedrock,
            )
        }
    })
}

/// Runs every wire and shape with a subscriber whose layer uses `filter`, and returns what it
/// logged.
async fn logged(filter: tracing_subscriber::EnvFilter) -> anyhow::Result<String> {
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
    for wire in WIRES {
        for shape in SHAPES {
            let (base, server) = provider(reply(wire, shape)).await?;
            let model = model(&models, wire, &base).await?;
            let result = model.step(&request, &mut Vec::new()).await;
            server.abort();
            let Err(error) = result else {
                anyhow::bail!("{shape:?} on {:?} did not fail", wire_name(wire));
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
        let text = logged(log_filter(spec)).await?;
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
async fn the_capture_sees_the_leak_the_filter_turns_off() -> anyhow::Result<()> {
    let unfiltered = logged(tracing_subscriber::EnvFilter::new("debug")).await?;
    assert!(
        unfiltered.lines().any(|line| line.contains(KEY)),
        "without the server's filter, Rig's stream-error warning carries the echoed key"
    );
    let opted_in = logged(log_filter(Some(
        "info,rig_core::providers::openai::wire::chat::finish=warn",
    )))
    .await?;
    assert!(
        opted_in.lines().any(|line| line.contains(KEY)),
        "a developer who names the target gets the line back"
    );
    Ok(())
}
