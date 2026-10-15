//! `/chat*` over HTTP: statuses, bodies, headers and SSE bytes of `apps/server/src/api/routes/chat.ts`,
//! driven through the real handlers with a `ScriptedEngine` and in-memory records.
//!
//! The zod 400 goldens in `fixtures/chat/zod-400.json` were captured from the TS stack itself
//! (Hono 4.13 + `@hono/zod-validator` over `ChatRequestSchema`/`ConversationIdParamSchema`),
//! so those comparisons are byte-for-byte against the TS server.

use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    routing::{delete, get, post},
};
use browseros_agent::llm::{HttpStack, LlmConfig, LlmConfigError};
use futures_util::StreamExt as _;
use http::{Request, Response, StatusCode};
use serde_json::{Value, json};
use server_rust::{
    activity::ServerActivity,
    chat::{
        BrowserOsEngine, BrowserOsEngineDeps, ChatDeps, ChatEngine, ChatRecords, ChatRejection,
        ChatService, ConversationRuns, ModelFactory, NoBrowser, ProviderRecord, RegistryLeases,
        ScriptedEngine, Turn, production_models,
    },
    db::{
        AcpAgentRow, AgentKind, ConversationSummary, Database, DbError, DbResult,
        ImportConversation, ProviderRow, SaveRawConversation, StoredConversation, TargetType,
    },
    http::{chat, security::RequestTrust},
    mcp::ToolLeases,
    oauth::TokenManager,
    paths::BrowserosPaths,
};
use std::{
    collections::HashMap,
    error::Error,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use tower::ServiceExt as _;
use ui_message_stream::{ChunkStream, FinishReason, UiChunk, sse};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;

// ───── fixtures ─────

#[derive(Default)]
struct MemoryRecords {
    providers: Mutex<HashMap<String, ProviderRecord>>,
    default_provider: Mutex<Option<ProviderRecord>>,
    agents: Mutex<HashMap<Uuid, AcpAgentRow>>,
    conversations: Mutex<HashMap<Uuid, StoredConversation>>,
    saved: Mutex<Vec<SaveRawConversation>>,
    /// `conversation` fails as a locked or corrupt database would.
    fail_reads: AtomicBool,
    fail_writes: AtomicBool,
    /// When set, `save_conversation` signals the first and waits for the second.
    save_gate: Mutex<Option<(Arc<Notify>, Arc<Notify>)>>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[async_trait::async_trait]
impl ChatRecords for MemoryRecords {
    async fn provider(&self, id: &str) -> DbResult<Option<ProviderRecord>> {
        Ok(locked(&self.providers).get(id).cloned())
    }
    async fn default_provider(&self) -> DbResult<Option<ProviderRecord>> {
        Ok(locked(&self.default_provider).clone())
    }
    async fn agent(&self, id: Uuid) -> DbResult<Option<AcpAgentRow>> {
        Ok(locked(&self.agents).get(&id).cloned())
    }
    async fn conversation(&self, id: Uuid) -> DbResult<Option<StoredConversation>> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(DbError::Query("database disk image is malformed".into()));
        }
        Ok(locked(&self.conversations).get(&id).cloned())
    }
    async fn save_conversation(&self, input: SaveRawConversation) -> DbResult<()> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(DbError::Query("database is locked".into()));
        }
        let gate = locked(&self.save_gate).clone();
        if let Some((entered, release)) = gate {
            entered.notify_one();
            release.notified().await;
        }
        locked(&self.saved).push(input);
        Ok(())
    }
}

fn provider_row(id: &str, provider_type: &str) -> ProviderRow {
    ProviderRow {
        id: id.to_owned(),
        kind: "llm".to_owned(),
        provider_type: provider_type.to_owned(),
        name: "Stored".to_owned(),
        model_id: Some("stored-model".to_owned()),
        is_default: false,
        base_url: None,
        headers: None,
        supports_images: false,
        context_window: Some(64_000),
        temperature: 0.2,
        api_key: Some("stored-key".to_owned()),
        access_key_id: None,
        secret_access_key: None,
        session_token: None,
        resource_name: None,
        region: None,
        reasoning_effort: None,
        reasoning_summary: None,
        working_directory: None,
        custom_config: None,
        created_at: 1,
        updated_at: 1,
    }
}

fn agent_row(id: Uuid, kind: AgentKind) -> AcpAgentRow {
    AcpAgentRow {
        id,
        kind,
        name: "Agent".to_owned(),
        model_id: None,
        reasoning_effort: None,
        working_directory: None,
        custom_config: None,
        created_at: 1,
        updated_at: 1,
    }
}

fn script() -> Vec<UiChunk> {
    vec![
        UiChunk::Start {
            message_metadata: None,
            message_id: Some("assistant-1".into()),
        },
        UiChunk::StartStep,
        UiChunk::TextStart {
            id: "t1".into(),
            provider_metadata: None,
        },
        UiChunk::TextDelta {
            id: "t1".into(),
            delta: "Hi".into(),
            provider_metadata: None,
        },
        UiChunk::TextEnd {
            id: "t1".into(),
            provider_metadata: None,
        },
        UiChunk::FinishStep,
        UiChunk::Finish {
            finish_reason: Some(FinishReason::Stop),
            message_metadata: None,
        },
    ]
}

struct Harness {
    chat: Arc<ChatService>,
    records: Arc<MemoryRecords>,
    scripted: Option<Arc<ScriptedEngine>>,
    acp: Arc<ScriptedEngine>,
    _dir: tempfile::TempDir,
}

async fn token_manager(dir: &Path) -> Result<Arc<TokenManager>, Box<dyn Error>> {
    let (db, _) = Database::open(&BrowserosPaths::new(dir.to_path_buf())).await?;
    Ok(TokenManager::new(
        db.oauth_tokens(),
        "browseros-test".to_owned(),
        reqwest::Client::new(),
    ))
}

async fn harness_with(
    browseros: Arc<dyn ChatEngine>,
    scripted: Option<Arc<ScriptedEngine>>,
    heartbeat: Duration,
) -> Result<Harness, Box<dyn Error>> {
    harness_with_acp(browseros, scripted, script(), heartbeat).await
}

async fn harness_with_acp(
    browseros: Arc<dyn ChatEngine>,
    scripted: Option<Arc<ScriptedEngine>>,
    acp_script: Vec<UiChunk>,
    heartbeat: Duration,
) -> Result<Harness, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let records = Arc::new(MemoryRecords::default());
    let acp = Arc::new(ScriptedEngine::new(acp_script));
    let chat = ChatService::new(ChatDeps {
        runs: ConversationRuns::new(Arc::new(ServerActivity::new()), heartbeat),
        records: records.clone(),
        browseros,
        acp: acp.clone(),
        oauth: token_manager(dir.path()).await?,
        tabs: None,
        use_mock_llm: false,
        telemetry: server_rust::chat::ChatTelemetry::default(),
    });
    Ok(Harness {
        chat,
        records,
        scripted,
        acp,
        _dir: dir,
    })
}

async fn harness() -> Result<Harness, Box<dyn Error>> {
    let engine = Arc::new(ScriptedEngine::new(script()));
    harness_with(engine.clone(), Some(engine), Duration::from_secs(5)).await
}

