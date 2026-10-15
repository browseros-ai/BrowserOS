//! `/test-provider` and `/refine-prompt` against the real HTTP handlers, SQLite stores and a
//! loopback provider.
//! The fake speaks OpenAI streaming chunks, so partial/error/empty output crosses the same
//! transport and generation path as the settings UI without spending a vendor credential.

use axum::{Json, Router, body::Body, extract::State, response::IntoResponse};
use http::{HeaderMap, Request, StatusCode};
use serde_json::{Value, json};
use server_rust::{
    AppState,
    config::{Config, EnvConfig},
    db::{AgentKind, CreateAgent, UpsertProvider},
    telemetry::metrics::{Metrics, MetricsConfig},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Fixture {
    state: AppState,
    _dir: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Self::with_mock(false).await
    }

    async fn with_mock(use_mock_llm: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let config_file = dir.path().join("config.json");
        std::fs::write(
            &config_file,
            // These handlers run in-process; the sidecar parser still requires nonzero ports.
            r#"{"ports":{"server":1,"cdp":1},"directories":{"resources":".","execution":"."}}"#,
        )?;
        let env = EnvConfig {
            browseros_dir: Some(dir.path().join("browseros")),
            use_mock_llm,
            ..Default::default()
        };
        let state = AppState::start(Config::load(&config_file, &env)?).await?;
        Ok(Self { state, _dir: dir })
    }

    async fn send(
        &self,
        path: &str,
        body: Value,
    ) -> Result<(StatusCode, Vec<u8>), Box<dyn std::error::Error>> {
        let response = server_rust::build_router(self.state.clone())
            .oneshot(
                Request::post(path)
                    .extension(axum::extract::ConnectInfo(
                        "127.0.0.1:12345".parse::<std::net::SocketAddr>()?,
                    ))
                    .header("host", "127.0.0.1")
                    .header(
                        "origin",
                        "chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk",
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))?,
            )
            .await?;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await?
            .to_vec();
        Ok((status, bytes))
    }

    async fn post(
        &self,
        path: &str,
        body: Value,
    ) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
        let (status, bytes) = self.send(path, body).await?;
        Ok((status, serde_json::from_slice(&bytes)?))
    }

    async fn save(&self, id: &str, fake: &FakeProvider, model: &str) -> TestResult {
        self.state
            .providers
            .upsert_llm(
                id,
                UpsertProvider {
                    provider_type: "openai-compatible".to_owned(),
                    name: id.to_owned(),
                    model_id: Some(model.to_owned()),
                    base_url: Some(Some(fake.url.clone())),
                    api_key: Some(Some("saved-key".to_owned())),
                    context_window: Some(128_000),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }
}

#[derive(Clone)]
struct FakeState {
    text: String,
    fail: bool,
    calls: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
}

struct FakeProvider {
    url: String,
    calls: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeProvider {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeProvider {
    async fn start(text: &str, fail: bool) -> Result<Self, Box<dyn std::error::Error>> {
        async fn respond(
            State(state): State<FakeState>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> axum::response::Response {
            state
                .calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((headers, body));
            if state.fail {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error":{"message":"invalid test key", "type":"invalid_api_key"}})),
                )
                    .into_response();
            }
            if state.text == "[stall]" {
                std::future::pending::<()>().await;
            }
            let content = json!({"id":"chatcmpl-test", "object":"chat.completion.chunk", "created":1, "model":"test", "choices":[{"index":0, "delta":{"role":"assistant", "content":state.text}, "finish_reason":null}]});
            let finish = json!({"id":"chatcmpl-test", "object":"chat.completion.chunk", "created":1, "model":"test", "choices":[{"index":0, "delta":{}, "finish_reason":"stop"}], "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
            (
                [("content-type", "text/event-stream")],
                format!("data: {content}\n\ndata: {finish}\n\ndata: [DONE]\n\n"),
            )
                .into_response()
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/v1", listener.local_addr()?);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let router = Router::new().fallback(respond).with_state(FakeState {
            text: text.to_owned(),
            fail,
            calls: calls.clone(),
        });
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Ok(Self { url, calls, task })
    }

    fn calls(&self) -> Vec<(HeaderMap, Value)> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn inline(&self) -> Value {
        json!({"provider":"openai-compatible", "model":"test-model", "apiKey":"inline-key", "baseUrl":self.url})
    }
}

/// A provider-store failure is not a provider-config problem: TS lets the read throw into
/// `app.onError`, whose `toChatError` classifies a plain `Error` as `unknown` and retryable.
#[tokio::test]
async fn provider_store_failure_answers_the_plain_error_envelope() -> TestResult {
    use sea_orm::sqlx::{Connection as _, sqlite::SqliteConnection};
    let fixture = Fixture::new().await?;
    let db_file = fixture.state.paths.db_file.display().to_string();
    let mut sqlite = SqliteConnection::connect(&format!("sqlite://{db_file}")).await?;
    sea_orm::sqlx::query("DROP TABLE providers")
        .execute(&mut sqlite)
        .await?;
    sqlite.close().await?;

    let fake = FakeProvider::start("ok", false).await?;
    let mut body = fake.inline();
    body["providerId"] = json!("saved");
    for (path, body) in [
        ("/test-provider", body),
        (
            "/refine-prompt",
            json!({"prompt":"check prices", "name":"Prices", "providerId":"saved"}),
        ),
    ] {
        let (status, body) = fixture.post(path, body).await?;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{path}: {body}");
        assert_eq!(body["error"]["code"], "unknown", "{path}: {body}");
        assert_eq!(body["error"]["title"], "Something went wrong");
        assert_eq!(body["error"]["retryable"], true);
        assert_eq!(body["error"]["name"], "InternalServerError");
    }
    Ok(())
}

#[tokio::test]
async fn provider_test_streams_preview_and_reports_time() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start(&"a".repeat(101), false).await?;
    let (status, body) = fixture.post("/test-provider", fake.inline()).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["success"], true);
    assert_eq!(
        body["message"],
        format!(
            "Connection successful. Response: \"{}...\"",
            "a".repeat(100)
        )
    );
    assert!(body["responseTime"].as_u64().is_some());
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].1["messages"]
            .to_string()
            .contains("Respond with exactly: 'ok'")
    );
    Ok(())
}

