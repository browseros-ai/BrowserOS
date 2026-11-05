#[cfg(feature = "test-support")]
use std::future;
use std::future::{Future, IntoFuture};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fmt, io};

use axum::Router;
use browseros_core::NativeCapabilities;
use browseros_trace::{
    EpochMillis, RedactionLimits, Redactor, RetentionPolicy, SecretSet, SecretSetConfig,
    TraceError, TraceSink, TraceStore, WriterConfig, spawn_writer,
};
use tokio::net::TcpListener;
use tokio::runtime::{Builder, Runtime};
use tokio::signal::unix::SignalKind;
use tokio::time::MissedTickBehavior;
use tokio::{fs, signal, time};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::browser::{BrowserLink, TabRegistry};
use crate::builtins::{self, TabJanitor};
use crate::config::{AppConfig, Defaults, HostedMode};
use crate::dispatch::{Dispatcher, Policy, RegistryBuilder};
use crate::http::agent_router;
use crate::mcp::McpServer;
use crate::run::{RunService, RunServiceParts};
#[cfg(feature = "test-support")]
use crate::test_support::FakeCdp;

/// Neo's sidecar, started: the agent listener bound, the browser link connecting, and every
/// long-lived part built.
///
/// ```text
///  start ─ bind the agent port (taken: exit 2) ─ trace store + SecretSet + writer
///        ─ TabRegistry + BrowserLink (connects in the background) ─ Registry (built-ins)
///        ─ Dispatcher ─ RunService ─ McpServer ─ housekeeping task ─ agent router
///  serve ─ until SIGTERM ─ runs stop (shutting_down) ─ connections drain ─ trace flushed
/// ```
///
/// `start` binds the agent port before anything else, so a taken port fails (exit 2) before
/// any slow or side-effecting work; `serve` runs until SIGTERM.
pub struct App {
    agent: TcpListener,
    router: Router,
    /// Cancelled on SIGTERM; every run stops on it.
    shutdown: CancellationToken,
    trace: TraceSink,
    #[cfg(feature = "test-support")]
    store: TraceStore,
}

