//! The v1 takeover flow (design D13): the host's owner opens the takeover link, and later is done.
//! It is a short composition over the parts. Each part stays useful alone and keeps every rule
//! about its own state, so a different flow replaces this module and leaves the parts as they
//! are.
//!
//! ```text
//!  start:    lock(tab, reads, ttl), its TTL release scheduled
//!            ─▶ hold the screen ─▶ fit(tab, viewer)? ─▶ active
//!            (still fitting when the TTL release is due ─▶ given back as a release
//!            is ─▶ released(expired), never active)
//!  release:  list the tabs opened while active ─▶ close its displays, each drained to Xvnc
//!            ─▶ restore(fit) ─▶ let the screen go ─▶ unlock ─▶ released(reason): by the host,
//!            10 s before the lock's end (expired), or when the tab closes (tab_closed, without
//!            the restore)
//! ```
//!
//! **The order keeps the owner's handoff intact.** The agent is locked out before the owner
//! can reach the tab, and agent moves of the front are held off before the owner's
//! display can open. Spectator streams remain available throughout. Release stops the owner's
//! input before restoring the fit and giving back the screen. The agent's lock lifts last,
//! so the agent never acts on a half-restored tab. The TTL release starts 10 s before the lock's deadline so the same
//! ordering holds when the owner leaves the takeover open.
//!
//! **A start cannot outrun its TTL release.** The release is scheduled as the lock is taken, and
//! the start waits for the fit only until the release is due. A start that is not done by then
//! is never published. It gives back what it took, in release order and under the lock, is
//! recorded and announced as released (`expired`), and is the start's error. Fitting runs in a
//! task of its own, so it is not cut short. The give-back waits for it within its budget like
//! any step, and one that takes longer is given back in the background once it is done.
//!
//! **No state of its own beyond what it took.** A takeover keeps the ids of its lock and fit,
//! its display binding, the screen hold, its state, and the tabs that opened while it was
//! active. The parts own the rest, which is what lets the owner change this flow without
//! touching them.
//!
//! **The hand-back lists, it never gives.** The owner may open tabs during a handoff: their bank,
//! a sign-in popup of the shared tab. Neo gives none of them to a task by itself. Each active
//! takeover notes every tab that opens ([`Takeovers::note_opened`]); the release lists those
//! still open as `openedTabs`, with their address, the one the owner left in front, the task
//! holding each and the tab that opened it, read once as the release begins, whatever ended it.
//! The host acts on the list by its own rule. Takeovers at once each list the tab.
//!
//! **Idempotent.** Starts run one at a time, and a start on a tab with an active takeover returns
//! that one, so a host that retries does not lock twice. Release runs its steps once; a second
//! caller waits for the first and gets the same answer.
//!
//! **A caller that hangs up changes nothing.** Start and release each run in a task of their own,
//! and the caller only waits for it. The service's routes are cancelled when a host disconnects,
//! and a start cut short after its lock would leave a takeover's lock that nothing can lift (the
//! lock route refuses it, decision 29); a release cut short would never unlock.
//!
//! **Bounded.** Release's steps share a 10 s budget. A step that fails, or is still running when
//! the budget is spent, is logged and the next one runs; the unlock always does. Each step runs
//! in a task of its own, so one the budget leaves behind finishes in the background instead of
//! stopping halfway: a fit's restore cut short would leave its fit active for good.

use crate::browser_tabs::ScreenHoldGuard;
use crate::limits::{
    GIVE_BACK, KEEP_RELEASED_TAKEOVER, MIN_TAKEOVER_TTL, OPENED_LISTING, OPENED_TABS,
    TAKEOVER_CHANGES,
};
use crate::{
    BrowserTabs, DisplayBinding, FitId, Fits, LockBinding, LockId, LockTtl, Locks, PartError,
    Reads, ScreenHolder, TakeoverId, Viewer,
};
use browseros_core::TabId;
use futures_util::future::{self, BoxFuture, FutureExt};
use neo_trace::EpochMillis;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinError;
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, DropGuard};