#[tokio::test]
async fn provider_preview_preserves_js_split_surrogate() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start(&format!("{}😀", "a".repeat(99)), false).await?;
    let (status, bytes) = fixture.send("/test-provider", fake.inline()).await?;
    assert_eq!(status, StatusCode::OK);
    let body = String::from_utf8(bytes)?;
    assert!(
        body.contains(&format!(r#"{}\ud83d...\"""#, "a".repeat(99))),
        "{body}"
    );
    Ok(())
}

#[tokio::test]
async fn provider_deadline_bounds_a_stalled_stream() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start("[stall]", false).await?;
    let (status, body) = tokio::time::timeout(
        std::time::Duration::from_secs(17),
        fixture.post("/test-provider", fake.inline()),
    )
    .await??;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["message"],
        "[openai-compatible] The operation timed out."
    );
    assert!(body["responseTime"].as_u64().is_some_and(|ms| ms >= 14_900));
    Ok(())
}

#[tokio::test]
async fn empty_test_output_succeeds_but_empty_refinement_fails() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start("", false).await?;
    let (status, body) = fixture.post("/test-provider", fake.inline()).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["message"],
        "Connection successful. Provider responded."
    );
    let mut request = fake.inline();
    request["name"] = json!("Morning");
    request["prompt"] = json!("Read the news");
    let (status, body) = fixture.post("/refine-prompt", request).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({"success":false, "message":"Provider returned an empty response"})
    );
    Ok(())
}

#[tokio::test]
async fn provider_error_has_provider_prefix_and_response_time() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start("", true).await?;
    let (status, body) = fixture.post("/test-provider", fake.inline()).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["success"], false);
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("[openai-compatible] ")
                && message.contains("invalid test key")),
        "{body}"
    );
    assert!(body["responseTime"].as_u64().is_some());
    Ok(())
}

