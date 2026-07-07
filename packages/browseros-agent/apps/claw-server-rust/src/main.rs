use anyhow::Context;
use axum::Router;
use clap::Parser;
use claw_server_rust::{AppState, build_router, config::Cli, mcp::browser_mcp_service};
use rmcp::{serve_server, transport::stdio};
use std::{io, net::SocketAddr, sync::Arc};
use tokio::{net::TcpListener, sync::oneshot};
use tracing::{error, info};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Arc::new(claw_server_rust::config::Config::load(&cli.config)?);
    let _guard = init_tracing(config.clone())?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let state = AppState::new(config.clone(), Some(shutdown_tx)).await?;
    state.browser.start();
    state
        .screencast
        .clone()
        .start(state.browser.clone(), state.tab_activity.clone());
    state.sessions.clone().spawn_idle_sweeper();
    if cli.stdio {
        return serve_stdio(state).await;
    }
    spawn_signal_shutdown(state.clone());
    heal_boot_config(&state).await;
    serve(build_router(state), config, shutdown_rx).await
}

fn init_tracing(config: Arc<claw_server_rust::config::Config>) -> anyhow::Result<WorkerGuard> {
    std::fs::create_dir_all(config.claw_dir.join("logs")).with_context(|| {
        format!(
            "failed to create log directory {}",
            config.claw_dir.join("logs").display()
        )
    })?;
    let file_appender =
        tracing_appender::rolling::daily(config.claw_dir.join("logs"), "claw-server.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let env_filter = EnvFilter::try_from_env("CLAW_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(file_writer),
        )
        .try_init()
        .context("failed to initialize tracing subscriber")?;
    Ok(guard)
}

async fn serve(
    app: Router,
    config: Arc<claw_server_rust::config::Config>,
    shutdown_rx: oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], config.server_port));
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            anyhow::bail!(
                "claw-server singleton is already running on 127.0.0.1:{}",
                config.server_port
            );
        }
        Err(err) => return Err(err).context("failed to bind claw-server listener"),
    };
    info!(%addr, "claw-server-rust listening");
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await
        .context("claw-server listener failed")
}

async fn serve_stdio(state: AppState) -> anyhow::Result<()> {
    let running = serve_server(browser_mcp_service(state.clone()), stdio())
        .await
        .context("failed to start stdio MCP server")?;
    running.waiting().await.context("stdio MCP server failed")?;
    state.sessions.shutdown().await?;
    state.screencast.stop();
    state.browser.stop();
    Ok(())
}

async fn heal_boot_config(state: &AppState) {
    match state.harness.heal_claude_code_http_tags().await {
        Ok(changed) if changed > 0 => info!(changed, "healed Claude Code MCP transport tags"),
        Ok(_) => {}
        Err(err) => error!(error = %err, "Claude Code MCP transport heal failed"),
    }
}

fn spawn_signal_shutdown(state: AppState) {
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        match state.sessions.shutdown().await {
            Ok(drained) => info!(drained, "drained sessions after shutdown signal"),
            Err(err) => error!(error = %err, "session drain after shutdown signal failed"),
        }
        state.screencast.stop();
        state.browser.stop();
        if let Some(tx) = state.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
    });
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let ctrl_c = tokio::signal::ctrl_c();
    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                _ = ctrl_c => {}
                _ = terminate.recv() => {}
            }
        }
        Err(err) => {
            error!(error = %err, "failed to install SIGTERM handler");
            let _ = ctrl_c.await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
