//! Neo's one screen, shown over RFB: to spectators as view displays, and to the owner as control
//! displays (design D12). Each display has its own shared RFB connection to Xvnc. Xvnc has no
//! auth, so a display is the only way a host reaches it, and this module is the gate.
//!
//! ```text
//!  admit ─ a view while the owner has control: DisplayPaused (409)
//!        ─ register ─ a view while a private tab is in front: DisplayPaused (409)
//!        ─ dial Xvnc (503) ─ control: Privacy::control_display_opened,
//!                                     the journal armed on every open tab
//!  serve ─ viewer bytes ─▶ ClientFramer ─▶ verdict(mode) ─▶ Xvnc
//!        ◀─ Xvnc bytes, untouched ─────────────────────────────┘
//!          view: follow runs beside it (follow.rs), and the front-tab watch closes it when a
//!                private tab is in front (front_tab.rs)
//!          control: at its end, the close sweep of every open tab, then the journal disarms
//! ```
//!
//! **Admission comes before the WebSocket upgrade**, so the host gets a refusal as an HTTP
//! status and not as a socket that opens and dies: [`Displays::admit`] checks the pause,
//! registers the display and dials Xvnc; [`Admitted::serve`] runs it.
//!
//! **The owner's first input waits.** On a control display, the first key, paste or click is
//! held while, in this order:
//! 1. every view display closes, and new views are refused while this display is open;
//! 2. every open tab is stamped ([`TaintStamps`]), so a reconcile whose listing missed it keeps
//!    its flag, and then made private, durably (`ControlDisplay::first_input`);
//! 3. only then does the input go to Xvnc.
//!
//! Pixels that show what the owner typed exist only once Xvnc has their input. So a spectator
//! closed in step 1 never gets them, and no capture of a tab made private in step 2 is kept. A
//! failed commit drops that input and the next input tries again: the owner sees a lost key,
//! never a tab that is not private. Later inputs go at once. While an input waits, the display
//! still sees its viewer leave and Xvnc close, and ends with the input dropped, so a commit that
//! never returns cannot keep it, and the views it shut out, waiting.
//!
//! **A control display needs its journal.** One whose open tabs cannot be listed is refused
//! before the upgrade, as the journal could not be armed on them.
//!
//! **Views close; they do not pause** (decision 22). Holding Xvnc's bytes back would break the
//! Tight encoding's zlib state, which spans rectangles. And noVNC asks for continuous updates,
//! so a held update request would be answered anyway. A closed view fails closed, and its viewer
//! comes back once the owner is done. Views are refused while a control display that took input
//! is open, and while a takeover holds the screen ([`Displays::hold_screen`]); the hold also
//! covers the gap while zuzu's control relay reconnects (synthesis item 2). A closed view
//! forwards nothing it reads from Xvnc after its close (decision 46).
//!
//! **The owner holds the screen** (decision 44) while a takeover's [`ScreenHold`] lives and while
//! any control display is open, input or not: each keeps a guard in the holds registry beside
//! the front turn (`browser_tabs`), and the agent's moves ([`Front`]) and every follower read
//! the screen rule over it. A control display's admission waits out a move already in flight,
//! so the owner's display never opens under it.
//!
//! **The agent's private tab comes forward only after every view's close** (decision 46). While
//! [`Front`] brings a private tab to the front, a switch hold closes every view and refuses new
//! ones, from before the activation is sent until the front reads that tab. It keeps views out
//! like a `ScreenHold`, but it is not the owner's, so the screen rule ignores it.
//!
//! **No view while a private tab is in front** (decision 34), however that tab went private.
//! Admission refuses a view, and every open view closes when such a tab comes to the front or
//! the tab in front goes private (front_tab.rs). A private tab stays private until it closes, so
//! after control ends, views come back only once a tab that is not private is in front.
//!
//! **Every control display arms the control journal on every open tab**, and on each tab that
//! opens while it is open ([`Displays::tab_opened`]); while it is open, `Privacy` starts each new
//! tab private (decision 31). **One that took input sweeps every open tab when it ends**, however
//! it ends: the viewer left, Xvnc dropped it, or its binding closed it. Over the one screen the
//! owner can reach any tab, and a value can reach a field without an `input` event the journal
//! sees. A takeover's release waits for this sweep ([`DisplayBinding::close`]) before it sweeps
//! its own tab.

