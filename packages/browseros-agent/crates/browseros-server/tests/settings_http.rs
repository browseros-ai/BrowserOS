//! Exercise the settings routes with extension requests and use SQL to verify effects that
//! public responses cannot show, including retained legacy rows and foreign keys.

mod support;

use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use browseros_server::store::{Credentials, DraftError, ProviderDraft, ProviderKind};
use sea_orm::sqlx::SqlitePool;
use sea_orm::sqlx::sqlite::SqliteConnectOptions;
use serde_json::{Value, json};
use support::TestServer;

type TestResult<T = ()> = anyhow::Result<T>;

const API_KEY: &str = "fixture-api-key-private";
const ACCESS_KEY: &str = "fixture-access-key-private";
const SECRET_KEY: &str = "fixture-secret-key-private";
const SESSION_TOKEN: &str = "fixture-session-token-private";
const PUBLIC_FLAGS: [&str; 4] = [
    "hasApiKey",
    "hasAccessKeyId",
    "hasSecretAccessKey",
    "hasSessionToken",
];

async fn reply(
    server: &TestServer,
    request: Request<Body>,
    expected: StatusCode,
) -> TestResult<Value> {
    let response = server.send(request).await;
    assert_eq!(response.status(), expected);
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    for secret in [API_KEY, ACCESS_KEY, SECRET_KEY, SESSION_TOKEN] {
        assert!(
            !bytes
                .windows(secret.len())
                .any(|window| window == secret.as_bytes()),
            "settings response contained a credential"
        );
    }
    Ok(serde_json::from_slice(&bytes)?)
}

async fn database(server: &TestServer) -> TestResult<SqlitePool> {
    Ok(SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(server.state_dir().join("db/browseros.sqlite"))
            .foreign_keys(true),
    )
    .await?)
}

fn provider_input() -> Value {
    json!({
        "type": "fireworks",
        "name": "Fireworks fixture",
        "modelId": "fixture-model",
        "contextWindow": 32000.6
    })
}

async fn put_provider(server: &TestServer, input: &Value) -> TestResult<Value> {
    let request = server.json(Method::PUT, "/providers/fixture", input);
    reply(server, request, StatusCode::OK).await
}

async fn select(server: &TestServer, id: &str, status: StatusCode) -> TestResult<Value> {
    let request = server.json(
        Method::PUT,
        "/providers/default",
        &json!({ "providerId": id }),
    );
    reply(server, request, status).await
}

async fn delete(server: &TestServer, path: &str, status: StatusCode) -> TestResult<Value> {
    let request = server.request(Method::DELETE, path).body(Body::empty())?;
    reply(server, request, status).await
}