impl Harness {
    /// The `/chat*` routes exactly as `http::ROUTES` registers them, with a fixed trust verdict
    /// standing in for the security middleware.
    fn router(&self, trusted_app: bool) -> Router {
        Router::new()
            .route("/chat", post(chat::post_chat))
            .route("/chat/panels", get(chat::panels))
            .route("/chat/panels/{tabId}", delete(chat::delete_panel))
            .route("/chat/{conversationId}/state", get(chat::state))
            .route("/chat/{conversationId}/stream", get(chat::resume_stream))
            .route("/chat/{conversationId}/stop", post(chat::stop))
            .route(
                "/chat/{conversationId}",
                delete(chat::delete_conversation_session),
            )
            .layer(Extension(RequestTrust { trusted_app }))
            .with_state(self.chat.clone())
    }

    async fn send(
        &self,
        trusted: bool,
        request: Request<Body>,
    ) -> Result<Response<Body>, Box<dyn Error>> {
        Ok(self.router(trusted).oneshot(request).await?)
    }

    async fn post_chat(
        &self,
        trusted: bool,
        body: &Value,
    ) -> Result<Response<Body>, Box<dyn Error>> {
        self.send(trusted, json_request("POST", "/chat", body)?)
            .await
    }

    fn scripted(&self) -> Result<&Arc<ScriptedEngine>, Box<dyn Error>> {
        self.scripted
            .as_ref()
            .ok_or_else(|| "no scripted engine".into())
    }
}

fn json_request(method: &str, uri: &str, body: &Value) -> Result<Request<Body>, Box<dyn Error>> {
    Ok(Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(body)?))?)
}

fn empty_request(method: &str, uri: &str) -> Result<Request<Body>, Box<dyn Error>> {
    Ok(Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())?)
}

async fn body_bytes(response: Response<Body>) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(to_bytes(response.into_body(), usize::MAX).await?.to_vec())
}

async fn body_json(response: Response<Body>) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&body_bytes(response).await?)?)
}

fn inline_body(conversation_id: Uuid) -> Value {
    json!({
        "conversationId": conversation_id,
        "message": "hello world",
        "provider": "openai",
        "model": "gpt-5",
        "apiKey": "sk-own",
        "browserContext": { "activeTab": { "id": 3, "url": "https://example.com", "title": "Example" } },
    })
}

fn header<'a>(response: &'a Response<Body>, name: &str) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

// ───── POST /chat ─────

#[tokio::test]
async fn post_chat_answers_with_the_ui_message_stream() -> TestResult {
    let h = harness().await?;
    let conversation = Uuid::new_v4();
    let response = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(response.status(), StatusCode::OK);
    for (name, value) in sse::UI_STREAM_HEADERS {
        assert_eq!(header(&response, name), Some(value), "header {name}");
    }
    let body = body_bytes(response).await?;
    let expected: Vec<u8> = script()
        .iter()
        .flat_map(|chunk| sse::frame(chunk).to_vec())
        .chain(sse::DONE_FRAME.iter().copied())
        .collect();
    assert_eq!(String::from_utf8(body)?, String::from_utf8(expected)?);

    let turns = h.scripted()?.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].conversation_id, conversation);
    let user = &turns[0].history[0];
    assert_eq!(
        serde_json::to_value(user)?["parts"],
        json!([{ "type": "text", "text": "hello world" }])
    );
    Ok(())
}

#[tokio::test]
async fn rejects_a_second_turn_with_409_while_the_first_is_running() -> TestResult {
    let gate = Arc::new(Notify::new());
    let engine = Arc::new(GatedEngine { gate: gate.clone() });
    let h = harness_with(engine, None, Duration::from_secs(5)).await?;
    let conversation = Uuid::new_v4();
    let first = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(first.status(), StatusCode::OK);

    let second = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(second).await?,
        json!({ "error": "An agent turn is already running" })
    );

    gate.notify_one();
    let body = String::from_utf8(body_bytes(first).await?)?;
    assert!(body.ends_with("data: [DONE]\n\n"));
    let third = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(third.status(), StatusCode::OK);
    gate.notify_one();
    Ok(())
}

/// Streams `start`, then waits for the test to open the gate (or for a stop) before finishing.
struct GatedEngine {
    gate: Arc<Notify>,
}

#[async_trait::async_trait]
impl ChatEngine for GatedEngine {
    async fn start_turn(&self, turn: Turn) -> Result<ChunkStream, ChatRejection> {
        let gate = self.gate.clone();
        let cancel = turn.run.cancel().clone();
        Ok(Box::pin(futures_util::stream::unfold(0, move |step| {
            let gate = gate.clone();
            let cancel = cancel.clone();
            async move {
                match step {
                    0 => Some((
                        UiChunk::Start {
                            message_metadata: None,
                            message_id: Some("gated".into()),
                        },
                        1,
                    )),
                    1 => {
                        tokio::select! {
                            () = gate.notified() => Some((UiChunk::Finish { finish_reason: Some(FinishReason::Stop), message_metadata: None }, 2)),
                            () = cancel.cancelled() => Some((server_rust::chat::stopped_chunk(), 2)),
                        }
                    }
                    _ => None,
                }
            }
        })))
    }

    async fn forget(&self, _conversation_id: Uuid) -> bool {
        false
    }
}

#[tokio::test]
async fn hydration_failures_are_400_with_the_ts_texts() -> TestResult {
    let h = harness().await?;
    let acp_id = Uuid::new_v4();
    locked(&h.records.providers).insert(
        acp_id.to_string(),
        ProviderRecord::CodingAgent {
            id: acp_id.to_string(),
        },
    );
    let cases = [
        (
            json!({ "conversationId": Uuid::new_v4(), "message": "hi" }),
            "No provider given and none is selected",
        ),
        (
            json!({ "conversationId": Uuid::new_v4(), "providerId": "p-missing" }),
            "Unknown provider p-missing",
        ),
        (
            json!({ "conversationId": Uuid::new_v4(), "target": { "type": "browseros", "providerId": acp_id } }),
            &format!(
                "Provider {acp_id} is a coding agent and cannot serve a browseros chat request"
            ),
        ),
    ];
    for (body, text) in cases {
        let response = h.post_chat(true, &body).await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body_json(response).await?, json!({ "error": text }));
    }
    assert!(h.scripted()?.turns().is_empty());
    Ok(())
}

#[tokio::test]
async fn server_supplied_credentials_require_the_trusted_app() -> TestResult {
    let h = harness().await?;
    locked(&h.records.providers).insert(
        "stored".into(),
        ProviderRecord::Llm(Box::new(provider_row("stored", "anthropic"))),
    );
    *locked(&h.records.default_provider) = Some(ProviderRecord::Llm(Box::new(provider_row(
        "stored",
        "anthropic",
    ))));
    let forbidden = [
        json!({ "conversationId": Uuid::new_v4(), "providerId": "stored" }),
        json!({ "conversationId": Uuid::new_v4(), "message": "uses the default" }),
        json!({ "conversationId": Uuid::new_v4(), "provider": "chatgpt-pro" }),
        json!({ "conversationId": Uuid::new_v4(), "target": { "type": "claude", "agentId": Uuid::new_v4() } }),
    ];
    for body in forbidden {
        let response = h.post_chat(false, &body).await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body_json(response).await?, json!({ "error": "Forbidden" }));
    }
    // Bringing one's own key is allowed without the app check.
    let own = h.post_chat(false, &inline_body(Uuid::new_v4())).await?;
    assert_eq!(own.status(), StatusCode::OK);
    // The trusted app may use the stored row, which overrides the inline fields.
    let stored = h
        .post_chat(
            true,
            &json!({ "conversationId": Uuid::new_v4(), "providerId": "stored", "provider": "openai", "model": "ignored" }),
        )
        .await?;
    assert_eq!(stored.status(), StatusCode::OK);
    body_bytes(stored).await?;
    Ok(())
}