use crate::browser_tabs::ScreenHoldGuard;
use crate::follow::Follower;
use crate::limits::{CLOSE_SWEEP, MAX_HELD_INPUT, SERVER_CHUNK, XVNC_DIAL};
use crate::rfb_client::{ClientFramer, DisplayMode, Verdict, verdict};
use crate::{
    ArmedJournal, BrowserTabs, Front, Journal, Locks, PartError, ScreenHolder, follow, front_tab,
    screen_rule, sweep,
};
use browseros_core::TabId;
use browseros_trace::{ControlDisplay, EpochMillis, Privacy, TaskId, TraceSink, TraceStore};
use bytes::{Bytes, BytesMut};
use futures_util::future::{BoxFuture, join_all};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
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
    /// A view closed because the owner has control, or because a private tab is in front. Its
    /// viewer may try again later.
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
    /// Each bound display while it serves, its close sweep included.
    live: TaskTracker,
}

impl DisplayBinding {
    /// A binding no display is bound to yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ends every display bound to this one, and returns once each has stopped forwarding and
    /// finished its end: the close sweep of every open tab (at most 5 s), then its journal's
    /// disarm. A display bound after this ends at once, so a release leaves no moment in which
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

/// Keeps view displays off while the agent brings a private tab forward (decision 46). On views
/// it is a [`ScreenHold`]; it is not the owner's, so the screen rule ignores it (counted as the
/// owner's, the move would refuse itself). Dropping it lets views back: a view's admission then
/// reads the front itself, and is refused while the private tab is there (decision 34).
#[must_use = "views are let back in as soon as the switch is dropped"]
pub(crate) struct SwitchHold {
    displays: Arc<Displays>,
}

impl Drop for SwitchHold {
    fn drop(&mut self) {
        let mut table = self.displays.table();
        table.switches = table.switches.saturating_sub(1);
    }
}

/// Stamps the tabs a control display's first input is about to make private, before any of their
/// flags is committed. After a browser reconnect, a reconcile ends what Neo keeps about every tab
/// its listing lacks, unless the tab was stamped after the reconcile's fence. A tab the input
/// makes private may have opened after that listing. The service implements this with the stamps
/// its routes take (`browseros-service`'s `tab_stamps`). This crate cannot depend on the service,
/// so `Displays` takes the stamps as a hook.
pub trait TaintStamps: Send + Sync {
    /// Returns once every tab in `tabs` is stamped. It waits while a reconcile is ending tab
    /// state, and the first input waits with it.
    fn stamp_all<'a>(&'a self, tabs: &'a [TabId]) -> BoxFuture<'a, ()>;
}

/// Every display in the process, and who may open one. The service builds one and its route
/// admits and serves displays; a takeover holds views off and binds its control display.
pub struct Displays {
    xvnc: SocketAddr,
    pub(crate) browser: BrowserTabs,
    /// Control displays commit through it; views and follow read which tabs are private.
    pub(crate) privacy: Arc<Privacy>,
    /// A control display's first input stamps here every tab it makes private, before it commits.
    stamps: Arc<dyn TaintStamps>,
    journal: Journal,
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
    /// Each open view's close, cancelled when the owner takes control, a private tab is in
    /// front, or the agent brings one forward.
    views: HashMap<u64, CancellationToken>,
    controls: HashMap<u64, ControlEntry>,
    /// Live [`ScreenHold`]s: the owner's, through a takeover.
    owner_holds: usize,
    /// Live [`SwitchHold`]s: the agent's private switch, in progress.
    switches: usize,
    /// Stops the front-tab watch, which runs exactly while `views` is not empty.
    front_watch: Option<CancellationToken>,
}

struct ControlEntry {
    /// The owner's input began on this display. Views stay closed until it ends.
    input: bool,
    /// Its journal once armed, so [`Displays::tab_opened`] can arm a new tab with it.
    journal: Option<Arc<ArmedJournal>>,
    /// The display's hold on the screen, from its registration to its end.
    _held: ScreenHoldGuard,
}

impl Table {
    fn next_id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// Whether views are kept out: a takeover holds the screen, the agent's private switch is
    /// running, or the owner typed on a control display.
    fn views_paused(&self) -> bool {
        self.owner_holds > 0
            || self.switches > 0
            || self.controls.values().any(|control| control.input)
    }

    fn close_views(&self) {
        for close in self.views.values() {
            close.cancel();
        }
    }

