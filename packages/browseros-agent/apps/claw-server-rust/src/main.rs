use anyhow::Context;
use axum::Router;
use claw_server_rust::{
    AppRuntime, AppState, ShutdownHandle, VERSION,
    analytics::{AnalyticsSink, events},
    api::mcp::browser_mcp_service,
    build_router,
    config::{Cli, CliAction},
};
use rmcp::{serve_server, transport::stdio};
use serde_json::json;
use std::{
    future::Future,
    io::{self, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::TcpListener;
use tracing::{error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const VERSION_MARKER: &str = concat!(
    "browseros-claw-server-version=",
    env!("CARGO_PKG_VERSION"),
    ";"
);
const POSTHOG_KEY_MARKER: Option<&str> = option_env!("CLAW_POSTHOG_KEY_MARKER");
/// Signals "the port is taken" to the supervising browser, which responds by
/// relaunching on another port. Any other non-zero exit is read as a failed
/// launch, so this must not be folded into the generic error path.
const EXIT_PORT_CONFLICT: i32 = 2;

/// Carried as an error rather than exiting where it is detected, so the runtime
/// still tears down and the exit code is decided in one place.
#[derive(Debug, thiserror::Error)]
#[error("claw-server singleton is already running on 127.0.0.1:{port}")]
struct PortConflict {
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    std::hint::black_box(VERSION_MARKER);
    std::hint::black_box(POSTHOG_KEY_MARKER);
    let (config_path, stdio_mode) = match Cli::parse_action() {
        CliAction::Version => {
            writeln!(io::stdout().lock(), "{VERSION}")?;
            return Ok(());
        }
        CliAction::Run { config, stdio } => (config, stdio),
    };
    let config = Arc::new(claw_server_rust::config::Config::load(config_path)?);
    let _guard = init_tracing(config.clone())?;
    // A boot that fails before the server binds used to leave no reason in the log: the error went
    // to stderr (which the supervising browser does not capture) and the buffered file appender
    // never flushed it. Capture panics synchronously and record the startup outcome so the failure
    // is always diagnosable from the log directory.
    let logs_dir = config.browserclaw_dir.join("logs");
    install_panic_hook(logs_dir.clone());
    record_startup(&logs_dir, "starting", None);
    let state = match AppState::new(config.clone()).await {
        Ok(state) => state,
        Err(error) => {
            let detail = error.to_string();
            error!(error = %error, "startup failed before the server could bind");
            record_startup(&logs_dir, "failed", Some(detail.as_str()));
            return Err(error.into());
        }
    };
    record_startup(&logs_dir, "initialized", None);
    let mut runtime = AppRuntime::start(state);
    let run_result = run(&mut runtime, config, stdio_mode).await;
    let shutdown_result = runtime.shutdown().await;
    let outcome = match (run_result, shutdown_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error.into()),
        (Err(run_error), Err(shutdown_error)) => {
            error!(error = %shutdown_error, "application teardown failed after server error");
            Err(run_error)
        }
    };
    // Decided here, after teardown, and only for this one cause: the supervising
    // browser relaunches on another port for this code and stops supervising for
    // the rest of its session for any other non-zero exit.
    if let Err(error) = &outcome
        && let Some(conflict) = error.downcast_ref::<PortConflict>()
    {
        error!(port = conflict.port, "{conflict}");
        std::process::exit(EXIT_PORT_CONFLICT);
    }
    outcome
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
        // Deliberately not fatal. Exiting here reads to the supervisor as a failed
        // launch, which stops the sidecar for the rest of the browser session over
        // what the reattach loop reseeds on its next epoch.
        warn!(
            epoch = initial_browser.epoch,
            "tab target identities were not seeded before startup; continuing and reseeding on reconnect"
        );
    }
    if stdio_mode {
        return serve_stdio(state).await;
    }
    serve(runtime, config).await
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Appends panics to a dedicated file synchronously. The non-blocking log appender drops its buffer
/// when the process exits on a panic, so without this a crash leaves no trace.
fn install_panic_hook(logs_dir: PathBuf) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let line = format!(
            "{} v{VERSION} pid={} PANIC: {info}\n",
            epoch_ms(),
            std::process::id()
        );
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs_dir.join("claw-server-panics.log"))
        {
            let _ = file.write_all(line.as_bytes());
        }
        let detail = info.to_string();
        record_startup(&logs_dir, "panicked", Some(detail.as_str()));
        default_hook(info);
    }));
}

/// Overwrites the latest startup outcome so a failed boot is diagnosable from the log directory
/// without live instrumentation. Outcomes: starting, initialized, failed, panicked.
fn record_startup(logs_dir: &Path, outcome: &str, detail: Option<&str>) {
    let record = json!({
        "version": VERSION,
        "pid": std::process::id(),
        "atMs": epoch_ms(),
        "outcome": outcome,
        "detail": detail,
    });
    if let Err(error) = std::fs::write(logs_dir.join("startup.json"), record.to_string()) {
        warn!(error = %error, "failed to write the startup record");
    }
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
    let analytics = state.analytics.clone();
    serve_with_boot_task(
        runtime,
        build_router(state),
        config,
        analytics,
        async move { heal_boot_config(&heal_state).await },
    )
    .await
}