#[tokio::test]
async fn oauth_providers_without_a_login_fail_with_the_ts_envelope() -> TestResult {
    let h = harness().await?;
    let response = h
        .post_chat(
            true,
            &json!({ "conversationId": Uuid::new_v4(), "provider": "chatgpt-pro" }),
        )
        .await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_json(response).await?;
    assert_eq!(
        body["error"]["message"],
        json!("Not authenticated with ChatGPT. Please login first.")
    );
    assert_eq!(body["error"]["name"], json!("InternalServerError"));
    Ok(())
}

#[tokio::test]
async fn acp_targets_resolve_the_agent_after_the_run_gate() -> TestResult {
    let h = harness().await?;
    let claude = Uuid::new_v4();
    locked(&h.records.agents).insert(claude, agent_row(claude, AgentKind::Claude));

    let unknown = h
        .post_chat(true, &json!({ "conversationId": Uuid::new_v4(), "target": { "type": "claude", "agentId": Uuid::new_v4() } }))
        .await?;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(unknown).await?,
        json!({ "error": "Unknown agent" })
    );

    let mismatch = h
        .post_chat(true, &json!({ "conversationId": Uuid::new_v4(), "target": { "type": "codex", "agentId": claude } }))
        .await?;
    assert_eq!(mismatch.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(mismatch).await?,
        json!({ "error": "Agent type mismatch" })
    );

    let conversation = Uuid::new_v4();
    let ok = h
        .post_chat(
            true,
            &json!({
                "conversationId": conversation,
                "message": "look",
                "target": { "type": "claude", "agentId": claude },
                "attachments": [{ "mediaType": "image/png", "data": "AAAA" }],
            }),
        )
        .await?;
    assert_eq!(ok.status(), StatusCode::OK);
    body_bytes(ok).await?;
    let turns = h.acp.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].acp_agent, Some(claude));
    assert_eq!(
        serde_json::to_value(&turns[0].history[0])?["parts"],
        json!([
            { "type": "text", "text": "look" },
            { "type": "file", "mediaType": "image/png", "url": "data:image/png;base64,AAAA" },
        ])
    );

    // DELETE of an ACP conversation needs the trusted app.
    let untrusted = h
        .send(
            false,
            empty_request("DELETE", &format!("/chat/{conversation}"))?,
        )
        .await?;
    assert_eq!(untrusted.status(), StatusCode::FORBIDDEN);
    let deleted = h
        .send(
            true,
            empty_request("DELETE", &format!("/chat/{conversation}"))?,
        )
        .await?;
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(h.acp.forgotten(), vec![conversation]);
    Ok(())
}

#[tokio::test]
async fn an_engine_rejection_maps_to_its_status_and_restores_published_history() -> TestResult {
    let h = harness().await?;
    let conversation = Uuid::new_v4();
    h.scripted()?.reject_next(ChatRejection::Busy);
    let busy = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(busy.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(busy).await?,
        json!({ "error": "An agent turn is already running" })
    );
    let state = body_json(
        h.send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/state"))?,
        )
        .await?,
    )
    .await?;
    assert_eq!(state["status"], json!("failed"));
    assert_eq!(state["messages"], json!([]));
    Ok(())
}

// ───── request validation ─────

#[tokio::test]
async fn validation_failures_match_the_ts_zod_bodies_byte_for_byte() -> TestResult {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/chat/zod-400.json"))?;
    let h = harness().await?;
    for (name, case) in fixtures.as_object().ok_or("fixtures are not an object")? {
        let request = match name.as_str() {
            "streamParam" => empty_request("GET", "/chat/not-a-uuid/stream")?,
            _ => {
                let builder = Request::builder().method("POST").uri("/chat").header(
                    "content-type",
                    case["contentType"].as_str().unwrap_or("application/json"),
                );
                builder.body(Body::from(serde_json::to_vec(&case["request"])?))?
            }
        };
        let response = h.send(true, request).await?;
        assert_eq!(
            response.status().as_u16(),
            case["status"].as_u64().unwrap_or_default() as u16,
            "{name}"
        );
        let body = String::from_utf8(body_bytes(response).await?)?;
        assert_eq!(body, case["body"].as_str().unwrap_or_default(), "{name}");
    }
    Ok(())
}

#[tokio::test]
async fn malformed_json_is_the_500_chat_envelope() -> TestResult {
    let h = harness().await?;
    let request = Request::builder()
        .method("POST")
        .uri("/chat")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))?;
    let response = h.send(true, request).await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_json(response).await?;
    assert_eq!(body["error"]["code"], json!("unknown"));
    assert_eq!(
        body["error"]["message"],
        json!("Malformed JSON in request body")
    );
    assert_eq!(body["error"]["name"], json!("InternalServerError"));
    assert_eq!(body["error"]["statusCode"], json!(500));
    Ok(())
}

// ───── state / stream / stop ─────

#[tokio::test]
async fn state_reports_the_prepared_run_with_no_store() -> TestResult {
    let h = harness().await?;
    let invalid = h
        .send(false, empty_request("GET", "/chat/nope/state")?)
        .await?;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(invalid).await?,
        json!({ "error": "Invalid conversation id" })
    );

    let conversation = Uuid::new_v4();
    let missing = h
        .send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/state"))?,
        )
        .await?;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(missing).await?,
        json!({ "error": "Conversation not found" })
    );

    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    let state = h
        .send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/state"))?,
        )
        .await?;
    assert_eq!(state.status(), StatusCode::OK);
    assert_eq!(header(&state, "cache-control"), Some("no-store"));
    let body = body_json(state).await?;
    let keys: Vec<&str> = body
        .as_object()
        .ok_or("not an object")?
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "conversationId",
            "runId",
            "status",
            "messages",
            "chunkCount"
        ]
    );
    assert_eq!(body["conversationId"], json!(conversation.to_string()));
    assert_eq!(body["status"], json!("completed"));
    assert_eq!(body["chunkCount"], json!(script().len()));
    assert_eq!(body["messages"][0]["role"], json!("user"));
    Ok(())
}

#[tokio::test]
async fn resume_is_204_without_a_run_409_for_a_stale_run_and_replays_a_finished_run() -> TestResult
{
    let h = harness().await?;
    let conversation = Uuid::new_v4();
    let none = h
        .send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/stream"))?,
        )
        .await?;
    assert_eq!(none.status(), StatusCode::NO_CONTENT);
    assert!(body_bytes(none).await?.is_empty());

    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    let stale = h
        .send(
            false,
            empty_request(
                "GET",
                &format!("/chat/{conversation}/stream?runId={}", Uuid::new_v4()),
            )?,
        )
        .await?;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(stale).await?,
        json!({ "error": "Conversation run changed" })
    );

    let state = body_json(
        h.send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/state"))?,
        )
        .await?,
    )
    .await?;
    let run_id = state["runId"].as_str().ok_or("no run id")?;
    let replay = h
        .send(
            false,
            empty_request(
                "GET",
                &format!("/chat/{conversation}/stream?runId={run_id}"),
            )?,
        )
        .await?;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(header(&replay, "x-vercel-ai-ui-message-stream"), Some("v1"));
    let chunks = sse::parse(&body_bytes(replay).await?)?;
    assert_eq!(chunks, script());
    Ok(())
}