/// How long a takeover may last: from 60 s to the lock's 20-minute cap (decision 6). Its lock
/// gets the same TTL, and the takeover is released 10 s before the lock's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeoverTtl(LockTtl);

impl TakeoverTtl {
    /// The shortest TTL.
    pub const MIN: Duration = MIN_TAKEOVER_TTL;
    /// The longest TTL, the lock's cap, and the contract's default.
    pub const MAX: Duration = LockTtl::MAX;

    /// `None` outside [`Self::MIN`] to [`Self::MAX`].
    #[must_use]
    pub fn new(ttl: Duration) -> Option<Self> {
        if ttl < Self::MIN {
            return None;
        }
        LockTtl::new(ttl).map(Self)
    }

    /// The duration.
    #[must_use]
    pub fn get(self) -> Duration {
        self.0.get()
    }
}

/// What starting a takeover needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeoverRequest {
    /// The tab the owner takes.
    pub tab: TabId,
    /// The owner's logical viewer and optional preview scale. Without it the tab is not fitted.
    pub viewer: Option<Viewer>,
    /// Whether the lock refuses the agent's reads of the tab too.
    pub reads: Reads,
    /// How long the takeover may last.
    pub ttl: TakeoverTtl,
}

/// Active, then released, once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeoverState {
    /// The owner has the tab. A release that is still running reads as active until it is done.
    Active,
    /// Given back.
    Released {
        /// Why.
        reason: ReleaseReason,
        /// When the agent's lock lifted.
        at: EpochMillis,
    },
}

/// Why a takeover was released.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseReason {
    /// The owner said they were done.
    Done,
    /// The host saw the owner go idle.
    Idle,
    /// The host ended it for a reason of its own.
    Host,
    /// Its TTL ran out.
    Expired,
    /// Its tab closed.
    TabClosed,
}

/// A takeover as a host reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Takeover {
    /// Its id.
    pub id: TakeoverId,
    /// The tab the owner has.
    pub tab: TabId,
    /// Active, or released and why.
    pub state: TakeoverState,
    /// The lock that keeps the agent off the tab.
    pub lock: LockId,
    /// The tab's fit, when the takeover started with a viewer.
    pub fit: Option<FitId>,
    /// When the agent was locked out.
    pub created_at: EpochMillis,
    /// When the lock's TTL ends. The takeover is released 10 s before.
    pub expires_at: EpochMillis,
    /// Whether Xvnc read all of the owner's input through the takeover's control displays
    /// ([`DisplayBinding::input_drained`]). Set by the release, which answers only after those
    /// displays ended, so a host can read what the owner typed without a key landing later.
    /// `None` while active, and when no display was bound to it.
    pub input_drained: Option<bool>,
    /// The tabs that first appeared while it was active and were still open when its release
    /// began. Set by the release, the same whatever its reason; `None` while active.
    pub opened_tabs: Option<Vec<OpenedTab>>,
}

/// A tab that first appeared while a takeover was active and was still open when its release
/// began (`openedTabs`). Neo never gives it to a task by itself: the host decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedTab {
    /// The tab.
    pub tab: TabId,
    /// Its address when the release began; `None` when the release could not list the
    /// browser's tabs.
    pub url: Option<String>,
    /// Whether the browser showed it in front when the release began, before the fit was
    /// restored or the screen let go: the owner's last focus. `None` when the release could not
    /// list the browser's tabs: unknown, never a guess, since a host may decide by it.
    pub in_front: Option<bool>,
    /// The task holding it then, or `None` when no task holds it.
    pub task_id: Option<String>,
    /// The tab that opened it, or `None` for one the owner opened by hand.
    pub opener: Option<TabId>,
}