#[tokio::test]
async fn saves_preserve_selection_creation_time_credentials_and_job_links() -> TestResult {
    let server = TestServer::start().await?;
    let db = database(&server).await?;
    let mut input = provider_input();
    input["createdAt"] = json!(123);
    input["apiKey"] = json!(API_KEY);
    input["accessKeyId"] = json!(ACCESS_KEY);
    input["secretAccessKey"] = json!(SECRET_KEY);
    input["sessionToken"] = json!(SESSION_TOKEN);
    input["baseUrl"] = json!("https://saved.invalid");
    input["headers"] = json!({ "x-conversation": "{{conversationId}}" });
    input["supportsImages"] = json!(false);
    input["temperature"] = json!(0.4);
    let created = put_provider(&server, &input).await?;
    assert_eq!(created["provider"]["contextWindow"], 32001);
    assert_eq!(created["provider"]["isDefault"], false);
    assert_eq!(created["provider"]["workingDirectory"], Value::Null);
    assert!(created["provider"].get("workingDirectory").is_some());
    assert_eq!(
        select(&server, "fixture", StatusCode::OK).await?["provider"]["isDefault"],
        true
    );

    sea_orm::sqlx::query(
        "INSERT INTO scheduled_jobs
         (id, name, query, schedule_type, provider_id, created_at, updated_at)
         VALUES ('job', 'job name', 'job query', 'daily', 'fixture', 2, 3)",
    )
    .execute(&db)
    .await?;

    let mut edit = provider_input();
    edit["name"] = json!("Renamed fixture");
    edit["createdAt"] = json!(999);
    let omitted = put_provider(&server, &edit).await?;
    for flag in PUBLIC_FLAGS {
        assert_eq!(omitted["provider"][flag], true);
    }

    let credential_fields = ["apiKey", "accessKeyId", "secretAccessKey", "sessionToken"];
    for field in credential_fields {
        edit[field] = json!("");
    }
    let blank = put_provider(&server, &edit).await?;
    for flag in PUBLIC_FLAGS {
        assert_eq!(blank["provider"][flag], true);
    }
    assert_eq!(blank["provider"]["createdAt"], 123);
    assert_eq!(blank["provider"]["isDefault"], true);
    assert_eq!(blank["provider"]["baseUrl"], "https://saved.invalid");
    assert_eq!(blank["provider"]["headers"], input["headers"]);
    assert_eq!(blank["provider"]["supportsImages"], false);
    assert_eq!(blank["provider"]["temperature"], 0.4);
    assert_eq!(blank["provider"]["name"], "Renamed fixture");

    // The legacy provider reader selects the type as text with no type whitelist.
    let legacy_type: String = sea_orm::sqlx::query_scalar(
        "SELECT type FROM providers WHERE id = 'fixture' AND kind = 'llm'",
    )
    .fetch_one(&db)
    .await?;
    assert_eq!(legacy_type, "fireworks");
    let job_after_save: (Option<String>, String, String, i64, i64) = sea_orm::sqlx::query_as(
        "SELECT provider_id, name, query, created_at, updated_at FROM scheduled_jobs WHERE id = 'job'",
    )
    .fetch_one(&db)
    .await?;
    assert_eq!(job_after_save.0.as_deref(), Some("fixture"));

    let listed = reply(&server, server.get("/providers"), StatusCode::OK).await?;
    let default = reply(&server, server.get("/providers/default"), StatusCode::OK).await?;
    assert_eq!(listed["providers"], json!([blank["provider"]]));
    assert_eq!(default["provider"], blank["provider"]);

    let providers = server.app.store.providers();
    let saved = providers
        .config(None)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing default config"))?;
    assert!(matches!(&saved.credentials, Credentials::ApiKey(key) if key.expose() == API_KEY));
    let draft = ProviderDraft {
        kind: ProviderKind::Fireworks,
        model: "different model".into(),
        base_url: Some("https://typed.invalid".into()),
        headers: Vec::new(),
        resource_name: Some("typed-resource".into()),
        region: None,
        api_key: None,
        access_key_id: None,
        secret_access_key: None,
        session_token: None,
    };
    let borrowed = draft
        .clone()
        .resolve(Some(saved))
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    assert_eq!(borrowed.base_url.as_deref(), Some("https://saved.invalid"));
    assert_eq!(borrowed.resource_name, None);
    assert!(matches!(borrowed.credentials, Credentials::ApiKey(key) if key.expose() == API_KEY));
    let mut wrong_type = draft;
    wrong_type.kind = ProviderKind::Azure;
    assert_eq!(
        wrong_type
            .resolve(providers.config(Some("fixture")).await?)
            .err(),
        Some(DraftError::TypeMismatch)
    );

    for field in credential_fields {
        edit[field] = Value::Null;
    }
    let cleared = put_provider(&server, &edit).await?;
    for flag in PUBLIC_FLAGS {
        assert_eq!(cleared["provider"][flag], false);
    }
    let database_secrets: i64 = sea_orm::sqlx::query_scalar(
        "SELECT count(*) FROM providers WHERE id = 'fixture'
         AND (api_key IS NOT NULL OR access_key_id IS NOT NULL
              OR secret_access_key IS NOT NULL OR session_token IS NOT NULL)",
    )
    .fetch_one(&db)
    .await?;
    assert_eq!(database_secrets, 0);

    assert_eq!(
        delete(&server, "/providers/fixture", StatusCode::OK).await?,
        json!({ "success": true })
    );
    delete(&server, "/providers/fixture", StatusCode::NOT_FOUND).await?;
    let job_after_delete = sea_orm::sqlx::query_as::<_, (Option<String>, String, String, i64, i64)>(
        "SELECT provider_id, name, query, created_at, updated_at FROM scheduled_jobs WHERE id = 'job'",
    )
    .fetch_one(&db)
    .await?;
    assert_eq!(
        job_after_delete,
        (
            None,
            job_after_save.1,
            job_after_save.2,
            job_after_save.3,
            job_after_save.4
        )
    );
    assert_eq!(
        reply(&server, server.get("/providers/default"), StatusCode::OK).await?,
        json!({ "provider": null })
    );
    Ok(())
}