#[tokio::test]
async fn a_client_disconnect_never_stops_the_run_but_stop_does() -> TestResult {
    let gate = Arc::new(Notify::new());
    let h = harness_with(
        Arc::new(GatedEngine { gate: gate.clone() }),
        None,
        Duration::from_secs(5),
    )
    .await?;
    let conversation = Uuid::new_v4();

    // Read the first frame, then hang up.
    let response = h.post_chat(false, &inline_body(conversation)).await?;
    let mut body = response.into_body().into_data_stream();
    let first = body.next().await.ok_or("no first frame")??;
    assert!(String::from_utf8(first.to_vec())?.starts_with("data: {\"type\":\"start\""));
    drop(body);

    tokio::time::sleep(Duration::from_millis(30)).await;
    let state = body_json(
        h.send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/state"))?,
        )
        .await?,
    )
    .await?;
    assert_eq!(state["status"], json!("running"));

    let stopped = h
        .send(
            false,
            empty_request("POST", &format!("/chat/{conversation}/stop"))?,
        )
        .await?;
    assert_eq!(body_json(stopped).await?, json!({ "stopped": true }));
    let state = body_json(
        h.send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/state"))?,
        )
        .await?,
    )
    .await?;
    assert_eq!(state["status"], json!("aborted"));
    let replay = h
        .send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/stream"))?,
        )
        .await?;
    let chunks = sse::parse(&body_bytes(replay).await?)?;
    // TS parity: the stopped run's log ends at the cancel, without the engine's `abort` chunk.
    assert!(!chunks.contains(&server_rust::chat::stopped_chunk()));
    assert!(!chunks.is_empty());

    let again = h
        .send(
            false,
            empty_request("POST", &format!("/chat/{conversation}/stop"))?,
        )
        .await?;
    assert_eq!(body_json(again).await?, json!({ "stopped": false }));
    let invalid = h
        .send(false, empty_request("POST", "/chat/nope/stop")?)
        .await?;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn a_resumed_stream_replays_then_tails_the_live_run() -> TestResult {
    let gate = Arc::new(Notify::new());
    let h = harness_with(
        Arc::new(GatedEngine { gate: gate.clone() }),
        None,
        Duration::from_secs(5),
    )
    .await?;
    let conversation = Uuid::new_v4();
    let original = h.post_chat(false, &inline_body(conversation)).await?;
    let resumed = h
        .send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/stream"))?,
        )
        .await?;
    assert_eq!(resumed.status(), StatusCode::OK);
    let tail = tokio::spawn(async move {
        to_bytes(resumed.into_body(), usize::MAX)
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| error.to_string())
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    gate.notify_one();
    let replayed = sse::parse(&tail.await??)?;
    let live = sse::parse(&body_bytes(original).await?)?;
    assert_eq!(replayed, live);
    assert_eq!(replayed.len(), 2);
    Ok(())
}

#[tokio::test]
async fn an_error_chunk_marks_the_run_failed() -> TestResult {
    let engine = Arc::new(ScriptedEngine::new(vec![
        UiChunk::Start {
            message_metadata: None,
            message_id: None,
        },
        UiChunk::Error {
            error_text: "{\"error\":{}}".into(),
        },
    ]));
    let h = harness_with(engine.clone(), Some(engine), Duration::from_secs(5)).await?;
    let conversation = Uuid::new_v4();
    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    let state = body_json(
        h.send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/state"))?,
        )
        .await?,
    )
    .await?;
    assert_eq!(state["status"], json!("failed"));
    // No assistant reply: nothing is persisted.
    assert!(locked(&h.records.saved).is_empty());
    Ok(())
}

// ───── history and persistence ─────

#[tokio::test]
async fn a_completed_turn_is_persisted_before_the_stream_ends() -> TestResult {
    let h = harness().await?;
    let conversation = Uuid::new_v4();
    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    let saved = locked(&h.records.saved).clone();
    assert_eq!(saved.len(), 1, "persisted after [DONE]");
    assert_eq!(saved[0].id, conversation);
    assert_eq!(saved[0].target_type, TargetType::BrowserOs);
    assert_eq!(saved[0].origin.as_deref(), Some("sidepanel"));
    let roles: Vec<&Value> = saved[0]
        .messages
        .iter()
        .map(|message| &message["role"])
        .collect();
    assert_eq!(roles, [&json!("user"), &json!("assistant")]);
    assert_eq!(saved[0].messages[1]["id"], json!("assistant-1"));
    Ok(())
}

#[tokio::test]
async fn history_hydrates_from_the_owning_target_or_previous_conversation() -> TestResult {
    let h = harness().await?;
    let stored = |target_type, agent_id: Option<String>, text: &str| StoredConversation {
        summary: ConversationSummary {
            id: Uuid::nil(),
            last_user_message: None,
            origin: None,
            target_type,
            agent_id,
            last_messaged_at: 1,
            created_at: 1,
            updated_at: 1,
        },
        messages: vec![
            json!({ "id": "old", "role": "user", "parts": [{ "type": "text", "text": text }] }),
        ],
    };

    // A browseros-owned row hydrates a persisted browseros turn.
    let owned = Uuid::new_v4();
    locked(&h.records.conversations).insert(owned, stored(TargetType::BrowserOs, None, "earlier"));
    body_bytes(h.post_chat(false, &inline_body(owned)).await?).await?;

    // An ACP-owned row never bleeds into the built-in agent.
    let foreign = Uuid::new_v4();
    locked(&h.records.conversations).insert(
        foreign,
        stored(
            TargetType::Claude,
            Some(Uuid::new_v4().to_string()),
            "acp history",
        ),
    );
    body_bytes(h.post_chat(false, &inline_body(foreign)).await?).await?;

    // Unpersisted: the store is ignored and the client's projection seeds the history.
    let ephemeral = Uuid::new_v4();
    locked(&h.records.conversations)
        .insert(ephemeral, stored(TargetType::BrowserOs, None, "stored"));
    let mut body = inline_body(ephemeral);
    body["persist"] = json!(false);
    body["previousConversation"] = json!([{ "role": "user", "content": "from client" }, { "role": "assistant", "content": "  " }]);
    body_bytes(h.post_chat(false, &body).await?).await?;

    let texts = |turn: usize| -> Result<Vec<Value>, Box<dyn Error>> {
        let turns = h.scripted()?.turns();
        Ok(turns[turn]
            .history
            .iter()
            .map(|message| {
                serde_json::to_value(message).map(|value| value["parts"][0]["text"].clone())
            })
            .collect::<Result<_, _>>()?)
    };
    assert_eq!(texts(0)?, vec![json!("earlier"), json!("hello world")]);
    assert_eq!(texts(1)?, vec![json!("hello world")]);
    assert_eq!(texts(2)?, vec![json!("from client"), json!("hello world")]);
    // The unpersisted conversation was never written.
    assert!(
        locked(&h.records.saved)
            .iter()
            .all(|saved| saved.id != ephemeral)
    );
    Ok(())
}

