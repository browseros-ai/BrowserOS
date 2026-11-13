//! Neo's one screen over RFB: views forward pixels while dropping input, and control displays
//! forward the owner's keys, clipboard and pointer to Xvnc. Admission connects to Xvnc
//! before the WebSocket upgrade, so failures stay ordinary HTTP errors.
//!
//! Control admission takes the owner's screen hold before waiting out an agent move already
//! in flight. The admitted socket owns that hold until forwarding ends, including failed upgrades.
//! Spectator streams stay open throughout control input and takeovers. Their followers pause
//! while the owner holds the front, so viewing cannot move the owner away from their page.
//!
//! A bound display runs under its binding's task tracker. Release cancels it and waits for its
//! forwarding to stop before restoring the fit and lifting the agent's lock. RFB bytes are
//! framed and judged by mode, never interpreted as field values.
//!
//! A control display does not end when its forwarding stops. It half-closes its connection to
//! Xvnc and waits, at most [`XVNC_DRAIN`], for Xvnc to close it. Xvnc reads a connection in order
//! and closes it at its end of stream, so that close shows the owner's last input was read, and
//! a host may read what they typed once the display ended. The display's binding keeps whether
//! that held, and a takeover's release reports it.
//!
//! ```text
//!  viewer's close frame, or release ─▶ forwarding stops ─▶ FIN to Xvnc
//!    ─▶ Xvnc's bytes read and dropped until it closes (≤ 2 s) ─▶ drained, or not
//!    ─▶ the display ends: the viewer's close is answered, a release restores the fit
//! ```

use crate::browser_tabs::{ScreenHoldGuard, ViewWatchGuard};
use crate::follow::Follower;
use crate::limits::{SERVER_CHUNK, XVNC_DIAL, XVNC_DRAIN};
use crate::rfb_client::{ClientFramer, DisplayMode, Verdict, verdict};
use crate::{BrowserTabs, Front, Locks, PartError, ScreenHolder, follow, screen_rule};
use browseros_core::TabId;
use bytes::{Bytes, BytesMut};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use neo_trace::{TaskId, TraceSink, TraceStore};
use std::io;
use std::net::SocketAddr;
use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// What a host opens. A view can follow and a control display can be bound, and neither can
/// carry the other's: the route turns `followTab` on a control display, or `release` on a view,
/// into a 400 before it gets here.
#[derive(Clone, Debug)]
pub enum DisplayRequest {
    /// A spectator's display: the screen, and nothing the viewer sends reaches the browser.
    View {
        /// What to keep in front of the screen while the display is open.
        follow: Option<Follow>,
    },
    /// The owner's display: their keys, pastes and clicks reach the browser.
    Control {
        /// Ends the display when it closes, such as a takeover's (`release=`).
        binding: Option<DisplayBinding>,
    },
}

/// What a view display keeps in front of Neo's one screen: `followTab` or `followTask`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Follow {
    /// This tab.
    Tab(TabId),
    /// The tab the task's latest operation targeted, the agent's own `page` default.
    Task(TaskId),
}

/// Why a display ended. The service turns each into the WebSocket close its viewer gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayEnd {
    /// The viewer went away, or stopped taking bytes.
    ViewerLeft,
    /// A bound display closed by its binding: its takeover was released.
    Released,
    /// The viewer sent bytes the filter refuses; the reason says what.
    Refused(&'static str),
    /// Xvnc closed the connection.
    ScreenClosed,
}

/// Ties control displays to something that ends them, such as a takeover (`release=`). Cheap to
/// clone; every clone is the same binding.
#[derive(Clone, Debug, Default)]
pub struct DisplayBinding {
    /// Cancelled once, by [`DisplayBinding::close`].
    released: CancellationToken,
    /// Each bound display while it forwards RFB bytes and drains its connection to Xvnc.
    live: TaskTracker,
    /// Whether every bound display that ended drained; `None` until one has ended.
    drained: Arc<Mutex<Option<bool>>>,
}

