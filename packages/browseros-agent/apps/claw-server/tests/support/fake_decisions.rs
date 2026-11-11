//! A stand-in for the decisions worker on `127.0.0.1:0`. It logs every request so a test can
//! check what left the server, answers jev and the writer with fixed replies, and can be
//! scripted to answer the next call on a route with a given status.
#![allow(dead_code)]

use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::task::JoinHandle;

pub const JEV_ROUTE: &str = "/v1/jev/systemone";
pub const WRITER_ROUTE: &str = "/v1/writer/field-values";

#[derive(Clone, Debug)]
pub struct Logged {
    pub path: String,
    pub bearer: Option<String>,
    pub body: Value,
}

#[derive(Default)]
pub struct FakeDecisionsState {
    requests: Vec<Logged>,
    scripted: HashMap<String, VecDeque<(StatusCode, Value)>>,
    pub values: Vec<String>,
}

#[derive(Clone)]
pub struct FakeDecisions {
    pub url: String,
    state: Arc<Mutex<FakeDecisionsState>>,
    _task: Arc<JoinHandle<()>>,
}

impl FakeDecisions {
    pub async fn start() -> anyhow::Result<Self> {
        let state = Arc::new(Mutex::new(FakeDecisionsState {
            values: vec!["sensodyne".to_string(), "sensodyne fresh mint".to_string()],
            ..FakeDecisionsState::default()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let router = Router::new().fallback(handle).with_state(state.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router.into_make_service()).await;
        });
        Ok(Self {
            url: format!("http://{address}/"),
            state,
            _task: Arc::new(task),
        })
    }

    pub fn url(&self) -> anyhow::Result<url::Url> {
        Ok(url::Url::parse(&self.url)?)
    }

    fn state(&self) -> MutexGuard<'_, FakeDecisionsState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The next call to `route` answers `status` with `body` instead.
    pub fn script(&self, route: &str, status: u16, body: Value) {
        self.state()
            .scripted
            .entry(route.to_string())
            .or_default()
            .push_back((
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                body,
            ));
    }

    pub fn requests(&self) -> Vec<Logged> {
        self.state().requests.clone()
    }
}

async fn handle(State(state): State<Arc<Mutex<FakeDecisionsState>>>, request: Request) -> Response {
    let path = request.uri().path().to_string();
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_string);
    let body = to_bytes(request.into_body(), 1 << 20)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    let mut state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.requests.push(Logged {
        path: path.clone(),
        bearer,
        body,
    });
    if let Some((status, body)) = state.scripted.get_mut(&path).and_then(VecDeque::pop_front) {
        return (status, axum::Json(body)).into_response();
    }
    match path.as_str() {
        JEV_ROUTE => axum::Json(json!({
            "model": "jev-1.13.0",
            "answers": {},
            "usage": { "input_tokens": 10, "output_tokens": 0 },
        }))
        .into_response(),
        WRITER_ROUTE => axum::Json(json!({ "values": state.values })).into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