#[tokio::test]
async fn stored_secret_pins_destination_but_explicit_secret_keeps_inline_url() -> TestResult {
    let fixture = Fixture::new().await?;
    let saved = FakeProvider::start("saved", false).await?;
    let inline = FakeProvider::start("inline", false).await?;
    fixture.save("saved", &saved, "stored-model").await?;
    let mut request = inline.inline();
    request["providerId"] = json!("saved");
    request["apiKey"] = json!("");
    let (status, body) = fixture.post("/test-provider", request.clone()).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(inline.calls().is_empty());
    assert_eq!(
        saved.calls()[0]
            .0
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer saved-key")
    );
    request["apiKey"] = json!("caller-key");
    let (status, body) = fixture.post("/test-provider", request).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        inline.calls()[0]
            .0
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer caller-key")
    );
    Ok(())
}

#[tokio::test]
async fn stored_secret_is_never_reused_for_a_different_provider_type() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start("unused", false).await?;
    fixture.save("saved", &fake, "stored-model").await?;
    let (status, body) = fixture.post("/test-provider", json!({"providerId":"saved", "provider":"anthropic", "model":"test", "baseUrl":fake.url})).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["message"],
        "[anthropic] Anthropic provider requires apiKey"
    );
    assert!(fake.calls().is_empty());
    Ok(())
}

#[tokio::test]
async fn every_saved_credential_field_pins_the_destination() -> TestResult {
    let fixture = Fixture::new().await?;
    let saved = FakeProvider::start("saved", false).await?;
    let other = FakeProvider::start("caller", false).await?;
    for (index, field) in ["apiKey", "accessKeyId", "secretAccessKey", "sessionToken"]
        .into_iter()
        .enumerate()
    {
        let id = format!("credential-{index}");
        let mut row = UpsertProvider {
            provider_type: "openai-compatible".to_owned(),
            name: id.clone(),
            model_id: Some("model".to_owned()),
            context_window: Some(128_000),
            base_url: Some(Some(saved.url.clone())),
            ..Default::default()
        };
        match field {
            "apiKey" => row.api_key = Some(Some("saved-key".to_owned())),
            "accessKeyId" => row.access_key_id = Some(Some("saved-access".to_owned())),
            "secretAccessKey" => row.secret_access_key = Some(Some("saved-secret".to_owned())),
            _ => row.session_token = Some(Some("saved-session".to_owned())),
        }
        fixture.state.providers.upsert_llm(&id, row).await?;
        let mut body = other.inline();
        body["providerId"] = json!(id);
        body[field] = json!("");
        let (status, response) = fixture.post("/test-provider", body).await?;
        assert_eq!(status, StatusCode::OK, "{field}: {response}");
    }
    assert!(other.calls().is_empty());
    assert_eq!(saved.calls().len(), 4);
    Ok(())
}

#[tokio::test]
async fn absent_saved_endpoint_fields_clear_untrusted_inline_selectors() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start("must not receive the saved key", false).await?;
    fixture
        .state
        .providers
        .upsert_llm(
            "azure",
            UpsertProvider {
                provider_type: "azure".to_owned(),
                name: "Azure".to_owned(),
                model_id: Some("model".to_owned()),
                context_window: Some(128_000),
                api_key: Some(Some("saved-key".to_owned())),
                ..Default::default()
            },
        )
        .await?;
    let (status, body) = fixture
        .post(
            "/test-provider",
            json!({
                "providerId":"azure", "provider":"azure", "model":"model",
                "baseUrl":fake.url,"resourceName":"untrusted-resource", "region":"untrusted-region"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["message"],
        "[azure] Azure provider requires apiKey and either resourceName or baseUrl"
    );
    assert!(fake.calls().is_empty());
    Ok(())
}

#[tokio::test]
async fn refinement_uses_ecmascript_whitespace_rules() -> TestResult {
    let fixture = Fixture::new().await?;
    for (text, expected) in [
        ("\u{feff} refined \u{feff}", "refined"),
        ("\u{85}", "\u{85}"),
    ] {
        let fake = FakeProvider::start(text, false).await?;
        let mut request = fake.inline();
        request["name"] = json!("Morning");
        request["prompt"] = json!("Read the news");
        let (status, body) = fixture.post("/refine-prompt", request).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["refined"], expected);
    }
    Ok(())
}