#[tokio::test]
async fn the_next_turn_continues_the_in_memory_history() -> TestResult {
    let h = harness().await?;
    let conversation = Uuid::new_v4();
    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    let turns = h.scripted()?.turns();
    let roles: Vec<Value> = turns[1]
        .history
        .iter()
        .map(|message| serde_json::to_value(message).map(|value| value["role"].clone()))
        .collect::<Result<_, _>>()?;
    assert_eq!(
        roles,
        vec![json!("user"), json!("assistant"), json!("user")]
    );
    Ok(())
}

// ───── DELETE /chat/{id} ─────

#[tokio::test]
async fn delete_reports_found_and_not_found_sessions() -> TestResult {
    let h = harness().await?;
    let conversation = Uuid::new_v4();
    let missing = h
        .send(
            false,
            empty_request("DELETE", &format!("/chat/{conversation}"))?,
        )
        .await?;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(missing).await?,
        json!({ "success": false, "message": format!("Session {conversation} not found") })
    );

    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    // Another live session: `sessionCount` is what the built-in engine still holds.
    body_bytes(h.post_chat(false, &inline_body(Uuid::new_v4())).await?).await?;
    let deleted = h
        .send(
            false,
            empty_request("DELETE", &format!("/chat/{conversation}"))?,
        )
        .await?;
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(
        String::from_utf8(body_bytes(deleted).await?)?,
        format!(
            r#"{{"success":true,"message":"Session {conversation} deleted","sessionCount":1}}"#
        )
    );
    // The built-in engine is asked to forget on every DELETE (TS `sessionStore.delete`), found or not.
    assert_eq!(h.scripted()?.forgotten(), vec![conversation, conversation]);
    let gone = h
        .send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/stream"))?,
        )
        .await?;
    assert_eq!(gone.status(), StatusCode::NO_CONTENT);
    Ok(())
}

// ───── panels ─────

#[tokio::test]
async fn panels_stream_assignments_with_a_heartbeat_and_no_done() -> TestResult {
    let engine = Arc::new(ScriptedEngine::new(script()));
    let h = harness_with(engine.clone(), Some(engine), Duration::from_millis(30)).await?;
    let untrusted = h.send(false, empty_request("GET", "/chat/panels")?).await?;
    assert_eq!(untrusted.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(untrusted).await?, json!({ "error": "Forbidden" }));

    let response = h.send(true, empty_request("GET", "/chat/panels")?).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), Some("text/event-stream"));
    assert_eq!(
        header(&response, "cache-control"),
        Some("no-cache, no-transform")
    );
    assert_eq!(header(&response, "connection"), Some("keep-alive"));
    assert_eq!(header(&response, "x-vercel-ai-ui-message-stream"), None);
    let mut frames = response.into_body().into_data_stream();
    let mut next_frame = async || -> Result<String, Box<dyn Error>> {
        let frame = tokio::time::timeout(Duration::from_secs(5), frames.next())
            .await?
            .ok_or("panel stream ended")??;
        Ok(String::from_utf8(frame.to_vec())?)
    };
    assert_eq!(next_frame().await?, "data: {\"assignments\":[]}\n\n");

    let conversation = Uuid::new_v4();
    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    let mut saw_tab = false;
    for _ in 0..10 {
        let frame = next_frame().await?;
        assert!(frame.starts_with("data: {\"assignments\":[") && frame.ends_with("]}\n\n"));
        assert!(!frame.contains("[DONE]"));
        if frame.contains("\"tabId\":3") && frame.contains("\"status\":\"completed\"") {
            assert!(frame.contains(&format!("\"conversationId\":\"{conversation}\"")));
            saw_tab = true;
            break;
        }
    }
    assert!(saw_tab, "the active tab's panel was never assigned");
    // Heartbeats keep repeating the full table.
    let heartbeat = next_frame().await?;
    assert!(heartbeat.contains("\"tabId\":3"));
    Ok(())
}

#[tokio::test]
async fn deleting_a_panel_validates_the_tab_id_like_js_number() -> TestResult {
    let h = harness().await?;
    let conversation = Uuid::new_v4();
    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    for bad in ["abc", "1.5", "-1", "Infinity"] {
        let response = h
            .send(
                true,
                empty_request("DELETE", &format!("/chat/panels/{bad}"))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(
            body_json(response).await?,
            json!({ "error": "Invalid tab id" })
        );
    }
    let untrusted = h
        .send(false, empty_request("DELETE", "/chat/panels/3")?)
        .await?;
    assert_eq!(untrusted.status(), StatusCode::FORBIDDEN);
    let removed = h
        .send(true, empty_request("DELETE", "/chat/panels/0x3")?)
        .await?;
    assert_eq!(removed.status(), StatusCode::OK);
    assert_eq!(
        String::from_utf8(body_bytes(removed).await?)?,
        r#"{"success":true}"#
    );
    let panels = h.send(true, empty_request("GET", "/chat/panels")?).await?;
    let mut frames = panels.into_body().into_data_stream();
    let frame = frames.next().await.ok_or("no frame")??;
    assert_eq!(
        String::from_utf8(frame.to_vec())?,
        "data: {\"assignments\":[]}\n\n"
    );
    Ok(())
}

// ───── the production records adapter over the real SQLite stores ─────

#[tokio::test]
async fn db_records_classify_acp_rows_and_persist_through_the_conversation_store() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (db, _) = Database::open(&BrowserosPaths::new(dir.path().to_path_buf())).await?;
    let agent = db
        .agents()
        .create(server_rust::db::CreateAgent {
            kind: AgentKind::Codex,
            name: "Codex".into(),
            model_id: None,
            reasoning_effort: None,
            working_directory: None,
            custom_config: None,
        })
        .await?;
    let engine = Arc::new(ScriptedEngine::new(script()));
    let chat = ChatService::new(ChatDeps {
        runs: ConversationRuns::new(Arc::new(ServerActivity::new()), Duration::from_secs(5)),
        records: Arc::new(server_rust::chat::DbChatRecords::new(&db)),
        browseros: engine.clone(),
        acp: Arc::new(ScriptedEngine::new(script())),
        // Production shares one Database across records and OAuth; a second Rust owner of
        // the same directory is now correctly rejected by the DB instance lock.
        oauth: TokenManager::new(
            db.oauth_tokens(),
            "browseros-test".to_owned(),
            reqwest::Client::new(),
        ),
        tabs: None,
        use_mock_llm: false,
        telemetry: server_rust::chat::ChatTelemetry::default(),
    });
    let router = Router::new()
        .route("/chat", post(chat::post_chat))
        .layer(Extension(RequestTrust { trusted_app: true }))
        .with_state(chat);

    // An ACP agent row named as a browseros provider is the "coding agent" 400.
    let response = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/chat",
            &json!({ "conversationId": Uuid::new_v4(), "providerId": agent.id }),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(response).await?,
        json!({ "error": format!("Provider {} is a coding agent and cannot serve a browseros chat request", agent.id) })
    );

    // A completed turn is in SQLite by the time the stream ends.
    let conversation = Uuid::new_v4();
    let response = router
        .oneshot(json_request("POST", "/chat", &inline_body(conversation))?)
        .await?;
    body_bytes(response).await?;
    let stored = db
        .conversations()
        .get(conversation)
        .await?
        .ok_or("not persisted")?;
    assert_eq!(stored.summary.target_type, TargetType::BrowserOs);
    assert_eq!(
        stored.summary.last_user_message.as_deref(),
        Some("hello world")
    );
    let messages = stored.ui_messages()?;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].id, "assistant-1");
    Ok(())
}

