//! Neo's one screen over RFB: views forward pixels while dropping input, and control displays
//! forward the owner's keys, clipboard and pointer to Xvnc. Admission reserves the display and
//! dials Xvnc before the WebSocket upgrade, so failures stay ordinary HTTP errors.
//!
//! Control admission takes the owner's screen hold before waiting out an agent move already
//! in flight. The registration owns that hold until forwarding ends, including failed upgrades.
//! A takeover also holds views off across control reconnects. A control display's first input
//! closes views and keeps new ones out until it ends; the bytes then go directly to Xvnc.
//!
//! A bound display runs under its binding's task tracker. Release cancels it and waits for its
//! forwarding to stop before restoring the fit and lifting the agent's lock. RFB bytes are
//! framed and judged by mode, never interpreted as field values.

use crate::browser_tabs::ScreenHoldGuard;
use crate::follow::Follower;
use crate::limits::{SERVER_CHUNK, XVNC_DIAL};
use crate::rfb_client::{ClientFramer, DisplayMode, Verdict, verdict};
use crate::{BrowserTabs, Front, Locks, PartError, ScreenHolder, follow, screen_rule};
use browseros_core::TabId;
use bytes::{Bytes, BytesMut};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use neo_trace::{TaskId, TraceSink, TraceStore};
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
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
    /// A view closed because the owner has control. Its viewer may try again later.
    Paused,
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
    /// Each bound display while it forwards RFB bytes.
    live: TaskTracker,
}

impl DisplayBinding {
    /// A binding no display is bound to yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ends every display bound to this one, and returns once each has stopped forwarding.
    /// A display bound after this ends at once, so a release leaves no moment in which
    /// a display that just bound could take input.
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
}

/// Keeps Neo's screen the owner's while it lives (decision 44): open view displays close, new
/// ones are refused, and the agent cannot move the front ([`Front`]). A takeover holds one from
/// its start to its release (synthesis item 2). A flow that keeps spectators off this way also
/// keeps the agent's moves off, with no second hold to forget.
#[must_use = "the screen is the agent's and the spectators' again as soon as the hold is dropped"]
pub struct ScreenHold {
    displays: Arc<Displays>,
    _held: ScreenHoldGuard,
}

impl Drop for ScreenHold {
    fn drop(&mut self) {
        let mut table = self.displays.table();
        table.owner_holds = table.owner_holds.saturating_sub(1);
    }
}

/// Every display in the process, and who may open one. The service builds one and its route
/// admits and serves displays; a takeover holds views off and binds its control display.
pub struct Displays {
    xvnc: SocketAddr,
    pub(crate) browser: BrowserTabs,
    /// Follow pauses while a lock holds the tab it follows.
    pub(crate) locks: Arc<Locks>,
    /// A followed task's operations, as the trace writer commits them.
    pub(crate) sink: TraceSink,
    /// A followed task's current tab before its next operation, and after a lag.
    pub(crate) store: TraceStore,
    table: Mutex<Table>,
}

/// The open displays by id. Every critical section is short and never awaits.
#[derive(Default)]
struct Table {
    next: u64,
    /// Each open view's close, cancelled when the owner takes control.
    views: HashMap<u64, CancellationToken>,
    controls: HashMap<u64, ControlEntry>,
    /// Live [`ScreenHold`]s: the owner's, through a takeover.
    owner_holds: usize,
}

struct ControlEntry {
    /// The owner's input began on this display. Views stay closed until it ends.
    input: bool,
    /// The display's hold on the screen, from its registration to its end.
    _held: ScreenHoldGuard,
}

impl Table {
    fn next_id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// Whether a takeover or a control display's input keeps views out.
    fn views_paused(&self) -> bool {
        self.owner_holds > 0 || self.controls.values().any(|control| control.input)
    }