impl DisplayBinding {
    /// A binding no display is bound to yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ends every display bound to this one, and returns once each has stopped forwarding and
    /// drained its connection to Xvnc, at most [`XVNC_DRAIN`] later. A display bound after this
    /// ends at once, so a release leaves no moment in which a display that just bound could take
    /// input.
    pub async fn close(&self) {
        self.released.cancel();
        self.live.close();
        self.live.wait().await;
    }

    /// Whether [`DisplayBinding::close`] was called.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.released.is_cancelled()
    }

    /// Whether Xvnc read all of the owner's input: `None` while no display bound to this one has
    /// ended, then `true` only if every one that ended forwarded each message whole and saw Xvnc
    /// close the connection it had half-closed. A display still forwarding makes it `false`, since
    /// its last input is unproven.
    #[must_use]
    pub fn input_drained(&self) -> Option<bool> {
        let drained = *self.drained.lock().unwrap_or_else(PoisonError::into_inner);
        if self.live.is_empty() {
            drained
        } else {
            Some(false)
        }
    }

    /// One bound display ended, drained or not.
    fn ended(&self, drained: bool) {
        let mut all = self.drained.lock().unwrap_or_else(PoisonError::into_inner);
        *all = Some(all.unwrap_or(true) && drained);
    }
}

/// The shared screen endpoint. Views filter input and may follow a tab; control displays
/// own a browser screen hold until their forwarding ends, including failed upgrades.
pub struct Displays {
    xvnc: SocketAddr,
    pub(crate) browser: BrowserTabs,
    /// Follow pauses while a lock holds the tab it follows.
    pub(crate) locks: Arc<Locks>,
    /// A followed task's operations, as the trace writer commits them.
    pub(crate) sink: TraceSink,
    /// A followed task's current tab before its next operation, and after a lag.
    pub(crate) store: TraceStore,
    /// Log correlation only; screen ownership lives in each admitted control display.
    next_id: AtomicU64,
}

impl Displays {
    /// Builds the shared screen endpoint; follow reads the same locks and trace as the service.
    #[must_use]
    pub fn new(
        xvnc: SocketAddr,
        browser: BrowserTabs,
        locks: Arc<Locks>,
        sink: TraceSink,
        store: TraceStore,
    ) -> Arc<Self> {
        Arc::new(Self {
            xvnc,
            browser,
            locks,
            sink,
            store,
            next_id: AtomicU64::new(1),
        })
    }

    /// Connects to Xvnc before the WebSocket upgrade. Control holds the owner's front before
    /// waiting out an agent move already in flight; views remain available throughout.
    ///
    /// # Errors
    /// `Screen` if Xvnc cannot be reached.
    pub async fn admit(self: &Arc<Self>, request: DisplayRequest) -> Result<Admitted, PartError> {
        let role = match request {
            DisplayRequest::View { follow } => {
                // In the registry before the follower looks up its target, so an implicit agent
                // move that decides after this sees the viewer; then a move already past its last
                // check lands before the view is admitted, as a control display waits it out.
                let watch = self.browser.watch_screen(follow.clone());
                drop(self.browser.front_turn().await);
                let follower = match follow {
                    Some(follow) => Some(follow::admit(follow, self).await),
                    None => None,
                };
                Role::View {
                    follower,
                    _watch: watch,
                }
            }
            DisplayRequest::Control { binding } => {
                let held = self.browser.hold_screen(ScreenHolder::ControlDisplay);
                // The hold refuses every new agent move. Waiting out the turn lets a move
                // already past its last check land before the owner's display opens.
                drop(self.browser.front_turn().await);
                Role::Control {
                    binding,
                    _held: held,
                }
            }
        };
        let server = self.dial().await?;
        Ok(Admitted {
            displays: Arc::clone(self),
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            role,
            server,
        })
    }

    /// The agent's side of this screen, for the service to hand to the app.
    #[must_use]
    pub fn front(self: &Arc<Self>) -> Front {
        Front::new(Arc::clone(self))
    }

    /// Whether the owner holds the screen: the screen rule over the holds registry. Follow
    /// pauses while it does.
    pub(crate) fn screen_held(&self) -> bool {
        screen_rule::decide(self.browser.screen_facts()).is_err()
    }