/// Binds the HTTP listener before starting non-critical boot work in the background.
async fn serve_with_boot_task(
    runtime: &mut AppRuntime,
    app: Router,
    config: Arc<claw_server_rust::config::Config>,
    analytics: Arc<dyn AnalyticsSink>,
    boot_task: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], config.server_port));
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            return Err(PortConflict {
                port: config.server_port,
            }
            .into());
        }
        Err(err) => return Err(err).context("failed to bind claw-server listener"),
    };
    // Deliberately after the bind and before serving, and only on this path. Closing the
    // sessions of a process that is gone means closing every session with no end row, and
    // that query cannot tell one of those from a session running right now. It is only
    // unambiguous while this process holds the port and has not yet served a request, so
    // nothing can have minted a session. Run any earlier and a second instance, including
    // one about to fail with "singleton is already running", would close the live sessions
    // of the instance that owns the port. The stdio transport deliberately does not do this:
    // it has no port, so it has no claim to exclusivity.
    if let Err(error) = runtime
        .state()
        .audit_log
        .close_sessions_open_from_previous_run()
        .await
    {
        // Tidying past runs must never stop this one from serving.
        warn!(error = %error, "closing sessions from a previous run failed");
    }
    analytics.capture(events::SERVER_STARTED, json!({}));
    // Use the ACTUAL bound address, not the requested port, so an OS-assigned
    // or dev port (config port 0) is still published correctly.
    let bound = listener.local_addr().unwrap_or(addr);
    info!(%bound, "claw-server-rust listening");
    // Publish the canonical MCP URL for external discovery (the Codex and Claude
    // Desktop plugins). This is the proxy port (the source of truth), falling
    // back to the direct server port in dev where the proxy is unavailable, so
    // runtime.json matches what the cockpit and connected agents advertise.
    // Fire-and-forget: a best-effort disk write must never gate the listener
    // from accepting connections, since a stalled FUSE / network /
    // container-mounted browserclaw_dir could otherwise leave the socket bound
    // but never served. Mirrors the archived TS server, which deliberately did
    // not await writeRuntimeFile for the same reason.
    let runtime_dir = config.browserclaw_dir.clone();
    // The proxy port is the advertised endpoint. Without a proxy (dev) publish
    // the ACTUAL bound port, which stays correct even when the config requested
    // an OS-assigned port (server_port == 0) that public_base_url would render
    // as an unreachable `:0`.
    let runtime_url = match config.proxy_port {
        Some(proxy) => format!("http://127.0.0.1:{proxy}"),
        None => format!("http://{bound}"),
    };
    runtime.spawn_task("runtime file publication", async move {
        claw_server_rust::services::runtime_file::write(&runtime_dir, &runtime_url).await;
    });
    let shutdown = runtime.state().shutdown;
    runtime.spawn_task("harness integration reconciliation", boot_task);
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await
        .context("claw-server listener failed")
}

async fn serve_stdio(state: AppState) -> anyhow::Result<()> {
    let running = ready_after(
        state.analytics.clone(),
        serve_server(browser_mcp_service(state.clone()), stdio()),
    )
    .await
    .context("failed to start stdio MCP server")?;
    running.waiting().await.context("stdio MCP server failed")?;
    Ok(())
}

/// Marks stdio ready only after transport construction returns a live handle.
async fn ready_after<T, E>(
    analytics: Arc<dyn AnalyticsSink>,
    start: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let running = start.await?;
    analytics.capture(events::SERVER_STARTED, json!({}));
    Ok(running)
}

/// Auto-connects harnesses once while preserving existing user choices.
async fn run_first_launch_auto_connect(state: &AppState) {
    use claw_server_rust::services::first_run;
    if first_run::is_first_run_connect_done(&state.config.browserclaw_dir).await {
        return;
    }
    match state
        .harness
        .first_run_connect(&state.config.public_mcp_url())
        .await
    {
        Ok(outcome) => {
            info!(
                connected = outcome.connected,
                failed = outcome.failed,
                already_linked = outcome.already_linked,
                seeded_existing = outcome.seeded_existing,
                "first-run harness auto-connect settled"
            );
            if let Err(err) =
                first_run::mark_first_run_connect_done(&state.config.browserclaw_dir).await
            {
                error!(error = %err, "failed to persist first-run auto-connect marker");
            }
        }
        // Listing the harnesses failed: leave the marker unset so the next
        // launch retries the sweep rather than skipping it forever.
        Err(err) => error!(error = %err, "first-run harness auto-connect skipped: listing failed"),
    }
}

