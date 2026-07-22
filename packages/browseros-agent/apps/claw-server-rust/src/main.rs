use anyhow::Context;
use axum::Router;
use claw_server_rust::{
    AppRuntime, AppState, ShutdownHandle, VERSION,
    api::mcp::browser_mcp_service,
    build_router,
    config::{Cli, CliAction},
};
use rmcp::{serve_server, transport::stdio};
use std::{
    future::Future,
    io::{self, Write},
    net::SocketAddr,
    sync::Arc,
};
use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (config_path, stdio_mode) = match Cli::parse_action() {
        CliAction::Version => {
            writeln!(io::stdout().lock(), "{VERSION}")?;
            return Ok(());
        }
        CliAction::Run { config, stdio } => (config, stdio),
    };
    let config = Arc::new(claw_server_rust::config::Config::load(config_path)?);
    let _guard = init_tracing(config.clone())?;
    let state = AppState::new(config.clone()).await?;
    let mut runtime = AppRuntime::start(state);
    let run_result = run(&mut runtime, config, stdio_mode).await;
    let shutdown_result = runtime.shutdown().await;
    match (run_result, shutdown_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error.into()),
        (Err(run_error), Err(shutdown_error)) => {
            error!(error = %shutdown_error, "application teardown failed after server error");
            Err(run_error)
        }
    }
}

async fn run(
    runtime: &mut AppRuntime,
    config: Arc<claw_server_rust::config::Config>,
    stdio_mode: bool,
) -> anyhow::Result<()> {
    let state = runtime.state();
    state.browser.wait_for_initial_attempt().await;
    let initial_browser = state.browser.state();
    if initial_browser.connected && !state.tab_registry.is_ready(initial_browser.epoch) {
        anyhow::bail!("failed to seed tab target identities before server startup");
    }
    if stdio_mode {
        return serve_stdio(state).await;
    }
    serve(runtime, config).await
}

fn init_tracing(config: Arc<claw_server_rust::config::Config>) -> anyhow::Result<WorkerGuard> {
    std::fs::create_dir_all(config.browserclaw_dir.join("logs")).with_context(|| {
        format!(
            "failed to create log directory {}",
            config.browserclaw_dir.join("logs").display()
        )
    })?;
    let file_appender =
        tracing_appender::rolling::daily(config.browserclaw_dir.join("logs"), "claw-server.log");
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
    runtime: &mut AppRuntime,
    config: Arc<claw_server_rust::config::Config>,
) -> anyhow::Result<()> {
    let state = runtime.state();
    let heal_state = state.clone();
    serve_with_boot_task(runtime, build_router(state), config, async move {
        heal_boot_config(&heal_state).await
    })
    .await
}

/// Binds the HTTP listener before starting non-critical boot work in the background.
async fn serve_with_boot_task(
    runtime: &mut AppRuntime,
    app: Router,
    config: Arc<claw_server_rust::config::Config>,
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
    let shutdown = runtime.state().shutdown;
    runtime.spawn_task("MCP config integrity scan", boot_task);
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await
        .context("claw-server listener failed")
}

async fn serve_stdio(state: AppState) -> anyhow::Result<()> {
    let running = serve_server(browser_mcp_service(state.clone()), stdio())
        .await
        .context("failed to start stdio MCP server")?;
    running.waiting().await.context("stdio MCP server failed")?;
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

async fn wait_for_shutdown(shutdown: ShutdownHandle) {
    tokio::select! {
        () = shutdown.requested() => {}
        () = wait_for_shutdown_signal() => shutdown.request(),
    }
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
    use claw_server_rust::{AppRuntime, AppState, config::Config};
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
            session_idle: Duration::from_secs(300),
            session_retention: Duration::from_secs(7_200),
            session_sweep_interval: Duration::from_secs(60),
            replay_retention_days: 7,
            dev_mode: false,
            auth_token: None,
        });
        let state = AppState::new_with_home(config.clone(), root.path().join("home")).await?;
        let shutdown = state.shutdown.clone();
        let mut runtime = AppRuntime::start(state);
        let (boot_started_tx, boot_started_rx) = oneshot::channel();
        let release = Arc::new(tokio::sync::Notify::new());
        let boot_release = release.clone();
        let client = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(1), boot_started_rx).await??;
            let stream = TcpStream::connect(("127.0.0.1", port)).await?;
            drop(stream);
            release.notify_one();
            shutdown.request();
            anyhow::Ok(())
        });

        serve_with_boot_task(&mut runtime, Router::new(), config, async move {
            let _ = boot_started_tx.send(());
            boot_release.notified().await;
        })
        .await?;
        client.await??;
        runtime.shutdown().await?;
        Ok(())
    }
}
