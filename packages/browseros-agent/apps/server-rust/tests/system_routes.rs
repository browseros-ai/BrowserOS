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
    let guard = state.activity.reserve();
    let response = router
        .clone()
        .oneshot(Request::get("/status").body(Body::empty())?)
        .await?;
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
    assert_eq!(body["can_update"], false);
    drop(guard);
    let guard = state.activity.reserve();
    assert!(!state.activity.can_update());
    let overlapping_run = state.activity.reserve();
    drop(guard);
    assert!(
        !state.activity.can_update(),
        "handoff leaves the overlapping chat reservation active"
    );
    drop(overlapping_run);
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
    let replacement = publication.path().with_file_name("replacement.json");
    std::fs::copy(publication.path(), &replacement)?;
    std::fs::rename(&replacement, publication.path())?;
    publication.remove().await;
    assert!(
        publication.path().exists(),
        "identical bytes from another publisher still belong to it"
    );
    publication.write(config.server_port).await?;
    publication.remove().await;
    assert!(!publication.path().exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn private_paths_reject_symlinks_and_leave_unrelated_state_alone() -> Result {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("data");
    let paths = BrowserosPaths::new(root.clone());
    paths.prepare().await?;
    assert_eq!(
        std::fs::metadata(&paths.tool_output_dir)?
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let config = config(dir.path())?;
    Identity::load(&config, &paths).await?;
    assert_eq!(
        std::fs::metadata(&paths.installation_file)?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let foreign = dir.path().join("foreign");
    std::fs::create_dir(&foreign)?;
    std::fs::write(foreign.join("keep"), "owned elsewhere")?;
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(31 * 24 * 60 * 60);
    std::fs::File::open(&foreign)?.set_times(std::fs::FileTimes::new().set_modified(old))?;
    symlink(&foreign, root.join("sessions/link"))?;
    let legacy = root.join("sessions/old");
    std::fs::create_dir(&legacy)?;
    std::fs::File::open(&legacy)?.set_times(std::fs::FileTimes::new().set_modified(old))?;
    paths.prepare().await?;
    assert!(!legacy.exists());
    assert!(foreign.join("keep").exists());
    assert!(root.join("sessions/link").is_symlink());
    std::fs::rename(root.join("sessions"), root.join("sessions-original"))?;
    symlink(&foreign, root.join("sessions"))?;
    let foreign_old = foreign.join("old");
    std::fs::create_dir(&foreign_old)?;
    std::fs::File::open(&foreign_old)?.set_times(std::fs::FileTimes::new().set_modified(old))?;
    paths.prepare().await?;
    assert!(
        foreign_old.exists(),
        "a symlinked sessions root does not transfer ownership"
    );
    std::fs::remove_dir(&paths.tool_output_dir)?;
    symlink(&foreign, &paths.tool_output_dir)?;
    assert!(paths.prepare().await.is_err());
    assert_eq!(
        std::fs::read_to_string(foreign.join("keep"))?,
        "owned elsewhere"
    );
    Ok(())
}

#[test]
fn linux_diagnostics_parser_handles_quotes_limits_and_missing_values() {
    use server_rust::http::system::parse_os_release;
    assert_eq!(
        parse_os_release("NAME=\"Debian GNU/Linux\"\nVERSION_ID='13'\nHOME=private"),
        ("Debian GNU/Linux".into(), Some("13".into()))
    );
    assert_eq!(parse_os_release(""), ("Linux".into(), None));
    assert_eq!(parse_os_release("NAME='"), ("'".into(), None));
    assert_eq!(
        parse_os_release(&format!("NAME={}", "x".repeat(210)))
            .0
            .len(),
        200
    );
}