    fn close_views(&self) {
        for close in self.views.values() {
            close.cancel();
        }
    }
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
            table: Mutex::default(),
        })
    }

    /// Registers a display and connects to Xvnc before the WebSocket upgrade. Views are refused
    /// while a takeover or owner input keeps them out; control waits out a front move in flight.
    ///
    /// # Errors
    /// `DisplayPaused` for a view while the owner has control; `Screen` if Xvnc cannot be reached.
    pub async fn admit(self: &Arc<Self>, request: DisplayRequest) -> Result<Admitted, PartError> {
        let (registration, role) = match request {
            DisplayRequest::View { follow } => {
                if self.views_paused() {
                    return Err(PartError::DisplayPaused);
                }
                let follower = match follow {
                    Some(follow) => Some(follow::admit(follow, self).await),
                    None => None,
                };
                let close = CancellationToken::new();
                let id = {
                    let mut table = self.table();
                    if table.views_paused() {
                        return Err(PartError::DisplayPaused);
                    }
                    let id = table.next_id();
                    table.views.insert(id, close.clone());
                    id
                };
                (self.registration(id), Role::View { close, follower })
            }
            DisplayRequest::Control { binding } => {
                let held = self.browser.hold_screen(ScreenHolder::ControlDisplay);
                let id = {
                    let mut table = self.table();
                    let id = table.next_id();
                    let entry = ControlEntry {
                        input: false,
                        _held: held,
                    };
                    table.controls.insert(id, entry);
                    id
                };
                let registration = self.registration(id);
                // From the hold on, the screen rule refuses every new agent move. Waiting out the
                // turn lets a move already past its last check land first, so the owner's display
                // never opens under a move still in flight.
                drop(self.browser.front_turn().await);
                (registration, Role::Control { binding })
            }
        };
        let server = self.dial().await?;
        Ok(Admitted {
            registration,
            role,
            server,
        })
    }

    /// The agent's side of this screen, for the service to hand to the app.
    #[must_use]
    pub fn front(self: &Arc<Self>) -> Front {
        Front::new(Arc::clone(self))
    }

    /// Holds the screen for its owner until the hold is dropped: closes every view display,
    /// refuses new ones, and refuses the agent's moves of the front.
    pub fn hold_screen(self: &Arc<Self>) -> ScreenHold {
        let held = self.browser.hold_screen(ScreenHolder::Takeover);
        let mut table = self.table();
        table.owner_holds += 1;
        table.close_views();
        ScreenHold {
            displays: Arc::clone(self),
            _held: held,
        }
    }

    /// Whether the owner holds the screen: the screen rule over the holds registry. Follow
    /// pauses while it does.
    pub(crate) fn screen_held(&self) -> bool {
        screen_rule::decide(self.browser.screen_facts()).is_err()
    }

    /// Whether a takeover or a control display's input keeps views out.
    pub(crate) fn views_paused(&self) -> bool {
        self.table().views_paused()
    }

    fn registration(self: &Arc<Self>, id: u64) -> Registration {
        Registration {
            displays: Arc::clone(self),
            id,
        }
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

    /// Marks the owner's input as begun on control display `id` and closes every view.
    fn input_began(&self, id: u64) {
        let mut table = self.table();
        if let Some(control) = table.controls.get_mut(&id) {
            control.input = true;
        }
        table.close_views();
    }

    /// Never held across an await.
    fn table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A display's place in the table, held for its whole life. Dropping it takes the display out,
/// whether or not it was ever served, so a failed upgrade leaves nothing behind.
struct Registration {
    displays: Arc<Displays>,
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let control = {
            let mut table = self.displays.table();
            table.views.remove(&self.id);
            table.controls.remove(&self.id)
        };
        // Drop the screen hold outside the table lock so other front movers can read it.
        drop(control);
    }
}

/// One admitted display, with its place in the table and its own connection to Xvnc, ready to
/// serve. Dropping it unserved ends it.
pub struct Admitted {
    registration: Registration,
    role: Role,
    server: TcpStream,
}

enum Role {
    View {
        close: CancellationToken,
        /// The view's follower as its admission left it: subscribed to what it follows from
        /// before its look-up, so it misses nothing that happens before the view is served.
        follower: Option<Follower>,
    },
    Control {
        binding: Option<DisplayBinding>,
    },
}

impl Admitted {
    /// Runs the display until its viewer leaves, Xvnc closes, the filter refuses the viewer, or
    /// it must close (a view when the owner takes control, a bound display at its release).
    /// `from_viewer` and `to_viewer` carry raw RFB bytes; the service takes them out of
    /// WebSocket frames and puts them back in.
    ///
    /// A bound display runs inside its binding's tracker, so [`DisplayBinding::close`] waits for
    /// its forwarding to stop before the takeover restores the fit.
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
            registration,
            role,
            server,
        } = self;
        let displays = &registration.displays;
        let id = registration.id;
        let end = match role {
            Role::View { close, follower } => {
                let closing = Closing::Paused(close);
                let proxy = Proxy::new(displays, id, DisplayMode::View).run(
                    server,
                    from_viewer,
                    to_viewer,
                    &closing,
                );
                match follower {
                    None => proxy.await,
                    Some(follower) => {
                        let mut proxy = pin!(proxy);
                        // Biased toward the proxy: a view whose close already fired ends before
                        // its follower gets a pass, which could send one activation for it.
                        tokio::select! {
                            biased;
                            end = &mut proxy => end,
                            () = follow::run(follower, displays) => proxy.await,
                        }
                    }
                }
            }
            Role::Control { binding } => {
                let closing = binding.map_or(Closing::Never, |binding| {
                    Closing::Released(binding.released)
                });
                Proxy::new(displays, id, DisplayMode::Control)
                    .run(server, from_viewer, to_viewer, &closing)
                    .await
            }
        };
        tracing::info!(display = id, ?end, "a display ended");
        // The registration goes last: until the end above is done, views stay out and follow
        // stays paused. The screen hold outlives every byte this display forwards.
        end
    }
}

