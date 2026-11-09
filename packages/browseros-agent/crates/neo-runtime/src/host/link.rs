//! Neo: claw-server's `Link` (`apps/claw-server/src/services/capture/link.rs`) and `CdpPortTransport` (`apps/claw-server/src/services/capture/ports.rs`), isolating frame traffic from run commands on a separate socket.
//!
//! The host side's own CDP connection: one `CdpClient` to the browser's port, never the runs'
//! client, shared by the presences and every capture, over `browseros_cdp::CdpClient` directly.
//!
//! Why a socket of its own: screencast frames are large and frequent, and on the runs'
//! connection they would queue every run command and keepalive behind them.
//! This client gives its one `events_targeted` subscriber for `Page.screencastFrame` to the
//! link, so frames bypass its 4096-slot ring, which then carries only `Target.*` events and
//! the host side's own sessions' events. A run never shares that subscriber: runs read their
//! own client's ring (`page.rs`'s watcher).
//!
//! ```text
//!  CdpClient ─ events_targeted(Page.screencastFrame) ─▶ route_frames ─▶ an mpsc per session
//!     └─ events() ring ─ Target.detachedFromTarget ─▶ watch_detaches ─▶ that Attached is gone
//!  watchdog (1 s): down or a new epoch ─▶ the Generation ends ─▶ all its Attached are gone
//! ```
//!
//! Generations: a flat session id belongs to one socket incarnation. browseros-cdp reconnects
//! by itself and announces nothing, so the watchdog compares `CdpClient::epoch` every
//! [`cdp::WATCHDOG_INTERVAL`]. A change ends the old [`Generation`], which cancels every
//! [`Attached`] made on it, and publishes the new one once the socket is up, which is what
//! wakes followers to re-lease. Every command on an attached session goes out with
//! `send_at_epoch` at the epoch it was attached in, so one racing the reconnect is refused
//! before it is written: no stale session id ever reaches the replacement socket.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId};
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{self, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::debug;

use crate::host::cdp;
use crate::run::PageId;

/// The host side's connection to the browser. Its tasks (frame routing, detach watching and
/// the watchdog) run until `shutdown`.
pub(crate) struct HostLink {
    client: CdpClient,
    /// The newest socket incarnation. Ended while the socket is down.
    generation: watch::Sender<Arc<Generation>>,
    /// Every session attached through the link and not yet released, by id: how a frame or a
    /// detach finds its owner.
    sessions: Mutex<HashMap<SessionId, Route>>,
    shutdown: CancellationToken,
    tasks: TaskTracker,
    directory_required: AtomicBool,
    directory_epoch: AtomicU64,
}

struct Route {
    gone: CancellationToken,
    frames: Option<mpsc::UnboundedSender<Value>>,
}

/// One socket incarnation. Its sessions exist only on it.
pub(crate) struct Generation {
    epoch: u64,
    /// Cancelled when the socket dropped or was replaced, and at shutdown.
    ended: CancellationToken,
}

/// A host-owned flat session on one tab, attached on one [`Generation`].
pub(crate) struct Attached {
    pub(crate) session: SessionId,
    generation: Arc<Generation>,
    /// Cancelled when the browser detaches the session (its tab closed), when the link
    /// releases it, or when its generation ends.
    gone: CancellationToken,
    client: CdpClient,
}

#[derive(Clone, Debug, thiserror::Error)]
pub(crate) enum LinkError {
    /// The socket is down and has not come back yet.
    #[error("the host side's browser connection is down")]
    Down,
    #[error(transparent)]
    Cdp(#[from] CdpError),
}

impl HostLink {
    /// Opens the host side's own client to the browser's CDP `port` and starts the link on it.
    /// `shutdown` also disconnects the client, which would otherwise keep reconnecting.
    pub(crate) async fn connect(
        port: u16,
        shutdown: CancellationToken,
    ) -> Result<Arc<Self>, CdpError> {
        let client = CdpClient::connect(cdp::connect_options(port)).await?;
        let link = Self::over(client, shutdown);
        let disconnect = link.client.clone();
        let stop = link.shutdown.clone();
        link.spawn(async move {
            stop.cancelled().await;
            disconnect.disconnect().await;
        });
        Ok(link)
    }

    /// Starts the link on `client`, which must be the host side's alone: the link takes its
    /// one `events_targeted` subscriber for screencast frames.
    pub(crate) fn over(client: CdpClient, shutdown: CancellationToken) -> Arc<Self> {
        let frames = client.events_targeted(cdp::SCREENCAST_FRAME);
        let events = client.events();
        let first = Generation::new(client.epoch(), &shutdown);
        if !client.is_connected() {
            first.ended.cancel();
        }
        let link = Arc::new(Self {
            client,
            generation: watch::Sender::new(Arc::new(first)),
            sessions: Mutex::new(HashMap::new()),
            shutdown,
            tasks: TaskTracker::new(),
            directory_required: AtomicBool::new(false),
            directory_epoch: AtomicU64::new(0),
        });
        link.spawn(route_frames(Arc::clone(&link), frames));
        link.spawn(watch_detaches(Arc::clone(&link), events));
        link.spawn(watchdog(Arc::clone(&link)));
        link
    }

    /// Every private-socket service, including abandoned attaches and late screenshots, stays
    /// owned until shutdown. The actor waits on this tracker within its single shutdown budget.
    pub(crate) fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.tasks.spawn(future)
    }

    pub(crate) fn stop(&self) {
        self.shutdown.cancel();
        self.tasks.close();
    }

    pub(crate) async fn stopped(&self) {
        self.tasks.wait().await;
    }

    /// The integrated host requires a seed of this socket before a follower can re-lease.
    /// Epoch equality closes the race where capture sees the new generation before discovery
    /// has even been scheduled. Standalone capture tests supply their directory views directly.
    pub(crate) fn require_directory(&self) {
        self.directory_required.store(true, Ordering::SeqCst);
    }
    pub(crate) fn directory_lost(&self) {
        self.directory_epoch.store(0, Ordering::SeqCst);
    }
    pub(crate) fn directory_seeded(&self, epoch: u64) {
        self.directory_epoch.store(epoch, Ordering::SeqCst);
    }
    pub(crate) fn capture_ready(&self) -> bool {
        !self.directory_required.load(Ordering::SeqCst)
            || (self.client.is_connected()
                && self.directory_epoch.load(Ordering::SeqCst) == self.client.epoch())
    }

    pub(crate) fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.client.events()
    }

    pub(crate) fn client(&self) -> &CdpClient {
        &self.client
    }

    /// The incarnation new sessions attach on, or [`LinkError::Down`] while there is none.
    pub(crate) fn current(&self) -> Result<Arc<Generation>, LinkError> {
        let generation = Arc::clone(&self.generation.borrow());
        if generation.is_ended() {
            return Err(LinkError::Down);
        }
        Ok(generation)
    }

    /// Changes whenever the watchdog publishes a new incarnation.
    pub(crate) fn generations(&self) -> watch::Receiver<Arc<Generation>> {
        self.generation.subscribe()
    }

    /// Attaches a host-owned flat session to `target` on the current incarnation.
    pub(crate) async fn attach(&self, target: &PageId) -> Result<Attached, LinkError> {
        let generation = self.current()?;
        let session = cdp::attach_to_target(&self.client, generation.epoch, target).await?;
        // A child of the generation's token: already cancelled if the socket went away while
        // the attach was in flight.
        let gone = generation.ended.child_token();
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(
                session.clone(),
                Route {
                    gone: gone.clone(),
                    frames: None,
                },
            );
        }
        Ok(Attached {
            session,
            generation,
            gone,
            client: self.client.clone(),
        })
    }

    /// The screencast frames Chromium sends on `session`, as `Page.screencastFrame` params.
    /// Register before `Page.startScreencast`, so the first frame finds its route. The channel
    /// is unbounded, but `maxFramesInFlight: 1` and the paced ack hold it to one frame.
    pub(crate) fn frames(&self, session: &Attached) -> mpsc::UnboundedReceiver<Value> {
        let (tx, rx) = mpsc::unbounded_channel();
        if let Ok(mut sessions) = self.sessions.lock()
            && let Some(route) = sessions.get_mut(&session.session)
        {
            route.frames = Some(tx);
        }
        rx
    }

    /// Forgets `session` and detaches it. Best effort: the tab may be gone, and a session of an
    /// ended incarnation is refused before anything is written.
    pub(crate) async fn release(&self, session: &Attached) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&session.session);
        }
        session.gone.cancel();
        if let Err(error) =
            cdp::detach_from_target(&self.client, session.generation.epoch, &session.session).await
        {
            debug!(session = %session.session, %error, "detaching a host session failed");
        }
    }

    /// Ends the published incarnation if the socket went down or was replaced, and publishes
    /// the socket's current one once it is up.
    fn check_epoch(&self) {
        let current = Arc::clone(&self.generation.borrow());
        let connected = self.client.is_connected();
        let epoch = self.client.epoch();
        if !current.is_ended() && (!connected || epoch != current.epoch) {
            current.ended.cancel();
        }
        if current.is_ended() && connected && !self.shutdown.is_cancelled() {
            self.generation
                .send_replace(Arc::new(Generation::new(epoch, &self.shutdown)));
        }
    }
}