#[tokio::test]
async fn refinement_resolves_named_then_inline_then_default_then_first_llm() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start("  refined instruction \n", false).await?;
    fixture.save("first", &fake, "first-model").await?;
    fixture.save("selected", &fake, "selected-model").await?;
    let base = json!({"name":"Morning brief", "prompt":"Check the news"});
    let (status, body) = fixture.post("/refine-prompt", base.clone()).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"success":true,"refined":"refined instruction"})
    );
    assert_eq!(fake.calls()[0].1["model"], "first-model");
    fixture.state.providers.set_default("selected").await?;
    let (status, body) = fixture.post("/refine-prompt", base.clone()).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(fake.calls()[1].1["model"], "selected-model");
    let mut inline = fake.inline();
    inline["name"] = base["name"].clone();
    inline["prompt"] = base["prompt"].clone();
    let (status, body) = fixture.post("/refine-prompt", inline.clone()).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(fake.calls()[2].1["model"], "test-model");
    inline["providerId"] = json!("first");
    let (status, body) = fixture.post("/refine-prompt", inline).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let calls = fake.calls();
    assert_eq!(calls[3].1["model"], "first-model");
    assert!(calls[3].1["messages"].to_string().contains("Morning brief"));
    assert!(
        calls[3].1["messages"]
            .to_string()
            .contains("Return ONLY the rewritten prompt")
    );
    Ok(())
}

#[tokio::test]
async fn refinement_rejects_missing_unknown_or_explicit_coding_agent() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start("refined", false).await?;
    let base = json!({"name":"Morning", "prompt":"Read the news"});
    let (status, body) = fixture.post("/refine-prompt", base.clone()).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["message"],
        "Connect an LLM provider in AI settings to refine a prompt"
    );
    fixture.save("first", &fake, "first-model").await?;
    let agent = fixture
        .state
        .agents
        .create(CreateAgent {
            kind: AgentKind::Claude,
            name: "Claude".to_owned(),
            model_id: None,
            reasoning_effort: None,
            working_directory: None,
            custom_config: None,
        })
        .await?;
    let mut request = base.clone();
    request["providerId"] = json!("missing");
    let (status, body) = fixture.post("/refine-prompt", request.clone()).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Unknown provider missing");
    request["providerId"] = json!(agent.id);
    let (status, body) = fixture.post("/refine-prompt", request).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["message"],
        format!(
            "Provider {} is a coding agent and cannot refine a prompt",
            agent.id
        )
    );
    Ok(())
}

#[tokio::test]
async fn refinement_falls_back_from_coding_agent_default() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start("refined", false).await?;
    fixture.save("first", &fake, "first-model").await?;
    let agent = fixture
        .state
        .agents
        .create(CreateAgent {
            kind: AgentKind::Claude,
            name: "Claude".to_owned(),
            model_id: None,
            reasoning_effort: None,
            working_directory: None,
            custom_config: None,
        })
        .await?;
    fixture
        .state
        .providers
        .set_default(&agent.id.to_string())
        .await?;
    let (status, body) = fixture
        .post(
            "/refine-prompt",
            json!({"name":"Morning", "prompt":"Read the news"}),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(fake.calls()[0].1["model"], "first-model");
    Ok(())
}

#[tokio::test]
async fn unknown_stored_llm_type_is_not_mistaken_for_a_coding_agent() -> TestResult {
    let fixture = Fixture::new().await?;
    fixture
        .state
        .providers
        .upsert_llm(
            "future",
            UpsertProvider {
                provider_type: "future-provider".to_owned(),
                name: "Future".to_owned(),
                model_id: Some("model".to_owned()),
                context_window: Some(128_000),
                ..Default::default()
            },
        )
        .await?;
    let (status, body) = fixture
        .post(
            "/refine-prompt",
            json!({"providerId":"future", "name":"Morning", "prompt":"Read the news"}),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({"success":false,"message":"Unknown provider: future-provider"})
    );
    Ok(())
}

