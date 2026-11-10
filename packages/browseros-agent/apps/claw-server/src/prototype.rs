//! Minimal boot path inside the shipped sidecar binary. The flag is checked before
//! AppState/AppRuntime exist, so legacy sessions, effects, recorders, cleanup jobs and
//! analytics cannot observe or change the prototype's tabs. The reusable Neo server owns
//! the MCP route, runtime and result lifecycle; this host adds system endpoints and skill installation.

use std::{future::Future, net::SocketAddr, sync::Arc};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use claw_api::models::{
    AgentSettings, BrowserLink, HealthResponse, ShutdownResponse, SystemInfo,
    UpdateAgentSettingsRequest,
};
use neo_http::NeoCapture;
use neo_sdk_server::{Config as NeoConfig, NeoSdkServer};
use rmcp::{serve_server, transport::stdio};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::{
    VERSION,
    analytics::NoopAnalyticsSink,
    api::mcp::neo_captcha,
    config::Config,
    services::{harness::HarnessService, harness_skills::load_playwright_skill, runtime_file},
};

pub const FLAG: &str = "BROWSERCLAW_PLAYWRIGHT_RUN";

/// Exact `1`, matching the hosted-mode switch. Read only at process boot; all work after
/// selection receives the selected service rather than consulting mutable environment.
#[must_use]
pub fn enabled() -> bool {
    std::env::var(FLAG).is_ok_and(|value| value == "1")
}

#[derive(Clone)]
struct StateData {
    config: Arc<Config>,
    service: NeoSdkServer,
    shutdown: CancellationToken,
}

/// Runs the prototype without constructing the legacy app. The caller owns OS signals and
/// port-conflict exit semantics; this function owns its MCP requests and browser connection.
pub async fn serve(
    config: Arc<Config>,
    stdio_mode: bool,
    signal: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();
    let service = NeoSdkServer::new(
        NeoConfig::new(config.cdp_port, config.browserclaw_dir.clone()),
        shutdown.clone(),
    );
    // Host policy stays in Claw; the reusable server binds this capability to its own
    // connection and retains the runtime's cancellation/input cleanup through shutdown.
    let service = neo_captcha::activation(
        |name| std::env::var(name).ok(),
        config.mode,
        &config.browserclaw_dir.join("artifacts"),
    )
    .install(service);
    if stdio_mode {
        let running = serve_server(service.clone(), stdio()).await?;
        let cancellation = running.cancellation_token();
        tokio::select! {
            result = running.waiting() => { result?; }
            () = signal => { cancellation.cancel(); }
        }
        shutdown.cancel();
        service.shutdown().await;
        return Ok(());
    }

    // Claim the port before any skill writes, preserving the installed sidecar's singleton
    // rule. A losing launch must not replace the live process's instructions.
    let address = SocketAddr::from(([127, 0, 0, 1], config.server_port));
    let listener = TcpListener::bind(address).await?;
    let bound = listener.local_addr()?;
    let state = StateData {
        config: config.clone(),
        service: service.clone(),
        shutdown: shutdown.clone(),
    };
    // Capture owns independent CDP sockets and is HTTP-only. Its presence sessions
    // outlive requests and close after the MCP runtime drains below.
    let capture = NeoCapture::start(config.cdp_port, shutdown.child_token());
    let app = Router::new()
        .route("/system/health", get(health))
        .route("/system/ready", get(ready))
        .route("/system/shutdown", post(stop))
        .route("/api/v1/system", get(info))
        // Docker's entrypoint disables human help after checking readiness. Keep that
        // handshake honest: effects stay disabled and enabling one is refused.
        .route("/api/v1/settings/agent", get(settings).put(update_settings))
        .with_state(state)
        .merge(service.router())
        .merge(capture.router());
    let runtime_url = config.proxy_port.map_or_else(
        || format!("http://{bound}"),
        |port| format!("http://127.0.0.1:{port}"),
    );
    runtime_file::write(&config.browserclaw_dir, &runtime_url).await;
    let skill_config = config.clone();
    let skill_task = tokio::spawn(async move {
        if !skill_config.dev_mode
            && let Err(error) = install_skill(&skill_config).await
        {
            tracing::warn!(%error, "prototype Playwright skill installation failed");
        }
    });
    tracing::info!(%bound, "Playwright prototype listening; legacy effects and background jobs are disabled");
    let stop_token = shutdown;
    let closing = service.clone();
    let result = axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(async move {
            tokio::select! {
                () = signal => {}
                () = stop_token.cancelled() => {}
            }
            // This runs while axum still serves existing responses. Keep capture on the
            // run-shutdown token: it ends here; MCP transport ends after replies are drained.
            closing.close_transport().await;
        })
        .await;
    service.shutdown().await;
    // Runs have drained. Only now may presence detach clear page focus.
    capture.close().await;
    skill_task
        .await
        .context("prototype skill installer task failed")?;
    result.context("prototype HTTP listener failed")
}

async fn install_skill(config: &Config) -> anyhow::Result<()> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .context("HOME or USERPROFILE is required for skill installation")?;
    let harness = HarnessService::new_with_managed_skill(
        config.browserclaw_dir.join("mcp-manager"),
        config.browserclaw_dir.join("harness-integrations"),
        home.into(),
        load_playwright_skill()?,
        Arc::new(NoopAnalyticsSink),
    );
    // Same managed skill identity and ownership checks as legacy boot. Restarting with
    // the flag off lets the normal reconciler restore the old guide.
    harness.run_skill_reconciliation().await?;
    Ok(())
}

fn health_report(connected: bool) -> HealthResponse {
    HealthResponse {
        browser: Some(Box::new(BrowserLink {
            connected,
            down_for_ms: None,
            last_error: None,
        })),
        ..HealthResponse::default()
    }
}

async fn health(State(state): State<StateData>) -> Json<HealthResponse> {
    Json(health_report(state.service.connected()))
}

async fn ready(State(state): State<StateData>) -> (StatusCode, Json<HealthResponse>) {
    let connected = state.service.ready().await;
    (
        if connected {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(health_report(connected)),
    )
}

async fn stop(State(state): State<StateData>) -> Json<ShutdownResponse> {
    state.shutdown.cancel();
    Json(ShutdownResponse::default())
}

async fn info(State(state): State<StateData>) -> Json<SystemInfo> {
    Json(SystemInfo::new(
        "BrowserOS neo (Playwright prototype)".to_string(),
        VERSION.to_string(),
        state.config.local_server_url(),
    ))
}

async fn settings() -> Json<AgentSettings> {
    Json(AgentSettings::new(false, false))
}

async fn update_settings(
    Json(request): Json<UpdateAgentSettingsRequest>,
) -> Result<Json<AgentSettings>, (StatusCode, Json<Value>)> {
    if request.human_help_enabled == Some(true) || request.task_cleanup_tip_enabled == Some(true) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Legacy effects are disabled in Playwright prototype mode"})),
        ));
    }
    Ok(settings().await)
}