impl Generation {
    fn new(epoch: u64, shutdown: &CancellationToken) -> Self {
        Self {
            epoch,
            ended: shutdown.child_token(),
        }
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.ended.is_cancelled()
    }
}

impl Attached {
    /// Sends `method` on this session at the epoch it was attached in: on any other socket
    /// incarnation the request is refused before it is written.
    pub(crate) async fn send(&self, method: &str, params: Value) -> Result<Value, CdpError> {
        self.client
            .send_at_epoch(self.generation.epoch, method, params, Some(&self.session))
            .await
    }

    pub(crate) fn is_gone(&self) -> bool {
        self.gone.is_cancelled()
    }

    /// Resolves once the session is gone: detached by the browser, released, or lost with its
    /// socket.
    pub(crate) async fn gone(&self) {
        self.gone.cancelled().await;
    }
}

/// The one targeted `Page.screencastFrame` subscriber, routing each frame to its session's
/// capture by the flat session id the event arrived on.
async fn route_frames(link: Arc<HostLink>, mut frames: mpsc::UnboundedReceiver<CdpEvent>) {
    loop {
        let event = tokio::select! {
            () = link.shutdown.cancelled() => return,
            event = frames.recv() => event,
        };
        // Closed only if something else took this client's targeted frame subscriber.
        let Some(event) = event else {
            debug!("the host link's screencast frame subscriber closed");
            return;
        };
        let Some(session) = event.session_id else {
            continue;
        };
        if let Ok(sessions) = link.sessions.lock()
            && let Some(frames) = sessions
                .get(&session)
                .and_then(|route| route.frames.as_ref())
        {
            // Fails only once the capture stopped reading, which it does before it releases.
            let _ = frames.send(event.params);
        }
    }
}