/// Why Neo could not start or keep serving. `main` maps `PortInUse` to exit 2 and everything
/// else to exit 1.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// Another process holds the agent port. Chromium reads exit 2 as "relaunch on the next
    /// port", so only this failure may produce it.
    #[error("the agent port {0} is in use")]
    PortInUse(SocketAddr),
    /// The built-in SDK methods did not register (a duplicate name, a bad redaction pointer):
    /// a programming error, which the first test that starts the app catches.
    #[error("the SDK method set: {0}")]
    Methods(String),
    /// The trace store could not be opened, even fresh.
    #[error("trace store: {0}")]
    Trace(#[from] TraceError),
    /// Binding, serving or installing the signal handler failed.
    #[error("{0}")]
    Io(#[from] io::Error),
}

impl App {
    /// The tokio runtime Neo runs on: multi-thread, with `Defaults::WORKER_STACK_BYTES` worker
    /// stacks, because a run nests QuickJS and the SDK call chain on one worker.
    pub fn runtime() -> io::Result<Runtime> {
        Builder::new_multi_thread()
            .enable_all()
            .thread_stack_size(Defaults::WORKER_STACK_BYTES)
            .build()
    }

    /// Starts Neo against Chromium's CDP port (`ports.cdp`).
    pub async fn start(config: AppConfig) -> Result<Self, StartError> {
        let cdp_port = config.launch.cdp_port;
        Self::start_on(config, move |native, tabs| {
            BrowserLink::start(cdp_port, native, tabs)
        })
        .await
    }

    /// Like [`App::start`], with the browser behind `fake` instead of Chromium's CDP port: the
    /// seam integration tests drive the real router and wiring through.
    #[cfg(feature = "test-support")]
    pub async fn start_with_fake_browser(
        config: AppConfig,
        fake: Arc<FakeCdp>,
    ) -> Result<Self, StartError> {
        Self::start_on(config, move |native, tabs| {
            BrowserLink::start_with(native, tabs, move || future::ready(fake.connect()))
        })
        .await
    }

    /// The agent listener's address: `ports.server`, or the port the OS picked for port 0.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.agent.local_addr()
    }

    /// The process's trace store, for tests that read what a run recorded.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn trace_store(&self) -> TraceStore {
        self.store.clone()
    }

    /// Startup, in dependency order. `link` starts the browser link over the real port or a
    /// test's fake; everything else is the same either way.
    async fn start_on(
        config: AppConfig,
        link: impl FnOnce(NativeCapabilities, Arc<TabRegistry>) -> Arc<BrowserLink>,
    ) -> Result<Self, StartError> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, config.launch.server_port));
        let agent = TcpListener::bind(address).await.map_err(|error| {
            if error.kind() == io::ErrorKind::AddrInUse {
                StartError::PortInUse(address)
            } else {
                StartError::Io(error)
            }
        })?;
        log_startup(&config, address);

        let store = open_trace(&config.env.state_dir.join("claw-sdk").join("trace.db")).await?;
        // One secret set for the process: every exit a script or the trace sees is masked
        // with it, and sdk-traces' privacy and protected fills register into it.
        let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
        let redactor = Redactor::new(Arc::clone(&secrets), RedactionLimits::default());
        let writer = WriterConfig {
            replay_window: Defaults::REPLAY_WINDOW,
            ..WriterConfig::default()
        };
        // No screenshot queue yet: browseros-service's camera takes the other end once the host
        // API is wired.
        let trace = spawn_writer(&store, redactor, writer, None);

        let tabs = Arc::new(TabRegistry::new());
        let native = NativeCapabilities::from_browseros_version(config.browseros_version());
        let browser = link(native, Arc::clone(&tabs));

        let mut methods = RegistryBuilder::default();
        builtins::builtin_set(&mut methods)
            .map_err(|error| StartError::Methods(error.to_string()))?;
        let methods = methods.build();
        let dispatcher = Arc::new(Dispatcher::new(
            methods,
            rules(),
            Arc::clone(&secrets),
            trace.clone(),
            Arc::clone(&browser),
            Arc::clone(&tabs),
        ));
        let shutdown = CancellationToken::new();
        let runs = RunService::new(RunServiceParts {
            dispatcher: Arc::clone(&dispatcher),
            browser: Arc::clone(&browser),
            tabs: Arc::clone(&tabs),
            store: store.clone(),
            trace: trace.clone(),
            secrets: Arc::clone(&secrets),
            shutdown: shutdown.clone(),
            orphan_grace: Defaults::ORPHAN_GRACE,
        });
        let mcp = McpServer::new(runs, dispatcher.registry());
        tokio::spawn(housekeeping(
            TabJanitor::new(dispatcher, Arc::clone(&browser), tabs, trace.clone()),
            secrets,
            trace.clone(),
            shutdown.clone(),
        ));
        Ok(Self {
            agent,
            router: agent_router(mcp, browser),
            shutdown,
            trace,
            #[cfg(feature = "test-support")]
            store,
        })
    }

    /// Serves the agent listener until SIGTERM (or SIGINT, for a developer's terminal). Then
    /// every run stops as `neo / shutting_down` and records its end, in-flight requests get
    /// `Defaults::SHUTDOWN_DRAIN` to answer, and the trace writer `Defaults::SHUTDOWN_FLUSH` to
    /// commit. Chromium sends SIGKILL 5 s after SIGTERM, so nothing may depend on this path
    /// completing; the store marks what it missed `interrupted` at the next start.
    pub async fn serve(self) -> Result<(), StartError> {
        let Self {
            agent,
            router,
            shutdown,
            trace,
            ..
        } = self;
        let mut terminate = signal::unix::signal(SignalKind::terminate())?;
        let interrupt = signal::ctrl_c();
        let stop = {
            let shutdown = shutdown.clone();
            async move {
                tokio::select! {
                    _ = terminate.recv() => info!("SIGTERM: stopping"),
                    _ = interrupt => info!("SIGINT: stopping"),
                }
                shutdown.cancel();
            }
        };
        let serving = axum::serve(agent, router).with_graceful_shutdown(stop);
        tokio::select! {
            served = serving.into_future() => served?,
            () = after(&shutdown, Defaults::SHUTDOWN_DRAIN) => {
                warn!("requests still open after the shutdown budget; exiting anyway");
            }
        }
        match time::timeout(Defaults::SHUTDOWN_FLUSH, trace.flush()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warn!(%error, "the trace writer failed to commit at shutdown"),
            Err(_elapsed) => warn!("the trace writer did not commit within the shutdown budget"),
        }
        Ok(())
    }
}

impl fmt::Debug for App {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("App")
            .field("agent", &self.agent)
            .finish_non_exhaustive()
    }
}

/// The dispatcher's rules, in their one fixed order (design D7 rule 4): sdk-traces' takeover
/// lock, then its tab privacy, then the CAPTCHA busy rule. Each lands as one entry here, and
/// the door runs them in this order for every op.
fn rules() -> Vec<Arc<dyn Policy>> {
    Vec::new()
}

