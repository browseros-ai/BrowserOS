//! The host API's life: its socket bound at startup, the service started once the browser
//! links, both served until SIGTERM.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use neo_http::{
    HostAuth, HostToken, PeerRule, Service, ServiceInputs, Settings, SettingsDefaults, TokenError,
};
use neo_takeover::Xvnc;
use neo_trace::{ShotJobs, TraceSink, TraceStore};
use tokio::fs;
use tokio_util::sync::CancellationToken;

use super::{AgentFront, FillService, HostHolds, TakeoverLock, tab_feed};
use crate::browser::{BrowserLink, NeoTab, TabRegistry};
use crate::config::HostApiConfig;
use crate::http::{BindError, HostListener};

/// Neo's host API (neo-http) from startup until it serves the host over the host socket.
///
/// ```text
///  start ─ bind: token file ─ unset or unusable: no socket (run still works)
///                           └ usable: HostListener::bind (failure fails the start) + peer rule
///        └▶ load settings.json in either case, before the first run
///  serve ─ the link's first connect ─▶ Service::start ─▶ TakeoverLock, HostHolds, the fill routes armed
///        ─▶ tab events subscribed ─▶ tab state reconciled with a watched listing ─▶ tab_feed
///        └▶ host_service(service.router(), auth) on the socket, until SIGTERM
/// ```
///
/// The service needs the agent's `BrowserSession`, which exists only once the link's first
/// connect lands, so it starts then, not at startup. It starts whether or not a socket is bound:
/// the camera answers the trace writer's screenshot jobs and the tab bookkeeping runs either
/// way, so the trace is the same with or without a host.
pub(crate) struct HostApi {
    inputs: HostInputs,
    socket: Option<HostSocket>,
    settings: Settings,
    /// The host's private typing, armed into the service as it starts ([`HostApi::with_fill`]).
    fill: Option<Arc<FillService>>,
}

/// What the service is built from that Neo holds at startup. The browser's half, the session
/// and the viewports, comes from the link once it connects. A plain data bag.
pub(crate) struct HostInputs {
    /// Read by the record routes, the event log and a lock's settle.
    pub(crate) store: TraceStore,
    /// The trace writer: its committed events feed `/v1/events`, and a lock flushes it.
    pub(crate) sink: TraceSink,
    /// The camera's end of the writer's screenshot queue (`spawn_writer` has the other).
    pub(crate) screenshots: ShotJobs,
    /// Neo's screen, from `CLAW_XVNC_ADDR` and `NEO_SCREEN`. The service puts it home as it
    /// starts, so a fit a Neo restart cut short does not leave Xvnc at the owner's size.
    pub(crate) xvnc: Xvnc,
    /// Chromium's CDP port (`ports.cdp`), the one the browser link dials. The service's live tab
    /// views open a connection of their own to it, so their frames never queue behind the
    /// agent's commands.
    pub(crate) cdp_port: u16,
    /// Where `/v1/settings` saves what the host set: `settings.json` beside `trace.db`.
    pub(crate) settings_file: PathBuf,
    /// The downloads default discovered by the app, also used when there is no host listener.
    pub(crate) settings_defaults: SettingsDefaults,
    /// The app's tab registry, whose task book `GET /v1/tabs` reads for each tab's `taskId`.
    pub(crate) tabs: Arc<TabRegistry>,
}

/// The bound host listener and the auth in front of every request on it.
struct HostSocket {
    listener: HostListener,
    auth: HostAuth,
}

