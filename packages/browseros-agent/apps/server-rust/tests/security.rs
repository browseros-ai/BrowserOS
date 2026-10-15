//! HTTP compatibility checks against the TS middleware ordering and Hono/zod error dialects.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    routing::post,
};
use http::{Request, StatusCode};
use serde_json::{Value, json};
use server_rust::{
    AppState,
    config::{Config, EnvConfig},
    http::extract::{ZodJson, ZodUuid},
};
use std::{net::SocketAddr, path::Path};
use tower::ServiceExt;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

async fn state(dir: &Path) -> Result<AppState> {
    let path = dir.join("config.json");
    std::fs::write(
        &path,
        r#"{"ports":{"server":9200,"cdp":9100},"directories":{"resources":".","execution":"."}}"#,
    )?;
    Ok(AppState::start(Config::load(
        &path,
        &EnvConfig {
            browseros_dir: Some(dir.join("data")),
            trusted_origins: vec![
                "http://localhost:3000/path".into(),
                "moz-extension://test/page".into(),
                "https://example.com/path".into(),
                "invalid".into(),
            ],
            ..Default::default()
        },
    )?)
    .await?)
}

async fn call(
    router: Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    peer: &str,
) -> Result<(StatusCode, http::HeaderMap, Value)> {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut request = request.body(Body::empty())?;
    request
        .extensions_mut()
        .insert(ConnectInfo(peer.parse::<SocketAddr>()?));
    let response = router.oneshot(request).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let body =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
    Ok((status, headers, body))
}

