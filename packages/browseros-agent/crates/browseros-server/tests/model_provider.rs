//! Drives the connection-test route through the real router and provider transport.

#[path = "support/fake_provider.rs"]
mod fake_provider;
mod support;

use browseros_server::model::Models;
use http::{HeaderMap, Method, StatusCode};
use serde_json::json;
use tokio::net::TcpSocket;

use fake_provider::{FakeProvider, Reply};
use support::{TestServer, body_json};

#[tokio::test]
async fn probe_reports_only_the_host_when_connection_is_refused() -> anyhow::Result<()> {
    // Keep the port reserved without listening so another process cannot claim it.
    let socket = TcpSocket::new_v4()?;
    socket.bind("127.0.0.1:0".parse()?)?;
    let mut server = TestServer::start().await?;
    server.app.models = Models::new(server.app.store.clone(), None);
    server.router = browseros_server::http::router(server.app.clone());
    let body = json!({
        "provider": "openai-compatible", "model": "fixture",
        "baseUrl": format!("http://127.0.0.1:{}/fixture-secret-path", socket.local_addr()?.port()),
        "apiKey": "fixture-secret-key",
    });

    let response = server
        .send(server.json(Method::POST, "/test-provider", &body))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(response).await,
        json!({"success":false,"message":"Could not connect to the provider at 127.0.0.1."})
    );
    Ok(())
}

#[tokio::test]
async fn probe_uses_the_saved_destination_and_makes_one_attempt() -> anyhow::Result<()> {
    let fake = FakeProvider::new(vec![
        Reply::events(&[
            json!({"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}),
        ]),
        Reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            HeaderMap::new(),
            json!({"error":{"message":"Fixture unavailable"}}).to_string(),
        ),
    ])
    .await?;
    let mut server = TestServer::start().await?;
    server.app.models = Models::new(server.app.store.clone(), None);
    server.router = browseros_server::http::router(server.app.clone());
    let saved = server.send(server.json(Method::PUT, "/providers/fixture", &json!({
        "type": "openai-compatible", "name": "Fixture", "modelId": "fixture", "contextWindow": 8000,
        "baseUrl": fake.base, "apiKey": "fixture-saved-key",
    }))).await;
    assert!(saved.status().is_success());
    let body = json!({
        "providerId": "fixture", "provider": "openai-compatible", "model": "fixture",
        "baseUrl": "http://127.0.0.1:1", "apiKey": " ",
        "headers": {"x-conversation": "{{conversationId}}"},
    });
    let ok = server
        .send(server.json(Method::POST, "/test-provider", &body))
        .await;
    assert_eq!(ok.status(), StatusCode::OK);
    let ok = body_json(ok).await;
    assert_eq!(ok["success"], true);
    assert!(ok["responseTime"].is_u64());

    let failure = server
        .send(server.json(Method::POST, "/test-provider", &body))
        .await;
    assert_eq!(failure.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(failure).await,
        json!({"success":false,"message":"Fixture unavailable"})
    );
    let calls = fake
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    assert_eq!(calls.len(), 2);
    for call in &*calls {
        assert_eq!(call.uri.path(), "/chat/completions");
        assert_eq!(call.headers["authorization"], "Bearer fixture-saved-key");
        uuid::Uuid::parse_str(call.headers["x-conversation"].to_str()?)?;
        assert_eq!(
            call.body["messages"],
            json!([{"role":"user","content":"Respond with exactly: 'ok'"}])
        );
        assert!(call.body.get("tools").is_none());
    }
    assert_ne!(
        calls[0].headers["x-conversation"],
        calls[1].headers["x-conversation"]
    );
    Ok(())
}