/// What the app knows of a tab that the takeovers do not, read once for each tab a release
/// lists. Neither may wait.
#[derive(Clone)]
pub struct TabLinks {
    /// The task holding the tab now, by its id.
    pub task: Arc<dyn Fn(TabId) -> Option<String> + Send + Sync>,
    /// The tab that opened it.
    pub opener: Arc<dyn Fn(TabId) -> Option<TabId> + Send + Sync>,
}

impl TabLinks {
    /// No tab held by a task and none opened by another: for a host with no task book.
    #[must_use]
    pub fn none() -> Self {
        Self {
            task: Arc::new(|_| None),
            opener: Arc::new(|_| None),
        }
    }
}

/// One takeover's place in the table.
struct Entry {
    /// Its latest snapshot, which `wait` and a second release watch.
    takeover: watch::Sender<Takeover>,
    phase: Phase,
}

/// Where a takeover's give-back stands.
enum Phase {
    /// What it holds of the parts, until a release takes them to give them back.
    Active(Held),
    /// A release is giving them back.
    Releasing,
    /// Given back at this instant; forgotten an hour later.
    Released(Instant),
}

/// What an active takeover holds besides the ids in its snapshot.
struct Held {
    /// Ties the owner's control displays to the takeover (`release=`).
    binding: DisplayBinding,
    /// Keeps the agent's moves off the owner's screen until the fit is restored.
    screen: ScreenHoldGuard,
    /// Stops the TTL release once the takeover's give-back has begun.
    timer: DropGuard,
    /// The tabs that opened while it was active, oldest first, at most [`OPENED_TABS`], each with
    /// its opener as the app knew it then.
    opened: Vec<Opened>,
}

/// A tab noted as opened, with the tab that opened it as the app knew it at the open. The opener
/// is read then because the app forgets the link once the opener closes, which a release that
/// the takeover's own tab closing started always comes after.
#[derive(Clone, Copy)]
struct Opened {
    tab: TabId,
    opener: Option<TabId>,
}

/// What a give-back returns to the parts. A published takeover holds them all. A start that ran
/// out of time may still be taking its fit. The give-back waits for that part
/// within its budget, as for any step, and gives it back once it is taken.
struct Parts {
    /// Ties the owner's control displays to the takeover. None is bound to an unpublished one.
    binding: DisplayBinding,
    screen: ScreenHoldGuard,
    fit: BoxFuture<'static, Option<FitId>>,
    /// Stops the TTL release once the give-back has begun.
    timer: DropGuard,
    /// The tabs that opened while it was active, for `openedTabs`.
    opened: Vec<Opened>,
}

impl Parts {
    /// What a takeover holds once it is published, with its fit as its snapshot names it.
    fn held(held: Held, fit: Option<FitId>) -> Self {
        let Held {
            binding,
            screen,
            timer,
            opened,
        } = held;
        Self {
            binding,
            screen,
            fit: future::ready(fit).boxed(),
            timer,
            opened,
        }
    }
}

/// Every takeover in the process. Takeovers live in memory only: a Neo restart ends them with
/// their locks and fits, and `Service::start` puts the screen home (decision 30).
pub struct Takeovers {
    locks: Arc<Locks>,
    fits: Arc<Fits>,
    browser: BrowserTabs,
    /// Held by a start from its first look at the table to its end, so two starts on one tab
    /// cannot both find none there. A tab close waits on it too, so a takeover started on a tab
    /// that was closing is found and released.
    starting: tokio::sync::Mutex<()>,
    table: Mutex<HashMap<TakeoverId, Entry>>,
    changes: broadcast::Sender<Takeover>,
    /// Who holds a tab and what opened it, for a release's `openedTabs`.
    links: TabLinks,
}