#[tokio::test]
async fn stored_header_validation_uses_factory_failure_message() -> TestResult {
    let fixture = Fixture::new().await?;
    fixture
        .state
        .providers
        .upsert_llm(
            "headers",
            UpsertProvider {
                provider_type: "openai-compatible".to_owned(),
                name: "Headers".to_owned(),
                model_id: Some("model".to_owned()),
                context_window: Some(128_000),
                headers: Some(Some(BTreeMap::from([
                    ("X-Token".to_owned(), "a".to_owned()),
                    ("x-token".to_owned(), "b".to_owned()),
                ]))),
                ..Default::default()
            },
        )
        .await?;
    let (status, body) = fixture
        .post(
            "/refine-prompt",
            json!({"providerId":"headers", "name":"Morning", "prompt":"Read the news"}),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["success"], false);
    let issues: Value =
        serde_json::from_str(body["message"].as_str().ok_or("missing factory error")?)?;
    assert_eq!(
        issues,
        json!([{"code":"custom","message":"Duplicate header name","path":["x-token"]}])
    );
    assert!(body.get("error").is_none());
    Ok(())
}

#[tokio::test]
async fn mock_harness_bypasses_oauth_and_stored_factory_validation() -> TestResult {
    let fixture = Fixture::with_mock(true).await?;
    let (status, body) = fixture
        .post(
            "/test-provider",
            json!({"provider":"chatgpt-pro", "model":"test"}),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["message"],
        "Connection successful. Response: \"Mock BrowserOS test response.\""
    );
    fixture
        .state
        .providers
        .upsert_llm(
            "mock",
            UpsertProvider {
                provider_type: "future-provider".to_owned(),
                name: "Mock".to_owned(),
                model_id: Some("model".to_owned()),
                context_window: Some(128_000),
                headers: Some(Some(BTreeMap::from([
                    ("X-Token".to_owned(), "a".to_owned()),
                    ("x-token".to_owned(), "b".to_owned()),
                ]))),
                ..Default::default()
            },
        )
        .await?;
    let (status, body) = fixture
        .post(
            "/refine-prompt",
            json!({"providerId":"mock", "name":"Morning", "prompt":"Read the news"}),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["refined"], "Mock BrowserOS test response.");
    Ok(())
}

