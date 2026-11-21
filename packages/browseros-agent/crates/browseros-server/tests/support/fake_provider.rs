//! A loopback HTTP peer that captures model requests and replays documented provider events.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub struct FakeProvider {
    pub base: String,
    pub calls: Arc<Mutex<Vec<Call>>>,
    server: JoinHandle<()>,
}

pub struct Call {
    pub uri: Uri,
    pub headers: HeaderMap,
    pub body: Value,
}

pub struct Reply(pub StatusCode, pub HeaderMap, pub String);

impl Reply {
    pub fn events(events: &[Value]) -> Self {
        let mut data = String::new();
        for event in events {
            data.push_str(&format!("data: {event}\n\n"));
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            http::HeaderValue::from_static("text/event-stream"),
        );
        Self(StatusCode::OK, headers, data)
    }
}

#[derive(Clone)]
struct Shared {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl FakeProvider {
    pub async fn new(replies: Vec<Reply>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().fallback(handler).with_state(Shared {
            replies: Arc::new(Mutex::new(replies.into())),
            calls: calls.clone(),
        });
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            base,
            calls,
            server,
        })
    }
}

impl Drop for FakeProvider {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn handler(State(shared): State<Shared>, request: Request) -> Response {
    use axum::extract::FromRequest;
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    let Ok(Json(body)) = Json::<Value>::from_request(request, &()).await else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if let Ok(mut calls) = shared.calls.lock() {
        calls.push(Call { uri, headers, body });
    }
    let reply = shared
        .replies
        .lock()
        .ok()
        .and_then(|mut replies| replies.pop_front());
    match reply {
        Some(Reply(status, headers, body)) => (status, headers, body).into_response(),
        None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn models() -> anyhow::Result<(tempfile::TempDir, browseros_server::model::Models)> {
    use browseros_server::{model::Models, store::Store};
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path()).await?;
    Ok((dir, Models::new(store, None)))
}

pub fn provider(
    kind: browseros_server::store::ProviderKind,
    base_url: String,
    credentials: browseros_server::store::Credentials,
) -> browseros_server::store::ProviderConfig {
    browseros_server::store::ProviderConfig {
        id: "fixture".into(),
        kind,
        model: "fixture-model".into(),
        base_url: Some(base_url),
        headers: Vec::new(),
        context_window: 8192,
        supports_images: true,
        reasoning_effort: None,
        reasoning_summary: None,
        resource_name: None,
        credentials,
    }
}

/// Both continuation calls use this same request shape, as does the production agent loop.
pub fn request(
    messages: &[browseros_server::model::ModelMessage],
) -> browseros_server::model::StepRequest<'_> {
    browseros_server::model::StepRequest {
        system: "Use the tools when needed.",
        messages,
        tools: &[],
        effort: Some(browseros_server::model::Effort::Medium),
    }
}
