//! The extension-facing surface through the in-process harness: the system routes, the trust
//! rule, the refusal shapes the contract promises, the private MCP mount, and probes that check
//! real responses against the generated OpenAPI document.

mod support;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use serde_json::{Value, json};
use support::{TestServer, body_json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const CONVERSATION: &str = "5f0d7c2a-8f7e-4f7c-9a55-2f0a8c1b9d10";

#[tokio::test]
async fn the_system_routes_answer_without_an_origin() -> TestResult {
    let server = TestServer::start().await?;
    let bare = |path: &str| {
        Request::get(path)
            .header(header::HOST, "127.0.0.1:9200")
            .body(Body::empty())
            .unwrap_or_default()
    };
    let health = server.send(bare("/system/health")).await;
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(
        body_json(health).await,
        json!({ "status": "ok", "cdpConnected": false })
    );
    let alias = server.send(bare("/health")).await;
    assert_eq!(alias.status(), StatusCode::OK);
    let status = body_json(server.send(bare("/status")).await).await;
    assert_eq!(
        status,
        json!({ "status": "ok", "cdpConnected": false, "can_update": true })
    );
    let diagnostics = body_json(server.send(bare("/system/diagnostics")).await).await;
    assert_eq!(diagnostics["version"], json!(browseros_server::VERSION));
    assert!(diagnostics["os"].is_string());
    Ok(())
}

#[tokio::test]
async fn the_trust_rule_refuses_pages_and_writes_without_an_origin() -> TestResult {
    let server = TestServer::start().await?;
    let from = |origin: Option<&str>, method: Method, path: &str, host: &str| {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host);
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        request.body(Body::empty()).unwrap_or_default()
    };

    let page = server
        .send(from(
            Some("https://example.com"),
            Method::GET,
            "/providers",
            "127.0.0.1:9200",
        ))
        .await;
    assert_eq!(page.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(page).await, json!({ "error": "Forbidden" }));

    let extension = server.send(server.get("/providers")).await;
    assert_eq!(extension.status(), StatusCode::OK);
    assert_eq!(
        extension
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .map(|v| v.as_bytes()),
        Some(support::EXTENSION_ORIGIN.as_bytes())
    );

    let write_without_origin = server
        .send(from(None, Method::POST, "/chat", "127.0.0.1:9200"))
        .await;
    assert_eq!(write_without_origin.status(), StatusCode::FORBIDDEN);

    let read_without_origin = server
        .send(from(None, Method::GET, "/agents", "127.0.0.1:9200"))
        .await;
    assert_eq!(read_without_origin.status(), StatusCode::OK);

    let rebinding = server
        .send(from(
            Some(support::EXTENSION_ORIGIN),
            Method::GET,
            "/providers",
            "evil.example:9200",
        ))
        .await;
    assert_eq!(rebinding.status(), StatusCode::FORBIDDEN);

    let preflight = server
        .send(from(
            Some(support::EXTENSION_ORIGIN),
            Method::OPTIONS,
            "/chat",
            "localhost:9200",
        ))
        .await;
    assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        preflight
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .map(|v| v.as_bytes()),
        Some(&b"Content-Type, Accept"[..])
    );
    Ok(())
}

#[tokio::test]
async fn a_trusted_development_origin_is_accepted() -> TestResult {
    let server = TestServer::with(support::TestOptions {
        trusted_origins: vec!["http://localhost:5173".into()],
        ..support::TestOptions::default()
    })
    .await?;
    let request = Request::get("/providers")
        .header(header::HOST, "127.0.0.1:9200")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())?;
    assert_eq!(server.send(request).await.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn refusals_answer_in_the_shapes_the_contract_promises() -> TestResult {
    let server = TestServer::start().await?;

    let bad_id = server.send(server.get("/chat/not-a-uuid/state")).await;
    assert_eq!(bad_id.status(), StatusCode::BAD_REQUEST);
    assert!(
        body_json(bad_id).await["error"].is_string(),
        "W1: 400 {{error}}"
    );

    let bad_chat = server
        .send(server.json(Method::POST, "/chat", &json!({ "target": 1 })))
        .await;
    assert_eq!(bad_chat.status(), StatusCode::BAD_REQUEST);
    let envelope = body_json(bad_chat).await;
    assert_eq!(envelope["error"]["code"], json!("unknown"));
    assert_eq!(envelope["error"]["retryable"], json!(false));

    let chat = json!({
        "conversationId": CONVERSATION,
        "target": { "type": "browseros" },
        "message": "hello",
        "someFieldAnOldExtensionStillSends": true
    });
    let parsed = server.send(server.json(Method::POST, "/chat", &chat)).await;
    assert_eq!(
        parsed.status(),
        StatusCode::NOT_IMPLEMENTED,
        "unknown fields are ignored"
    );

    let bad_test = server
        .send(server.json(Method::POST, "/test-provider", &json!({})))
        .await;
    assert_eq!(bad_test.status(), StatusCode::BAD_REQUEST);
    let result = body_json(bad_test).await;
    assert_eq!(
        result["success"],
        json!(false),
        "W6: the result shape at any status"
    );
    assert!(result["message"].is_string());

    let bad_provider =
        json!({ "type": "openai", "name": "x", "modelId": "m", "contextWindow": -1 });
    let refused = server
        .send(server.json(Method::PUT, "/providers/p1", &bad_provider))
        .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(refused).await,
        json!({ "error": "contextWindow must be a positive number" })
    );
    Ok(())
}

