//! The in-process test harness every chunk's tests stand on: the real router over a temporary
//! state folder (so a real `browseros.sqlite`, never `~/.browseros`), the scripted model, the
//! `Browser` double, and a scripted `TurnRunner` for tests of `conversations` that should not
//! depend on the agent loop. No port is bound: requests go through `tower::ServiceExt::oneshot`.
//!
//! ```ignore
//! mod support;
//! let server = support::TestServer::start().await?;
//! let response = server.send(server.get("/system/health")).await;
//! ```

// Each test file includes this module and uses part of it.
#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Body;
use axum::http::{Method, Request, Response, header};
use browseros_server::app::{App, Launch, Parts, Shutdown};
use browseros_server::browser::{Browser, BrowserConfig, ScriptedBrowser, ScriptedReply};
use browseros_server::model::{Models, Script};
use browseros_server::store::{Store, StoreError};
use browseros_server::turn::{
    ConversationId, EventSink, Target, TurnEnd, TurnEvent, TurnInput, TurnRunner,
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// The extension's origin, which the trust rule accepts.
pub const EXTENSION_ORIGIN: &str = "chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk";

/// What a `TestServer` is built from. `TestOptions::default()` is the scripted model's default
/// answer, no browser engine, and turns run through the real `agent::Agent`.
#[derive(Default)]
pub struct TestOptions {
    pub script: Script,
    /// `Some` uses the browser double with these replies; `None` is the "no engine" browser.
    pub browser_replies: Option<Vec<ScriptedReply>>,
    /// A runner in place of `agent::Agent`, for tests of `conversations`.
    pub runner: Option<Arc<dyn TurnRunner>>,
    pub trusted_origins: Vec<String>,
}

/// A server in this process, over its own temporary state folder.
pub struct TestServer {
    pub app: App,
    pub router: axum::Router,
    /// The browser double's handle, when the options asked for one.
    pub browser: Option<ScriptedBrowser>,
    state: tempfile::TempDir,
}

impl TestServer {
    pub async fn start() -> Result<Self, StoreError> {
        Self::with(TestOptions::default()).await
    }

    pub async fn with(options: TestOptions) -> Result<Self, StoreError> {
        let state = tempfile::tempdir().map_err(|source| StoreError::Io {
            path: std::env::temp_dir(),
            source,
        })?;
        let store = Store::open(state.path()).await?;
        let (browser, handle) = match options.browser_replies {
            Some(replies) => {
                let (browser, handle) = Browser::scripted(replies);
                (browser, Some(handle))
            }
            None => (
                Browser::start(
                    BrowserConfig::Off {
                        reason: "the test harness has no browser",
                    },
                    0,
                    CancellationToken::new(),
                )
                .await,
                None,
            ),
        };
        let launch = launch(state.path(), options.trusted_origins);
        let models = Models::scripted(store.clone(), launch.install_id.clone(), options.script);
        let app = App::assemble(Parts {
            launch,
            store,
            models,
            browser,
            runner: options.runner,
            shutdown: Shutdown::new(),
        });
        let router = browseros_server::http::router(app.clone());
        Ok(Self {
            app,
            router,
            browser: handle,
            state,
        })
    }

    /// The server's state folder: `db/browseros.sqlite` is inside it.
    pub fn state_dir(&self) -> &Path {
        self.state.path()
    }

    /// A request as the extension sends it: loopback `Host` and the extension's `Origin`.
    pub fn request(&self, method: Method, path: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1:9200")
            .header(header::ORIGIN, EXTENSION_ORIGIN)
    }

    pub fn get(&self, path: &str) -> Request<Body> {
        self.request(Method::GET, path)
            .body(Body::empty())
            .unwrap_or_default()
    }

    /// A JSON request from the extension.
    pub fn json(&self, method: Method, path: &str, body: &serde_json::Value) -> Request<Body> {
        self.request(method, path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap_or_default()
    }

    pub async fn send(&self, request: Request<Body>) -> Response<Body> {
        match self.router.clone().oneshot(request).await {
            Ok(response) => response,
            Err(infallible) => match infallible {},
        }
    }
}

/// The body of a response as JSON (`Null` when it is not JSON).
pub async fn body_json(response: Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// The body of a response as text.
pub async fn body_text(response: Response<Body>) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn launch(state_dir: &Path, trusted_origins: Vec<String>) -> Launch {
    Launch {
        server_port: 9200,
        cdp_port: 9100,
        resources: state_dir.join("resources"),
        execution: state_dir.join("execution"),
        state_dir: state_dir.to_owned(),
        dev_mode: true,
        browseros_version: Some("0.0.0-test".into()),
        install_id: Some("00000000-0000-4000-8000-000000000000".into()),
        trusted_origins,
    }
}

/// A `TurnRunner` that plays a fixed list of events, then ends as scripted. With
/// `wait_for_cancel`, it holds after the events until the run's cancel token fires and returns
/// `Stopped`, like a backend in the middle of a long tool call. Every turn it ran is recorded.
pub struct ScriptedRunner {
    pub events: Vec<TurnEvent>,
    pub end: TurnEnd,
    pub wait_for_cancel: bool,
    turns: Mutex<Vec<(Target, TurnInput)>>,
    ended_sessions: Mutex<Vec<ConversationId>>,
}

impl ScriptedRunner {
    pub fn new(events: Vec<TurnEvent>, end: TurnEnd) -> Arc<Self> {
        Arc::new(Self {
            events,
            end,
            wait_for_cancel: false,
            turns: Mutex::new(Vec::new()),
            ended_sessions: Mutex::new(Vec::new()),
        })
    }

    pub fn until_cancelled(events: Vec<TurnEvent>) -> Arc<Self> {
        Arc::new(Self {
            events,
            end: TurnEnd::Stopped,
            wait_for_cancel: true,
            turns: Mutex::new(Vec::new()),
            ended_sessions: Mutex::new(Vec::new()),
        })
    }

    /// Every turn this runner was asked to run, in order.
    pub fn turns(&self) -> Vec<(Target, TurnInput)> {
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn ended_sessions(&self) -> Vec<ConversationId> {
        self.ended_sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl TurnRunner for ScriptedRunner {
    async fn run_turn(
        &self,
        target: &Target,
        input: &TurnInput,
        sink: &mut dyn EventSink,
        cancel: &CancellationToken,
    ) -> TurnEnd {
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((target.clone(), input.clone()));
        for event in &self.events {
            sink.emit(event.clone()).await;
        }
        if self.wait_for_cancel {
            cancel.cancelled().await;
            return TurnEnd::Stopped;
        }
        self.end.clone()
    }

    async fn end_session(&self, conversation: ConversationId) {
        self.ended_sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(conversation);
    }
}
