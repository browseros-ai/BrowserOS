//! `browseros-server-rs` (staged as `resources/bin/browseros_server`): parses the CLI, loads the
//! sidecar, starts `AppState`, binds `127.0.0.1:<ports.server>` and serves until shutdown.
//! Exit codes are the Chromium supervisor contract (`config::exit`). Owned by P3.

use server_rust::{
    AppState, VERSION, build_router,
    config::{self, CliAction, Config, EnvConfig, exit},
    server_file::ServerFile,
    telemetry,
};
use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    },
};

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
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
            Ok(runtime) => runtime.block_on(serve(&config)),
            Err(error) => fatal(&format!("failed to start runtime: {error}"), exit::GENERAL),
        },
    }
}

async fn serve(config_path: &Path) -> i32 {
    let env = EnvConfig::from_env();
    telemetry::logging::init(&env);
    let config = match Config::load(config_path, &env) {
        Ok(config) => config,
        Err(error) => return fatal(&error.to_string(), exit::GENERAL),
    };
    let port = config.server_port;
    let state = match AppState::start(config).await {
        Ok(state) => state,
        Err(error) => return fatal(&error.to_string(), exit::GENERAL),
    };
    let listener =
        match tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                return fatal(
                    &format!(
                        "Port {port} is already in use. Chromium should try a different port."
                    ),
                    exit::PORT_CONFLICT,
                );
            }
            Err(error) => return fatal(&error.to_string(), exit::GENERAL),
        };
    let server_file = ServerFile::new(&state.paths.browseros_dir);
    if let Err(error) = server_file.write(port).await {
        tracing::warn!(%error, "failed to write server.json");
    }
    tracing::info!(port, version = VERSION, "http_server.started");

    // Which exit code the process ends with is decided by what stopped the server: the shutdown
    // route exits 0, a termination signal exits 3.
    let exit_code = Arc::new(AtomicI32::new(exit::SUCCESS));
    let signal_code = exit_code.clone();
    let shutdown = state.shutdown.clone();
    let result = axum::serve(listener, build_router(state))
        .with_graceful_shutdown(async move {
            tokio::select! {
                () = shutdown.triggered() => {}
                () = termination_signal() => signal_code.store(exit::SIGNAL, Ordering::SeqCst),
            }
        })
        .await;
    server_file.remove().await;
    match result {
        Ok(()) => exit_code.load(Ordering::SeqCst),
        Err(error) => fatal(&error.to_string(), exit::GENERAL),
    }
}

async fn termination_signal() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// Chromium does not capture stderr reliably, but the TS server's `[FATAL]` lines are what
/// operators grep for; keep the prefix.
fn fatal(message: &str, code: i32) -> i32 {
    let _ = writeln!(std::io::stderr().lock(), "[FATAL] {message}");
    code
}