    async fn dial(&self) -> Result<TcpStream, PartError> {
        let server = tokio::time::timeout(XVNC_DIAL, TcpStream::connect(self.xvnc))
            .await
            .unwrap_or_else(|_| {
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("Xvnc did not accept a connection within {XVNC_DIAL:?}"),
                ))
            })
            .map_err(PartError::Screen)?;
        // A key is a few bytes, and it should reach Xvnc as it is sent.
        if let Err(error) = server.set_nodelay(true) {
            tracing::debug!(%error, "a display's connection to Xvnc kept Nagle's algorithm");
        }
        Ok(server)
    }
}

/// A connected display ready for WebSocket forwarding. A control display owns its screen hold
/// here, so cancellation, a failed upgrade, or dropping it unserved always releases ownership.
pub struct Admitted {
    displays: Arc<Displays>,
    id: u64,
    role: Role,
    server: TcpStream,
}

/// Control owns its screen hold here; views never take ownership of the front.
enum Role {
    View {
        /// The view's follower as its admission left it: subscribed to what it follows from
        /// before its look-up, so it misses nothing that happens before the view is served.
        follower: Option<Follower>,
        /// The view in the registry the agent's implicit moves read, until the display ends.
        _watch: ViewWatchGuard,
    },
    Control {
        binding: Option<DisplayBinding>,
        _held: ScreenHoldGuard,
    },
}

impl Admitted {
    /// Runs the display until its viewer leaves, Xvnc closes, the filter refuses the viewer, or
    /// its takeover binding closes at release.
    /// `from_viewer` and `to_viewer` carry raw RFB bytes; the service takes them out of
    /// WebSocket frames and puts them back in.
    ///
    /// A bound display runs inside its binding's tracker, so [`DisplayBinding::close`] waits for
    /// its forwarding to stop, and its drain, before the takeover restores the fit.
    pub async fn serve<I, O>(self, from_viewer: I, to_viewer: O) -> DisplayEnd
    where
        I: Stream<Item = Bytes> + Send,
        O: Sink<Bytes> + Send,
    {
        let binding = match &self.role {
            Role::Control {
                binding: Some(binding),
                ..
            } => Some(binding.clone()),
            Role::View { .. } | Role::Control { binding: None, .. } => None,
        };
        match binding {
            Some(binding) => {
                binding
                    .live
                    .track_future(self.run(from_viewer, to_viewer))
                    .await
            }
            None => self.run(from_viewer, to_viewer).await,
        }
    }

    async fn run<I, O>(self, from_viewer: I, to_viewer: O) -> DisplayEnd
    where
        I: Stream<Item = Bytes> + Send,
        O: Sink<Bytes> + Send,
    {
        let Self {
            displays,
            id,
            role,
            server,
        } = self;
        let (end, drained) = match role {
            Role::View {
                follower,
                _watch: watch,
            } => {
                let _watch = watch;
                let closing = Closing::Never;
                let proxy =
                    Proxy::new(DisplayMode::View).run(server, from_viewer, to_viewer, &closing);
                let ended = match follower {
                    None => proxy.await,
                    Some(follower) => {
                        let mut proxy = pin!(proxy);
                        // A viewer or Xvnc that already ended stops its follower before the
                        // follower can send another activation.
                        tokio::select! {
                            biased;
                            ended = &mut proxy => ended,
                            () = follow::run(follower, &displays) => proxy.await,
                        }
                    }
                };
                (ended.end, ended.drained)
            }
            Role::Control {
                binding,
                _held: held,
            } => {
                let closing = binding.as_ref().map_or(Closing::Never, |binding| {
                    Closing::Released(binding.released.clone())
                });
                let ended = Proxy::new(DisplayMode::Control)
                    .run(server, from_viewer, to_viewer, &closing)
                    .await;
                // Before the display leaves its binding's tracker, so a release that waited for
                // it reads the outcome.
                if let (Some(binding), Some(drained)) = (&binding, ended.drained) {
                    binding.ended(drained);
                }
                // Owner front ownership outlives every byte this display forwards, and the
                // drain that proves Xvnc took them.
                drop(held);
                (ended.end, ended.drained)
            }
        };
        tracing::info!(display = id, ?end, ?drained, "a display ended");
        end
    }
}

