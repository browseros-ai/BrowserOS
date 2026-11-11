#[cfg(feature = "test-support")]
use std::future;
use std::future::{Future, IntoFuture};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fmt, io};

use axum::Router;
use browseros_core::NativeCapabilities;
use neo_captcha::CaptchaSolver;
use neo_captcha::config::{CAPSOLVER_KEY_FILE_ENV, TWOCAPTCHA_KEY_FILE_ENV};
use neo_trace::{
    EpochMillis, Privacy, RedactionLimits, Redactor, RetentionPolicy, SecretSet, SecretSetConfig,
    TraceError, TraceSink, TraceStore, WriterConfig,
};
use tokio::net::TcpListener;
use tokio::runtime::{Builder, Runtime};
use tokio::signal::unix::SignalKind;
use tokio::time::MissedTickBehavior;
use tokio::{fs, signal, time};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::browser::{BrowserLink, TabRegistry};
use crate::builtins::{
    self, BuiltinDeps, CaptchaClaims, PlaywrightFacade, PrivacyHooks, TabJanitor,
};
use crate::config::{AppConfig, Defaults, HostedMode, SecretFiles};
use crate::dispatch::{Dispatcher, Observer, RegistryBuilder};
use crate::host::{FlagRecovery, HostApi, HostInputs, TakeoverLock};
use crate::http::agent_router;
use crate::mcp::McpServer;
use crate::run::{RunService, RunServiceParts};
#[cfg(feature = "test-support")]
use crate::test_support::FakeCdp;

/// Neo's sidecar, started: the agent listener bound, the browser link connecting, and every
/// long-lived part built.
///
/// ```text
///  start ─ bind the agent port (taken: exit 2) ─ trace store + SecretSet + writer (with the
///          screenshot queue) + Privacy ─ host socket (with a usable token; bind failure: exit 1)
///        ─ TabRegistry + BrowserLink (connects in the background) ─ PlaywrightFacade (built on
///          the link's first connect) ─ CaptchaSolver (with a provider key) ─ Registry
///          (built-ins, neo-playwright's ops)
///        ─ Dispatcher (rules: builtins::door_rules) ─ RunService ─ McpServer
///        ─ host API task (starts neo-service once the link connects, then serves the
///          socket) ─ housekeeping task ─ agent router
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
    /// A usable host token is configured, but its socket could not be bound.
    #[error("host socket: {0}")]
    HostSocket(String),
    /// Binding, serving or installing the signal handler failed.
    #[error("{0}")]
    Io(#[from] io::Error),
}

impl App {
    /// The hosted stack shares trace recovery with the sidecar, without binding its listeners.
    #[cfg(feature = "hosted-run")]
    pub(crate) async fn hosted_trace(
        directory: &Path,
    ) -> Result<(TraceStore, Arc<FlagRecovery>), StartError> {
        let OpenedTrace { store, recovery } = open_trace(directory).await?;
        Ok((store, recovery))
    }

    /// The same janitor, secret expiration and retention schedule, owned by the hosted runtime.
    #[cfg(feature = "hosted-run")]
    pub(crate) async fn hosted_housekeeping(
        janitor: TabJanitor,
        secrets: Arc<SecretSet>,
        trace: TraceSink,
        shutdown: CancellationToken,
    ) {
        housekeeping(janitor, secrets, trace, shutdown).await;
    }

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
        let agent = TcpListener::bind(address)
            .await
            .map_err(|error| bind_error(address, error))?;
        log_startup(&config, address);

        let OpenedTrace { store, recovery } = open_trace(&config.env.trace_dir()).await?;
        // One secret set for the process: every exit a script or the trace sees is masked
        // with it, and sdk-traces' privacy and protected fills register into it.
        let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
        let redactor = Redactor::new(Arc::clone(&secrets), RedactionLimits::default());
        let writer = WriterConfig {
            replay_window: Defaults::REPLAY_WINDOW,
            ..WriterConfig::default()
        };
        // The writer decides which ops get a screenshot; neo-service's camera takes them
        // once the browser links.
        let (shots, screenshots) = neo_trace::screenshot_queue();
        let trace = neo_trace::spawn_writer(&store, redactor, writer, Some(shots));
        // Private flags committed by an earlier Neo still hold until their tabs are gone.
        let privacy = Privacy::new(
            Arc::clone(&secrets),
            Arc::new(trace.clone()),
            store.private_tabs().await?,
        );
        let host = HostApi::bind(
            &config.env.host_api,
            HostInputs {
                store: store.clone(),
                sink: trace.clone(),
                privacy: Arc::clone(&privacy),
                screenshots,
                xvnc: config.env.xvnc,
                cdp_port: config.launch.cdp_port,
                recovery: Arc::clone(&recovery),
            },
        )
        .await
        .map_err(|error| StartError::HostSocket(error.to_string()))?;