    /// The new front-tab watch's stop, when none runs yet; the caller starts the watch.
    fn start_front_watch(&mut self) -> Option<CancellationToken> {
        if self.front_watch.is_some() {
            return None;
        }
        let stop = CancellationToken::new();
        self.front_watch = Some(stop.clone());
        Some(stop)
    }
}

impl Displays {
    /// No displays yet. `xvnc` is Xvnc's RFB address. Control displays stamp the tabs they make
    /// private through `stamps`, then commit privacy through `privacy`, and arm `journal`; follow
    /// reads `locks`, `sink`'s events and `store`.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "each part has a type of its own, so a swapped argument does not compile"
    )]
    pub fn new(
        xvnc: SocketAddr,
        browser: BrowserTabs,
        privacy: Arc<Privacy>,
        stamps: Arc<dyn TaintStamps>,
        journal: Journal,
        locks: Arc<Locks>,
        sink: TraceSink,
        store: TraceStore,
    ) -> Arc<Self> {
        Arc::new(Self {
            xvnc,
            browser,
            privacy,
            stamps,
            journal,
            locks,
            sink,
            store,
            table: Mutex::default(),
        })
    }

    /// Admits one display before its WebSocket upgrade. A view is refused while the owner has
    /// control, and while a private tab is in front; a view that follows a tab first starts its
    /// follower, which brings that tab forward when it would (`follow::admit`). Then the display is
    /// registered and gets its own connection to Xvnc. A control display also opens `Privacy`'s
    /// control display, so tabs that open from now on start private, and arms the journal on
    /// every open tab before it returns, so the owner's first key cannot reach a tab the journal
    /// has not been armed on.
    ///
    /// A view is registered before the tab in front is read and Xvnc is dialed, so a tab that
    /// goes private or comes to the front after the read, and the owner's input during the
    /// dial, still close it.
    ///
    /// # Errors
    /// `DisplayPaused` for a view while the owner has control or a private tab is in front;
    /// `Browser` when the browser cannot list its tabs, as a view cannot tell what is in front
    /// and a control display cannot arm the journal on them; `Screen` when Xvnc cannot be
    /// reached.
    pub async fn admit(self: &Arc<Self>, request: DisplayRequest) -> Result<Admitted, PartError> {
        let (registration, role) = match request {
            DisplayRequest::View { follow } => {
                if self.views_paused() {
                    return Err(PartError::DisplayPaused);
                }
                // Before the view is registered: once it is, the front-tab watch may read the
                // private tab this replaces and close the view, if the task's lookup or the
                // activation outlasts one of its polls.
                let follower = match follow {
                    Some(follow) => Some(follow::admit(follow, self).await),
                    None => None,
                };
                let close = CancellationToken::new();
                let (id, front_watch) = {
                    let mut table = self.table();
                    if table.views_paused() {
                        return Err(PartError::DisplayPaused);
                    }
                    let id = table.next_id();
                    table.views.insert(id, close.clone());
                    (id, table.start_front_watch())
                };
                if let Some(stop) = front_watch {
                    // Subscribed before this view reads the front, so a flag committed after
                    // that read still wakes the watch, however late its task first runs.
                    let events = self.sink.subscribe();
                    tokio::spawn(front_tab::watch(Arc::clone(self), stop, events));
                }
                // Dropping it on a refusal takes the view out of the table, and the watch stops
                // with the last view.
                let registration = self.registration(id);
                if front_tab::private_in_front(&self.browser, &self.privacy)
                    .await?
                    .is_some()
                {
                    return Err(PartError::DisplayPaused);
                }
                (registration, Role::View { close, follower })
            }
            DisplayRequest::Control { binding } => {
                let held = self.browser.hold_screen(ScreenHolder::ControlDisplay);
                let id = {
                    let mut table = self.table();
                    let id = table.next_id();
                    let entry = ControlEntry {
                        input: false,
                        journal: None,
                        _held: held,
                    };
                    table.controls.insert(id, entry);
                    id
                };
                let registration = self.registration(id);
                let privacy = self.privacy.control_display_opened();
                // From the hold on, the screen rule refuses every new agent move. Waiting out the
                // turn lets a move already past its last check land first, so the owner's display
                // never opens under a move still in flight.
                drop(self.browser.front_turn().await);
                (registration, Role::Control { privacy, binding })
            }
        };
        let server = self.dial().await?;
        if matches!(role, Role::Control { .. }) {
            // A refusal drops the registration, and the journal in it disarms.
            self.arm_journal(registration.id).await?;
        }
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

    /// Closes every view and refuses new ones until dropped, for the agent's move of a private
    /// tab. Sync: every view's close is cancelled under the table's mutex before this returns,
    /// so before the activation is sent.
    pub(crate) fn hold_for_switch(self: &Arc<Self>) -> SwitchHold {
        let mut table = self.table();
        table.switches += 1;
        table.close_views();
        SwitchHold {
            displays: Arc::clone(self),
        }
    }

    /// A tab opened: every open control display arms its journal on it, since the owner can
    /// bring it to the front and type there (decision 31). Returns once each has; a tab that
    /// cannot be armed is logged, and a close sweep still reads it.
    pub async fn tab_opened(&self, tab: TabId) {
        let journals = self
            .table()
            .controls
            .values()
            .filter_map(|control| control.journal.clone())
            .collect::<Vec<_>>();
        for journal in journals {
            arm_tabs(&journal, &[tab]).await;
        }
    }

    /// Whether the owner holds the screen: the screen rule over the holds registry. Follow
    /// pauses while it does.
    pub(crate) fn screen_held(&self) -> bool {
        screen_rule::decide(self.browser.screen_facts()).is_err()
    }

    /// Whether views are kept out: a takeover holds the screen, the agent's private switch is
    /// running, or the owner typed on a control display.
    pub(crate) fn views_paused(&self) -> bool {
        self.table().views_paused()
    }

    /// Closes every open view. The front-tab watch calls it when a private tab is in front, or
    /// when it cannot tell.
    pub(crate) fn close_views(&self) {
        self.table().close_views();
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

    /// Arms a journal for control display `id` on every open tab. The journal goes into the table
    /// before the tabs are listed: a tab that opens before the listing is in it, and one that
    /// opens after finds the journal through [`Displays::tab_opened`]. Arming a tab twice is
    /// harmless.
    ///
    /// # Errors
    /// `Browser` when the open tabs cannot be listed, and the display is refused. With no journal
    /// on them, a password the owner typed and submitted would be gone from its field before the
    /// close sweep could read it. A single tab that fails to arm is logged instead, and the sweep
    /// still reads it.
    async fn arm_journal(&self, id: u64) -> Result<(), PartError> {
        let journal = Arc::new(self.journal.arm(&[]).await);
        if let Some(control) = self.table().controls.get_mut(&id) {
            control.journal = Some(Arc::clone(&journal));
        }
        let tabs = self.browser.open_tabs().await?;
        arm_tabs(&journal, &tabs).await;
        Ok(())
    }

    /// Marks the owner's input as begun on control display `id` and closes every view.
    fn input_began(&self, id: u64) {
        let mut table = self.table();
        if let Some(control) = table.controls.get_mut(&id) {
            control.input = true;
        }
        table.close_views();
    }

    /// The end of a control display that took input. Every open tab is swept at once, within
    /// 5 s. A tab that cannot be swept is logged; what the others hold is registered either way.
    /// The summary counts what the secret set refused, which is masked nowhere.
    async fn sweep_open_tabs(&self) {
        let secrets = self.privacy.secrets();
        let swept = tokio::time::timeout(CLOSE_SWEEP, async {
            let tabs = self.browser.open_tabs().await?;
            Ok::<_, PartError>(
                join_all(
                    tabs.iter()
                        .map(|&tab| async move { (tab, sweep(&self.browser, secrets, tab).await) }),
                )
                .await,
            )
        })
        .await;
        match swept {
            Err(_) => {
                tracing::warn!("a control display's close sweep ran out of its {CLOSE_SWEEP:?}")
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "a control display's close sweep could not list the tabs");
            }
            Ok(Ok(swept)) => {
                let (mut registered, mut refused) = (0, 0);
                for (tab, result) in &swept {
                    match result {
                        Ok(tab_swept) => {
                            registered += tab_swept.registered;
                            refused += tab_swept.refused;
                        }
                        Err(error) => tracing::warn!(
                            tab = tab.0,
                            %error,
                            "a control display's close sweep skipped a tab"
                        ),
                    }
                }
                tracing::info!(
                    tabs = swept.len(),
                    registered,
                    refused,
                    "a control display that took input swept every open tab at its end"
                );
            }
        }
    }

    /// Disarms control display `id`'s journal: what it holds is registered, then its scripts go.
    async fn disarm_journal(&self, id: u64) {
        let journal = self
            .table()
            .controls
            .get_mut(&id)
            .and_then(|control| control.journal.take());
        let Some(journal) = journal else {
            return;
        };
        match Arc::try_unwrap(journal) {
            Ok(journal) => journal.disarm().await,
            // A `tab_opened` is arming a tab with it right now. Dropping the last handle disarms
            // it in the background, in the same order.
            Err(arming) => drop(arming),
        }
    }

    /// Never held across an await.
    fn table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Arms `journal` on each of `tabs` at once.
async fn arm_tabs(journal: &ArmedJournal, tabs: &[TabId]) {
    join_all(tabs.iter().map(|&tab| async move {
        if let Err(error) = journal.arm_tab(tab).await {
            tracing::warn!(tab = tab.0, %error, "the control journal could not arm a tab");
        }
    }))
    .await;
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
            // With the last view gone, the front-tab watch has nothing left to close.
            if table.views.is_empty()
                && let Some(stop) = table.front_watch.take()
            {
                stop.cancel();
            }
            table.controls.remove(&self.id)
        };
        // Outside the lock: dropping a journal that was never disarmed starts its disarm.
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
        /// Open until the display's end: while it is, every tab that opens starts private.
        privacy: ControlDisplay,
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
    /// all of it, the close sweep included.
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
                let proxy =
                    Proxy::new(displays, id, None).run(server, from_viewer, to_viewer, &closing);
                match follower {
                    None => proxy.await.0,
                    Some(follower) => {
                        let mut proxy = pin!(proxy);
                        // Biased toward the proxy: a view whose close already fired ends before
                        // its follower gets a pass, which could send one activation for it.
                        tokio::select! {
                            biased;
                            (end, _) = &mut proxy => end,
                            () = follow::run(follower, displays) => proxy.await.0,
                        }
                    }
                }
            }
            Role::Control { privacy, binding } => {
                let closing = binding.map_or(Closing::Never, |binding| {
                    Closing::Released(binding.released)
                });
                let (end, forwarded) = Proxy::new(displays, id, Some(&privacy))
                    .run(server, from_viewer, to_viewer, &closing)
                    .await;
                if forwarded {
                    displays.sweep_open_tabs().await;
                }
                displays.disarm_journal(id).await;
                end
            }
        };
        tracing::info!(display = id, ?end, "a display ended");
        // The registration goes last: until the end above is done, views stay out and follow
        // stays paused, and `Privacy` (dropped with the role) kept every new tab private.
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
    /// A control display's privacy; `None` for a view.
    control: Option<&'a ControlDisplay>,
    framer: ClientFramer,
    /// Every open tab was made private, durably, for this display's input.
    tainted: bool,
    /// Input reached Xvnc, so the display's end sweeps.
    forwarded: bool,
}