/// `Target.detachedFromTarget` → that session is gone. A lagged ring can lose one; then the
/// session's users still end with the session view (the tab leaves it) or the generation.
async fn watch_detaches(link: Arc<HostLink>, mut events: broadcast::Receiver<CdpEvent>) {
    loop {
        let event = tokio::select! {
            () = link.shutdown.cancelled() => return,
            event = events.recv() => event,
        };
        match event {
            Ok(event) => {
                if let Some(session) = cdp::detached_session(&event)
                    && let Ok(sessions) = link.sessions.lock()
                    && let Some(route) = sessions.get(&session)
                {
                    route.gone.cancel();
                }
            }
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}

async fn watchdog(link: Arc<HostLink>) {
    let mut tick = time::interval(cdp::WATCHDOG_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = link.shutdown.cancelled() => return,
            _ = tick.tick() => {}
        }
        link.check_epoch();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;
    use tokio::time;
    use tokio_util::sync::CancellationToken;

    use super::{Attached, HostLink};
    use crate::fake_host_cdp::FakeHostCdp;
    use crate::run::PageId;

    async fn link(browser: &FakeHostCdp) -> Arc<HostLink> {
        HostLink::connect(browser.port(), CancellationToken::new())
            .await
            .unwrap_or_else(|error| panic!("connect the host link: {error}"))
    }

    async fn attach(link: &HostLink, target: &str) -> Attached {
        link.attach(&PageId::new(target.to_string()))
            .await
            .unwrap_or_else(|error| panic!("attach {target}: {error}"))
    }

    /// Waits until the link publishes an incarnation other than `epoch`.
    async fn next_generation(link: &HostLink, epoch: u64) {
        let mut generations = link.generations();
        let wait =
            generations.wait_for(|generation| !generation.is_ended() && generation.epoch != epoch);
        time::timeout(Duration::from_secs(30), wait)
            .await
            .unwrap_or_else(|_| panic!("no new socket incarnation after epoch {epoch}"))
            .unwrap_or_else(|_| panic!("the link stopped publishing incarnations"));
    }

    #[tokio::test(start_paused = true)]
    async fn frames_reach_only_the_session_they_arrived_on() {
        let browser = FakeHostCdp::start().await;
        let link = link(&browser).await;
        let first = attach(&link, "target-1").await;
        let second = attach(&link, "target-2").await;
        let mut first_frames = link.frames(&first);
        let mut second_frames = link.frames(&second);

        let number = browser.frame(first.session.as_str(), b"jpeg", Some(1.0));

        let frame = time::timeout(Duration::from_secs(5), first_frames.recv()).await;
        assert_eq!(
            frame
                .ok()
                .flatten()
                .map(|params| params["sessionId"].clone()),
            Some(json!(number)),
            "the frame reached its own session"
        );
        time::sleep(Duration::from_millis(100)).await;
        assert!(
            second_frames.try_recv().is_err(),
            "the other session got nothing"
        );
        let attaches = browser.sent("Target.attachToTarget");
        assert_eq!(
            attaches
                .iter()
                .map(|command| command.params.clone())
                .collect::<Vec<_>>(),
            [
                json!({ "targetId": "target-1", "flatten": true }),
                json!({ "targetId": "target-2", "flatten": true }),
            ],
            "flat sessions on each target"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_the_browser_detaches_is_gone() {
        let browser = FakeHostCdp::start().await;
        let link = link(&browser).await;
        let attached = attach(&link, "target-1").await;

        browser.detach(attached.session.as_str());

        assert!(
            time::timeout(Duration::from_secs(5), attached.gone())
                .await
                .is_ok(),
            "the detach reached the session"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn releasing_detaches_and_stops_routing_frames() {
        let browser = FakeHostCdp::start().await;
        let link = link(&browser).await;
        let attached = attach(&link, "target-1").await;
        let mut frames = link.frames(&attached);

        link.release(&attached).await;
        browser.frame(attached.session.as_str(), b"late", Some(1.0));

        let detach = browser.wait_for("Target.detachFromTarget", 1).await;
        assert_eq!(
            detach[0].params,
            json!({ "sessionId": attached.session.as_str() }),
            "the released session was detached"
        );
        assert!(attached.is_gone(), "a released session is gone");
        assert_eq!(
            time::timeout(Duration::from_secs(5), frames.recv())
                .await
                .ok()
                .flatten(),
            None,
            "no frame is routed to a released session"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_socket_ends_every_session_of_the_old_one_and_never_sees_their_ids() {
        let browser = FakeHostCdp::start().await;
        let link = link(&browser).await;
        let old = attach(&link, "target-1").await;
        let epoch = old.generation.epoch;

        browser.drop_connection();
        next_generation(&link, epoch).await;

        assert!(old.is_gone(), "the old incarnation's session is gone");
        assert!(
            old.send("Page.stopScreencast", json!({})).await.is_err(),
            "a command on an old session is refused"
        );
        link.release(&old).await;
        let fresh = attach(&link, "target-1").await;
        assert!(
            fresh.send("Page.stopScreencast", json!({})).await.is_ok(),
            "a session of the new incarnation works"
        );
        let reconnected = browser.commands();
        let stale = reconnected
            .iter()
            .filter(|command| command.connection > 1)
            .filter(|command| {
                command.session.as_deref() == Some(old.session.as_str())
                    || command.params["sessionId"] == json!(old.session.as_str())
            })
            .count();
        assert_eq!(
            stale, 0,
            "no command named an old session id on the new socket"
        );
        assert!(
            reconnected
                .iter()
                .any(|command| command.connection == 2 && command.method == "Target.attachToTarget"),
            "the fresh attach went out on the new socket"
        );
    }
}