/// Why the token file gives no host token. The host API is then off, and `run` still works.
#[derive(Debug, thiserror::Error)]
enum UnusableToken {
    #[error("the token file cannot be read: {0}")]
    Unreadable(#[from] io::Error),
    #[error(transparent)]
    Invalid(#[from] TokenError),
}

impl HostApi {
    /// Binds the host socket when `CLAW_HOST_TOKEN_FILE` holds a usable token. Without one there
    /// is no socket, as neo-http asks: never a host API behind a weak or unusable
    /// token. A socket that cannot be bound fails the start, because a host that configured a
    /// token expects the API to be there.
    ///
    /// Settings are loaded regardless of the token. A host's startup file must apply before
    /// the first run even when its API token is missing or unusable.
    pub(crate) async fn bind(
        config: &HostApiConfig,
        inputs: HostInputs,
    ) -> Result<Self, BindError> {
        let socket = match &config.token_file {
            None => {
                tracing::warn!("no host API: CLAW_HOST_TOKEN_FILE is unset");
                None
            }
            Some(token_file) => match read_token(token_file).await {
                Ok(token) => Some(bind_socket(config, token).await?),
                Err(why) => {
                    tracing::error!(token_file = %token_file.display(), %why, "no host API: the host token is unusable");
                    None
                }
            },
        };
        let settings = Settings::load(
            inputs.settings_file.clone(),
            inputs.settings_defaults.clone(),
        )
        .await;
        Ok(Self {
            inputs,
            socket,
            settings,
            fill: None,
        })
    }

    /// The host's private typing (`POST /v1/tabs/{tabId}/fill`), which `serve` arms into the
    /// service before its socket serves. Without it the fill routes answer 503.
    #[must_use]
    pub(crate) fn with_fill(mut self, fill: Arc<FillService>) -> Self {
        self.fill = Some(fill);
        self
    }

    /// The settings every part reads (the janitor, the retention sweep), live: a host's change
    /// reaches them through these same values.
    #[must_use]
    pub(crate) fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Starts the service once the browser link's first connect lands, then serves it on the
    /// socket until `shutdown`. `takeover` is armed before the socket serves, so a lock a host
    /// takes refuses the agent from its first moment, and the first reconciliation of tab state
    /// runs before it serves, so it cannot end a lock a host just set. `holds` is armed with it,
    /// so the janitor sees every lock and fit a host takes.
    pub(crate) async fn serve(
        self,
        link: Arc<BrowserLink>,
        takeover: Arc<TakeoverLock>,
        front: Arc<AgentFront>,
        holds: Arc<HostHolds>,
        blank_tab: Option<Arc<NeoTab>>,
        shutdown: CancellationToken,
    ) {
        let Self {
            inputs,
            socket,
            settings,
            fill,
        } = self;
        let session = tokio::select! {
            () = shutdown.cancelled() => return,
            session = link.linked() => session,
        };
        let HostInputs {
            store,
            sink,
            screenshots,
            xvnc,
            cdp_port,
            settings_file: _,
            settings_defaults: _,
            tabs,
        } = inputs;
        let service = Service::start(ServiceInputs {
            store,
            sink,
            browser: session,
            viewports: link.viewports(),
            screenshots,
            xvnc,
            cdp_port,
            settings,
            // `GET /v1/tabs` marks Neo's blank tab, which no script sees, as Neo's.
            tab_owner: Arc::new(move |tab| {
                let held = blank_tab.as_ref().and_then(|blank_tab| blank_tab.current());
                (held == Some(tab)).then_some("neo")
            }),
            // `GET /v1/tabs` names the task holding each tab now: the one task scope binds it to.
            // A released takeover's `openedTabs` read it too, and the tab that opened each one.
            tab_task: {
                let tabs = Arc::clone(&tabs);
                Arc::new(move |tab| tabs.holder(tab).map(|holder| holder.task.to_string()))
            },
            tab_opener: Arc::new(move |tab| tabs.opener(tab)),
        });
        takeover.arm(Arc::clone(service.locks()));
        holds.arm({
            let service = service.clone();
            move |tab| service.tab_hold(tab)
        });
        // Before the socket serves, so no host request reaches an unarmed fill route, and the
        // camera knows every tab the fill types into from the fill's first character.
        if let Some(fill) = &fill {
            service.arm_fill(fill.door());
            service.arm_private_tabs(fill.took_private_text());
            service.arm_picture_check(fill.checks_picture());
        }
        // The owner's two claims, the tab and the screen, from the same moment: no implicit
        // agent move runs past a viewer or a takeover once a host can open one.
        front.arm(service.front().clone());
        // Subscribed before the reconciling listing is taken, which happens while tab_watch
        // follows the socket: every tab that closes after that listing reaches the feed as an
        // event, so no close falls between the two.
        let events = link.tab_events();
        // Until it returns, connections wait in the socket's backlog: no host request can set
        // tab state before the state of tabs that closed while Neo was not watching is gone.
        let reconciled = tokio::select! {
            () = shutdown.cancelled() => return,
            reconciled = tab_feed::reconcile(&service, &link) => reconciled,
        };
        tokio::spawn(tab_feed::tab_feed(
            service.clone(),
            Arc::clone(&link),
            events,
            reconciled,
        ));
        let Some(HostSocket { listener, auth }) = socket else {
            return;
        };
        tracing::info!("host API serving");
        if let Err(error) = listener
            .serve(neo_http::host_service(service.router(), auth), shutdown)
            .await
        {
            tracing::error!(%error, "the host listener stopped; the host API is down until Neo restarts");
        }
    }
}

/// Binds the listener, then reads the peer rule from the socket file it made: Neo created it,
/// so its owner is Neo's own uid and its group is the one a host joins to connect.
async fn bind_socket(config: &HostApiConfig, token: HostToken) -> Result<HostSocket, BindError> {
    let listener = HostListener::bind(config).await?;
    let peer = PeerRule::from_socket(&config.socket).map_err(|source| BindError::Io {
        what: format!("reading the owner of {}", config.socket.display()),
        source,
    })?;
    tracing::info!(
        socket = %config.socket.display(),
        tcp = ?listener.tcp_addr().ok().flatten(),
        "host API bound; it serves once the browser links"
    );
    // One auth for both transports: a Unix peer passes the rule and then the token, and a TCP
    // peer (`CLAW_HOST_TCP`, opt-in) the token alone.
    let auth = HostAuth {
        token,
        peer: Some(peer),
    };
    Ok(HostSocket { listener, auth })
}

async fn read_token(path: &Path) -> Result<HostToken, UnusableToken> {
    let contents = fs::read_to_string(path).await?;
    Ok(HostToken::parse(&contents)?)
}