impl<'a> Proxy<'a> {
    fn new(displays: &'a Displays, id: u64, control: Option<&'a ControlDisplay>) -> Self {
        Self {
            displays,
            id,
            control,
            framer: ClientFramer::default(),
            tainted: false,
            forwarded: false,
        }
    }

    /// Runs until one side ends, the viewer is refused, or `closing` fires. Returns why, and
    /// whether any input reached Xvnc.
    ///
    /// The two directions never wait on each other, and `closing` races both: a viewer that
    /// stopped reading, a stuck Xvnc or a held commit never keeps a display that must close
    /// forwarding, and a release never waits on them. Xvnc's bytes keep reaching the viewer while
    /// an input waits for its commit.
    async fn run<I, O>(
        mut self,
        server: TcpStream,
        from_viewer: I,
        to_viewer: O,
        closing: &Closing,
    ) -> (DisplayEnd, bool)
    where
        I: Stream<Item = Bytes>,
        O: Sink<Bytes>,
    {
        let (from_server, to_server) = server.into_split();
        let end = tokio::select! {
            biased;
            end = closing.closed() => end,
            end = screen_to_viewer(from_server, to_viewer, closing) => end,
            end = self.viewer_to_screen(from_viewer, to_server) => end,
        };
        (end, self.forwarded)
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
            if let Err(end) = self.forward(&mut from_viewer, &mut to_server).await {
                return end;
            }
        }
    }

    /// Every whole message the framer holds: each is judged, and the ones that pass go to Xvnc
    /// in order.
    async fn forward<I>(
        &mut self,
        from_viewer: &mut Pin<&mut I>,
        to_server: &mut OwnedWriteHalf,
    ) -> Result<(), DisplayEnd>
    where
        I: Stream<Item = Bytes>,
    {
        while let Some((message, bytes)) =
            self.framer.next_message().map_err(DisplayEnd::Refused)?
        {
            let input = match verdict(self.mode(), message) {
                Verdict::Drop => continue,
                Verdict::Pass => false,
                Verdict::Input => true,
            };
            if input && !self.taint(from_viewer).await? {
                continue;
            }
            to_server
                .write_all(&bytes)
                .await
                .map_err(|_| DisplayEnd::ScreenClosed)?;
            self.forwarded |= input;
        }
        Ok(())
    }

    fn mode(&self) -> DisplayMode {
        match self.control {
            Some(_) => DisplayMode::Control,
            None => DisplayMode::View,
        }
    }

    /// Whether this input may go to Xvnc. The first time: every view closes, then every open tab
    /// is stamped and made private, durably, and the input waits for that. When a commit fails it
    /// may not go, and the next input tries again.
    ///
    /// The viewer is still read while the commit is pending, so its leaving ends the display at
    /// once and drops the commit. A store that never answers cannot keep the display open, nor
    /// the views it shut out refused. What the viewer sends meanwhile waits in the framer, behind
    /// this input, up to [`MAX_HELD_INPUT`].
    async fn taint<I>(&mut self, from_viewer: &mut Pin<&mut I>) -> Result<bool, DisplayEnd>
    where
        I: Stream<Item = Bytes>,
    {
        if self.tainted {
            return Ok(true);
        }
        // The verdict table marks input only on a control display; a view's input never goes.
        let Some(control) = self.control else {
            return Ok(false);
        };
        self.displays.input_began(self.id);
        let mut commit = pin!(make_open_tabs_private(self.displays, self.id, control));
        let committed = loop {
            tokio::select! {
                biased;
                committed = &mut commit => break committed,
                bytes = from_viewer.next() => {
                    let Some(bytes) = bytes else {
                        return Err(DisplayEnd::ViewerLeft);
                    };
                    self.framer.push(&bytes);
                    if self.framer.buffered() > MAX_HELD_INPUT {
                        return Err(DisplayEnd::Refused(
                            "the viewer sent over 2 MiB while its first input waited",
                        ));
                    }
                }
            }
        };
        let Some(tabs) = committed else {
            return Ok(false);
        };
        tracing::info!(
            display = self.id,
            tabs,
            "the owner's first input: every open tab is private; sending it on"
        );
        self.tainted = true;
        Ok(true)
    }
}