// ───── review regressions (P8-fix) ─────

async fn state_json(h: &Harness, conversation: Uuid) -> Result<Value, Box<dyn Error>> {
    body_json(
        h.send(
            false,
            empty_request("GET", &format!("/chat/{conversation}/state"))?,
        )
        .await?,
    )
    .await
}

/// Streams `start`, then waits for stop (the registry drops the stream at the cancel). `forget`
/// is a slow cleanup (300 ms), or with a gate signals and then waits on it, so a test can hold a
/// DELETE mid-cleanup.
#[derive(Default)]
struct LingeringEngine {
    forget_gate: Option<(Arc<Notify>, Arc<Notify>)>,
    forgotten: Mutex<Vec<Uuid>>,
}

#[async_trait::async_trait]
impl ChatEngine for LingeringEngine {
    async fn start_turn(&self, turn: Turn) -> Result<ChunkStream, ChatRejection> {
        let cancel = turn.run.cancel().clone();
        Ok(Box::pin(futures_util::stream::unfold(0, move |step| {
            let cancel = cancel.clone();
            async move {
                match step {
                    0 => Some((
                        UiChunk::Start {
                            message_metadata: None,
                            message_id: Some("lingering".into()),
                        },
                        1,
                    )),
                    1 => {
                        cancel.cancelled().await;
                        Some((server_rust::chat::stopped_chunk(), 2))
                    }
                    _ => None,
                }
            }
        })))
    }

    async fn forget(&self, conversation_id: Uuid) -> bool {
        match &self.forget_gate {
            Some((entered, release)) => {
                entered.notify_one();
                release.notified().await;
            }
            None => tokio::time::sleep(Duration::from_millis(300)).await,
        }
        locked(&self.forgotten).push(conversation_id);
        true
    }
}

/// axum drops a handler whose client hung up. A DELETE dropped while the engine still cleans up
/// used to leave the record `deleting` forever (409 until restart) with the session never
/// forgotten.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delete_cut_short_by_its_client_still_frees_the_conversation() -> TestResult {
    let engine = Arc::new(LingeringEngine::default());
    let h = harness_with(engine.clone(), None, Duration::from_secs(5)).await?;
    let conversation = Uuid::new_v4();
    let first = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(first.status(), StatusCode::OK);
    drop(first);

    let delete = empty_request("DELETE", &format!("/chat/{conversation}"))?;
    let cut = tokio::time::timeout(Duration::from_millis(50), h.send(false, delete)).await;
    assert!(
        cut.is_err(),
        "the delete should still be waiting on the engine's cleanup"
    );

    let mut freed = false;
    for _ in 0..250 {
        if h.chat.state(conversation).await.is_none() {
            freed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(freed, "the run record was never removed");
    assert_eq!(*locked(&engine.forgotten), vec![conversation]);
    let again = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(again.status(), StatusCode::OK);
    Ok(())
}

/// The record goes last: a turn posted while DELETE is still releasing the engine session is
/// 409, not a fresh turn on a session about to be forgotten.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_post_during_delete_cleanup_is_409() -> TestResult {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let engine = Arc::new(LingeringEngine {
        forget_gate: Some((entered.clone(), release.clone())),
        ..LingeringEngine::default()
    });
    let h = harness_with(engine.clone(), None, Duration::from_secs(5)).await?;
    let conversation = Uuid::new_v4();
    drop(h.post_chat(false, &inline_body(conversation)).await?);

    let deleting = tokio::spawn({
        let router = h.router(false);
        let request = empty_request("DELETE", &format!("/chat/{conversation}"))?;
        async move { router.oneshot(request).await }
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified()).await?;
    let racing = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(racing.status(), StatusCode::CONFLICT);

    release.notify_one();
    let deleted = deleting.await??;
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(*locked(&engine.forgotten), vec![conversation]);
    let next = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(next.status(), StatusCode::OK);
    Ok(())
}

