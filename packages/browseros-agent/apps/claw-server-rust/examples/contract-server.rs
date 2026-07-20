//! Contract-testable claw-server-rust: the full router over real app
//! state seeded with one live and one ended session, no browser or CDP
//! required. The cross-server contract suite
//! (`contracts/claw-api/tests`) builds and spawns this binary to run
//! the shared cases against the Rust implementation; it can also be run
//! by hand: `cargo run --example contract-server <port> <data-dir>`.

use axum::Router;
use browseros_core::TargetId;
use claw_server_rust::{
    AppRuntime, AppState, build_router,
    capture::audit::{DispatchResultSummary, RecordToolDispatchInput},
    config::Config,
    identity::{ClientIdentity, ConversationIdentity},
    ids::{DispatchId, SessionId},
    sessions::Session,
    tabs::activity::{RecordToolInput, ScreencastFrame},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (port, root) = arguments()?;
    let config = Arc::new(Config {
        server_port: port,
        cdp_port: 49_337,
        proxy_port: None,
        resources_dir: root.join("resources"),
        browserclaw_dir: root.join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        screencast_screenshot_fallback: true,
        dev_mode: false,
        auth_token: None,
    });
    let state = AppState::new_with_home(config, root.join("home")).await?;
    seed(&state).await?;

    let runtime = AppRuntime::start(state);
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    serve(runtime.state(), listener).await?;
    runtime.shutdown().await?;
    Ok(())
}

fn arguments() -> anyhow::Result<(u16, PathBuf)> {
    let mut args = std::env::args().skip(1);
    let port = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing port"))?
        .parse()?;
    let root = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("missing data directory"))?;
    Ok((port, root))
}

async fn seed(state: &AppState) -> anyhow::Result<()> {
    let live = Session::new(
        SessionId::new("session-live"),
        ClientIdentity::Ephemeral {
            slug: "codex".to_string(),
            label: "Codex".to_string(),
        },
        ConversationIdentity::new("codex", "Research BrowserClaw".to_string()),
        tokio::time::Instant::now(),
    );
    state.sessions.insert_for_testing(live).await;

    state
        .audit
        .record_session_start(
            "session-live",
            "codex-research-browserclaw",
            "codex",
            "Codex",
            "Codex",
            "1.0",
        )
        .await?;
    let dispatch_id = seed_dispatch(state, "session-live", 7, "target-7").await?;
    state.audit.mark_screenshot(dispatch_id).await?;
    state
        .audit
        .record_session_start(
            "session-ended",
            "codex-ended",
            "codex",
            "Codex",
            "Codex",
            "1.0",
        )
        .await?;
    seed_dispatch(state, "session-ended", 8, "target-8").await?;
    state
        .audit
        .record_session_end("session-ended", "closed", Some("fixture"))
        .await?;

    state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-7".to_string()),
            tab_id: 101,
            page_id: 7,
            session_id: "session-live".to_string(),
            agent_id: "codex-research-browserclaw".to_string(),
            slug: "codex".to_string(),
            tool_name: "snapshot".to_string(),
        })
        .await;
    state
        .audit
        .claim_target_for_session("target-7", "session-live", "codex-research-browserclaw", 0)
        .await?;
    state
        .screencast
        .cache_frame(
            7,
            ScreencastFrame {
                jpeg_base64: "/9g=".to_string(),
                captured_at: 123,
            },
        )
        .await;
    state
        .screenshots
        .write(&dispatch_id.to_string(), &[0xff, 0xd8])
        .await?;
    Ok(())
}

async fn seed_dispatch(
    state: &AppState,
    session_id: &str,
    page_id: i64,
    target_id: &str,
) -> anyhow::Result<i64> {
    Ok(state
        .audit
        .record_tool_dispatch(RecordToolDispatchInput {
            agent_id: format!("codex-{session_id}"),
            slug: "codex".to_string(),
            agent_label: "Codex".to_string(),
            session_id: session_id.to_string(),
            tool_name: "snapshot".to_string(),
            page_id: Some(page_id),
            target_id: Some(target_id.to_string()),
            url: Some("https://browseros.com".to_string()),
            title: Some("BrowserOS".to_string()),
            raw_args: json!({}),
            duration_ms: 5,
            dispatch_id: DispatchId::new(),
            result: DispatchResultSummary {
                is_error: false,
                structured_content: json!({}),
                content: json!([]),
            },
        })
        .await?)
}

async fn serve(state: AppState, listener: TcpListener) -> anyhow::Result<()> {
    let shutdown = state.shutdown.clone();
    let app: Router = build_router(state);
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(async move {
            shutdown.requested().await;
        })
        .await?;
    Ok(())
}