impl Takeovers {
    /// No takeovers. The flow uses the same parts the host's own routes do:
    /// `locks`, `fits`, and `browser` to check the tab and hold its front. The flow owns
    /// its display binding, which drains owner input before returning the fitted surface.
    /// `links` names the task holding a tab and the tab that opened it, for `openedTabs`.
    #[must_use]
    pub fn new(
        locks: Arc<Locks>,
        fits: Arc<Fits>,
        browser: BrowserTabs,
        links: TabLinks,
    ) -> Arc<Self> {
        Arc::new(Self {
            locks,
            fits,
            browser,
            starting: tokio::sync::Mutex::default(),
            table: Mutex::default(),
            changes: broadcast::channel(TAKEOVER_CHANGES).0,
            links,
        })
    }

    /// Starts a takeover of the tab, in the order the module doc argues for: locked
    /// with its TTL release scheduled, the screen held, then fitted when a viewer
    /// is given. A tab that has an active takeover gets it back as it is. A failed fit gives
    /// back what was taken, in release order, and is the error.
    /// A start not done when its TTL release is due gives back what it
    /// took as that release would, and is released (`Expired`) without ever being active.
    ///
    /// The start runs in a task of its own and goes on if the caller stops waiting, as a host
    /// that hangs up does: cut short, it would leave a takeover's lock that nothing can lift, or
    /// a fit halfway through its plan. A host that retries gets the takeover it made.
    ///
    /// # Errors
    /// `TabNotFound`;
    /// the fit's `Screen`, `Browser`, `TimedOut` or `UnusableViewer`; `TimedOut` when the fit
    /// was not done 10 s before the lock's deadline.
    pub async fn start(self: &Arc<Self>, request: TakeoverRequest) -> Result<Takeover, PartError> {
        let takeovers = Arc::clone(self);
        to_the_end(async move { takeovers.begin(request).await }).await
    }

    async fn begin(self: &Arc<Self>, request: TakeoverRequest) -> Result<Takeover, PartError> {
        let TakeoverRequest {
            tab,
            viewer,
            reads,
            ttl,
        } = request;
        let _one_at_a_time = self.starting.lock().await;
        if let Some(active) = self.active_on(tab).await {
            return Ok(active);
        }
        // A lock on a tab that is not open would protect nothing, and could hold a
        // tab that opens later with the same id.
        self.browser.page(tab).await?;
        let id = TakeoverId::new();
        // Reckoned before the lock is taken, so the TTL release can only come early, never after
        // the lock's own deadline.
        let release_at = Instant::now() + ttl.get().saturating_sub(GIVE_BACK);
        let lock = self
            .locks
            .lock(tab, reads, ttl.0, LockBinding::Takeover(id))
            .await;
        let timer = CancellationToken::new();
        tokio::spawn(release_when_expired(
            Arc::downgrade(self),
            id,
            release_at,
            timer.clone(),
        ));
        let timer = timer.drop_guard();
        let screen = self.browser.hold_screen(ScreenHolder::Takeover);
        let mut takeover = Takeover {
            id,
            tab,
            state: TakeoverState::Active,
            lock: lock.id,
            fit: None,
            created_at: lock.created_at,
            expires_at: lock.expires_at,
            input_drained: None,
            opened_tabs: None,
        };
        if let Some(viewer) = viewer {
            let fits = Arc::clone(&self.fits);
            match done_by(release_at, async move { fits.fit(tab, viewer).await }).await {
                Ok(Ok(fit)) => takeover.fit = Some(fit.id),
                Ok(Err(error)) => {
                    tracing::warn!(
                        takeover = %id,
                        tab = tab.0,
                        %error,
                        "a takeover did not start: its fit failed"
                    );
                    drop(screen);
                    self.unlock(id, lock.id);
                    return Err(error);
                }
                Err(fitting) => {
                    let parts = Parts {
                        binding: DisplayBinding::new(),
                        screen,
                        // A fit that failed holds nothing to restore.
                        fit: fitting.map(|fit| fit.ok().map(|fit| fit.id)).boxed(),
                        timer,
                        // Never active, so the owner opened nothing during it.
                        opened: Vec::new(),
                    };
                    return Err(self.overdue(takeover, parts).await);
                }
            }
        }
        let held = Held {
            binding: DisplayBinding::new(),
            screen,
            timer,
            opened: Vec::new(),
        };
        // Published only before the TTL release is due, decided under the table's mutex, which
        // the release takes to find the takeover. A start done just in time can reach here late,
        // and a takeover published after its release found nothing would never be released.
        let late = {
            let mut table = self.table();
            if Instant::now() < release_at {
                table.insert(
                    id,
                    Entry {
                        takeover: watch::channel(takeover.clone()).0,
                        phase: Phase::Active(held),
                    },
                );
                self.announce(&takeover);
                None
            } else {
                Some(held)
            }
        };
        if let Some(held) = late {
            let parts = Parts::held(held, takeover.fit);
            return Err(self.overdue(takeover, parts).await);
        }
        tracing::info!(
            takeover = %id,
            tab = tab.0,
            lock = %lock.id,
            fit = ?takeover.fit,
            "a takeover started"
        );
        Ok(takeover)
    }