/// A stored history that cannot be read fails the turn (TS: 500) instead of seeding a fresh one
/// whose persist would overwrite the row.
#[tokio::test]
async fn an_unreadable_stored_history_fails_the_turn_and_is_never_overwritten() -> TestResult {
    let h = harness().await?;
    h.records.fail_reads.store(true, Ordering::SeqCst);
    let conversation = Uuid::new_v4();
    let response = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_json(response).await?;
    assert_eq!(body["error"]["code"], json!("unknown"));
    assert_eq!(body["error"]["retryable"], json!(true));
    assert!(h.scripted()?.turns().is_empty(), "the engine never ran");
    assert!(locked(&h.records.saved).is_empty(), "nothing was written");
    assert_eq!(
        state_json(&h, conversation).await?["status"],
        json!("failed")
    );

    // ACP display copies are persisted unconditionally, so the same holds for them.
    let claude = Uuid::new_v4();
    locked(&h.records.agents).insert(claude, agent_row(claude, AgentKind::Claude));
    let acp = h
        .post_chat(
            true,
            &json!({
                "conversationId": Uuid::new_v4(),
                "message": "look",
                "target": { "type": "claude", "agentId": claude },
            }),
        )
        .await?;
    assert_eq!(acp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(h.acp.turns().is_empty());
    assert!(locked(&h.records.saved).is_empty());

    // An unpersisted turn never reads the row, so it still runs.
    let mut ephemeral = inline_body(Uuid::new_v4());
    ephemeral["persist"] = json!(false);
    let response = h.post_chat(false, &ephemeral).await?;
    assert_eq!(response.status(), StatusCode::OK);
    body_bytes(response).await?;
    Ok(())
}

/// Real stored shapes this build cannot decode: what the import path writes (no `id`), an AI SDK
/// v4 `tool-invocation` part, the v4 tool state `call`, and a `data` message.
fn legacy_messages() -> Vec<Value> {
    vec![
        json!({ "role": "user", "parts": [{ "type": "text", "text": "imported question" }] }),
        json!({ "id": "m-v4", "role": "assistant", "parts": [{ "type": "tool-invocation",
            "toolInvocation": { "state": "result", "toolCallId": "call-1", "toolName": "search",
                "args": { "q": "x" }, "result": { "hits": 1 } } }] }),
        json!({ "id": "m-call", "role": "assistant", "parts": [{ "type": "tool-search",
            "toolCallId": "call-2", "state": "call", "input": { "q": "y" } }] }),
        json!({ "id": "m-data", "role": "data", "parts": [{ "type": "text", "text": "metadata" }] }),
    ]
}

fn stored_row(id: Uuid, messages: Vec<Value>) -> StoredConversation {
    StoredConversation {
        summary: ConversationSummary {
            id,
            last_user_message: None,
            origin: None,
            target_type: TargetType::BrowserOs,
            agent_id: None,
            last_messaged_at: 1,
            created_at: 1,
            updated_at: 1,
        },
        messages,
    }
}

/// Stored history is never rewritten through the typed form: undecodable messages are left out of
/// what the agent sees, and every save writes the row back verbatim with the turn appended.
#[tokio::test]
async fn legacy_stored_messages_survive_every_save_verbatim() -> TestResult {
    for legacy in legacy_messages() {
        assert!(
            serde_json::from_value::<ui_message_stream::UiMessage>(legacy.clone()).is_err(),
            "fixture decodes, so it tests nothing: {legacy}"
        );
    }
    let h = harness().await?;
    let conversation = Uuid::new_v4();
    let legacy = legacy_messages();
    let stored = vec![
        json!({ "id": "u1", "role": "user", "parts": [{ "type": "text", "text": "earlier" }] }),
        legacy[0].clone(),
        json!({ "id": "a1", "role": "assistant", "parts": [{ "type": "text", "text": "reply" }] }),
        legacy[1].clone(),
        legacy[2].clone(),
        legacy[3].clone(),
    ];
    locked(&h.records.conversations).insert(conversation, stored_row(conversation, stored.clone()));

    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    let texts: Vec<Value> = h.scripted()?.turns()[0]
        .history
        .iter()
        .map(|message| serde_json::to_value(message).map(|value| value["parts"][0]["text"].clone()))
        .collect::<Result<_, _>>()?;
    assert_eq!(
        texts,
        vec![json!("earlier"), json!("reply"), json!("hello world")]
    );
    let first = locked(&h.records.saved)[0].messages.clone();
    assert_eq!(first.len(), stored.len() + 2);
    assert_eq!(
        first[..stored.len()],
        stored[..],
        "the stored prefix is verbatim"
    );
    assert_eq!(first[stored.len()]["role"], json!("user"));
    assert_eq!(first[stored.len() + 1]["role"], json!("assistant"));
    // Panels see the typed view.
    let state = state_json(&h, conversation).await?;
    assert_eq!(state["messages"].as_array().map(Vec::len), Some(4));

    // A turn served from the in-memory cache appends to the same verbatim list.
    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    let second = locked(&h.records.saved)[1].messages.clone();
    assert_eq!(second.len(), first.len() + 2);
    assert_eq!(second[..first.len()], first[..]);
    Ok(())
}

/// The same through the real SQLite store and the import path (`PUT /conversations/{id}` writes
/// id-less messages): the row keeps every legacy message after a chat turn.
#[tokio::test]
async fn imported_legacy_rows_keep_their_messages_through_a_chat_turn() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (db, _) = Database::open(&BrowserosPaths::new(dir.path().to_path_buf())).await?;
    let conversation = Uuid::new_v4();
    let legacy = legacy_messages();
    db.conversations()
        .import_if_absent(
            conversation,
            ImportConversation {
                messages: legacy.clone(),
                last_messaged_at: Some(10),
                target_type: TargetType::BrowserOs,
                origin: None,
                agent_id: None,
            },
        )
        .await?
        .ok_or("not imported")?;
    let chat = ChatService::new(ChatDeps {
        runs: ConversationRuns::new(Arc::new(ServerActivity::new()), Duration::from_secs(5)),
        records: Arc::new(server_rust::chat::DbChatRecords::new(&db)),
        browseros: Arc::new(ScriptedEngine::new(script())),
        acp: Arc::new(ScriptedEngine::new(script())),
        oauth: TokenManager::new(
            db.oauth_tokens(),
            "browseros-test".to_owned(),
            reqwest::Client::new(),
        ),
        tabs: None,
        use_mock_llm: false,
        telemetry: server_rust::chat::ChatTelemetry::default(),
    });
    let response = Router::new()
        .route("/chat", post(chat::post_chat))
        .layer(Extension(RequestTrust { trusted_app: true }))
        .with_state(chat)
        .oneshot(json_request("POST", "/chat", &inline_body(conversation))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    body_bytes(response).await?;
    let stored = db
        .conversations()
        .get(conversation)
        .await?
        .ok_or("row vanished")?;
    assert_eq!(stored.messages.len(), legacy.len() + 2);
    assert_eq!(stored.messages[..legacy.len()], legacy[..]);
    assert_eq!(
        stored.summary.last_user_message.as_deref(),
        Some("hello world")
    );
    Ok(())
}

fn engine_with(runs: &Arc<ConversationRuns>, models: ModelFactory) -> Arc<BrowserOsEngine> {
    Arc::new(BrowserOsEngine::new(BrowserOsEngineDeps {
        models,
        leases: Arc::new(RegistryLeases(ToolLeases::new(runs.clone()))),
        pages: Arc::new(NoBrowser),
        // Nothing listens here: the required loopback MCP client cannot connect.
        mcp_port: 1,
        klavis_status: Arc::new(|| None),
        connector_names: Vec::new(),
    }))
}

/// Built-in engine setup failures are classified by message, as TS classifies the plain `Error`
/// it throws: an MCP connect failure is a retryable `unknown` (it used to be "Provider not
/// configured", retryable: false), while a real config failure stays `provider_config`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn builtin_engine_setup_failures_are_classified_by_message() -> TestResult {
    let unconfigured: ModelFactory = Arc::new(|_: &LlmConfig| {
        Err(LlmConfigError(
            "Anthropic provider requires apiKey".to_owned(),
        ))
    });
    let cases = [
        (production_models(HttpStack::new(), true), "unknown", true),
        (unconfigured, "provider_config", false),
    ];
    for (models, code, retryable) in cases {
        let dir = tempfile::tempdir()?;
        let runs = ConversationRuns::new(Arc::new(ServerActivity::new()), Duration::from_secs(5));
        let chat = ChatService::new(ChatDeps {
            browseros: engine_with(&runs, models),
            runs,
            records: Arc::new(MemoryRecords::default()),
            acp: Arc::new(ScriptedEngine::new(script())),
            oauth: token_manager(dir.path()).await?,
            tabs: None,
            use_mock_llm: true,
            telemetry: server_rust::chat::ChatTelemetry::default(),
        });
        let response = Router::new()
            .route("/chat", post(chat::post_chat))
            .layer(Extension(RequestTrust { trusted_app: false }))
            .with_state(chat)
            .oneshot(json_request("POST", "/chat", &inline_body(Uuid::new_v4()))?)
            .await?;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_json(response).await?;
        assert_eq!(body["error"]["code"], json!(code), "{body}");
        assert_eq!(body["error"]["retryable"], json!(retryable), "{body}");
    }
    Ok(())
}

/// A later turn that fails before publishing history (unknown ACP agent → 404) leaves the
/// conversation hydrating as it was, not as an empty conversation.
#[tokio::test]
async fn a_later_turn_failing_early_keeps_the_conversation_visible() -> TestResult {
    let h = harness().await?;
    let conversation = Uuid::new_v4();
    body_bytes(h.post_chat(false, &inline_body(conversation)).await?).await?;
    let before = state_json(&h, conversation).await?;
    assert_eq!(before["messages"].as_array().map(Vec::len), Some(2));

    let failed = h
        .post_chat(
            true,
            &json!({
                "conversationId": conversation,
                "message": "again",
                "target": { "type": "claude", "agentId": Uuid::new_v4() },
            }),
        )
        .await?;
    assert_eq!(failed.status(), StatusCode::NOT_FOUND);
    let after = state_json(&h, conversation).await?;
    assert_eq!(after["status"], json!("failed"));
    assert_eq!(after["messages"], before["messages"]);
    Ok(())
}

