use anyhow::Context;
use axum::Router;
use clap::Parser;
use claw_server_rust::{AppState, build_router, config::Cli, mcp::browser_mcp_service};
use rmcp::{serve_server, transport::stdio};
use std::{future::Future, io, net::SocketAddr, sync::Arc};
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
    serve(state, config, shutdown_rx).await
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
    state: AppState,
    config: Arc<claw_server_rust::config::Config>,
    shutdown_rx: oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    let heal_state = state.clone();
    serve_with_boot_task(build_router(state), config, shutdown_rx, async move {
        heal_boot_config(&heal_state).await
    })
    .await
}

/// Binds the HTTP listener before starting non-critical boot work in the background.
async fn serve_with_boot_task(
    app: Router,
    config: Arc<claw_server_rust::config::Config>,
    shutdown_rx: oneshot::Receiver<()>,
    boot_task: impl Future<Output = ()> + Send + 'static,
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
    tokio::spawn(boot_task);
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
    match state.harness.run_integrity_scan().await {
        Ok(outcome) => info!(
            verified = outcome.verified,
            drifted = outcome.drifted,
            missing = outcome.missing,
            healed = outcome.healed,
            failed = outcome.failed,
            "completed MCP config integrity scan"
        ),
        Err(err) => error!(error = %err, "MCP config integrity scan failed"),
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

#[cfg(test)]
mod tests {
    use super::serve_with_boot_task;
    use axum::Router;
    use claw_server_rust::config::Config;
    use std::{sync::Arc, time::Duration};
    use tempfile::tempdir;
    use tokio::{net::TcpStream, sync::oneshot};

    #[tokio::test]
    async fn listener_binds_while_boot_task_is_still_running() -> anyhow::Result<()> {
        let root = tempdir()?;
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = probe.local_addr()?.port();
        drop(probe);
        let config = Arc::new(Config {
            server_port: port,
            cdp_port: 49337,
            proxy_port: None,
            resources_dir: root.path().join("resources"),
            browserclaw_dir: root.path().to_path_buf(),
            claw_dir: root.path().to_path_buf(),
            session_idle: Duration::from_secs(300),
            session_sweep_interval: Duration::from_secs(60),
            screencast_screenshot_fallback: true,
            dev_mode: false,
            auth_token: None,
        });
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (boot_started_tx, boot_started_rx) = oneshot::channel();
        let release = Arc::new(tokio::sync::Notify::new());
        let boot_release = release.clone();
        let server = tokio::spawn(serve_with_boot_task(
            Router::new(),
            config,
            shutdown_rx,
            async move {
                let _ = boot_started_tx.send(());
                boot_release.notified().await;
            },
        ));

        tokio::time::timeout(Duration::from_secs(1), boot_started_rx).await??;
        let stream = TcpStream::connect(("127.0.0.1", port)).await?;
        drop(stream);
        release.notify_one();
        let _ = shutdown_tx.send(());
        server.await??;
        Ok(())
    }
}
