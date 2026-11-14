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
use neo_http::Settings;
use neo_trace::{
    EpochMillis, RedactionLimits, Redactor, RetentionPolicy, RunOutputs, ShotJobs, TraceError,
    TraceSink, TraceStore, WriterConfig,
};
use tokio::net::TcpListener;
use tokio::runtime::{Builder, Runtime};
use tokio::signal::unix::SignalKind;
use tokio::sync::broadcast;
use tokio::time::MissedTickBehavior;
use tokio::{fs, signal, time};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::browser::{BrowserLink, NeoTab, TabEvent, TabRegistry};
use crate::builtins::{self, BuiltinDeps, CaptchaClaims, PlaywrightFacade, TabJanitor};
use crate::config::{AppConfig, Defaults, HostedMode};
use crate::dispatch::{Dispatcher, Observer, RegistryBuilder};
use crate::host::{
    AgentFront, FillLedger, FillService, FillServiceParts, FilledTabs, HolderFeed, HostApi,
    HostHolds, HostInputs, TabHolders, TakeoverLock,
};
use crate::http::agent_router;
use crate::mcp::McpServer;
use crate::run::{RunService, RunServiceParts};
#[cfg(feature = "test-support")]
use crate::test_support::FakeCdp;

/// The host's private typing (`POST /v1/tabs/{tabId}/fill`), over the app's one door. Its record
/// of request ids lives beside the trace and keeps what the trace keeps: the host's
/// `trace.retentionSeconds`, read at every prune, as the retention sweep reads it.
/// The parts the host's private typing shares with the rest of the app. A plain data bag.
struct PrivateTypingParts<'a> {
    dispatcher: &'a Arc<Dispatcher>,
    browser: &'a Arc<BrowserLink>,
    takeover: &'a Arc<TakeoverLock>,
    playwright: &'a Arc<PlaywrightFacade>,
    filled: Arc<FilledTabs>,
}

async fn private_typing(
    trace_dir: &Path,
    settings: &Settings,
    parts: PrivateTypingParts<'_>,
    shutdown: &CancellationToken,
) -> Arc<FillService> {
    let PrivateTypingParts {
        dispatcher,
        browser,
        takeover,
        playwright,
        filled,
    } = parts;
    let retention = settings.clone();
    let pictures = settings.clone();
    let entry = Arc::clone(playwright);
    let reader = Arc::clone(playwright);
    // The entry's own set refuses a script's picture of a filled tab only under `fill.pictures:
    // never` (ruling R58): it gets the setting now and after every change, before the change
    // answers, so a picture reads it as it stands when it is asked for.
    let follower = Arc::clone(playwright);
    settings
        .watch(move |settings| follower.set_fill_pictures_never(settings.fill_pictures_never()));
    FillService::new(FillServiceParts {
        dispatcher: Arc::clone(dispatcher),
        browser: Arc::clone(browser),
        takeover: Arc::clone(takeover),
        ledger: FillLedger::open(trace_dir, move || retention.trace_retention()).await,
        filled,
        pictures_never: Arc::new(move || pictures.fill_pictures_never()),
        shutdown: shutdown.clone(),
        took: Arc::new(move |tab| entry.withholds_picture(tab)),
        checks: Arc::new(move |tab| {
            let reader = Arc::clone(&reader);
            Box::pin(async move { reader.picture_verdict(tab).await })
        }),
    })
}