#[tokio::test]
async fn agents_share_one_default_and_legacy_targets_remain_in_sqlite() -> TestResult {
    let server = TestServer::start().await?;
    assert_eq!(
        reply(&server, server.get("/agents"), StatusCode::OK).await?,
        json!({ "agents": [] })
    );
    let db = database(&server).await?;
    sea_orm::sqlx::query(
        "INSERT INTO providers (id, kind, type, name, model_id, context_window, is_default,
          created_at, updated_at, api_key)
         VALUES ('old-gh', 'llm', 'github-copilot', 'old GH', 'm', 1, 1, 1, 1, ?1),
                ('old-qwen', 'llm', 'qwen-code', 'old Qwen', 'm', 1, 0, 1, 1, ?1),
                ('old-custom', 'acp', 'custom', 'old custom', NULL, NULL, 0, 1, 1, ?1)",
    )
    .bind(API_KEY)
    .execute(&db)
    .await?;
    assert_eq!(
        reply(&server, server.get("/providers/default"), StatusCode::OK).await?,
        json!({ "provider": null })
    );
    select(&server, "old-qwen", StatusCode::NOT_FOUND).await?;
    let legacy_default: String =
        sea_orm::sqlx::query_scalar("SELECT id FROM providers WHERE is_default = 1")
            .fetch_one(&db)
            .await?;
    assert_eq!(legacy_default, "old-gh");

    let agent = reply(
        &server,
        server.json(
            Method::POST,
            "/agents",
            &json!({ "type": "claude", "name": "  Claude  ", "modelId": "model" }),
        ),
        StatusCode::CREATED,
    )
    .await?;
    let id = agent["agent"]["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing id"))?;
    uuid::Uuid::parse_str(id)?;
    assert_eq!(agent["agent"]["name"], "Claude");
    assert_eq!(agent["agent"]["type"], "claude");
    let agent_path = format!("/agents/{id}");
    let provider_path = format!("/providers/{id}");
    let patch = json!({ "modelId": null, "workingDirectory": "/workspace" });

    reply(
        &server,
        server.json(Method::PUT, &provider_path, &provider_input()),
        StatusCode::BAD_REQUEST,
    )
    .await?;
    delete(&server, &provider_path, StatusCode::NOT_FOUND).await?;
    delete(&server, "/agents/old-custom", StatusCode::NOT_FOUND).await?;
    put_provider(&server, &provider_input()).await?;
    reply(
        &server,
        server.json(Method::PUT, "/agents/fixture", &patch),
        StatusCode::NOT_FOUND,
    )
    .await?;

    let (llm, acp) = tokio::join!(
        select(&server, "fixture", StatusCode::OK),
        select(&server, id, StatusCode::OK),
    );
    assert_eq!(llm?["provider"]["kind"], "llm");
    assert_eq!(acp?["provider"]["kind"], "acp");
    let selected = select(&server, id, StatusCode::OK).await?;
    let defaults: Vec<String> =
        sea_orm::sqlx::query_scalar("SELECT id FROM providers WHERE is_default = 1")
            .fetch_all(&db)
            .await?;
    assert_eq!(defaults, [id]);

    let edited = reply(
        &server,
        server.json(Method::PUT, &agent_path, &patch),
        StatusCode::OK,
    )
    .await?;
    assert!(edited["agent"].get("modelId").is_none());
    assert_eq!(edited["agent"]["workingDirectory"], "/workspace");
    assert_eq!(edited["agent"]["createdAt"], agent["agent"]["createdAt"]);
    assert_eq!(
        reply(&server, server.get("/agents"), StatusCode::OK).await?["agents"],
        json!([edited["agent"]])
    );
    let providers = reply(&server, server.get("/providers"), StatusCode::OK).await?;
    assert_eq!(providers["providers"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        reply(&server, server.get("/providers/default"), StatusCode::OK).await?["provider"]["id"],
        selected["provider"]["id"]
    );
    assert!(server.app.store.providers().config(None).await?.is_none());

    delete(&server, &agent_path, StatusCode::OK).await?;
    delete(&server, &agent_path, StatusCode::NOT_FOUND).await?;
    let legacy_rows: i64 = sea_orm::sqlx::query_scalar(
        "SELECT count(*) FROM providers WHERE type IN ('github-copilot', 'qwen-code', 'custom')",
    )
    .fetch_one(&db)
    .await?;
    assert_eq!(legacy_rows, 3);
    Ok(())
}