/// Stamps every open tab, then commits it private, for the first input on control display `id`,
/// and returns how many there were; `None` when the input must be dropped. It borrows nothing
/// from the proxy, so the viewer's bytes can still be framed while it waits.
async fn make_open_tabs_private(
    displays: &Displays,
    id: u64,
    control: &ControlDisplay,
) -> Option<usize> {
    let tabs = match displays.browser.open_tabs().await {
        Ok(tabs) => tabs,
        Err(error) => {
            tracing::warn!(
                display = id,
                %error,
                "the owner's input was dropped: the open tabs could not be listed"
            );
            return None;
        }
    };
    // Each tab is stamped before any flag is set, as a route stamps its tab before it changes it.
    // A reconcile that sees one of these flags then sees its stamp too, and keeps the flag though
    // its listing missed the tab. A reconcile that is already ending tab state holds the stamp
    // table until it is done, so the stamps, and this commit, wait for it. Committed first, a
    // flag could be ended as if its tab had closed, and views and screenshots would see a tab
    // the owner may have typed into.
    displays.stamps.stamp_all(&tabs).await;
    match control.first_input(&tabs, EpochMillis::now()).await {
        Ok(()) => Some(tabs.len()),
        Err(error) => {
            tracing::warn!(
                display = id,
                %error,
                "the owner's input was dropped: a tab's private flag was not committed"
            );
            None
        }
    }
}

/// Xvnc's bytes as they come, untouched, to the viewer, until `closing` fires.
///
/// The close is checked after every read, before its bytes go out (decision 46). The biased
/// select in [`Proxy::run`] checks it only between polls, and within one poll this loop reads and
/// sends for as long as Xvnc's socket has data, so a chunk read after the close could still go
/// out. With the check, nothing Xvnc sends after a close reaches the viewer: a close cancelled
/// before an activation is sent keeps every pixel of the new front off the screen, for every
/// closer (the agent's private switch, the owner's first input, the front-tab watch). Bytes read
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