    /// Ends a start that was not done when its TTL release was due. What it took is given back
    /// as that release would give it back, under the lock, and the start answers `TimedOut`. The
    /// takeover is recorded and announced as released (`Expired`), never as active.
    async fn overdue(&self, takeover: Takeover, parts: Parts) -> PartError {
        tracing::warn!(
            takeover = %takeover.id,
            tab = takeover.tab.0,
            "a takeover's start was not done when its TTL release was due; giving back what it took"
        );
        self.give_back(takeover, parts, ReleaseReason::Expired)
            .await;
        PartError::TimedOut("a takeover's start")
    }

    /// Releases a takeover, in the order the module doc argues for, within the 10 s give-back
    /// budget. Idempotent: a takeover that was released, or is being released, is returned as
    /// that release left it, so the first reason stands.
    ///
    /// The give-back runs in a task of its own and goes on if the caller stops waiting: cut
    /// short, it would leave the takeover releasing for good, with its lock held to the TTL.
    ///
    /// # Errors
    /// `NotFound` for an id no takeover has, or one released over an hour ago.
    pub async fn release(
        self: &Arc<Self>,
        id: TakeoverId,
        reason: ReleaseReason,
    ) -> Result<Takeover, PartError> {
        // What the takeover holds moves out under the table's mutex, so exactly one release
        // gives it back. Nothing awaits between this and the task taking it over.
        let taken = {
            let mut table = self.table();
            let entry = table.get_mut(&id).ok_or(PartError::NotFound)?;
            match std::mem::replace(&mut entry.phase, Phase::Releasing) {
                Phase::Active(held) => Ok((entry.takeover.borrow().clone(), held)),
                phase => {
                    entry.phase = phase;
                    Err(entry.takeover.subscribe())
                }
            }
        };
        let (takeover, held) = match taken {
            Ok(taken) => taken,
            Err(changes) => return Ok(released(changes).await),
        };
        let takeovers = Arc::clone(self);
        let parts = Parts::held(held, takeover.fit);
        Ok(to_the_end(async move { takeovers.give_back(takeover, parts, reason).await }).await)
    }