#[tokio::test]
async fn reopening_a_conversation_ignores_an_unreadable_transcript_in_another_row() -> TestResult {
    let server = TestServer::start().await?;
    let db = database(&server).await?;
    let id = uuid::Uuid::new_v4();
    let other = uuid::Uuid::new_v4();
    // SQLite accepts a BLOB in a text column. An unfiltered load fails when SQLx reads it
    // as text, even if the caller never attempts to decode the JSON.
    sea_orm::sqlx::query(
        "INSERT INTO conversations
         (id, messages, last_user_message, target_type, last_messaged_at, created_at, updated_at)
         VALUES (?, ?, 'Question', 'browseros', 110, 90, 110),
                (?, X'80', NULL, 'browseros', 120, 100, 120)",
    )
    .bind(id.to_string())
    .bind(r#"[{"id":"user","role":"user","parts":[{"type":"text","text":"Question"}]}]"#)
    .bind(other.to_string())
    .execute(&db)
    .await?;

    let response = reply(
        &server,
        server.get(&format!("/conversations/{id}")),
        StatusCode::OK,
    )
    .await?;
    assert_eq!(response["conversation"]["id"], id.to_string());
    assert_eq!(response["conversation"]["createdAt"], 90);
    assert_eq!(
        response["conversation"]["messages"][0]["parts"][0]["text"],
        "Question"
    );
    Ok(())
}

#[tokio::test]
async fn history_routes_follow_the_trust_and_error_contract() -> TestResult {
    let server = TestServer::start().await?;
    let id = uuid::Uuid::new_v4();
    let path = format!("/conversations/{id}");

    let denied = Request::delete(&path)
        .header("Host", "127.0.0.1:9200")
        .body(Body::empty())?;
    assert_eq!(
        reply(&server, denied, StatusCode::FORBIDDEN).await?,
        json!({ "error": "Forbidden" })
    );

    for verb in [Method::GET, Method::DELETE] {
        let invalid = server
            .request(verb, "/conversations/bad-id")
            .body(Body::empty())?;
        assert!(reply(&server, invalid, StatusCode::BAD_REQUEST).await?["error"].is_string());
    }

    // The runs chunk owns the backing methods. Check the route envelope before those bodies
    // land and the empty/missing responses after the runs implementation is integrated.
    let (list_status, detail_status, list_body, detail_body) =
        match server.app.conversations.list().await {
            Err(browseros_server::store::StoreError::NotImplemented(_)) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": "Internal server error" }),
                json!({ "error": "Internal server error" }),
            ),
            Ok(rows) if rows.is_empty() => (
                StatusCode::OK,
                StatusCode::NOT_FOUND,
                json!({ "conversations": [] }),
                json!({ "error": "Unknown conversation" }),
            ),
            Err(error) => return Err(error.into()),
            Ok(_) => anyhow::bail!("a fresh database contains history"),
        };

    assert_eq!(
        reply(&server, server.get("/conversations"), list_status).await?,
        list_body
    );
    assert_eq!(
        reply(&server, server.get(&path), detail_status).await?,
        detail_body
    );
    assert_eq!(delete(&server, &path, detail_status).await?, detail_body);
    Ok(())
}