#[tokio::test]
async fn origin_app_and_browser_guards_are_distinct() -> Result {
    let dir = tempfile::tempdir()?;
    let router = server_rust::build_router(state(dir.path()).await?);
    let local = "127.0.0.1:40000";
    let trusted = "chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk";
    let (status, _, body) = call(
        router.clone(),
        "GET",
        "/health",
        &[("Origin", "https://evil.test")],
        local,
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body,
        json!({"error":{"name":"ForbiddenOrigin","message":"Origin not allowed","code":"FORBIDDEN_ORIGIN","statusCode":403}})
    );
    // GET without Origin passes the app guard only with genuine socket and Host evidence.
    for (method, headers, peer, forbidden) in [
        ("GET", vec![("Host", "localhost:9200")], local, false),
        ("POST", vec![("Host", "localhost:9200")], local, true),
        (
            "POST",
            vec![("Host", "localhost:9200"), ("Origin", trusted)],
            local,
            false,
        ),
        (
            "GET",
            vec![("Host", "localhost:9200")],
            "192.0.2.1:10",
            true,
        ),
        ("GET", vec![("Host", "[::1]:9200")], "[::1]:10", true),
        (
            "GET",
            vec![("Host", "localhost:9200")],
            "[::ffff:127.0.0.1]:10",
            false,
        ),
        (
            "GET",
            vec![
                ("Host", "localhost"),
                ("Referer", "chrome-extension://test/page"),
            ],
            local,
            true,
        ),
        (
            "GET",
            vec![("Host", "localhost"), ("Referer", "http://localhost/page")],
            local,
            false,
        ),
        (
            "GET",
            vec![("Host", "localhost"), ("Origin", "https://example.com")],
            local,
            true,
        ),
        (
            "GET",
            vec![("Host", "localhost"), ("Origin", "http://localhost:3000")],
            local,
            false,
        ),
        (
            "GET",
            vec![("Host", "localhost"), ("Origin", "moz-extension://test")],
            local,
            false,
        ),
        (
            "GET",
            vec![
                ("Host", "localhost"),
                ("Origin", "chrome-extension://other"),
            ],
            local,
            true,
        ),
    ] {
        let (status, _, body) = call(router.clone(), method, "/agents", &headers, peer).await?;
        assert_eq!(
            status == StatusCode::FORBIDDEN,
            forbidden,
            "{method} {headers:?} {peer}: {body}"
        );
    }
    // Prefix guards also cover unregistered descendants; public paths do not acquire app auth.
    assert_eq!(
        call(router.clone(), "POST", "/agents/missing/deep", &[], local)
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(router.clone(), "GET", "/health", &[], "192.0.2.1:10")
            .await?
            .0,
        StatusCode::OK
    );
    let (status, _, body) = call(
        router.clone(),
        "GET",
        "/mcp",
        &[("Sec-Fetch-Site", "")],
        local,
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body,
        json!({"error":{"name":"ForbiddenBrowserRequest","message":"Browser requests are not allowed on this endpoint","code":"FORBIDDEN_BROWSER_REQUEST","statusCode":403}})
    );
    assert_ne!(
        call(router, "GET", "/mcp", &[("Host", "lan.test:9200")], local)
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    Ok(())
}

#[tokio::test]
async fn cors_runs_before_security_and_keeps_the_missing_put() -> Result {
    let dir = tempfile::tempdir()?;
    let router = server_rust::build_router(state(dir.path()).await?);
    for origin in ["http://localhost:3000", "https://evil.test"] {
        let (status, headers, body) = call(
            router.clone(),
            "OPTIONS",
            "/agents",
            &[
                ("Origin", origin),
                ("Access-Control-Request-Method", "PUT"),
                ("Access-Control-Request-Headers", "Content-Type"),
            ],
            "192.0.2.1:10",
        )
        .await?;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        assert_eq!(
            headers["access-control-allow-methods"],
            "GET,POST,DELETE,OPTIONS"
        );
        assert_eq!(
            headers["access-control-allow-headers"],
            "Content-Type,Authorization,Accept"
        );
        assert_eq!(headers["access-control-allow-credentials"], "true");
        assert_eq!(
            headers
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            (origin != "https://evil.test").then_some(origin)
        );
    }
    Ok(())
}

#[tokio::test]
async fn json_body_compatibility_and_strict_uuid() -> Result {
    let router = Router::new()
        .route(
            "/body",
            post(|ZodJson(body): ZodJson<Value>| async move { axum::Json(body) }),
        )
        .route(
            "/uuid/{id}",
            post(|ZodUuid(id): ZodUuid| async move { id.to_string() }),
        );
    for content_type in [
        None,
        Some("text/plain"),
        Some("application/json"),
        Some("application/merge-patch+json; charset=utf-8"),
    ] {
        let mut request = Request::post("/body");
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::from(r#"{"sent":true}"#))?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let actual: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
        assert_eq!(
            actual,
            if content_type.is_none() || content_type == Some("text/plain") {
                json!({})
            } else {
                json!({"sent":true})
            }
        );
    }
    let response = router
        .clone()
        .oneshot(
            Request::post("/body")
                .header("content-type", "application/json")
                .body(Body::from("{"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
    assert_eq!(
        body,
        json!({"error":{"code":"unknown","title":"Something went wrong","message":"Malformed JSON in request body","retryable":true,"details":"Malformed JSON in request body","name":"InternalServerError","statusCode":500}})
    );
    for (uuid, valid) in [
        ("00000000-0000-0000-0000-000000000000", true),
        ("ffffffff-ffff-ffff-ffff-ffffffffffff", true),
        ("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF", false),
        ("00000000-0000-8000-A000-000000000000", true),
        ("00000000-0000-9000-8000-000000000000", false),
        ("00000000-0000-4000-7000-000000000000", false),
        ("00000000000040008000000000000000", false),
    ] {
        let response = router
            .clone()
            .oneshot(Request::post(format!("/uuid/{uuid}")).body(Body::empty())?)
            .await?;
        assert_eq!(
            response.status(),
            if valid {
                StatusCode::OK
            } else {
                StatusCode::BAD_REQUEST
            },
            "{uuid}"
        );
        if !valid {
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
            assert_eq!(body["error"]["name"], "ZodError");
            let issues: Value =
                serde_json::from_str(body["error"]["message"].as_str().ok_or("missing issues")?)?;
            assert_eq!(issues[0]["format"], "uuid");
            assert_eq!(issues[0]["path"], json!(["id"]));
        }
    }
    Ok(())
}