    /// Gives back `parts`, then marks the takeover released and announces it. A step's failure
    /// is logged by [`give_back_step`], and the unlock runs whatever the steps before it did.
    async fn give_back(&self, takeover: Takeover, parts: Parts, reason: ReleaseReason) -> Takeover {
        let Parts {
            binding,
            screen,
            fit,
            timer,
            opened,
        } = parts;
        drop(timer);
        let budget = Instant::now() + GIVE_BACK;
        let id = takeover.id;
        // Read first, while the owner's front and screen are still as they left them.
        let opened_tabs = self.opened_tabs(id, &opened).await;
        let tab_open = reason != ReleaseReason::TabClosed;
        let displays = binding.clone();
        give_back_step(budget, id, "closing its displays", async move {
            binding.close().await;
            Ok(())
        })
        .await;
        let input_drained = displays.input_drained();
        if tab_open {
            let fits = Arc::clone(&self.fits);
            give_back_step(budget, id, "restoring its fit", async move {
                match fit.await {
                    Some(fit) => fits.restore(fit).await.map(drop),
                    None => Ok(()),
                }
            })
            .await;
        }
        drop(screen);
        self.unlock(id, takeover.lock);
        let released = Takeover {
            state: TakeoverState::Released {
                reason,
                at: EpochMillis::now(),
            },
            input_drained,
            opened_tabs: Some(opened_tabs),
            ..takeover
        };
        {
            let mut table = self.table();
            match table.get_mut(&id) {
                Some(entry) => {
                    entry.phase = Phase::Released(Instant::now());
                    entry.takeover.send_replace(released.clone());
                }
                // A start that ran out of time was never published, so released is its first state.
                None => {
                    table.insert(
                        id,
                        Entry {
                            takeover: watch::channel(released.clone()).0,
                            phase: Phase::Released(Instant::now()),
                        },
                    );
                }
            }
            self.announce(&released);
        }
        tracing::info!(takeover = %id, tab = takeover.tab.0, ?reason, "a takeover was released");
        released
    }

    /// A release's `openedTabs`: each tab `opened` noted that is still open, as one listing of the
    /// browser's tabs shows it now, with the task holding it and the tab that opened it. A
    /// listing that fails or takes over [`OPENED_LISTING`] lists every noted tab with its address
    /// and front unknown (`None`): a host that treats these rows as private never misses one the
    /// owner opened, and one that decides by the owner's last focus is not misled by a guess.
    async fn opened_tabs(&self, id: TakeoverId, opened: &[Opened]) -> Vec<OpenedTab> {
        if opened.is_empty() {
            return Vec::new();
        }
        let listed = match tokio::time::timeout(OPENED_LISTING, self.browser.listed_tabs()).await {
            Ok(Ok(listed)) => Some(listed),
            Ok(Err(error)) => {
                tracing::warn!(takeover = %id, %error, "a release could not list the browser's tabs; its openedTabs name every tab it noted, url and inFront unknown");
                None
            }
            Err(_elapsed) => {
                tracing::warn!(takeover = %id, "a release's listing of the browser's tabs took too long; its openedTabs name every tab it noted, url and inFront unknown");
                None
            }
        };
        opened
            .iter()
            .filter_map(|&Opened { tab, opener }| {
                let (url, in_front) = match &listed {
                    Some(listed) => {
                        let found = listed.iter().find(|listed| listed.tab == tab)?;
                        (Some(found.url.clone()), Some(found.active))
                    }
                    None => (None, None),
                };
                Some(OpenedTab {
                    tab,
                    url,
                    in_front,
                    task_id: (self.links.task)(tab),
                    // As at the open, else as now: a popup registered after its open.
                    opener: opener.or_else(|| (self.links.opener)(tab)),
                })
            })
            .collect()
    }

    /// `tab` opened, and was not open before: every active takeover notes it, up to
    /// [`OPENED_TABS`] each, for its release's `openedTabs`. A takeover still starting or already
    /// releasing notes nothing: the owner reaches the screen once one is active, and a release
    /// lists what it noted when it began.
    pub fn note_opened(&self, tab: TabId) {
        let opener = (self.links.opener)(tab);
        for entry in self.table().values_mut() {
            if let Phase::Active(held) = &mut entry.phase
                && held.opened.len() < OPENED_TABS
                && !held.opened.iter().any(|opened| opened.tab == tab)
            {
                held.opened.push(Opened { tab, opener });
            }
        }
    }

    /// One takeover, active or released. `None` for an id no takeover has, or one released over
    /// an hour ago.
    #[must_use]
    pub fn get(&self, id: TakeoverId) -> Option<Takeover> {
        self.table()
            .get(&id)
            .map(|entry| entry.takeover.borrow().clone())
    }