/// Neo's periodic upkeep, one task until `shutdown`: every [`Defaults::HOUSEKEEPING_EVERY`] the
/// janitor closes idle agent tabs and the secret set drops expired values; every
/// [`Defaults::TRACE_SWEEP_EVERY`] the trace writer deletes records past retention.
///
/// Both intervals fire once at start, so a Neo that restarts often still sweeps. A step that
/// fails is logged and tried again on its next tick: none of them may take the process down,
/// and none blocks a run. A sweep in progress at SIGTERM is dropped with the runtime; the
/// store marks its run `interrupted` at the next start.
async fn housekeeping(
    janitor: TabJanitor,
    secrets: Arc<SecretSet>,
    trace: TraceSink,
    shutdown: CancellationToken,
) {
    let mut upkeep = time::interval(Defaults::HOUSEKEEPING_EVERY);
    let mut retention = time::interval(Defaults::TRACE_SWEEP_EVERY);
    // A tick missed while a sweep waited on the browser is not worth catching up on.
    upkeep.set_missed_tick_behavior(MissedTickBehavior::Delay);
    retention.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return,
            _ = upkeep.tick() => {
                let now = time::Instant::now();
                if let Some(run) = janitor.sweep(now).await {
                    debug!(%run, "idle-tab sweep recorded");
                }
                let expired = secrets.expire(now.into_std());
                if expired > 0 {
                    debug!(expired, "secret values expired");
                }
            }
            _ = retention.tick() => {
                match trace.sweep(RetentionPolicy::default(), EpochMillis::now()).await {
                    Ok(swept) => debug!(?swept, "trace retention sweep"),
                    Err(error) => warn!(%error, "the trace retention sweep failed; retrying on the next tick"),
                }
            }
        }
    }
}

/// Opens the trace store, or, when the file does not open (a schema this build does not know,
/// a corrupt file), moves it aside and opens a fresh one: Neo must not crash-loop on its own
/// records. The old file is kept beside the new one for a human to look at.
async fn open_trace(path: &Path) -> Result<TraceStore, StartError> {
    match TraceStore::open(path).await {
        Ok(store) => Ok(store),
        Err(error) => {
            let aside = PathBuf::from(format!(
                "{}.broken-{}",
                path.display(),
                EpochMillis::now().millis()
            ));
            warn!(%error, path = %path.display(), aside = %aside.display(), "the trace store does not open; starting a fresh one");
            fs::rename(path, &aside).await?;
            for suffix in ["-wal", "-shm"] {
                let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
                // Absent when SQLite never wrote one.
                let _ = fs::remove_file(sidecar).await;
            }
            Ok(TraceStore::open(path).await?)
        }
    }
}

/// Resolves `budget` after `token` is cancelled.
fn after(token: &CancellationToken, budget: time::Duration) -> impl Future<Output = ()> + use<> {
    let token = token.clone();
    async move {
        token.cancelled().await;
        time::sleep(budget).await;
    }
}

/// The start's facts and its warnings, one line each. The hosted-mode and host-listener lines
/// are never failures: `run` works without either.
fn log_startup(config: &AppConfig, address: SocketAddr) {
    let launch = &config.launch;
    let env = &config.env;
    info!(
        version = env!("CARGO_PKG_VERSION"),
        %address,
        cdp_port = launch.cdp_port,
        resources = %launch.resources_dir.display(),
        browseros_version = config.browseros_version(),
        browseros_version_from_env = env.native_version.is_some(),
        state_dir = %env.state_dir.display(),
        "claw-sdk serving"
    );
    if let HostedMode::Off { value } = &env.hosted_mode {
        error!(
            ?value,
            "BROWSERCLAW_HOSTED_MODE is not \"1\": popups will open as windows Neo cannot drive"
        );
    }
    if let Some(path) = &env.ignored_config_file {
        info!(
            path = %path.display(),
            "claw-sdk reads no config file; BROWSERCLAW_CONFIG_FILE is ignored"
        );
    }
    let host = &env.host_api;
    match &host.token_file {
        None => warn!("no host listener: CLAW_HOST_TOKEN_FILE is unset"),
        Some(token_file) => warn!(
            socket = %host.socket.display(),
            tcp = ?host.tcp,
            token_file = %token_file.display(),
            "host listener not started: it serves browseros-service's routes, not built yet"
        ),
    }
}