async fn heal_boot_config(state: &AppState) {
    match state
        .harness
        .migrate_browseros_identity(&state.config.public_mcp_url())
        .await
    {
        Ok(outcome) => info!(
            migrated = outcome.migrated,
            skipped = outcome.skipped,
            failed = outcome.failed,
            "completed BrowserOS MCP identity migration"
        ),
        Err(err) => error!(error = %err, "BrowserOS MCP identity migration failed"),
    }
    run_first_launch_auto_connect(state).await;
    // Re-point every connected agent at the current canonical URL first (the
    // proxy port may have moved on this app launch), then repair any config
    // that still drifted from the now-current manifest spec.
    match state
        .harness
        .migrate_connected_urls(&state.config.public_mcp_url())
        .await
    {
        Ok(outcome) => info!(
            migrated = outcome.migrated,
            failed = outcome.failed,
            "re-synced connected MCP agents to the current URL"
        ),
        Err(err) => error!(error = %err, "MCP URL migration failed"),
    }
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
    match state.harness.run_skill_reconciliation().await {
        Ok(outcome) => {
            for warning in &outcome.warnings {
                warn!(target = %warning.target.display(), warning = %warning.message, "harness skill reconciliation needs a retry");
            }
            info!(
                installed = outcome.installed,
                updated = outcome.updated,
                removed = outcome.removed,
                unchanged = outcome.unchanged,
                warnings = outcome.warnings.len(),
                "completed harness skill reconciliation"
            );
        }
        Err(err) => error!(error = %err, "harness skill reconciliation failed"),
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
    use super::{PortConflict, ready_after, serve_with_boot_task};
    use axum::Router;
    use claw_server_rust::{
        AppRuntime, AppState,
        analytics::{AnalyticsSink, events},
        config::Config,
    };
    use serde_json::Value;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tempfile::tempdir;
    use tokio::{net::TcpStream, sync::oneshot};

    #[derive(Default)]
    struct RecordingAnalytics {
        events: Mutex<Vec<events::EventDefinition>>,
    }

    impl AnalyticsSink for RecordingAnalytics {
        fn capture(&self, event: events::EventDefinition, _properties: Value) {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(event);
        }
    }

    impl RecordingAnalytics {
        fn snapshot(&self) -> Vec<events::EventDefinition> {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

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
        });
        let state = AppState::new_with_home(config.clone(), root.path().join("home")).await?;
        let shutdown = state.shutdown.clone();
        let mut runtime = AppRuntime::start(state);
        let analytics = Arc::new(RecordingAnalytics::default());
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

        serve_with_boot_task(
            &mut runtime,
            Router::new(),
            config,
            analytics.clone(),
            async move {
                let _ = boot_started_tx.send(());
                boot_release.notified().await;
            },
        )
        .await?;
        assert_eq!(analytics.snapshot(), vec![events::SERVER_STARTED]);
        client.await??;
        runtime.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn listener_bind_failure_emits_no_server_started_event() -> anyhow::Result<()> {
        let root = tempdir()?;
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = Arc::new(Config {
            server_port: occupied.local_addr()?.port(),
            cdp_port: 49337,
            proxy_port: None,
            resources_dir: root.path().join("resources"),
            browserclaw_dir: root.path().to_path_buf(),
            session_idle: Duration::from_secs(300),
            session_retention: Duration::from_secs(7_200),
            session_sweep_interval: Duration::from_secs(60),
            replay_retention_days: 7,
            dev_mode: false,
        });
        let state = AppState::new_with_home(config.clone(), root.path().join("home")).await?;
        let mut runtime = AppRuntime::start(state);
        let analytics = Arc::new(RecordingAnalytics::default());

        let result = serve_with_boot_task(
            &mut runtime,
            Router::new(),
            config,
            analytics.clone(),
            async {},
        )
        .await;
        let Err(error) = result else {
            panic!("binding an occupied port cannot succeed");
        };
        // The supervising browser only relaunches on another port for the
        // port-conflict exit code, so the cause has to stay distinguishable here
        // rather than collapsing into a generic startup failure.
        assert!(
            error.downcast_ref::<PortConflict>().is_some(),
            "a taken port must surface as a port conflict, got: {error}"
        );
        assert!(analytics.snapshot().is_empty());
        runtime.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn stdio_readiness_emits_only_after_transport_start_succeeds() {
        let analytics = Arc::new(RecordingAnalytics::default());
        let running = ready_after(analytics.clone(), async { Ok::<_, &str>("running") }).await;
        assert_eq!(running, Ok("running"));
        assert_eq!(analytics.snapshot(), vec![events::SERVER_STARTED]);

        let failed = ready_after(analytics.clone(), async { Err::<(), _>("failed") }).await;
        assert_eq!(failed, Err("failed"));
        assert_eq!(analytics.snapshot(), vec![events::SERVER_STARTED]);
    }
}