    /// The takeover as soon as it is released, or as it stands at `until` (`GET
    /// /v1/takeovers/{id}?wait=`). Release is the only change a takeover goes through, and a
    /// released one is returned at once. `None` as [`Self::get`].
    pub async fn wait(&self, id: TakeoverId, until: Instant) -> Option<Takeover> {
        let changes = self.table().get(&id)?.takeover.subscribe();
        tokio::time::timeout_at(until, released(changes.clone()))
            .await
            .ok()
            .or_else(|| Some(changes.borrow().clone()))
    }

    /// What a control display passes to `Displays::admit` to belong to this takeover
    /// (`release=`): its release ends the display.
    ///
    /// # Errors
    /// `NotFound` as [`Self::get`]; `TakeoverReleased` once the release has begun.
    pub fn binding(&self, id: TakeoverId) -> Result<DisplayBinding, PartError> {
        match &self.table().get(&id).ok_or(PartError::NotFound)?.phase {
            Phase::Active(held) => Ok(held.binding.clone()),
            Phase::Releasing | Phase::Released(_) => Err(PartError::TakeoverReleased),
        }
    }

    /// Releases the takeover on a tab that closed (`TabClosed`). The restore step needs the tab and
    /// is skipped: the fits restore a closed tab's fit themselves. A
    /// start that is running finishes first, so a takeover it makes on the tab is released too.
    pub async fn tab_closed(self: &Arc<Self>, tab: TabId) {
        self.release_closed(|on| on == tab).await;
    }

    /// The browser link reconnected with `open` tabs: every takeover of another tab is released
    /// as [`Self::tab_closed`] does.
    pub async fn retain_tabs(self: &Arc<Self>, open: &[TabId]) {
        self.release_closed(|on| !open.contains(&on)).await;
    }

    /// Each takeover as it starts and as it is released: the `takeover.state` events.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Takeover> {
        self.changes.subscribe()
    }

    /// The active takeover on `tab`, once a release running there has finished. Called with
    /// `starting` held, so no other takeover can appear on the tab meanwhile.
    async fn active_on(&self, tab: TabId) -> Option<Takeover> {
        loop {
            let releasing = {
                let table = self.table();
                let entry = table.values().find(|entry| {
                    !matches!(entry.phase, Phase::Released(_)) && entry.takeover.borrow().tab == tab
                })?;
                match entry.phase {
                    Phase::Active(_) => return Some(entry.takeover.borrow().clone()),
                    Phase::Releasing | Phase::Released(_) => entry.takeover.subscribe(),
                }
            };
            released(releasing).await;
        }
    }

    /// Lifts the takeover's lock, the binding's own unlock (decision 29). It fails only for a
    /// lock that is long gone, which leaves nothing to lift.
    fn unlock(&self, takeover: TakeoverId, lock: LockId) {
        if let Err(error) = self.locks.unlock_takeover(lock, takeover) {
            tracing::warn!(%takeover, %lock, %error, "a takeover's lock could not be lifted");
        }
    }

    /// Releases every takeover on a tab `closed` names.
    async fn release_closed(self: &Arc<Self>, closed: impl Fn(TabId) -> bool) {
        // A start that is running finishes first, so its takeover is in the table when it is
        // read. Taking the mutex and dropping it is enough: a start after this finds the tab
        // closed.
        drop(self.starting.lock().await);
        let ids = self
            .table()
            .iter()
            .filter(|(_, entry)| {
                !matches!(entry.phase, Phase::Released(_)) && closed(entry.takeover.borrow().tab)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in ids {
            // One already being released returns once that release is done; the id cannot be
            // forgotten while it is.
            let _ = self.release(id, ReleaseReason::TabClosed).await;
        }
    }

    /// The takeovers, with released ones forgotten an hour after their release. Never held
    /// across an await.
    fn table(&self) -> MutexGuard<'_, HashMap<TakeoverId, Entry>> {
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        table.retain(|_, entry| match entry.phase {
            Phase::Released(at) => at.elapsed() < KEEP_RELEASED_TAKEOVER,
            Phase::Active(_) | Phase::Releasing => true,
        });
        table
    }

    /// Broadcasts a state change. Callers hold the table's mutex, which keeps the announcements
    /// in the order the changes happened.
    fn announce(&self, takeover: &Takeover) {
        // No subscriber is fine: the service subscribes when it starts, and tests may not.
        let _ = self.changes.send(takeover.clone());
    }
}