/// While the finished turn is being persisted `/state` is still `running`, so it must not yet
/// carry the final assistant message (a resumed panel would replay the reply onto it twice).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn state_while_persisting_is_running_without_the_final_reply() -> TestResult {
    let h = harness().await?;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *locked(&h.records.save_gate) = Some((entered.clone(), release.clone()));
    let conversation = Uuid::new_v4();
    let response = h.post_chat(false, &inline_body(conversation)).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let reader = tokio::spawn(async move { to_bytes(response.into_body(), usize::MAX).await });
    tokio::time::timeout(Duration::from_secs(5), entered.notified()).await?;

    let persisting = state_json(&h, conversation).await?;
    assert_eq!(persisting["status"], json!("running"));
    assert_eq!(persisting["messages"].as_array().map(Vec::len), Some(1));
    assert_eq!(persisting["messages"][0]["role"], json!("user"));

    release.notify_one();
    reader.await??;
    let done = state_json(&h, conversation).await?;
    assert_eq!(done["status"], json!("completed"));
    assert_eq!(done["messages"].as_array().map(Vec::len), Some(2));
    Ok(())
}

/// TS persists the ACP display copy after every turn, aborted and failed ones included (only
/// built-in turns need an assistant reply to be durable).
#[tokio::test]
async fn acp_turns_without_a_reply_are_still_persisted() -> TestResult {
    let engine = Arc::new(ScriptedEngine::new(script()));
    let failing = vec![
        UiChunk::Start {
            message_metadata: None,
            message_id: None,
        },
        UiChunk::Error {
            error_text: "{\"error\":{}}".into(),
        },
    ];
    let h = harness_with_acp(
        engine.clone(),
        Some(engine),
        failing,
        Duration::from_secs(5),
    )
    .await?;
    let claude = Uuid::new_v4();
    locked(&h.records.agents).insert(claude, agent_row(claude, AgentKind::Claude));
    let conversation = Uuid::new_v4();
    let response = h
        .post_chat(
            true,
            &json!({
                "conversationId": conversation,
                "message": "look",
                "target": { "type": "claude", "agentId": claude },
            }),
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    body_bytes(response).await?;
    assert_eq!(
        state_json(&h, conversation).await?["status"],
        json!("failed")
    );
    let saved = locked(&h.records.saved).clone();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].target_type, TargetType::Claude);
    assert_eq!(saved[0].agent_id, Some(claude.to_string()));
    assert_eq!(saved[0].messages[0]["role"], json!("user"));
    Ok(())
}

/// Scheduled runs use a fresh conversation id each and are never DELETEd: once finished they
/// release the engine session, the history cache entry and (with no reader left) the replay log.
#[tokio::test]
async fn scheduled_runs_release_their_server_state() -> TestResult {
    let h = harness().await?;
    let conversation = Uuid::new_v4();
    let mut scheduled = inline_body(conversation);
    scheduled["isScheduledTask"] = json!(true);
    let response = h.post_chat(false, &scheduled).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let chunks = sse::parse(&body_bytes(response).await?)?;
    assert_eq!(chunks, script(), "the starter still reads the whole run");

    assert_eq!(h.scripted()?.forgotten(), vec![conversation]);
    let state = state_json(&h, conversation).await?;
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(state["chunkCount"], json!(0));
    assert_eq!(state["messages"].as_array().map(Vec::len), Some(2));

    // The cached history went too: the next (unpersisted) turn starts from the client's copy.
    let mut next = inline_body(conversation);
    next["persist"] = json!(false);
    body_bytes(h.post_chat(false, &next).await?).await?;
    let turns = h.scripted()?.turns();
    assert_eq!(turns[1].history.len(), 1);
    Ok(())
}

#[tokio::test]
async fn failed_history_save_reaches_the_stream_and_failed_turn_state() -> TestResult {
    for scheduled in [false, true] {
        let h = harness().await?;
        h.records.fail_writes.store(true, Ordering::SeqCst);
        let conversation = Uuid::new_v4();
        let mut input = inline_body(conversation);
        input["isScheduledTask"] = json!(scheduled);
        if scheduled {
            // Built-in scheduled runs intentionally do not persist. ACP display history does,
            // so use that real persistence policy to exercise failure plus scheduled cleanup.
            let agent = Uuid::new_v4();
            locked(&h.records.agents).insert(agent, agent_row(agent, AgentKind::Claude));
            input["target"] = json!({"type":"claude", "agentId":agent});
        }
        let response = h.post_chat(scheduled, &input).await?;
        assert_eq!(response.status(), StatusCode::OK); // Headers precede the durable-save phase.
        let chunks = sse::parse(&body_bytes(response).await?)?;
        assert!(chunks.iter().any(|chunk| matches!(chunk, UiChunk::Error { error_text } if error_text.contains("database is locked"))),
            "save failure did not reach the subscriber (scheduled={scheduled})");
        let error_text = chunks
            .iter()
            .find_map(|chunk| match chunk {
                UiChunk::Error { error_text } => Some(error_text),
                _ => None,
            })
            .ok_or("missing save error")?;
        let envelope: Value = serde_json::from_str(error_text)?;
        assert_eq!(envelope["error"]["code"], "unknown");
        assert_eq!(envelope["error"]["retryable"], true);
        if !scheduled {
            let replay = h
                .send(
                    false,
                    empty_request("GET", &format!("/chat/{conversation}/stream"))?,
                )
                .await?;
            assert_eq!(sse::parse(&body_bytes(replay).await?)?, chunks);
        }
        let state = state_json(&h, conversation).await?;
        assert_eq!(state["status"], "failed");
        assert_eq!(
            state["messages"].as_array().map(Vec::len),
            Some(2),
            "keep the unsaved reply visible"
        );
        assert!(locked(&h.records.saved).is_empty());
        if scheduled {
            assert_eq!(h.acp.forgotten(), vec![conversation]);
        }
    }
    Ok(())
}

#[tokio::test]
async fn a_real_sqlite_busy_save_fails_the_turn_without_creating_history() -> TestResult {
    use sea_orm::sqlx::{self, Connection};
    let dir = tempfile::tempdir()?;
    let paths = BrowserosPaths::new(dir.path().to_path_buf());
    let (db, _) = Database::open(&paths).await?;
    let engine = Arc::new(ScriptedEngine::new(script()));
    let chat = ChatService::new(ChatDeps {
        runs: ConversationRuns::new(Arc::new(ServerActivity::new()), Duration::from_secs(5)),
        records: Arc::new(server_rust::chat::DbChatRecords::new(&db)),
        browseros: engine.clone(),
        acp: engine,
        oauth: TokenManager::new(
            db.oauth_tokens(),
            "browseros-test".into(),
            reqwest::Client::new(),
        ),
        tabs: None,
        use_mock_llm: false,
        telemetry: server_rust::chat::ChatTelemetry::default(),
    });
    let mut ts = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(&paths.db_file),
    )
    .await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut ts).await?;
    let conversation = Uuid::new_v4();
    let response = Router::new()
        .route("/chat", post(chat::post_chat))
        .layer(Extension(RequestTrust { trusted_app: true }))
        .with_state(chat.clone())
        .oneshot(json_request("POST", "/chat", &inline_body(conversation))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = tokio::time::timeout(Duration::from_secs(5), body_bytes(response)).await??;
    let chunks = sse::parse(&bytes)?;
    assert!(chunks.iter().any(|chunk| matches!(chunk, UiChunk::Error { error_text } if error_text.contains("database is locked"))));
    assert_eq!(
        chat.state(conversation).await.ok_or("missing run")?.status,
        server_rust::chat::RunStatus::Failed
    );
    assert!(db.conversations().get(conversation).await?.is_none());
    sqlx::query("ROLLBACK").execute(&mut ts).await?;
    ts.close().await?;
    db.close().await?;
    Ok(())
}