/// Drops a closed tab's target from the record of tabs that took a fill, in the order the
/// link's tab watch saw it. A close seen on a socket since replaced is skipped: after a browser
/// restart its tab id may name another tab, and the next start's listing prunes the record.
async fn forget_closed_filled_tabs(browser: Arc<BrowserLink>, filled: Arc<FilledTabs>) {
    let mut events = browser.tab_events();
    loop {
        match events.recv().await {
            Ok(TabEvent::Closed { epoch, tab }) if browser.epoch() == Some(epoch) => {
                filled.closed(tab).await;
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// Neo's sidecar, started: the agent listener bound, the browser link connecting, and every
/// long-lived part built.
///
/// ```text
///  start ─ bind the agent port (taken: exit 2) ─ trace store + writer (with the
///          screenshot queue) ─ host socket (with a usable token; bind failure: exit 1)
///          and the host's settings (saved beside trace.db, or the defaults with no socket)
///        ─ TabRegistry + BrowserLink (connects in the background) ─ TabHolders (the task
///          book's writer: on the link's first connect it gives every task back the saved tabs
///          the browser still has, then saves each change) ─ PlaywrightFacade (built on the
///          link's first connect) ─ CaptchaSolver (with a provider key) ─ Registry (built-ins,
///          neo-playwright's ops)
///        ─ Dispatcher (rules: builtins::door_rules) ─ RunService ─ McpServer ─ FillService
///          (the host's private typing, with its request record beside the trace)
///        ─ host API task (starts neo-http once the link connects, then serves the
///          socket) ─ housekeeping task (reads the host's settings each pass) ─ agent router
///  serve ─ until SIGTERM ─ runs stop (shutting_down) ─ connections drain ─ trace and tab
///          holders flushed
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
    /// The task book's changes on their way to the tab-holders file, flushed at shutdown.
    holders: Arc<HolderFeed>,
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
    /// The built-in SDK methods did not register because a name was duplicated:
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
    pub(crate) async fn hosted_trace(directory: &Path) -> Result<TraceStore, StartError> {
        open_trace(directory).await
    }

    /// The same idle-tab janitor and retention schedule, owned by the hosted runtime.
    #[cfg(feature = "hosted-run")]
    pub(crate) async fn hosted_housekeeping(
        janitor: TabJanitor,
        trace: TraceSink,
        outputs: RunOutputs,
        settings: Settings,
        shutdown: CancellationToken,
    ) {
        housekeeping(janitor, trace, outputs, settings, shutdown).await;
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
    #[expect(
        clippy::too_many_lines,
        reason = "the composition root: one top-to-bottom startup order, which split helpers would hide"
    )]
    async fn start_on(
        config: AppConfig,
        link: impl FnOnce(NativeCapabilities, Arc<TabRegistry>) -> Arc<BrowserLink>,
    ) -> Result<Self, StartError> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, config.launch.server_port));
        let agent = TcpListener::bind(address)
            .await
            .map_err(|error| bind_error(address, error))?;
        log_startup(&config, address);

        let store = open_trace(&config.env.trace_dir()).await?;
        let (trace, screenshots) = start_trace(&store);
        let tabs = Arc::new(TabRegistry::new());
        let host = HostApi::bind(
            &config.env.host_api,
            HostInputs {
                store: store.clone(),
                sink: trace.clone(),
                screenshots,
                xvnc: config.env.xvnc,
                cdp_port: config.launch.cdp_port,
                settings_file: config.env.settings_file(),
                settings_defaults: config.settings_defaults(),
                tabs: Arc::clone(&tabs),
            },
        )
        .await
        .map_err(|error| StartError::HostSocket(error.to_string()))?;
        // Loaded by the bind, before anything below reads them, so the first run follows them.
        let settings = host.settings().clone();
        follow_trace_policy(&settings, trace.clone());

        let native = NativeCapabilities::from_browseros_version(config.browseros_version());
        let browser = link(native, Arc::clone(&tabs));
        // Which task holds which tab, kept across a restart of Neo's server: every change of the
        // task book goes to the file from now on, and once the link first connects the book takes
        // back the saved tabs the browser still has. Runs start only after that (`RunService`).
        let holders = TabHolders::open(&config.env.trace_dir()).await;
        let (holder_feed, holder_notes) = HolderFeed::channel();
        tabs.set_book_sink(Arc::clone(&holder_feed) as _);
        tokio::spawn(Arc::clone(&holders).keep(
            Arc::clone(&browser),
            Arc::clone(&tabs),
            holder_notes,
        ));
        // Armed by the host API with the owner's screen and views, beside the takeover lock.
        let front = Arc::new(AgentFront::default());
        // Read back before the facade starts, which marks its tabs before any op runs.
        let filled = FilledTabs::open(&config.env.trace_dir()).await;
        tokio::spawn(forget_closed_filled_tabs(
            Arc::clone(&browser),
            Arc::clone(&filled),
        ));
        let playwright = PlaywrightFacade::start(
            Arc::clone(&browser),
            matches!(&config.env.hosted_mode, HostedMode::On),
            Arc::clone(&front),
            config.file_access(settings.clone()),
            Some(Arc::clone(&filled)),
        );

        let claims = Arc::new(CaptchaClaims::default());
        let mut methods = RegistryBuilder::default();
        // One solver for the process: its paid-task budget and page reservations are its own.
        let deps = BuiltinDeps {
            solver: CaptchaSolver::new(config.captcha.clone()).map(Arc::new),
            claims: Arc::clone(&claims),
            playwright: Arc::clone(&playwright),
        };
        builtins::builtin_set(&mut methods, deps)
            .map_err(|error| StartError::Methods(error.to_string()))?;
        let takeover = Arc::new(TakeoverLock::default());
        let dispatcher = Arc::new(Dispatcher::new(
            methods.build(),
            builtins::door_rules(&takeover, &claims),
            trace.clone(),
            Arc::clone(&browser),
            Arc::clone(&tabs),
            Observer::new(Arc::clone(&playwright) as _),
        ));
        let shutdown = CancellationToken::new();
        let blank_tab = keep_blank_tab(&browser, &tabs, &store, &playwright, &holders, &shutdown);
        let runs = RunService::new(RunServiceParts {
            dispatcher: Arc::clone(&dispatcher),
            browser: Arc::clone(&browser),
            tabs: Arc::clone(&tabs),
            store: store.clone(),
            trace: trace.clone(),
            settings: settings.clone(),
            shutdown: shutdown.clone(),
            orphan_grace: Defaults::ORPHAN_GRACE,
            blank_tab: Some(Arc::clone(&blank_tab)),
            holders: Some(Arc::clone(&holders)),
        });
        let mcp = McpServer::new(runs, dispatcher.registry());
        // Armed by the host API with its locks and fits, for the janitor.
        let holds = Arc::new(HostHolds::default());
        let fill = private_typing(
            &config.env.trace_dir(),
            &settings,
            PrivateTypingParts {
                dispatcher: &dispatcher,
                browser: &browser,
                takeover: &takeover,
                playwright: &playwright,
                filled,
            },
            &shutdown,
        )
        .await;
        tokio::spawn(host.with_fill(fill).serve(
            Arc::clone(&browser),
            takeover,
            front,
            Arc::clone(&holds),
            Some(blank_tab),
            shutdown.clone(),
        ));
        tokio::spawn(housekeeping(
            TabJanitor::new(
                dispatcher,
                Arc::clone(&browser),
                tabs,
                holds,
                settings.clone(),
            ),
            trace.clone(),
            store.outputs().clone(),
            settings,
            shutdown.clone(),
        ));
        Ok(Self {
            agent,
            router: agent_router(mcp, browser, Some(holders)),
            shutdown,
            trace,
            holders: holder_feed,
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
            holders,
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
        // Both in one budget: the tab holders' last changes need no sync, only a write.
        let (flushed, holders_written) = tokio::join!(
            time::timeout(Defaults::SHUTDOWN_FLUSH, trace.flush()),
            time::timeout(Defaults::SHUTDOWN_FLUSH, holders.flush()),
        );
        match flushed {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warn!(%error, "the trace writer failed to commit at shutdown"),
            Err(_elapsed) => warn!("the trace writer did not commit within the shutdown budget"),
        }
        if holders_written.is_err() {
            warn!("which task holds which tab was not all written within the shutdown budget");
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

/// The trace writer over `store`, and the camera's end of its screenshot queue. The writer
/// starts at the default limits until the host's `trace.valueBytes` reaches it, which `start_on`
/// does as soon as the settings are loaded and before any run. It decides which ops get a
/// screenshot; neo-http's camera takes them once the browser links.
fn start_trace(store: &TraceStore) -> (TraceSink, ShotJobs) {
    let redactor = Redactor::new(RedactionLimits::default());
    let writer = WriterConfig {
        replay_window: Defaults::REPLAY_WINDOW,
        ..WriterConfig::default()
    };
    let (shots, screenshots) = neo_trace::screenshot_queue();
    let trace = neo_trace::spawn_writer(store, redactor, writer, Some(shots));
    (trace, screenshots)
}

/// Neo's blank tab (`NeoTab`), kept open for the process's life: every close first makes sure of
/// it (`PlaywrightFacade::keep`), and a task follows the link's watch starts and its closes. The
/// sidecar's alone: the Node-hosted embedding's host launched its own browser.
fn keep_blank_tab(
    browser: &Arc<BrowserLink>,
    tabs: &Arc<TabRegistry>,
    store: &TraceStore,
    playwright: &Arc<PlaywrightFacade>,
    holders: &Arc<TabHolders>,
    shutdown: &CancellationToken,
) -> Arc<NeoTab> {
    let blank_tab = NeoTab::new(Arc::clone(browser), Arc::clone(tabs), store.clone());
    playwright.keep(Arc::clone(&blank_tab));
    let follower = Arc::clone(&blank_tab);
    let holders = Arc::clone(holders);
    let shutdown = shutdown.clone();
    tokio::spawn(async move {
        // Not before the task book took back its saved tabs: until then the registry cannot
        // tell a task's blank tab from a free one, and Neo's tab would adopt it. The follow
        // starts from a fresh listing, so nothing it missed meanwhile is lost.
        holders.ready().await;
        follower.follow(shutdown).await;
    });
    blank_tab
}

/// Tells the trace writer what to record (`trace.screenshots`, `trace.valueBytes`, `trace.values`) now and after
/// every change of the host's settings, before the change answers. The policy travels in the
/// writer's own queue, so a change applies from the next operation that ends.
fn follow_trace_policy(settings: &Settings, trace: TraceSink) {
    settings.watch(move |settings| trace.record_policy(settings.trace_policy()));
}

/// Neo's periodic upkeep, one task until `shutdown`: every [`Defaults::HOUSEKEEPING_EVERY`] the
/// janitor closes idle agent tabs; every
/// [`Defaults::TRACE_SWEEP_EVERY`] the trace writer deletes records past retention.
///
/// Both intervals fire once at start, so a Neo that restarts often still sweeps. A step that
/// fails is logged and tried again on its next tick: none of them may take the process down,
/// and none blocks a run. A sweep in progress at SIGTERM is dropped with the runtime; the
/// store marks its run `interrupted` at the next start. Each step reads the host's settings as
/// it starts (`tabs.idleCloseSeconds`, `trace.retentionSeconds`), so a change applies at its
/// next tick, with no restart.
async fn housekeeping(
    janitor: TabJanitor,
    trace: TraceSink,
    outputs: RunOutputs,
    settings: Settings,
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
                if let Err(error) = outputs.expire_blobs(EpochMillis::now()).await {
                    warn!(%error, "expired result blobs could not be removed; retrying next minute");
                }
                let now = time::Instant::now();
                if let Some(run) = janitor.sweep(now).await {
                    debug!(%run, "idle-tab sweep recorded");
                }
            }
            _ = retention.tick() => {
                let policy = RetentionPolicy {
                    max_age: settings.trace_retention(),
                    ..RetentionPolicy::default()
                };
                match trace.sweep(policy, EpochMillis::now()).await {
                    Ok(swept) => debug!(?swept, "trace retention sweep"),
                    Err(error) => warn!(%error, "the trace retention sweep failed; retrying on the next tick"),
                }
            }
        }
    }
}

/// Opens the trace store in `trace_dir` (`EnvConfig::trace_dir`), or, when the file does not open (a schema this build does not know, a corrupt
/// file), moves it aside and opens a fresh one: Neo must not crash-loop on its own records. The
/// old file is kept beside the new one for a human to look at.
///
/// A fresh store loses request history: a host retrying a `requestId` from before the
/// replacement runs it again. Log that boundary for the operator before moving the old store.
async fn open_trace(trace_dir: &Path) -> Result<TraceStore, StartError> {
    let path = trace_dir.join("trace.db");
    let store = match TraceStore::open(&path).await {
        Ok(store) => store,
        Err(error) => {
            let aside = PathBuf::from(format!(
                "{}.broken-{}",
                path.display(),
                EpochMillis::now().millis()
            ));
            error!(%error, path = %path.display(), aside = %aside.display(), "the trace store does not open; starting a fresh one. Its request ids are lost, so a requestId retried from before this start runs again");
            fs::rename(&path, &aside).await?;
            for suffix in ["-wal", "-shm"] {
                let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
                // Absent when SQLite never wrote one.
                let _ = fs::remove_file(sidecar).await;
            }
            TraceStore::open(&path).await?
        }
    };
    // Output recovery must not pass through the database replacement path: a failed file
    // cleanup is not a corrupt trace and must never discard request history.
    store
        .outputs()
        .restore_blob_expiries(EpochMillis::now())
        .await?;
    Ok(store)
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
    if env.ignored_trace_value_bytes {
        warn!(
            "CLAW_TRACE_VALUE_BYTES is ignored: set trace.valueBytes through PUT or PATCH /v1/settings"
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
}

#[cfg(test)]
mod housekeeping_tests {
    use std::sync::Arc;
    use std::task::Poll;
    use std::time::Duration;

    use futures_util::poll;
    use neo_trace::{BlobId, EpochMillis, RunId};
    use tokio::{task, time};

    use super::housekeeping;
    use crate::builtins::TabJanitor;
    use crate::config::Defaults;
    use crate::host::HostHolds;
    use crate::test_rig::{TestError, TestRig, within};

    #[tokio::test]
    async fn minute_housekeeping_expires_blobs_after_its_startup_pass() -> Result<(), TestError> {
        let rig = TestRig::new().await?;
        let outputs = rig.store.outputs().clone();
        let janitor = TabJanitor::new(
            Arc::clone(&rig.dispatcher),
            Arc::clone(&rig.link),
            Arc::clone(&rig.tabs),
            Arc::new(HostHolds::default()),
            rig.settings.clone(),
        );
        // Every wait below only bounds a hang: a passing run takes milliseconds, and a deadline
        // costs nothing then, so it is generous for GitHub's runners and a loaded machine (one
        // run in 1,600 missed 5 s with 200 copies running at once).
        let hang = Duration::from_secs(30);
        let mut upkeep = Box::pin(housekeeping(
            janitor,
            rig.sink.clone(),
            outputs.clone(),
            rig.settings.clone(),
            rig.shutdown.clone(),
        ));

        // The minute pass's startup tick must have run before the test writes a blob that is
        // already due, or that pass removes it. Two polls do not guarantee it: an interval's
        // first tick is due at once, but tokio fires a timer only once its millisecond has
        // passed, so neither tick may be ready on the first poll, and `select!` then takes the
        // two ready ticks in random order, so the retention tick can take the second poll and
        // leave the minute pass for later (1 run in 50 at a load of 12). So the test waits for
        // the pass itself: a sentinel blob, already due, is gone only once the startup pass ran.
        let sentinel = RunId::new();
        let blob = BlobId::new(0);
        outputs
            .keep_blobs_until(sentinel, EpochMillis::now().plus_millis(-1))
            .await?;
        outputs.keep_blob(sentinel, blob, b"due at startup").await?;
        task::yield_now().await;
        let starting = time::Instant::now();
        within(hang, async {
            loop {
                assert!(poll!(upkeep.as_mut()).is_pending());
                // The retention tick's request settles here, whichever pass the poll took.
                rig.sink.flush().await?;
                if outputs.open_blob(sentinel, blob).await?.is_none() {
                    return Ok::<(), TestError>(());
                }
                task::yield_now().await;
            }
        })
        .await??;
        // The minute pass's next tick is due a period after the startup tick was taken, at the
        // latest by now. Only that pass matters to the blob: retention removes a run's outputs
        // with its rows, and the run below has none (checked).
        let started_by = time::Instant::now();

        let expiring = RunId::new();
        let ordinary = RunId::new();
        // Blob expiry uses wall time, independently of the paused interval clock below. The
        // startup pass may still be removing the sentinel's expiry sidecar, and it holds the
        // expiry list's lock until it ends, which `keep_blobs_until` waits for: only polling
        // the housekeeping future ends that pass, so the writes run while the test drives it.
        let mut writes = Box::pin(async {
            outputs
                .keep_blobs_until(expiring, EpochMillis::now().plus_millis(-1))
                .await?;
            outputs.keep_blob(expiring, blob, b"expires").await?;
            outputs
                .keep_blob(ordinary, blob, b"ordinary retention")
                .await?;
            Ok::<(), TestError>(())
        });
        within(hang, async {
            loop {
                assert!(poll!(upkeep.as_mut()).is_pending());
                if let Poll::Ready(written) = poll!(writes.as_mut()) {
                    return written;
                }
                task::yield_now().await;
            }
        })
        .await??;
        // No SQLite row exists for retention to select and remove with its outputs.
        assert!(rig.store.run_summary(expiring).await?.is_none());
        let blob_path = rig
            .dir
            .path()
            .join("outputs")
            .join(expiring.to_string())
            .join("blobs")
            .join(blob.to_string());

        time::pause();
        let worker = tokio::spawn(upkeep);
        let before_tick = starting + Defaults::HOUSEKEEPING_EVERY - Duration::from_secs(1);
        time::advance(before_tick.saturating_duration_since(time::Instant::now())).await;
        // A synchronous read prevents filesystem awaits from auto-advancing paused time.
        assert!(
            blob_path.try_exists()?,
            "startup must not remove the new blob"
        );
        let next_tick = started_by + Defaults::HOUSEKEEPING_EVERY;
        time::advance(next_tick.saturating_duration_since(time::Instant::now())).await;
        time::resume();

        let expired = within(hang, async {
            while outputs.open_blob(expiring, blob).await?.is_some() {
                time::sleep(Duration::from_millis(5)).await;
            }
            Ok::<(), TestError>(())
        })
        .await;
        rig.shutdown.cancel();
        within(hang, worker).await??;
        expired??;
        assert!(outputs.open_blob(ordinary, blob).await?.is_some());
        Ok(())
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
    async fn a_newer_migration_preserves_the_old_store_and_starts_fresh()
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

        // Exercise the startup boundary: refuse the unknown schema, await close, preserve
        // the refused file, then start successfully on a fresh store.
        let store = open_trace(directory.path()).await?;
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
        assert!(store.runs(RunQuery::default()).await?.items.is_empty());
        assert!(
            store
                .run_by_request(&RequestId::parse("before-rollback")?, None)
                .await?
                .is_none(),
            "the owner accepted lost request history across a newer-migration rollback"
        );
        Ok(())
    }
}
