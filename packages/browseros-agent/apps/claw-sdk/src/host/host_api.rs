//! The host API's life: its socket bound at startup, the service started once the browser
//! links, both served until SIGTERM.

use std::io;
use std::path::Path;
use std::sync::Arc;

use browseros_service::{HostAuth, HostToken, PeerRule, Service, ServiceInputs, TokenError};
use browseros_takeover::Xvnc;
use browseros_trace::{Privacy, ShotJobs, TraceSink, TraceStore};
use tokio::fs;
use tokio_util::sync::CancellationToken;

use super::{TakeoverLock, tab_feed};
use crate::browser::BrowserLink;
use crate::config::HostApiConfig;
use crate::http::{BindError, HostListener};

/// Neo's host API (browseros-service) from startup until it serves zuzu over the host socket.
///
/// ```text
///  start ─ bind: token file ─ unset or unusable: no socket (run still works)
///                           └ usable: HostListener::bind (failure fails the start) + peer rule
///  serve ─ the link's first connect ─▶ Service::start ─▶ TakeoverLock armed
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
}

/// What the service is built from that Neo holds at startup. The browser's half, the session
/// and the viewports, comes from the link once it connects. A plain data bag.
pub(crate) struct HostInputs {
    /// Read by the record routes, the event log and a lock's settle.
    pub(crate) store: TraceStore,
    /// The trace writer: its committed events feed `/v1/events`, and a lock flushes it.
    pub(crate) sink: TraceSink,
    /// Tab privacy over the secret set the dispatcher masks with.
    pub(crate) privacy: Arc<Privacy>,
    /// The camera's end of the writer's screenshot queue (`spawn_writer` has the other).
    pub(crate) screenshots: ShotJobs,
    /// Neo's screen, from `CLAW_XVNC_ADDR` and `NEO_SCREEN`. The service puts it home as it
    /// starts, so a fit a Neo restart cut short does not leave Xvnc at the owner's size.
    pub(crate) xvnc: Xvnc,
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
    /// is no socket, as browseros-service asks: never a host API behind a weak or unusable
    /// token. A socket that cannot be bound fails the start, because a host that configured a
    /// token expects the API to be there.
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
        Ok(Self { inputs, socket })
    }

    /// Starts the service once the browser link's first connect lands, then serves it on the
    /// socket until `shutdown`. `takeover` is armed before the socket serves, so a lock a host
    /// takes refuses the agent from its first moment, and the first reconciliation of tab state
    /// runs before it serves, so it cannot end a lock or private flag a host just set.
    pub(crate) async fn serve(
        self,
        link: Arc<BrowserLink>,
        takeover: Arc<TakeoverLock>,
        shutdown: CancellationToken,
    ) {
        let Self { inputs, socket } = self;
        let session = tokio::select! {
            () = shutdown.cancelled() => return,
            session = link.linked() => session,
        };
        let HostInputs {
            store,
            sink,
            privacy,
            screenshots,
            xvnc,
        } = inputs;
        let service = Service::start(ServiceInputs {
            store,
            sink,
            privacy,
            browser: session,
            viewports: link.viewports(),
            screenshots,
            xvnc,
        });
        takeover.arm(Arc::clone(service.locks()));
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
            .serve(
                browseros_service::host_service(service.router(), auth),
                shutdown,
            )
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