/// What ends a display before its viewer or Xvnc does.
enum Closing {
    /// A view: the owner took control, or a takeover holds views off.
    Paused(CancellationToken),
    /// A bound control display: its binding closed.
    Released(CancellationToken),
    /// An unbound control display: only its viewer or Xvnc ends it.
    Never,
}

impl Closing {
    /// How the display ends if its close already fired; `None` while it is open. Sync, so the
    /// proxy can ask between a read and its send.
    fn ended(&self) -> Option<DisplayEnd> {
        match self {
            Self::Paused(close) => close.is_cancelled().then_some(DisplayEnd::Paused),
            Self::Released(released) => released.is_cancelled().then_some(DisplayEnd::Released),
            Self::Never => None,
        }
    }

    async fn closed(&self) -> DisplayEnd {
        match self {
            Self::Paused(close) => {
                close.cancelled().await;
                DisplayEnd::Paused
            }
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
struct Proxy<'a> {
    displays: &'a Displays,
    id: u64,
    /// The protocol mode is independent of tab state and persists for the whole connection.
    mode: DisplayMode,
    framer: ClientFramer,
    /// The first input has closed views; later inputs need no further transition.
    input_began: bool,
}

impl<'a> Proxy<'a> {
    fn new(displays: &'a Displays, id: u64, mode: DisplayMode) -> Self {
        Self {
            displays,
            id,
            mode,
            framer: ClientFramer::default(),
            input_began: false,
        }
    }

    /// Runs until one side ends, the viewer is refused, or `closing` fires.
    ///
    /// The two directions never wait on each other, and `closing` races both: a viewer that
    /// stopped reading, a stuck Xvnc never keeps a display that must close
    /// forwarding, and a release never waits on them.
    async fn run<I, O>(
        mut self,
        server: TcpStream,
        from_viewer: I,
        to_viewer: O,
        closing: &Closing,
    ) -> DisplayEnd
    where
        I: Stream<Item = Bytes>,
        O: Sink<Bytes>,
    {
        let (from_server, to_server) = server.into_split();
        tokio::select! {
            biased;
            end = closing.closed() => end,
            end = screen_to_viewer(from_server, to_viewer, closing) => end,
            end = self.viewer_to_screen(from_viewer, to_server) => end,
        }
    }

    /// The viewer's bytes as they come, through the framer and the verdict table, to Xvnc.
    async fn viewer_to_screen<I>(
        &mut self,
        from_viewer: I,
        mut to_server: OwnedWriteHalf,
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
            if let Err(end) = self.forward(&mut to_server).await {
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
            let input = match verdict(self.mode, message) {
                Verdict::Drop => continue,
                Verdict::Pass => false,
                Verdict::Input => true,
            };
            if input && !self.input_began {
                self.displays.input_began(self.id);
                self.input_began = true;
            }
            to_server
                .write_all(&bytes)
                .await
                .map_err(|_| DisplayEnd::ScreenClosed)?;
        }
        Ok(())
    }
}

/// Xvnc's bytes as they come, untouched, to the viewer, until `closing` fires.
///
/// The close is checked after every read, before its bytes go out (decision 46). The biased
/// select in [`Proxy::run`] checks it only between polls, and within one poll this loop reads and
/// sends for as long as Xvnc's socket has data, so a chunk read after the close could still go
/// out. With the check, nothing Xvnc sends after a close reaches the viewer: a close cancelled
/// before an activation is sent keeps every pixel of the new front off the screen, for every
/// closer (the owner's first input or the takeover's release). Bytes read
/// before the close show the old front, and may still go out.
async fn screen_to_viewer<O>(
    mut from_server: OwnedReadHalf,
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

#[cfg(test)]
mod tests {
    use super::{Closing, DisplayEnd, screen_to_viewer};
    use bytes::Bytes;
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// A view's close cancelled between two reads: the first read went out before the close, and
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
            let closing = Closing::Paused(close.clone());
            async move { screen_to_viewer(from_server, to_viewer, &closing).await }
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
            DisplayEnd::Paused,
            "the read after the close ends it"
        );
        Ok(())
    }
}
