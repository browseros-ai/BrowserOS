//! HTTP compatibility checks against the TS CRUD routes, using a fresh on-disk database per
//! test. Requests cross the real router so validation, persistence and response omission are
//! exercised together; the temporary BrowserOS directory never touches the user's profile.

use axum::{
    Router,
    body::{Body, to_bytes},
    response::IntoResponse,
};
use http::{Request, StatusCode};
use serde_json::{Value, json};
use server_rust::{
    AppState, build_router,
    config::{Config, EnvConfig},
};
use tower::ServiceExt as _;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const CONVERSATION: &str = "10000000-0000-4000-8000-000000000001";
const OTHER_CONVERSATION: &str = "10000000-0000-4000-8000-000000000002";

struct Server {
    router: Router,
    state: AppState,
    _dir: tempfile::TempDir,
}

impl Server {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"ports":{"server":19444,"cdp":19445},"directories":{"resources":".","execution":"."}}"#,
        )?;
        let env = EnvConfig {
            browseros_dir: Some(dir.path().join("browseros")),
            ..Default::default()
        };
        let state = AppState::start(Config::load(&path, &env)?).await?;
        Ok(Self {
            router: build_router(state.clone()),
            state,
            _dir: dir,
        })
    }

    async fn raw(
        &self,
        method: &str,
        path: &str,
        body: &str,
        content_type: Option<&str>,
    ) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1")
            .header(
                "origin",
                "chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk",
            );
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        let mut request = request.body(Body::from(body.to_owned()))?;
        // Production obtains this from axum::serve's socket; in-process requests have no socket.
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:12345".parse::<std::net::SocketAddr>()?,
        ));
        let response = self.router.clone().oneshot(request).await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await?;
        Ok((status, serde_json::from_slice(&bytes)?))
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Value,
    ) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
        self.raw(method, path, &body.to_string(), Some("application/json"))
            .await
    }

    async fn ok(
        &self,
        method: &str,
        path: &str,
        body: Value,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let (status, value) = self.request(method, path, body).await?;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {value}");
        Ok(value)
    }
}

fn provider() -> Value {
    json!({"type":"not-an-enum", "name":"Provider", "modelId":"model", "contextWindow":8192, "apiKey":"test-secret", "accessKeyId":"access", "secretAccessKey":"secret", "sessionToken":"session", "createdAt":12})
}

fn daily_job() -> Value {
    json!({"name":"Job", "query":"Work", "scheduleType":"daily", "scheduleTime":"09:00"})
}

// OpenAPI DTOs serialize number fields as f64; compare JSON numbers by their numeric value,
// including the explicit HTTP-to-i64 normalization required by the merged store contract.
fn assert_number(actual: &Value, expected: f64) {
    assert_eq!(actual.as_f64(), Some(expected), "{actual}");
}

fn assert_zod(status: StatusCode, value: &Value) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    assert_eq!(value["success"], false);
    assert_eq!(value["error"]["name"], "ZodError");
    Ok(serde_json::from_str(
        value["error"]["message"]
            .as_str()
            .ok_or("missing issue array")?,
    )?)
}