#[tokio::test]
async fn one_shot_headers_get_fresh_random_conversation_ids() -> TestResult {
    let fixture = Fixture::new().await?;
    let fake = FakeProvider::start("ok", false).await?;
    let mut request = fake.inline();
    request["headers"] = json!({"x-conversation":"{{conversationId}}/{{conversationId}}"});
    for _ in 0..2 {
        let (status, body) = fixture.post("/test-provider", request.clone()).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let calls = fake.calls();
    let mut ids = Vec::new();
    for (headers, _) in calls {
        let value = headers
            .get("x-conversation")
            .ok_or("missing conversation header")?
            .to_str()?;
        let (left, right) = value.split_once('/').ok_or("missing second placeholder")?;
        assert_eq!(left, right);
        ids.push(uuid::Uuid::parse_str(left)?);
    }
    assert_ne!(ids[0], ids[1]);
    Ok(())
}

#[tokio::test]
async fn invalid_input_uses_zod_error_dialect() -> TestResult {
    let fixture = Fixture::new().await?;
    for (path, body) in [
        ("/test-provider", json!({"provider":"openai", "model":""})),
        ("/refine-prompt", json!({"name":"", "prompt":""})),
        (
            "/test-provider",
            json!({"provider":"openai", "model":"test", "headers":{"X-Token":"a","x-token":"b"}}),
        ),
        (
            "/refine-prompt",
            json!({"name":"Morning", "prompt":"check", "apiKey":null}),
        ),
    ] {
        let (status, body) = fixture.post(path, body).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"]["name"], "ZodError");
        let issues: Vec<Value> =
            serde_json::from_str(body["error"]["message"].as_str().ok_or("missing issues")?)?;
        assert!(!issues.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn metrics_never_enable_without_install_identity() {
    for install_id in [None, Some(String::new())] {
        let metrics = Metrics::initialize(MetricsConfig {
            install_id,
            properties: BTreeMap::from([("install_id".to_owned(), json!("spoofed"))]),
            ..Default::default()
        })
        .await;
        assert!(!metrics.is_enabled());
        metrics.capture("http_server.started", json!({}));
        metrics.capture("tool_executed", json!({"tool_name":"run","success":true}));
        metrics.shutdown().await;
    }
}

// Captured with Bun from LLMConfigSchema and the two route schema extensions, as
// declared in the TS source. Keep the full pretty error message: clients receive
// this JSON as a string, so field and issue ordering are part of the wire contract.
const TS_VALIDATION_FIXTURES: &str = r####"[
  {
    "path": "/test-provider",
    "input": {},
    "issues": [
      {
        "code": "invalid_value",
        "values": [
          "anthropic",
          "openai",
          "google",
          "openrouter",
          "azure",
          "ollama",
          "lmstudio",
          "bedrock",
          "openai-compatible",
          "moonshot",
          "chatgpt-pro",
          "github-copilot",
          "qwen-code"
        ],
        "path": [
          "provider"
        ],
        "message": "Invalid option: expected one of \"anthropic\"|\"openai\"|\"google\"|\"openrouter\"|\"azure\"|\"ollama\"|\"lmstudio\"|\"bedrock\"|\"openai-compatible\"|\"moonshot\"|\"chatgpt-pro\"|\"github-copilot\"|\"qwen-code\""
      },
      {
        "expected": "string",
        "code": "invalid_type",
        "path": [
          "model"
        ],
        "message": "Invalid input: expected string, received undefined"
      }
    ]
  },
  {
    "path": "/test-provider",
    "input": {
      "provider": "bad",
      "model": ""
    },
    "issues": [
      {
        "code": "invalid_value",
        "values": [
          "anthropic",
          "openai",
          "google",
          "openrouter",
          "azure",
          "ollama",
          "lmstudio",
          "bedrock",
          "openai-compatible",
          "moonshot",
          "chatgpt-pro",
          "github-copilot",
          "qwen-code"
        ],
        "path": [
          "provider"
        ],
        "message": "Invalid option: expected one of \"anthropic\"|\"openai\"|\"google\"|\"openrouter\"|\"azure\"|\"ollama\"|\"lmstudio\"|\"bedrock\"|\"openai-compatible\"|\"moonshot\"|\"chatgpt-pro\"|\"github-copilot\"|\"qwen-code\""
      },
      {
        "origin": "string",
        "code": "too_small",
        "minimum": 1,
        "inclusive": true,
        "path": [
          "model"
        ],
        "message": "Model name is required"
      }
    ]
  },
  {
    "path": "/test-provider",
    "input": {
      "provider": "openai",
      "model": "m",
      "headers": {
        "X-Token": "a",
        "x-token": "b"
      }
    },
    "issues": [
      {
        "code": "custom",
        "message": "Duplicate header name",
        "path": [
          "headers",
          "x-token"
        ]
      }
    ]
  },
  {
    "path": "/test-provider",
    "input": {
      "provider": "openai",
      "model": "m",
      "headers": {
        "bad name": "a",
        "ok": "a\n"
      }
    },
    "issues": [
      {
        "code": "invalid_key",
        "origin": "record",
        "issues": [
          {
            "origin": "string",
            "code": "invalid_format",
            "format": "regex",
            "pattern": "/^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])/",
            "path": [],
            "message": "Invalid string: must match pattern /^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])/"
          }
        ],
        "path": [
          "headers",
          "bad name"
        ],
        "message": "Invalid key in record"
      },
      {
        "origin": "string",
        "code": "invalid_format",
        "format": "regex",
        "pattern": "/^[\\t\\x20-\\x7e\\x80-\\xff]*(?![\\s\\S])/",
        "path": [
          "headers",
          "ok"
        ],
        "message": "Invalid string: must match pattern /^[\\t\\x20-\\x7e\\x80-\\xff]*(?![\\s\\S])/"
      }
    ]
  },
  {
    "path": "/test-provider",
    "input": {
      "provider": "openai",
      "model": "m",
      "headers": {
        "ok": 4
      }
    },
    "issues": [
      {
        "expected": "string",
        "code": "invalid_type",
        "path": [
          "headers",
          "ok"
        ],
        "message": "Invalid input: expected string, received number"
      }
    ]
  },
  {
    "path": "/test-provider",
    "input": {
      "provider": "openai",
      "model": "m",
      "headers": [
        "x"
      ]
    },
    "issues": [
      {
        "expected": "record",
        "code": "invalid_type",
        "path": [
          "headers"
        ],
        "message": "Invalid input: expected record, received array"
      }
    ]
  },
  {
    "path": "/test-provider",
    "input": {
      "provider": "openai",
      "model": "m",
      "headers": {
        "bad name": 3,
        "X-K": "a",
        "x-k": "b"
      }
    },
    "issues": [
      {
        "code": "invalid_key",
        "origin": "record",
        "issues": [
          {
            "origin": "string",
            "code": "invalid_format",
            "format": "regex",
            "pattern": "/^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])/",
            "path": [],
            "message": "Invalid string: must match pattern /^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])/"
          }
        ],
        "path": [
          "headers",
          "bad name"
        ],
        "message": "Invalid key in record"
      }
    ]
  },
  {
    "path": "/refine-prompt",
    "input": {
      "name": "",
      "prompt": "",
      "apiKey": null
    },
    "issues": [
      {
        "expected": "string",
        "code": "invalid_type",
        "path": [
          "apiKey"
        ],
        "message": "Invalid input: expected string, received null"
      },
      {
        "origin": "string",
        "code": "too_small",
        "minimum": 1,
        "inclusive": true,
        "path": [
          "prompt"
        ],
        "message": "Prompt cannot be empty"
      },
      {
        "origin": "string",
        "code": "too_small",
        "minimum": 1,
        "inclusive": true,
        "path": [
          "name"
        ],
        "message": "Task name cannot be empty"
      }
    ]
  },
  {
    "path": "/refine-prompt",
    "input": [],
    "issues": [
      {
        "expected": "object",
        "code": "invalid_type",
        "path": [],
        "message": "Invalid input: expected object, received array"
      }
    ]
  },
  {
    "path": "/refine-prompt",
    "input": {
      "name": 3,
      "prompt": true,
      "reasoningEffort": "bad"
    },
    "issues": [
      {
        "code": "invalid_value",
        "values": [
          "none",
          "low",
          "medium",
          "high",
          "xhigh",
          "max"
        ],
        "path": [
          "reasoningEffort"
        ],
        "message": "Invalid option: expected one of \"none\"|\"low\"|\"medium\"|\"high\"|\"xhigh\"|\"max\""
      },
      {
        "expected": "string",
        "code": "invalid_type",
        "path": [
          "prompt"
        ],
        "message": "Invalid input: expected string, received boolean"
      },
      {
        "expected": "string",
        "code": "invalid_type",
        "path": [
          "name"
        ],
        "message": "Invalid input: expected string, received number"
      }
    ]
  },
  {
    "path": "/refine-prompt",
    "input": {
      "name": "n",
      "prompt": "p",
      "model": "",
      "upstreamProvider": 3,
      "providerId": null
    },
    "issues": [
      {
        "expected": "string",
        "code": "invalid_type",
        "path": [
          "providerId"
        ],
        "message": "Invalid input: expected string, received null"
      },
      {
        "origin": "string",
        "code": "too_small",
        "minimum": 1,
        "inclusive": true,
        "path": [
          "model"
        ],
        "message": "Too small: expected string to have >=1 characters"
      },
      {
        "expected": "string",
        "code": "invalid_type",
        "path": [
          "upstreamProvider"
        ],
        "message": "Invalid input: expected string, received number"
      }
    ]
  },
  {
    "path": "/test-provider",
    "input": null,
    "issues": [
      {
        "expected": "object",
        "code": "invalid_type",
        "path": [],
        "message": "Invalid input: expected object, received null"
      }
    ]
  },
  {
    "path": "/test-provider",
    "input": true,
    "issues": [
      {
        "expected": "object",
        "code": "invalid_type",
        "path": [],
        "message": "Invalid input: expected object, received boolean"
      }
    ]
  }
]"####;

#[tokio::test]
async fn validation_messages_match_captured_ts_bytes() -> TestResult {
    let fixture = Fixture::new().await?;
    let examples: Vec<Value> = serde_json::from_str(TS_VALIDATION_FIXTURES)?;
    for example in examples {
        let path = example["path"].as_str().ok_or("fixture without route")?;
        let (status, body) = fixture.post(path, example["input"].clone()).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{example}: {body}");
        assert_eq!(body["error"]["name"], "ZodError");
        assert_eq!(
            body["error"]["message"],
            serde_json::to_string_pretty(&example["issues"])?
        );
    }
    Ok(())
}
