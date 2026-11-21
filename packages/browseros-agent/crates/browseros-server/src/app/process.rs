//! The serving process: start order, the startup and shutdown budgets, logging and signals.
//!
//! The browser's supervisor gives a launched server 30 s to answer its first health check, and
//! sends SIGKILL 5 s after SIGTERM. So both ends have one total budget here:
//!
//! ```text
//!   start  ≤ 20 s   logs → state dir → store (migrations) → install id → engine (background)
//!                   → App::assemble → bind 127.0.0.1:<port>          over budget: exit 1
//!   stop   ≤ 4 s    runs (2.5 s) ‖ engine stdin closed → listener → store closed → logs flushed
//! ```
//!
//! `/system/health` answers as soon as the listener is bound, and by then the store is open and
//! every contract route usable. The engine is not part of the start budget: it starts in the
//! background, and the server serves without it.

use std::fs::OpenOptions;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use tokio::net::TcpListener;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use super::launch::{Launch, LaunchEnv, read_install_id};
use super::report::Reporter;
use super::{App, Parts, Shutdown};
use crate::browser::{Browser, BrowserConfig};
use crate::model::{Models, Script};
use crate::store::Store;

/// From process start to "listener bound": the launcher's 30 s less a margin.
const STARTUP_BUDGET: Duration = Duration::from_secs(20);
/// From SIGTERM to returning from `main`: the launcher kills at 5 s.
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(4);
/// The runs' share of the shutdown budget: cancelled at once, sealed rows written by then.
const RUNS_BUDGET: Duration = Duration::from_millis(2500);
/// Kept back at the end of shutdown for closing the store and flushing the log.
const CLOSE_RESERVE: Duration = Duration::from_millis(500);

/// The exit code the browser reads as "port in use, relaunch on the next one". Nothing else
/// may return it, including an engine child's own exit.
const EXIT_PORT_IN_USE: u8 = 2;
/// After SIGTERM or Ctrl-C, as the old server does: the browser restarts a server it did not
/// stop itself.
const EXIT_SIGNALLED: u8 = 3;

/// Why the server did not start. Only `PortInUse` is exit 2.
#[derive(Debug, thiserror::Error)]
enum StartError {
    #[error("creating the state directory {path}: {source}")]
    StateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("opening the database: {0}")]
    Store(#[from] crate::store::StoreError),
    #[error("loading the mock model's script: {0}")]
    Script(String),
    #[error("127.0.0.1:{port} is already in use")]
    PortInUse { port: u16 },
    #[error("binding 127.0.0.1:{port}: {source}")]
    Bind { port: u16, source: std::io::Error },
    #[error("start-up took longer than {}s", STARTUP_BUDGET.as_secs())]
    TooSlow,
}

impl StartError {
    /// What a failed start reports: the kind only, never the message, which names folders.
    fn kind(&self) -> &'static str {
        match self {
            StartError::StateDir { .. } => "state directory",
            StartError::Store(_) => "store",
            StartError::Script(_) => "mock script",
            StartError::PortInUse { .. } => "port in use",
            StartError::Bind { .. } => "bind",
            StartError::TooSlow => "start-up budget",
        }
    }
}