/// What ends a display before its viewer or Xvnc does.
enum Closing {
    /// A bound control display: its binding closed.
    Released(CancellationToken),
    /// A view or an unbound control display: only its viewer or Xvnc ends it.
    Never,
}

impl Closing {
    /// How the display ends if its close already fired; `None` while it is open. Sync, so the
    /// proxy can ask between a read and its send.
    fn ended(&self) -> Option<DisplayEnd> {
        match self {
            Self::Released(released) => released.is_cancelled().then_some(DisplayEnd::Released),
            Self::Never => None,
        }
    }

    async fn closed(&self) -> DisplayEnd {
        match self {
            Self::Released(released) => {
                released.cancelled().await;
                DisplayEnd::Released
            }
            Self::Never => std::future::pending().await,
        }
    }
}

/// One display's two directions, run side by side: viewer bytes framed and judged on the way to
/// Xvnc, Xvnc's passed to the viewer as they came.
struct Proxy {
    mode: DisplayMode,
    framer: ClientFramer,
    /// A message's write to Xvnc began and has not finished. A release cancels forwarding at
    /// once, so it can cut a message in two: Xvnc then reads a truncated message before Neo's
    /// end of stream, and its close proves nothing about the owner's input.
    writing: bool,
}

impl Proxy {
    fn new(mode: DisplayMode) -> Self {
        Self {
            mode,
            framer: ClientFramer::default(),
            writing: false,
        }
    }

    /// Runs until one side ends, the viewer is refused, or `closing` fires; then a control
    /// display drains its connection to Xvnc ([`drain`]).
    ///
    /// The two directions never wait on each other, and `closing` races both: a viewer that
    /// stopped reading, a stuck Xvnc never keeps a display that must close
    /// forwarding, and a release never waits on them. The drain is bounded by [`XVNC_DRAIN`].
    async fn run<I, O>(
        mut self,
        server: TcpStream,
        from_viewer: I,
        to_viewer: O,
        closing: &Closing,
    ) -> Ended
    where
        I: Stream<Item = Bytes>,
        O: Sink<Bytes>,
    {
        let (mut from_server, mut to_server) = server.into_split();
        let end = tokio::select! {
            biased;
            end = closing.closed() => end,
            end = screen_to_viewer(&mut from_server, to_viewer, closing) => end,
            end = self.viewer_to_screen(from_viewer, &mut to_server) => end,
        };
        let drained = match (self.mode, end) {
            (DisplayMode::View, _) => None,
            // Xvnc ended the connection, or refused a write, before Neo was done: nothing shows
            // it read the last input.
            (DisplayMode::Control, DisplayEnd::ScreenClosed) => Some(false),
            (DisplayMode::Control, _) => {
                let closed = drain(from_server, to_server).await;
                Some(closed && !self.writing)
            }
        };
        Ended { end, drained }
    }

    /// The viewer's bytes as they come, through the framer and the verdict table, to Xvnc.
    async fn viewer_to_screen<I>(
        &mut self,
        from_viewer: I,
        to_server: &mut OwnedWriteHalf,
    ) -> DisplayEnd
    where
        I: Stream<Item = Bytes>,
    {
        let mut from_viewer = pin!(from_viewer);
        loop {
            let Some(bytes) = from_viewer.next().await else {
                return DisplayEnd::ViewerLeft;
            };
            self.framer.push(&bytes);
            if let Err(end) = self.forward(to_server).await {
                return end;
            }
        }
    }

    /// Every whole message the framer holds: each is judged, and the ones that pass go to Xvnc
    /// in order.
    async fn forward(&mut self, to_server: &mut OwnedWriteHalf) -> Result<(), DisplayEnd> {
        while let Some((message, bytes)) =
            self.framer.next_message().map_err(DisplayEnd::Refused)?
        {
            match verdict(self.mode, message) {
                Verdict::Drop => continue,
                Verdict::Pass => {}
            }
            self.writing = true;
            to_server
                .write_all(&bytes)
                .await
                .map_err(|_| DisplayEnd::ScreenClosed)?;
            self.writing = false;
        }
        Ok(())
    }
}

