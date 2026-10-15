//! System responses, discovery ownership, and canonical installation identity never use real user state.
use axum::body::{Body, to_bytes};
use http::{Request, StatusCode};
use serde_json::{Value, json};
use server_rust::{
    AppState, VERSION,
    config::{Config, EnvConfig},
    identity::Identity,
    paths::BrowserosPaths,
    server_file::ServerFile,
};
use tower::ServiceExt;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
fn config(dir: &std::path::Path) -> Result<Config> {
    let path = dir.join("config.json");
    std::fs::write(
        &path,
        r#"{"ports":{"server":9200,"cdp":9100},"directories":{"resources":".","execution":"."},"instance":{"install_id":"sidecar-is-not-authoritative","browseros_version":"0.51.1.3"}}"#,
    )?;
    Ok(Config::load(
        &path,
        &EnvConfig {
            browseros_dir: Some(dir.join("data")),
            ..Default::default()
        },
    )?)
}

#[tokio::test]
async fn health_status_aliases_and_shutdown() -> Result {
    let dir = tempfile::tempdir()?;
    let state = AppState::start(config(dir.path())?).await?;
    let router = server_rust::build_router(state.clone());
    for path in [
        "/health",
        "/system/health",
        "/status",
        "/system/diagnostics",
    ] {
        let response = router
            .clone()
            .oneshot(Request::get(path).body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
        if path == "/system/diagnostics" {
            assert_eq!(body["version"], VERSION);
            assert!(body["os"].is_string());
            assert_eq!(body.as_object().ok_or("not an object")?.len(), 3);
        } else {
            assert_eq!(body["status"], "ok");
            assert!(body["cdpConnected"].is_boolean());
        }
    }
    let guard = state.activity.run_started();
    let response = router
        .clone()
        .oneshot(Request::get("/status").body(Body::empty())?)
        .await?;
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
    assert_eq!(body["can_update"], false);
    drop(guard);
    let guard = state.activity.mcp_post_started();
    assert!(!state.activity.can_update());
    drop(guard);
    assert!(state.activity.can_update());
    for method in ["GET", "HEAD", "PUT"] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/health/")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = router
        .clone()
        .oneshot(Request::post("/shutdown").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await?.as_ref(),
        br#"{"status":"ok"}"#
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        state.shutdown.triggered(),
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn installation_identity_is_race_safe_and_corruption_is_preserved() -> Result {
    let dir = tempfile::tempdir()?;
    let config = config(dir.path())?;
    let paths = BrowserosPaths::new(config.browseros_dir.clone());
    let (one, two) = tokio::join!(
        Identity::load(&config, &paths),
        Identity::load(&config, &paths)
    );
    let one = one?;
    assert_eq!(one, two?);
    assert_ne!(one.browseros_id, "sidecar-is-not-authoritative");
    assert_eq!(one.install_id.as_ref(), Some(&one.browseros_id));
    let stored = std::fs::read(&paths.installation_file)?;
    assert_eq!(Identity::load(&config, &paths).await?, one);
    assert_eq!(std::fs::read(&paths.installation_file)?, stored);
    std::fs::write(&paths.installation_file, "broken")?;
    let ephemeral = Identity::load(&config, &paths).await?;
    assert!(ephemeral.install_id.is_none());
    assert_ne!(ephemeral.browseros_id, one.browseros_id);
    assert_eq!(std::fs::read_to_string(&paths.installation_file)?, "broken");
    Ok(())
}

#[tokio::test]
async fn discovery_shape_and_cleanup_ownership() -> Result {
    let dir = tempfile::tempdir()?;
    let config = config(dir.path())?;
    let paths = BrowserosPaths::new(config.browseros_dir.clone());
    let identity = Identity::load(&config, &paths).await?;
    let publication = ServerFile::with_config(&config, &identity);
    publication.write(config.server_port).await?;
    let body: Value = serde_json::from_slice(&std::fs::read(publication.path())?)?;
    assert_eq!(
        body,
        json!({"server_port":9200,"cdp_port":9100,"url":"http://127.0.0.1:9200","server_version":VERSION,"browseros_version":"0.51.1.3","browseros_id":identity.browseros_id})
    );
    std::fs::write(publication.path(), r#"{"replacement":true}"#)?;
    publication.remove().await;
    assert!(
        publication.path().exists(),
        "must not remove another process's discovery"
    );
    publication.write(config.server_port).await?;
    publication.remove().await;
    assert!(!publication.path().exists());
    Ok(())
}