/// `--print-openapi`: the generated contract on stdout, for `scripts/codegen/server-api.ts`.
pub(super) fn print_openapi() -> ExitCode {
    match crate::http::openapi().to_pretty_json() {
        Ok(json) => {
            #[expect(clippy::print_stdout, reason = "the codegen script reads stdout")]
            {
                println!("{json}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            report_before_logging(&format!("serialising the OpenAPI document: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// `--config <file>`: serve until SIGTERM, Ctrl-C or `POST /system/shutdown`.
pub(super) fn serve(config: &Path) -> ExitCode {
    let started = Instant::now();
    let launch = match Launch::read(config, &LaunchEnv::from_process()) {
        Ok(launch) => launch,
        Err(error) => {
            report_before_logging(&error.to_string());
            return ExitCode::FAILURE;
        }
    };
    // Declared before the runtime, so it drops after it: the last lines of shutdown still reach
    // the file.
    let reporter = Reporter::from_env();
    let _logging = install_logging(&launch.execution, reporter.clone());
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "building the tokio runtime");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(run(launch, started, reporter.clone()));
    // A task still holding a socket or a child must not keep the process past the launcher's
    // kill: the shutdown budget already gave everything its time.
    runtime.shutdown_timeout(Duration::from_millis(100));
    match result {
        Ok(exit) => exit,
        Err(StartError::PortInUse { port }) => {
            tracing::error!(
                port,
                "another process holds the server port; the browser relaunches on the next"
            );
            ExitCode::from(EXIT_PORT_IN_USE)
        }
        Err(error) => {
            tracing::error!(%error, "the server did not start");
            reporter.failed_start(error.kind());
            reporter.flush(Duration::from_millis(500));
            ExitCode::FAILURE
        }
    }
}

async fn run(launch: Launch, started: Instant, reporter: Reporter) -> Result<ExitCode, StartError> {
    let shutdown = Shutdown::new();
    let engine_stop = tokio_util::sync::CancellationToken::new();
    // Installed before start-up: SIGTERM while the store waits on a lock must still end in
    // exit 3 with the logs flushed, not in the default "killed by signal 15".
    let signals = tokio::spawn(forward_signals(shutdown.clone()));
    let remaining = STARTUP_BUDGET.saturating_sub(started.elapsed());
    let starting = tokio::time::timeout(
        remaining,
        start(launch, shutdown.clone(), engine_stop.clone()),
    );
    let (app, listener) = tokio::select! {
        started = starting => started.map_err(|_| StartError::TooSlow)??,
        () = shutdown.token().cancelled() => {
            // Dropping the start future rolls back any migration in flight; the engine never
            // started, and nothing was bound.
            tracing::info!("stopped before start-up finished");
            signals.abort();
            return Ok(ExitCode::from(EXIT_SIGNALLED));
        }
    };
    tracing::info!(
        port = app.launch.server_port,
        state_dir = %app.launch.state_dir.display(),
        elapsed_ms = started.elapsed().as_millis(),
        "listening"
    );

    let router = crate::http::router(app.clone()).layer(axum::middleware::from_fn({
        let reporter = reporter.clone();
        move |request, next| reporter.clone().watch_500s(request, next)
    }));
    let stopping = shutdown.token().clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { stopping.cancelled().await })
            .await
    });

    shutdown.token().cancelled().await;
    let stop_started = Instant::now();
    tracing::info!(requested = shutdown.exit_requested(), "shutting down");
    // Runs and the engine stop side by side: every run's cancel fires at once and its sealed
    // row is written, while the engine child sees its stdin lifeline close.
    engine_stop.cancel();
    app.conversations.shutdown(RUNS_BUDGET).await;
    // In-flight responses get what is left; an endless stream (`/chat/panels`) is cut.
    let left = SHUTDOWN_BUDGET
        .saturating_sub(stop_started.elapsed())
        .saturating_sub(CLOSE_RESERVE);
    match tokio::time::timeout(left, server).await {
        Ok(Ok(Ok(()))) | Err(_) => {}
        Ok(Ok(Err(error))) => tracing::warn!(%error, "the listener stopped with an error"),
        Ok(Err(error)) => tracing::warn!(%error, "the listener task failed"),
    }
    signals.abort();
    app.store.close().await;
    reporter.flush(
        SHUTDOWN_BUDGET
            .saturating_sub(stop_started.elapsed())
            .min(CLOSE_RESERVE),
    );
    tracing::info!(elapsed_ms = stop_started.elapsed().as_millis(), "stopped");
    Ok(if shutdown.exit_requested() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_SIGNALLED)
    })
}

/// Everything before serving, in the design's order. The caller bounds it by the start budget.
async fn start(
    mut launch: Launch,
    shutdown: Shutdown,
    engine_stop: tokio_util::sync::CancellationToken,
) -> Result<(App, TcpListener), StartError> {
    std::fs::create_dir_all(&launch.state_dir).map_err(|source| StartError::StateDir {
        path: launch.state_dir.clone(),
        source,
    })?;
    let store = open_store(&launch.state_dir).await?;
    launch.install_id = match read_install_id(&launch.state_dir) {
        Ok(id) => Some(id),
        Err(error) => {
            // The old server's behaviour: the rest of the server works, sign-in answers 503.
            tracing::error!(%error, "installation identity unavailable; sign-in is off until the file is repaired");
            None
        }
    };
    let models = choose_models(&store, launch.install_id.clone())?;
    let browser = Browser::start(browser_config(&launch), launch.server_port, engine_stop).await;
    let port = launch.server_port;
    let app = App::assemble(Parts {
        launch,
        store,
        models,
        browser,
        runner: None,
        shutdown,
    });
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = TcpListener::bind(address).await.map_err(|source| {
        if source.kind() == std::io::ErrorKind::AddrInUse {
            StartError::PortInUse { port }
        } else {
            StartError::Bind { port, source }
        }
    })?;
    Ok((app, listener))
}