        let tabs = Arc::new(TabRegistry::new());
        let native = NativeCapabilities::from_browseros_version(config.browseros_version());
        let browser = link(native, Arc::clone(&tabs));
        // neo-playwright's privacy callbacks feed the same secret set the door masks with,
        // and name Neo's own secret files to its upload deny list. The crate holds them from
        // `PlaywrightFacade::start` on, so the list is config's, located before the start.
        let hooks = Arc::new(PrivacyHooks::new(
            privacy,
            Arc::clone(&tabs),
            recovery,
            config.secret_files,
            config.env.typed_quiet,
        ));
        let playwright = PlaywrightFacade::start(Arc::clone(&browser), Arc::clone(&hooks) as _);

        // One solver for the process: its paid-task budget and page reservations are its own.
        let solver = CaptchaSolver::new(config.captcha.clone()).map(Arc::new);
        let claims = Arc::new(CaptchaClaims::default());
        let mut methods = RegistryBuilder::default();
        let deps = BuiltinDeps {
            solver,
            claims: Arc::clone(&claims),
            playwright: Arc::clone(&playwright),
        };
        builtins::builtin_set(&mut methods, deps)
            .map_err(|error| StartError::Methods(error.to_string()))?;
        let methods = methods.build();
        let takeover = Arc::new(TakeoverLock::default());
        let dispatcher = Arc::new(Dispatcher::new(
            methods,
            builtins::door_rules(&takeover, &hooks, &claims),
            Arc::clone(&secrets),
            trace.clone(),
            // What every run's records and reply are masked with beyond the secret set: the
            // values typed or swept that the set learns only once their fields go quiet.
            hooks.typed(),
            Arc::clone(&browser),
            Arc::clone(&tabs),
            Observer::new(playwright),
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
        tokio::spawn(host.serve(
            Arc::clone(&browser),
            Arc::clone(&takeover),
            shutdown.clone(),
        ));
        tokio::spawn(housekeeping(
            TabJanitor::new(dispatcher, Arc::clone(&browser), tabs, takeover),
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

/// The trace store Neo starts on, and whether it owes private flags that a replaced store lost.
/// A plain data bag.
struct OpenedTrace {
    store: TraceStore,
    /// Pending after this start's replacement, or an earlier one's that never finished.
    recovery: Arc<FlagRecovery>,
}

/// Opens the trace store in `trace_dir` (`EnvConfig::trace_dir`, which Neo's secret files deny to
/// uploads too), or, when the file does not open (a schema this build does not know, a corrupt
/// file), moves it aside and opens a fresh one: Neo must not crash-loop on its own records. The
/// old file is kept beside the new one for a human to look at.
///
/// A fresh store lost two things that keep Neo safe, and neo-trace cannot read either from
/// a file it refused to open. The private flags: every tab open at the browser link's first
/// connect is made private instead ([`FlagRecovery`]), and a marker written before the old file
/// moves keeps that owed at every start until it is done. The request ids: a host that retries a
/// `requestId` from before the replacement runs it again, which nothing here can prevent, so it
/// is logged as an error for the operator.
async fn open_trace(trace_dir: &Path) -> Result<OpenedTrace, StartError> {
    let path = trace_dir.join("trace.db");
    match TraceStore::open(&path).await {
        Ok(store) => Ok(OpenedTrace {
            store,
            recovery: FlagRecovery::after_opening(&path).await,
        }),
        Err(error) => {
            let aside = PathBuf::from(format!(
                "{}.broken-{}",
                path.display(),
                EpochMillis::now().millis()
            ));
            error!(%error, path = %path.display(), aside = %aside.display(), "the trace store does not open; starting a fresh one. Its private flags are lost, so every tab open now is made private; its request ids are lost, so a requestId retried from before this start runs again");
            let recovery = FlagRecovery::before_replacing(&path).await?;
            fs::rename(&path, &aside).await?;
            for suffix in ["-wal", "-shm"] {
                let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
                // Absent when SQLite never wrote one.
                let _ = fs::remove_file(sidecar).await;
            }
            Ok(OpenedTrace {
                store: TraceStore::open(&path).await?,
                recovery,
            })
        }
    }
}

/// Why the agent port did not bind. Only a taken port is `PortInUse`, which `main` exits 2 on.
fn bind_error(address: SocketAddr, error: io::Error) -> StartError {
    if error.kind() == io::ErrorKind::AddrInUse {
        StartError::PortInUse(address)
    } else {
        StartError::Io(error)
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

/// The start's facts and its warnings, one line each. The hosted-mode and CAPTCHA lines are
/// never failures: `run` works without either. `HostApi::bind` logs the host socket's own line.
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
        xvnc = %env.xvnc.addr,
        screen = %env.xvnc.home,
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
    // The key files were read before logging started, so this line is the operator's only
    // word on a key file that could not be read.
    let captcha = &config.captcha;
    if captcha.has_provider() {
        info!(
            capsolver = captcha.capsolver.is_some(),
            twocaptcha = captcha.twocaptcha.is_some(),
            paid_task_limit = captcha.paid_task_limit,
            "neo.solveCaptcha is on"
        );
    } else {
        warn!(
            "neo.solveCaptcha is off: no readable CAPTCHA provider key ({CAPSOLVER_KEY_FILE_ENV}, {TWOCAPTCHA_KEY_FILE_ENV}); a call fails host / captcha_unavailable"
        );
    }
    // setInputFiles' deny list, located before logging started. An entry the crate cannot
    // resolve makes it refuse every upload from a path, and these lines are the operator's only
    // word on why.
    match &config.secret_files {
        SecretFiles::Named(files) => {
            info!(?files, "setInputFiles never uploads Neo's own secret files");
            for file in files {
                if let Some(error) = file.unresolvable() {
                    warn!(
                        kind = ?file.kind,
                        path = %file.path.display(),
                        %error,
                        "Neo cannot resolve one of its secret files, so setInputFiles refuses every upload from a path until it can"
                    );
                }
            }
        }
        SecretFiles::Unlocated { missing, why } => warn!(
            ?missing,
            why,
            "Neo cannot place one of its secret files, so setInputFiles uploads no file from Neo's disk; payloads still upload"
        ),
    }
}

#[cfg(test)]
mod storage_tests {
    use std::error::Error;
    use std::fs;
    use std::path::Path;

    use neo_trace::{RequestId, RunId, RunQuery, TraceStore};
    use sqlx::Connection;
    use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};

    use super::open_trace;
    use crate::host::FlagRecovery;

    /// Compare logical contents: the setup connection's WAL checkpoint can change file bytes
    /// even when an unknown schema is refused without changing a row.
    async fn logical_snapshot(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {
        let mut conn =
            SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(path)).await?;
        let mut snapshot: Vec<String> = sqlx::query_scalar("SELECT type || '|' || name || '|' || tbl_name || '|' || quote(sql) FROM sqlite_master ORDER BY name")
            .fetch_all(&mut conn).await?;
        for table in [
            "runs",
            "operations",
            "run_tabs",
            "private_tabs",
            "seaql_migrations",
        ] {
            let columns: Vec<String> =
                sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                    .bind(table)
                    .fetch_all(&mut conn)
                    .await?;
            let fields = columns
                .iter()
                .map(|name| format!("quote(\"{name}\")"))
                .collect::<Vec<_>>()
                .join(" || '|' || ");
            let mut rows: Vec<String> =
                sqlx::query_scalar(&format!("SELECT {fields} FROM {table}"))
                    .fetch_all(&mut conn)
                    .await?;
            rows.sort();
            snapshot.push(table.to_owned());
            snapshot.extend(rows);
        }
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut conn)
            .await?;
        snapshot.push(version.to_string());
        conn.close().await?;
        Ok(snapshot)
    }

    #[tokio::test]
    async fn a_newer_migration_replaces_the_store_with_flag_recovery_pending()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("trace.db");
        drop(TraceStore::open(&path).await?);
        let mut conn =
            SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&path)).await?;
        sqlx::raw_sql("ALTER TABLE operations ADD COLUMN later_field TEXT; INSERT INTO seaql_migrations VALUES ('m9999_later', 1); INSERT INTO private_tabs VALUES (42, 1, 'host')")
            .execute(&mut conn).await?;
        sqlx::query("INSERT INTO runs (id, started_at, ended_at, status, request_id, complete) VALUES (?, 1, 2, 'ok', 'before-rollback', 1)")
            .bind(RunId::new().to_string()).execute(&mut conn).await?;
        conn.close().await?;
        let before = logical_snapshot(&path).await?;

        // Exercise the real startup boundary: classify/refuse, await close, mark flags lost,
        // preserve the refused file, then start successfully on a fresh store.
        let opened = open_trace(directory.path()).await?;
        assert!(
            opened.recovery.is_pending(),
            "capture remains blocked until every open tab is private"
        );
        assert!(directory.path().join("trace.db.flags-lost").is_file());
        assert!(
            FlagRecovery::after_opening(&path).await.is_pending(),
            "a stop before recovery keeps the debt"
        );
        let aside = fs::read_dir(directory.path())?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("trace.db.broken-")
            })
            .ok_or("missing refused store")?
            .path();
        assert_eq!(
            logical_snapshot(&aside).await?,
            before,
            "startup keeps the refused schema and all rows for a human"
        );
        assert!(
            opened
                .store
                .runs(RunQuery::default())
                .await?
                .items
                .is_empty()
        );
        assert!(opened.store.private_tabs().await?.is_empty());
        assert!(
            opened
                .store
                .run_by_request(&RequestId::parse("before-rollback")?, None)
                .await?
                .is_none(),
            "the owner accepted lost request history across a newer-migration rollback"
        );
        Ok(())
    }
}