#[tokio::test]
async fn a_negative_tab_id_is_refused_and_a_large_attachment_is_not() -> TestResult {
    let server = TestServer::start().await?;
    let negative = server
        .send(
            server
                .request(Method::DELETE, "/chat/panels/-1")
                .body(Body::empty())?,
        )
        .await;
    assert_eq!(negative.status(), StatusCode::BAD_REQUEST, "W5");
    assert!(body_json(negative).await["error"].is_string());

    let image = format!("data:image/png;base64,{}", "A".repeat(3 * 1024 * 1024));
    let chat = json!({
        "conversationId": CONVERSATION,
        "target": { "type": "codex", "agentId": CONVERSATION },
        "attachments": [{ "mediaType": "image/png", "data": image }]
    });
    let response = server.send(server.json(Method::POST, "/chat", &chat)).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_IMPLEMENTED,
        "a 3 MiB attachment parses"
    );
    Ok(())
}

#[tokio::test]
async fn the_private_mcp_endpoint_is_outside_the_trust_rule() -> TestResult {
    let server = TestServer::start().await?;
    let request = Request::post(format!("/mcp/{CONVERSATION}"))
        .header(header::HOST, "127.0.0.1:9200")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
        ))?;
    let response = server.send(request).await;
    // Chunk `browser` mounts the endpoint; until then nothing answers. What matters here is
    // that a POST without `Origin` is not refused by the extension's trust rule.
    assert_ne!(response.status(), StatusCode::FORBIDDEN);
    Ok(())
}

/// The generated document, as the codegen and the extension see it.
fn document() -> Result<Value, serde_json::Error> {
    serde_json::to_value(browseros_server::http::openapi())
}

/// `true` when every key `schema` requires is in `value`, and every key of `value` is a
/// property the schema names.
fn matches_schema(document: &Value, schema_name: &str, value: &Value) -> bool {
    let schema = &document["components"]["schemas"][schema_name];
    let properties = schema["properties"].as_object();
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|keys| keys.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(object) = value.as_object() else {
        return false;
    };
    required.iter().all(|key| object.contains_key(*key))
        && properties
            .is_some_and(|properties| object.keys().all(|key| properties.contains_key(key)))
}

#[tokio::test]
async fn real_responses_match_the_generated_document() -> TestResult {
    let document = document()?;
    let server = TestServer::start().await?;
    for (path, schema) in [
        ("/system/health", "Health"),
        ("/status", "Status"),
        ("/system/diagnostics", "Diagnostics"),
    ] {
        let response = server.send(server.get(path)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            document["paths"][path]["get"]["responses"]["200"].is_object(),
            "{path} documents 200"
        );
        let body = body_json(response).await;
        assert!(
            matches_schema(&document, schema, &body),
            "{path} answered {body}, not a {schema}"
        );
    }
    let forbidden = server
        .send(
            Request::get("/conversations")
                .header(header::HOST, "127.0.0.1:9200")
                .header(header::ORIGIN, "https://example.com")
                .body(Body::empty())?,
        )
        .await;
    assert!(document["paths"]["/conversations"]["get"]["responses"]["403"].is_object());
    assert!(matches_schema(
        &document,
        "ApiError",
        &body_json(forbidden).await
    ));
    Ok(())
}

#[test]
fn the_document_has_every_route_and_lists_the_trust_refusal() -> Result<(), serde_json::Error> {
    let document = document()?;
    let paths = document["paths"].as_object().map(|paths| paths.len());
    assert_eq!(paths, Some(23));
    let mut operations = 0;
    for item in document["paths"]
        .as_object()
        .into_iter()
        .flat_map(|paths| paths.values())
    {
        for (method, operation) in item.as_object().into_iter().flatten() {
            if !["get", "put", "post", "delete", "patch"].contains(&method.as_str()) {
                continue;
            }
            operations += 1;
            let system = operation["tags"]
                .as_array()
                .is_some_and(|tags| tags.contains(&json!("system")));
            assert_eq!(
                operation["responses"]["403"].is_object(),
                !system,
                "{method} {} lists 403 exactly when it is behind the trust rule",
                operation["operationId"]
            );
        }
    }
    assert_eq!(operations, 28);
    assert!(
        document["paths"]["/health"].is_null(),
        "the alias is served, not documented"
    );
    for (schema, key) in [
        ("Diagnostics", "osVersion"),
        ("DefaultProvider", "provider"),
        ("AgentProbe", "reasoning"),
        ("AgentProbe", "agentInfo"),
        ("Provider", "baseUrl"),
    ] {
        let required = &document["components"]["schemas"][schema]["required"];
        assert!(
            required
                .as_array()
                .is_some_and(|keys| keys.contains(&json!(key))),
            "{schema}.{key} is always sent, so it is required (it may be null)"
        );
    }
    Ok(())
}