/// Opens the store, waiting out another process's write lock (the old server finishing a
/// write, a backup tool). The caller's start budget bounds the wait: a lock held past it is
/// exit 1, never a hang, and the file is never moved for being locked.
async fn open_store(state_dir: &Path) -> Result<Store, StartError> {
    loop {
        match Store::open(state_dir).await {
            Err(error) if error.is_busy() => {
                tracing::warn!(%error, "the database is locked by another process; waiting");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            result => return result.map_err(StartError::from),
        }
    }
}

/// `BROWSEROS_USE_MOCK_LLM=true` replaces every model with the scripted one, from
/// `BROWSEROS_MOCK_LLM_SCRIPT` when it names a file.
fn choose_models(store: &Store, install_id: Option<String>) -> Result<Models, StartError> {
    if std::env::var("BROWSEROS_USE_MOCK_LLM").as_deref() != Ok("true") {
        return Ok(Models::new(store.clone(), install_id));
    }
    let script = match std::env::var_os("BROWSEROS_MOCK_LLM_SCRIPT") {
        Some(path) => Script::load(Path::new(&path)).map_err(StartError::Script)?,
        None => Script::default_answer(),
    };
    tracing::warn!("BROWSEROS_USE_MOCK_LLM is on: every model is the scripted one");
    Ok(Models::scripted(store.clone(), install_id, script))
}

/// The engine on Unix unless `BROWSEROS_BROWSER=off`; none on Windows, where it cannot be built.
fn browser_config(launch: &Launch) -> BrowserConfig {
    if cfg!(not(unix)) {
        return BrowserConfig::Off {
            reason: "this build cannot drive the browser on Windows",
        };
    }
    if std::env::var("BROWSEROS_BROWSER").as_deref() == Ok("off") {
        return BrowserConfig::Off {
            reason: "the browser is turned off by BROWSEROS_BROWSER=off",
        };
    }
    BrowserConfig::Neo {
        cdp_port: launch.cdp_port,
        state_dir: launch.state_dir.clone(),
        execution_dir: launch.execution.clone(),
        browseros_version: launch.browseros_version.clone(),
    }
}

/// SIGTERM (the browser's stop) and Ctrl-C both stop the server with exit 3.
async fn forward_signals(shutdown: Shutdown) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = terminate.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(error) => {
                tracing::warn!(%error, "cannot watch SIGTERM; only Ctrl-C stops the server");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    shutdown.signal();
}

/// `tracing` to stderr and to a daily file in the execution directory, filtered by
/// `BROWSEROS_LOG` (default `info`), and a panic hook that writes the panic to the same folder
/// and reports its location before the process dies. The returned guard flushes the file when dropped.
fn install_logging(execution: &Path, reporter: Reporter) -> Option<WorkerGuard> {
    let filter =
        || EnvFilter::try_from_env("BROWSEROS_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter());
    let file = std::fs::create_dir_all(execution)
        .ok()
        .and_then(|()| {
            tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("browseros-server")
                .filename_suffix("log")
                .max_log_files(7)
                .build(execution)
                .ok()
        })
        .map(tracing_appender::non_blocking);
    let (file_layer, guard) = match file {
        Some((writer, guard)) => (
            Some(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(writer)
                    .with_filter(filter()),
            ),
            Some(guard),
        ),
        None => (None, None),
    };
    let _ = tracing_subscriber::registry()
        .with(stderr)
        .with(file_layer)
        .try_init();

    let panics = execution.join("browseros-server-panics.log");
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Synchronous on purpose: the non-blocking log writer may never get to run again.
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&panics) {
            let _ = writeln!(file, "{info}");
        }
        reporter.panic(info.location());
        previous(info);
    }));
    guard
}

/// Before logging exists, stderr is the only place an error can go.
fn report_before_logging(message: &str) {
    #[expect(clippy::print_stderr, reason = "logging is not installed yet")]
    {
        eprintln!("browseros_server: {message}");
    }
}
