//! Chromium's process protocol: resolve CLI/sidecar, probe the port, compose services,
//! bind on IPv4 LAN, publish discovery, and clean up within its five-second grace.
//! Shutdown exits 0 while signals exit 3 so Chromium can distinguish intentional stops.

use server_rust::{
    AppState, VERSION, build_router,
    config::{self, CliAction, Config, EnvConfig, exit},
    server_file::ServerFile,
    telemetry::{self, sentry::SentryGuard},
};
use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

fn main() {
    std::process::exit(run());
}
fn run() -> i32 {
    // Before anything can fail: the panic hook and startup captures need the client, and the
    // guard's drop (after the runtime has shut down) is what flushes queued events.
    let sentry = Arc::new(telemetry::sentry::init());
    // CLI usage errors are commander exits in TS and are not reported.
    let action = match config::parse_cli(std::env::args_os().skip(1)) {
        Ok(action) => action,
        Err(error) => return fatal(&error.to_string(), exit::GENERAL),
    };
    match action {
        CliAction::Version => {
            let _ = writeln!(std::io::stdout().lock(), "{VERSION}");
            exit::SUCCESS
        }
        CliAction::Run { config } => match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => {
                let code = runtime.block_on(serve(&config, &sentry));
                // Runtime drop otherwise waits indefinitely for blocking jobs,
                // even after the async service drain met Chromium's deadline.
                runtime.shutdown_timeout(Duration::from_millis(250));
                code
            }
            Err(error) => startup_failure(
                &sentry,
                &format!("failed to start runtime: {error}"),
                exit::GENERAL,
            ),
        },
    }
}
async fn serve(config_path: &Path, sentry: &Arc<SentryGuard>) -> i32 {
    let env = EnvConfig::from_env();
    let config = match Config::load(config_path, &env) {
        Ok(config) => config,
        Err(error) => return startup_failure(sentry, &error.to_string(), exit::GENERAL),
    };
    telemetry::logging::init_file(&env, &config.execution_dir);
    if std::env::var("BROWSEROS_AI_SDK_DEVTOOLS").as_deref() == Ok("true") {
        tracing::warn!("BROWSEROS_AI_SDK_DEVTOOLS is unsupported in Rust");
    }
    let port = config.server_port;
    // Probe loopback separately before the LAN bind, just as TS's exclusive net
    // listener does. Chromium advances ports only on this distinct exit code.
    match std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
        Ok(probe) => drop(probe),
        Err(error) => return bind_failure(sentry, port, &error),
    }
    let state = match AppState::start_with_sentry(config, sentry.clone()).await {
        Ok(state) => state,
        Err(error) => return startup_failure(sentry, &error.to_string(), exit::GENERAL),
    };
    let listener = match tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await {
        Ok(listener) => listener,
        Err(error) => {
            let _ = tokio::time::timeout(Duration::from_secs(1), state.stop()).await;
            return bind_failure(sentry, port, &error);
        }
    };
    let server_file = ServerFile::with_config(&state.config, &state.identity);
    let stop_http = CancellationToken::new();
    let server_stop = stop_http.clone();
    let router = build_router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
    });
    // The listener and HTTP owner now exist; failed discovery is a warning, never
    // an excuse to kill an otherwise healthy server (TS parity).
    if let Err(error) = server_file.write(port).await {
        tracing::warn!(%error,"Failed to write server config for auto-discovery");
    }
    // Reconciliation is a boot one-shot after serving starts. The manifest owner
    // decides which links to repair and whether a public proxy URL was provided.
    let mcp_manager = state.mcp_manager.clone();
    let reconcile = tokio::spawn(async move { mcp_manager.self_heal().await });
    tracing::info!(port, version = VERSION, "http_server.started");
    state.metrics.capture(
        "http_server.started",
        serde_json::json!({ "version": VERSION }),
    );
    let code = tokio::select! {
        () = state.shutdown.triggered() => exit::SUCCESS,
        () = termination_signal() => exit::SIGNAL,
        result = &mut server => {
            tracing::error!(?result,"HTTP server stopped unexpectedly");
            reconcile.abort();
            server_file.remove().await;
            let _ = tokio::time::timeout(Duration::from_secs(1),state.stop()).await;
            return exit::GENERAL;
        }
    };
    stop_http.cancel();
    reconcile.abort();
    // Idle keep-alives and SSE clients must not consume Chromium's entire grace.
    // Graceful drain flushes the shutdown response; the deadline bounds stuck work.
    if tokio::time::timeout(Duration::from_secs(3), &mut server)
        .await
        .is_err()
    {
        server.abort();
        let _ = server.await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), state.stop()).await;
    server_file.remove().await;
    code
}
fn bind_failure(sentry: &SentryGuard, port: u16, error: &std::io::Error) -> i32 {
    if error.kind() == std::io::ErrorKind::AddrInUse {
        // Expected: Chromium retries on the next port. TS does not report it either.
        fatal(
            &format!("Port {port} is already in use. Chromium should try a different port."),
            exit::PORT_CONFLICT,
        )
    } else {
        startup_failure(
            sentry,
            &format!("Failed to start HTTP server on port {port}: {error}"),
            exit::GENERAL,
        )
    }
}
/// Reports a fatal startup error (TS `Sentry.captureException` in index.ts/main.ts), then exits.
fn startup_failure(sentry: &SentryGuard, message: &str, code: i32) -> i32 {
    sentry.capture_startup_error(message);
    fatal(message, code)
}
async fn termination_signal() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}
fn fatal(message: &str, code: i32) -> i32 {
    let _ = writeln!(std::io::stderr().lock(), "[FATAL] {message}");
    code
}