/// Xvnc's bytes as they come, untouched, to the viewer, until `closing` fires.
///
/// Release is checked after every read, before sending its bytes. A single poll can read and
/// send multiple chunks, so the outer select alone would allow bytes read after cancellation
/// to escape. Binding close waits for this forwarding to end before the fit is restored.
async fn screen_to_viewer<O>(
    from_server: &mut OwnedReadHalf,
    to_viewer: O,
    closing: &Closing,
) -> DisplayEnd
where
    O: Sink<Bytes>,
{
    let mut to_viewer = pin!(to_viewer);
    let mut chunk = BytesMut::new();
    loop {
        chunk.reserve(SERVER_CHUNK);
        match from_server.read_buf(&mut chunk).await {
            Ok(0) | Err(_) => return DisplayEnd::ScreenClosed,
            Ok(_) => {}
        }
        if let Some(end) = closing.ended() {
            return end;
        }
        if to_viewer.send(chunk.split().freeze()).await.is_err() {
            return DisplayEnd::ViewerLeft;
        }
    }
}

/// How a display's proxy ended, and for a control display whether its drain held.
struct Ended {
    end: DisplayEnd,
    /// `None` for a view, which forwards no input to prove.
    drained: Option<bool>,
}

/// Half-closes a control display's connection to Xvnc, then reads and drops what Xvnc still sends
/// until Xvnc closes it, for at most [`XVNC_DRAIN`]. Whether Xvnc closed in time.
///
/// Xvnc reads a connection in order and closes it at its end of stream, so its close shows it
/// read every byte before Neo's FIN, the owner's last input included. Nothing read here reaches
/// the viewer, which keeps a release's rule that no frame read after it does. Reading to the end
/// also leaves nothing unread on the socket, so closing it sends a FIN, not a reset.
async fn drain(mut from_server: OwnedReadHalf, mut to_server: OwnedWriteHalf) -> bool {
    if to_server.shutdown().await.is_err() {
        return false;
    }
    let mut dropped = vec![0; SERVER_CHUNK];
    let closed = async {
        loop {
            match from_server.read(&mut dropped).await {
                Ok(0) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    };
    tokio::time::timeout(XVNC_DRAIN, closed)
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{Closing, DisplayEnd, screen_to_viewer};
    use bytes::Bytes;
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// A bound display's release cancelled between two reads: the first read went out before the close, and
    /// the bytes Xvnc sent after it are waiting on the socket. Nothing of those reaches the
    /// viewer, so a close cancelled before an activation is sent keeps every byte Xvnc paints
    /// after it from the viewer.
    #[tokio::test]
    async fn screen_to_viewer_forwards_nothing_read_after_its_close() -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let display = TcpStream::connect(listener.local_addr()?).await?;
        let (mut xvnc, _) = listener.accept().await?;
        let (from_server, _to_server) = display.into_split();
        let close = CancellationToken::new();
        let (to_viewer, mut viewer) = mpsc::unbounded_channel::<Bytes>();
        let to_viewer =
            futures_util::sink::unfold(to_viewer, |to_viewer, bytes: Bytes| async move {
                to_viewer.send(bytes).map_err(|_| "the viewer is gone")?;
                Ok::<_, &'static str>(to_viewer)
            });
        let proxy = tokio::spawn({
            let closing = Closing::Released(close.clone());
            let mut from_server = from_server;
            async move { screen_to_viewer(&mut from_server, to_viewer, &closing).await }
        });

        xvnc.write_all(b"before").await?;
        let mut before = Vec::new();
        while before.len() < b"before".len() {
            before.extend_from_slice(&viewer.recv().await.ok_or("the proxy ended")?);
        }
        assert_eq!(before, b"before", "read before the close, so it goes out");

        close.cancel();
        xvnc.write_all(b"after").await?;
        drop(xvnc);
        let mut after = Vec::new();
        while let Some(chunk) = viewer.recv().await {
            after.extend_from_slice(&chunk);
        }
        assert_eq!(
            String::from_utf8_lossy(&after),
            "",
            "nothing read after the close reaches the viewer"
        );
        assert_eq!(
            proxy.await?,
            DisplayEnd::Released,
            "the read after the close ends it"
        );
        Ok(())
    }
}