#[tokio::test]
async fn tests_that_empty_collections_and_missing_rows_match_ts() -> TestResult {
    let server = Server::new().await?;
    for (path, key) in [
        ("/conversations", "conversations"),
        ("/agents", "agents"),
        ("/providers", "providers"),
        ("/scheduled-jobs", "jobs"),
        ("/scheduled-job-runs", "runs"),
    ] {
        assert_eq!(server.ok("GET", path, Value::Null).await?, json!({key:[]}));
    }
    assert_eq!(
        server.ok("GET", "/providers/default", Value::Null).await?,
        json!({"provider":null})
    );
    for (path, message) in [
        (
            format!("/conversations/{CONVERSATION}"),
            "Unknown conversation",
        ),
        (format!("/agents/{CONVERSATION}"), "Unknown agent"),
        ("/providers/missing".into(), "Unknown provider"),
        ("/scheduled-jobs/missing".into(), "Unknown scheduled job"),
        ("/scheduled-job-runs/missing".into(), "Unknown run"),
    ] {
        for method in ["GET", "DELETE"] {
            let (status, value) = server.request(method, &path, Value::Null).await?;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {value}");
            assert_eq!(value, json!({"error":message}));
        }
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_conversation_import_preserves_unvalidated_messages_and_never_overwrites()
-> TestResult {
    let server = Server::new().await?;
    let path = format!("/conversations/{CONVERSATION}");
    let messages = json!([[], {"arbitrary":true}, {"id":"u", "role":"user", "parts":[{"type":"text","text":"hello"}]}]);
    let input = json!({"messages":messages,"origin":"legacy","targetType":"codex","agentId":"agent","lastMessagedAt":42.5,"ignored":true});
    let imported = server.ok("PUT", &path, input).await?;
    assert_eq!(imported["imported"], true);
    assert_number(&imported["conversation"]["lastMessagedAt"], 42.0);
    assert_eq!(imported["conversation"]["targetType"], "codex");
    assert_eq!(imported["conversation"]["agentId"], "agent");
    assert_eq!(imported["conversation"]["lastUserMessage"], "hello");
    let detail = server.ok("GET", &path, Value::Null).await?;
    assert_eq!(detail["conversation"]["messages"], messages);
    assert_eq!(
        server.ok("PUT", &path, json!({"messages":[]})).await?,
        json!({"conversation":null,"imported":false})
    );
    assert_eq!(server.ok("GET", &path, Value::Null).await?, detail);
    assert_eq!(
        server.ok("DELETE", &path, Value::Null).await?,
        json!({"success":true})
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_conversation_order_and_omission_match_ts() -> TestResult {
    let server = Server::new().await?;
    for (id, timestamp) in [(CONVERSATION, 10), (OTHER_CONVERSATION, 20)] {
        server
            .ok(
                "PUT",
                &format!("/conversations/{id}"),
                json!({"messages":[],"lastMessagedAt":timestamp}),
            )
            .await?;
    }
    let value = server.ok("GET", "/conversations", Value::Null).await?;
    assert_eq!(value["conversations"][0]["id"], OTHER_CONVERSATION);
    let summary = &value["conversations"][1];
    assert_eq!(summary["targetType"], "browseros");
    for key in ["lastUserMessage", "origin", "agentId", "messages"] {
        assert!(summary.get(key).is_none(), "{key}: {summary}");
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_provider_put_returns_raw_secrets_and_gets_redact_all_four() -> TestResult {
    let server = Server::new().await?;
    let saved = server.ok("PUT", "/providers/first", provider()).await?;
    assert_eq!(saved["provider"]["apiKey"], "test-secret");
    assert_eq!(saved["provider"]["kind"], "llm");
    assert_number(&saved["provider"]["createdAt"], 12.0);
    assert!(saved["provider"].get("hasApiKey").is_none());
    for key in [
        "baseUrl",
        "headers",
        "workingDirectory",
        "customConfig",
        "resourceName",
        "region",
        "reasoningEffort",
        "reasoningSummary",
    ] {
        assert_eq!(saved["provider"].get(key), Some(&Value::Null), "{key}");
    }
    let public = server.ok("GET", "/providers/first", Value::Null).await?;
    for (secret, flag) in [
        ("apiKey", "hasApiKey"),
        ("accessKeyId", "hasAccessKeyId"),
        ("secretAccessKey", "hasSecretAccessKey"),
        ("sessionToken", "hasSessionToken"),
    ] {
        assert!(public["provider"].get(secret).is_none());
        assert_eq!(public["provider"][flag], true);
    }
    let list = server.ok("GET", "/providers", Value::Null).await?;
    assert_eq!(list["providers"][0], public["provider"]);
    let selected = server
        .ok("PUT", "/providers/default", json!({"providerId":"first"}))
        .await?;
    assert!(selected["provider"].get("apiKey").is_none());
    assert_eq!(selected["provider"]["isDefault"], true);
    Ok(())
}

#[tokio::test]
async fn tests_that_provider_credentials_preserve_blanks_and_clear_only_on_null() -> TestResult {
    let server = Server::new().await?;
    server.ok("PUT", "/providers/p", provider()).await?;
    server
        .ok("PUT", "/providers/default", json!({"providerId":"p"}))
        .await?;
    let mut update = provider();
    update["apiKey"] = json!("");
    update
        .as_object_mut()
        .ok_or("object")?
        .remove("accessKeyId");
    update["secretAccessKey"] = Value::Null;
    update["sessionToken"] = Value::Null;
    update["createdAt"] = json!(999);
    update["headers"] = json!({"X-Session":"{{conversationId}}"});
    update["contextWindow"] = json!(1024.5);
    let saved = server.ok("PUT", "/providers/p", update).await?;
    assert_eq!(saved["provider"]["apiKey"], "test-secret");
    assert_eq!(saved["provider"]["accessKeyId"], "access");
    assert_eq!(saved["provider"]["secretAccessKey"], Value::Null);
    assert_eq!(saved["provider"]["sessionToken"], Value::Null);
    assert_number(&saved["provider"]["createdAt"], 12.0);
    assert_number(&saved["provider"]["contextWindow"], 1024.0);
    assert_eq!(saved["provider"]["isDefault"], true);
    assert_eq!(
        saved["provider"]["headers"]["X-Session"],
        "{{conversationId}}"
    );
    let public = server.ok("GET", "/providers/p", Value::Null).await?;
    assert_eq!(public["provider"]["hasSecretAccessKey"], false);
    assert_eq!(public["provider"]["hasSessionToken"], false);
    Ok(())
}

#[tokio::test]
async fn tests_that_default_switch_is_atomic_and_unknown_default_is_404() -> TestResult {
    let server = Server::new().await?;
    for id in ["z", "a"] {
        server
            .ok("PUT", &format!("/providers/{id}"), provider())
            .await?;
    }
    for id in ["z", "a"] {
        server
            .ok("PUT", "/providers/default", json!({"providerId":id}))
            .await?;
    }
    assert_eq!(
        server.ok("GET", "/providers/default", Value::Null).await?["provider"]["id"],
        "a"
    );
    let (status, value) = server
        .request("PUT", "/providers/default", json!({"providerId":"absent"}))
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(value, json!({"error":"Unknown provider"}));
    let list = server.ok("GET", "/providers", Value::Null).await?;
    assert_eq!(
        list["providers"][0]["id"], "z",
        "row order must not become alphabetical"
    );
    assert_eq!(list["providers"][0]["isDefault"], false);
    assert_eq!(list["providers"][1]["isDefault"], true);
    Ok(())
}

#[tokio::test]
async fn tests_that_agents_trim_parse_config_and_share_provider_rows() -> TestResult {
    let server = Server::new().await?;
    let (status, created) = server.request("POST", "/agents", json!({"name":"  Custom  ","type":"custom","customConfig":{"command":"  agent --acp  ","icon":"  robot  ","fullAccessModes":["  full  "],"env":{"KEY":" value "}}})).await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let agent = &created["agent"];
    assert_eq!(agent["name"], "Custom");
    assert_eq!(agent["customConfig"]["command"], "agent --acp");
    assert_eq!(agent["customConfig"]["env"]["KEY"], " value ");
    assert_eq!(agent["customConfig"]["fullAccessModes"], json!(["full"]));
    assert!(agent.get("modelId").is_none());
    let id = agent["id"].as_str().ok_or("id")?;
    assert_eq!(uuid::Uuid::parse_str(id)?.get_version_num(), 4);
    let public = server
        .ok("GET", &format!("/providers/{id}"), Value::Null)
        .await?;
    assert_eq!(public["provider"]["kind"], "acp");
    let raw = public["provider"]["customConfig"]
        .as_str()
        .ok_or("config must stay a raw string in provider GET")?;
    assert_eq!(serde_json::from_str::<Value>(raw)?, agent["customConfig"]);
    let renamed = server
        .ok(
            "PUT",
            &format!("/agents/{id}"),
            json!({"name":" New ","modelId":" model "}),
        )
        .await?;
    assert_eq!(renamed["agent"]["modelId"], "model");
    let cleared = server
        .ok("PUT", &format!("/agents/{id}"), json!({"modelId":null}))
        .await?;
    assert!(cleared["agent"].get("modelId").is_none());
    assert_eq!(
        server
            .ok("DELETE", &format!("/providers/{id}"), Value::Null)
            .await?,
        json!({"success":true})
    );
    assert_eq!(
        server
            .request("GET", &format!("/agents/{id}"), Value::Null)
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_builtin_agent_config_update_is_400_and_unknown_agent_is_404() -> TestResult {
    let server = Server::new().await?;
    let (status, created) = server
        .request("POST", "/agents", json!({"name":"Builtin","type":"claude"}))
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["agent"]["id"].as_str().ok_or("id")?;
    let patch = json!({"customConfig":{"command":"tool"}});
    let (status, value) = server
        .request("PUT", &format!("/agents/{id}"), patch.clone())
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        value,
        json!({"error":"customConfig is only allowed for custom agents"})
    );
    assert_eq!(
        server
            .request("PUT", &format!("/agents/{CONVERSATION}"), patch)
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server
            .ok("DELETE", &format!("/agents/{id}"), Value::Null)
            .await?,
        json!({"success":true})
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_agent_validation_is_strict_and_counts_utf16_after_trimming() -> TestResult {
    let server = Server::new().await?;
    for input in [
        json!({"name":"a","type":"custom"}),
        json!({"name":"a","type":"claude","customConfig":{"command":"x"}}),
        json!({"name":"a","type":"codex","extra":true}),
        json!({"name":"a","type":"custom","customConfig":{"command":"x","extra":true}}),
        json!({"name":"😀".repeat(41),"type":"codex"}),
        json!({"name":"a","type":"custom","customConfig":{"command":"x","icon":"😀".repeat(33)}}),
        json!({"name":null,"type":"codex"}),
    ] {
        let (status, value) = server.request("POST", "/agents", input).await?;
        assert_zod(status, &value)?;
    }
    let (status, value) = server
        .request(
            "POST",
            "/agents",
            json!({"name":format!("  {}  ", "😀".repeat(40)),"type":"codex"}),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    let (status, value) = server
        .request("POST", "/agents", json!({"name":" ","type":"codex"}))
        .await?;
    let issues = assert_zod(status, &value)?;
    assert_eq!(issues, json!([{"origin":"string","code":"too_small","minimum":1,"inclusive":true,"path":["name"],"message":"Too small: expected string to have >=1 characters"}]).as_array().ok_or("array")?.clone());
    Ok(())
}

#[tokio::test]
async fn tests_that_schedule_validation_checks_changed_cadence_and_preserves_partial_columns()
-> TestResult {
    let server = Server::new().await?;
    for input in [
        json!({"name":"j","query":"q","scheduleType":"daily","scheduleTime":"24:00"}),
        json!({"name":"j","query":"q","scheduleType":"hourly","scheduleInterval":1.5}),
        json!({"name":"j","query":"q","scheduleType":"minutes","scheduleInterval":61}),
    ] {
        let kind = input["scheduleType"].as_str().ok_or("kind")?;
        let message = if kind == "daily" {
            "daily jobs require scheduleTime as HH:MM (24-hour)".to_owned()
        } else {
            format!("{kind} jobs require an integer scheduleInterval between 1 and 60")
        };
        let (status, value) = server.request("PUT", "/scheduled-jobs/j", input).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
        assert_eq!(value, json!({"error":message}));
    }
    let mut input = daily_job();
    input["enabled"] = json!(false);
    input["lastRunAt"] = json!(4.5);
    input["createdAt"] = json!(7.5);
    let saved = server.ok("PUT", "/scheduled-jobs/j", input).await?;
    assert_number(&saved["job"]["createdAt"], 7.0);
    assert_eq!(saved["job"]["scheduleInterval"], Value::Null);
    assert_eq!(saved["job"]["providerId"], Value::Null);
    let mut update = daily_job();
    update["createdAt"] = json!(500);
    let saved = server.ok("PUT", "/scheduled-jobs/j", update).await?;
    assert_number(&saved["job"]["createdAt"], 7.0);
    assert_eq!(
        saved["job"]["enabled"], false,
        "omitted enabled preserves an existing false"
    );
    assert_number(&saved["job"]["lastRunAt"], 4.0);
    Ok(())
}

#[tokio::test]
async fn tests_that_runs_prune_to_fifteen_on_every_write_and_cascade_on_job_delete() -> TestResult {
    let server = Server::new().await?;
    server.ok("PUT", "/scheduled-jobs/j", daily_job()).await?;
    for index in 0..20 {
        let saved = server
            .ok(
                "PUT",
                &format!("/scheduled-job-runs/r{index}"),
                json!({"jobId":"j","status":"running","startedAt":index,"createdAt":3.5}),
            )
            .await?;
        assert_number(&saved["run"]["createdAt"], 3.0);
        for field in [
            "completedAt",
            "result",
            "finalResult",
            "executionLog",
            "toolCalls",
            "error",
        ] {
            assert_eq!(saved["run"].get(field), Some(&Value::Null), "{field}");
        }
    }
    let runs = server.ok("GET", "/scheduled-job-runs", Value::Null).await?;
    assert_eq!(runs["runs"].as_array().ok_or("runs")?.len(), 15);
    assert_eq!(runs["runs"][0]["id"], "r19");
    assert_eq!(runs["runs"][14]["id"], "r5");
    let saved = server.ok("PUT", "/scheduled-job-runs/r19", json!({"jobId":"j","status":"completed","startedAt":19.5,"completedAt":20.5,"finalResult":"done","toolCalls":[{"id":"call","name":"tool","input":null,"output":{"ok":true},"timestamp":"now","ignored":true}]})).await?;
    assert_number(&saved["run"]["createdAt"], 3.0);
    assert_number(&saved["run"]["startedAt"], 19.0);
    assert_number(&saved["run"]["completedAt"], 20.0);
    assert!(saved["run"]["toolCalls"][0].get("ignored").is_none());
    assert_eq!(
        server
            .ok("DELETE", "/scheduled-job-runs/r19", Value::Null)
            .await?,
        json!({"success":true})
    );
    server
        .ok("DELETE", "/scheduled-jobs/j", Value::Null)
        .await?;
    assert_eq!(
        server.ok("GET", "/scheduled-job-runs", Value::Null).await?,
        json!({"runs":[]})
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_foreign_key_failures_use_500_envelope_and_provider_delete_sets_null()
-> TestResult {
    let server = Server::new().await?;
    let mut job = daily_job();
    job["providerId"] = json!("unknown");
    for (path, body) in [
        ("/scheduled-jobs/j", job.clone()),
        (
            "/scheduled-job-runs/r",
            json!({"jobId":"absent","status":"running","startedAt":1}),
        ),
    ] {
        let (status, value) = server.request("PUT", path, body).await?;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{value}");
        assert_eq!(value["error"]["name"], "InternalServerError");
        assert_eq!(value["error"]["statusCode"], 500);
        assert!(value["error"]["code"].is_string());
        assert!(value["error"]["message"].is_string());
        assert_eq!(
            value,
            json!({"error": {
                "code":"unknown", "title":"Something went wrong",
                "message":"FOREIGN KEY constraint failed", "retryable":true,
                "details":"FOREIGN KEY constraint failed", "name":"InternalServerError", "statusCode":500,
            }})
        );
    }
    server.ok("PUT", "/providers/unknown", provider()).await?;
    server.ok("PUT", "/scheduled-jobs/j", job).await?;
    server
        .ok("DELETE", "/providers/unknown", Value::Null)
        .await?;
    assert_eq!(
        server.ok("GET", "/scheduled-jobs/j", Value::Null).await?["job"]["providerId"],
        Value::Null
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_json_content_type_malformed_json_and_uuid_quirks_match_ts() -> TestResult {
    let server = Server::new().await?;
    let (status, value) = server
        .raw("PUT", "/providers/p", &provider().to_string(), None)
        .await?;
    assert_zod(status, &value)?;
    let (status, value) = server
        .raw("POST", "/agents", "{", Some("application/json"))
        .await?;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{value}");
    assert_eq!(value["error"]["name"], "InternalServerError");
    for id in [
        "bad",
        "10000000000040008000000000000001",
        "10000000-0000-0000-8000-000000000001",
        "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF",
    ] {
        let (status, value) = server
            .request("GET", &format!("/conversations/{id}"), Value::Null)
            .await?;
        assert_zod(status, &value)?;
    }
    for id in [
        "00000000-0000-0000-0000-000000000000",
        "ffffffff-ffff-ffff-ffff-ffffffffffff",
        "10000000-0000-8000-8000-000000000001",
    ] {
        assert_eq!(
            server
                .request("GET", &format!("/conversations/{id}"), Value::Null)
                .await?
                .0,
            StatusCode::NOT_FOUND
        );
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_header_records_reject_invalid_names_values_and_case_duplicates() -> TestResult {
    let server = Server::new().await?;
    for headers in [
        json!({"Bad Name":"x"}),
        json!({"X-Test":"line\nbreak"}),
        json!({"X-Test":"x", "x-test":"y"}),
    ] {
        let mut body = provider();
        body["headers"] = headers;
        let (status, value) = server.request("PUT", "/providers/p", body).await?;
        assert_zod(status, &value)?;
    }
    let mut body = provider();
    body["headers"] = json!({"X-Value":"\t\u{00ff}"});
    server.ok("PUT", "/providers/p", body).await?;
    Ok(())
}

#[tokio::test]
async fn tests_that_validation_issue_strings_match_zod_goldens() -> TestResult {
    use server_api::models as api;
    use server_rust::http::wire::crud::decode;

    // Captured from createAgentRoutes (Hono/zod 4.6.5), rather than reconstructed from
    // Rust's validator. String equality matters because the issues are nested JSON text.
    let cases = [
        (
            json!({"name":" ","type":"codex"}),
            r#"[
  {
    "origin": "string",
    "code": "too_small",
    "minimum": 1,
    "inclusive": true,
    "path": [
      "name"
    ],
    "message": "Too small: expected string to have >=1 characters"
  }
]"#,
        ),
        (
            json!({"name":"a","type":"custom","extra":1}),
            r#"[
  {
    "code": "unrecognized_keys",
    "keys": [
      "extra"
    ],
    "path": [],
    "message": "Unrecognized key: \"extra\""
  },
  {
    "code": "custom",
    "message": "customConfig is required for custom agents",
    "path": [
      "customConfig"
    ]
  }
]"#,
        ),
        (
            json!({"name":"a","type":"custom","customConfig":null}),
            r#"[
  {
    "expected": "object",
    "code": "invalid_type",
    "path": [
      "customConfig"
    ],
    "message": "Invalid input: expected object, received null"
  }
]"#,
        ),
    ];
    for (input, expected) in cases {
        let response = match decode::<api::CreateAgentRequest>(input) {
            Ok(_) => return Err("invalid input was accepted".into()),
            Err(response) => response.into_response(),
        };
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
        assert_eq!(body["error"]["message"], expected);
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_invalid_uuid_wins_over_malformed_body() -> TestResult {
    let server = Server::new().await?;
    for path in ["/agents/bad", "/conversations/bad"] {
        let (status, body) = server
            .raw("PUT", path, "{", Some("application/json"))
            .await?;
        let issues = assert_zod(status, &body)?;
        assert_eq!(issues[0]["format"], "uuid");
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_import_accepts_more_than_axums_default_json_limit() -> TestResult {
    let server = Server::new().await?;
    let body = json!({"messages":[{"text":"x".repeat(2 * 1024 * 1024 + 1)}]});
    let saved = server
        .ok(
            "PUT",
            &format!("/conversations/{CONVERSATION}"),
            body.clone(),
        )
        .await?;
    assert_eq!(saved["imported"], true);
    let detail = server
        .ok(
            "GET",
            &format!("/conversations/{CONVERSATION}"),
            Value::Null,
        )
        .await?;
    assert_eq!(detail["conversation"]["messages"], body["messages"]);
    Ok(())
}

#[tokio::test]
async fn tests_that_unchanged_legacy_invalid_cadence_remains_writable() -> TestResult {
    let server = Server::new().await?;
    // A direct store write represents a pre-validation database. The HTTP interface must
    // allow maintenance of that row without granting a new invalid cadence to other writes.
    server
        .state
        .schedules
        .upsert_job(
            "legacy",
            server_rust::db::UpsertJob {
                name: "Legacy".into(),
                query: "Work".into(),
                schedule_type: "daily".into(),
                schedule_time: Some(Some("25:00".into())),
                schedule_interval: None,
                enabled: Some(true),
                provider_id: None,
                last_run_at: None,
                created_at: None,
            },
        )
        .await?;
    let input = json!({"name":"Legacy","query":"Work","scheduleType":"daily","scheduleTime":"25:00","enabled":false,"lastRunAt":8});
    let saved = server
        .ok("PUT", "/scheduled-jobs/legacy", input.clone())
        .await?;
    assert_eq!(saved["job"]["enabled"], false);
    assert_number(&saved["job"]["lastRunAt"], 8.0);
    let mut changed = input;
    changed["scheduleInterval"] = json!(1);
    let (status, body) = server
        .request("PUT", "/scheduled-jobs/legacy", changed)
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({"error":"daily jobs require scheduleTime as HH:MM (24-hour)"})
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_malformed_stored_agent_config_is_omitted_only_from_agent_projection()
-> TestResult {
    let server = Server::new().await?;
    for raw in [
        "not JSON",
        r#"{"command":" "}"#,
        r#"{"command":"x","unexpected":true}"#,
    ] {
        let row = server
            .state
            .agents
            .create(server_rust::db::CreateAgent {
                kind: server_rust::db::AgentKind::Custom,
                name: "Legacy".into(),
                model_id: None,
                reasoning_effort: None,
                working_directory: None,
                custom_config: Some(raw.into()),
            })
            .await?;
        let detail = server
            .ok("GET", &format!("/agents/{}", row.id), Value::Null)
            .await?;
        assert!(detail["agent"].get("customConfig").is_none());
        let public = server
            .ok("GET", &format!("/providers/{}", row.id), Value::Null)
            .await?;
        assert_eq!(public["provider"]["customConfig"], raw);
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_agent_list_orders_by_last_update() -> TestResult {
    let server = Server::new().await?;
    let (_, first) = server
        .request("POST", "/agents", json!({"type":"codex","name":"First"}))
        .await?;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let (_, second) = server
        .request("POST", "/agents", json!({"type":"claude","name":"Second"}))
        .await?;
    let list = server.ok("GET", "/agents", Value::Null).await?;
    assert_eq!(list["agents"][0]["id"], second["agent"]["id"]);
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    server
        .ok(
            "PUT",
            &format!("/agents/{}", first["agent"]["id"].as_str().ok_or("id")?),
            json!({"name":"Updated first"}),
        )
        .await?;
    let list = server.ok("GET", "/agents", Value::Null).await?;
    assert_eq!(list["agents"][0]["id"], first["agent"]["id"]);
    Ok(())
}

#[tokio::test]
async fn tests_that_concurrent_imports_insert_exactly_one_history() -> TestResult {
    let server = Server::new().await?;
    let path = format!("/conversations/{CONVERSATION}");
    let (first, second) = tokio::join!(
        server.request("PUT", &path, json!({"messages":[{"text":"first"}]})),
        server.request("PUT", &path, json!({"messages":[{"text":"second"}]})),
    );
    let (first_status, first) = first?;
    let (second_status, second) = second?;
    assert_eq!(first_status, StatusCode::OK);
    assert_eq!(second_status, StatusCode::OK);
    assert_ne!(first["imported"], second["imported"]);
    let detail = server.ok("GET", &path, Value::Null).await?;
    assert_eq!(
        detail["conversation"]["messages"][0]["text"],
        if first["imported"] == true {
            "first"
        } else {
            "second"
        }
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_each_crud_body_preserves_its_zod_validation_rules() -> TestResult {
    let server = Server::new().await?;
    for (method, path, body) in [
        (
            "PUT",
            format!("/conversations/{CONVERSATION}"),
            json!({"messages":[null]}),
        ),
        (
            "PUT",
            format!("/conversations/{CONVERSATION}"),
            json!({"messages":[],"targetType":"custom"}),
        ),
        (
            "PUT",
            format!("/conversations/{CONVERSATION}"),
            json!({"messages":[],"origin":null}),
        ),
        (
            "PUT",
            format!("/agents/{CONVERSATION}"),
            json!({"type":"claude"}),
        ),
        (
            "PUT",
            format!("/agents/{CONVERSATION}"),
            json!({"modelId":" "}),
        ),
        (
            "PUT",
            format!("/agents/{CONVERSATION}"),
            json!({"customConfig":null}),
        ),
        ("PUT", "/providers/default".into(), json!({"providerId":""})),
        (
            "PUT",
            "/providers/p".into(),
            json!({"type":"openai","name":"n","modelId":"m"}),
        ),
        (
            "PUT",
            "/scheduled-jobs/j".into(),
            json!({"name":"j","query":"q","scheduleType":"weekly"}),
        ),
        (
            "PUT",
            "/scheduled-jobs/j".into(),
            json!({"name":"j","query":"q","scheduleType":"daily","enabled":null}),
        ),
        (
            "PUT",
            "/scheduled-job-runs/r".into(),
            json!({"jobId":"j","status":"unknown","startedAt":1}),
        ),
        (
            "PUT",
            "/scheduled-job-runs/r".into(),
            json!({"jobId":"j","status":"running","startedAt":1,"toolCalls":[{"id":"c","name":"t","timestamp":"now"}]}),
        ),
        (
            "PUT",
            "/scheduled-job-runs/r".into(),
            json!({"jobId":"j","status":"running","startedAt":1,"createdAt":null}),
        ),
    ] {
        let (status, body) = server.request(method, &path, body).await?;
        assert_zod(status, &body)?;
    }
    Ok(())
}