/// Runs `work` in a task of its own and waits for it. A caller that stops waiting leaves the work
/// to finish, so a start or a release is never cut short halfway.
async fn to_the_end<T: Send + 'static>(work: impl Future<Output = T> + Send + 'static) -> T {
    output(tokio::spawn(work).await).await
}

/// Runs `work` in a task of its own and waits for it until `deadline`. Work still running then
/// goes on, never cut short halfway, and comes back as its output once it is done.
async fn done_by<T: Send + 'static>(
    deadline: Instant,
    work: impl Future<Output = T> + Send + 'static,
) -> Result<T, BoxFuture<'static, T>> {
    let mut task = tokio::spawn(work);
    match tokio::time::timeout_at(deadline, &mut task).await {
        Ok(done) => Ok(output(done).await),
        Err(_) => Err(async move { output(task.await).await }.boxed()),
    }
}

/// What a task's work returned, as if it had run in place.
async fn output<T>(joined: Result<T, JoinError>) -> T {
    match joined {
        Ok(done) => done,
        Err(error) => match error.try_into_panic() {
            // A bug in the work is the caller's, as it would be had the work run in place.
            Ok(panic) => std::panic::resume_unwind(panic),
            // Only a runtime that is shutting down cancels the task, and it drops this caller
            // with it.
            Err(_) => std::future::pending().await,
        },
    }
}

/// The takeover once it reads as released. Its sender lives as long as its entry, an hour past
/// the release, so a closed channel only means it was forgotten; the last snapshot stands.
async fn released(mut changes: watch::Receiver<Takeover>) -> Takeover {
    let _ = changes
        .wait_for(|takeover| takeover.state != TakeoverState::Active)
        .await;
    changes.borrow().clone()
}

/// Runs one give-back step in a task of its own and waits for it until `budget`. A step still
/// running then goes on in the background and the release moves on: the unlock never waits past
/// the budget, and no part is left halfway through a change.
async fn give_back_step(
    budget: Instant,
    takeover: TakeoverId,
    step: &'static str,
    work: impl Future<Output = Result<(), PartError>> + Send + 'static,
) {
    match tokio::time::timeout_at(budget, tokio::spawn(work)).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => {
            tracing::warn!(%takeover, %error, "a takeover's give-back failed {step}; it goes on");
        }
        Ok(Err(error)) => {
            tracing::warn!(%takeover, %error, "a takeover's give-back stopped {step}; it goes on");
        }
        Err(_) => tracing::warn!(
            %takeover,
            "a takeover's give-back ran out of its {GIVE_BACK:?} {step}; that finishes in the \
             background"
        ),
    }
}

/// The TTL release: `Expired` at `at`, 10 s before the lock's deadline, unless `stop` fires
/// first because another release began. Only a weak handle waits, so a timer keeps nothing alive.
///
/// It is scheduled as the lock is taken. A takeover still starting at `at` is not in the table
/// yet, so this finds nothing. Its start waits only until the same `at` and gives back itself.
async fn release_when_expired(
    takeovers: Weak<Takeovers>,
    id: TakeoverId,
    at: Instant,
    stop: CancellationToken,
) {
    tokio::select! {
        () = stop.cancelled() => {}
        () = tokio::time::sleep_until(at) => {
            if let Some(takeovers) = takeovers.upgrade() {
                // A release that began first answers this one with its own reason.
                let _ = takeovers.release(id, ReleaseReason::Expired).await;
            }
        }
    }
}
