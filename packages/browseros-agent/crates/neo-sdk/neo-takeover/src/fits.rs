//! Fits a tab to its owner's screen, keeps it fitted while the owner's viewer resizes the
//! screen, and restores the tab, window and screen when the owner gives control back.
//!
//! State, each piece with one owner:
//! - the fits, in a table behind a `std::sync::Mutex` that no code holds across an await, so
//!   [`Fits::view`] answers from it at once and a host's poll never touches the browser;
//! - Neo's one screen and one window, a pure [`Surface`] behind a `tokio::sync::Mutex`. It is
//!   held across a plan's CDP and RFB calls because two plans must never interleave: a claim
//!   and a resize running together could leave a fullscreen window with a desktop viewport, or
//!   the screen at one fit's size with the tab at another's;
//! - the owner view reads, behind a second `tokio::sync::Mutex` that a release takes too: a
//!   read can close the fitted tab's popups and move it, and that must land before the tab
//!   goes back to the agent. A release takes it while it holds the surface's; nothing takes
//!   them the other way round;
//! - the screen's size as the watcher last saw it, on a `watch` channel;
//! - each fit's hold on the screen for its owner (decision 44), a guard in the front's registry
//!   (`browser_tabs`): taken after Surface resolves the request and before browser work, kept
//!   in its table entry while active, and let go once its restore's plan ran. The agent cannot move the front and
//!   followers pause while any fit holds it, so neither strands the owner's phone fit.
//!
//! ```text
//!  fit(tab, viewer) ── surface ──▶ Seen ─▶ Surface::claim ─▶ surface_steps::run ─▶ Fit
//!                                  └─ the first fit starts the fits loop and the watcher
//!  fits loop:  watched size ── surface ──▶ Surface::follow_screen ─▶ run ─▶ every fit
//!              every 750 ms ── owner views ──▶ owner_view::read for each active fit
//!                                              ─▶ its keyboard, bar, popups
//!  restore(id), tab_closed(tab) ── surface, owner views ──▶ Surface::release ─▶ run
//!                                                          ─▶ the last stops the loop
//! ```
//!
//! A fit is idempotent per tab: fitting a tab whose fit is active refits it for the new
//! viewer and returns the same fit. Restoring is idempotent per fit.

use crate::browser_tabs::{ScreenHoldGuard, WindowState};
use crate::fit_rule::{FitMode, Layout};
use crate::limits::{
    KEEP_RESTORED_FIT, NAVIGATE_TIMEOUT, OWNER_VIEW_BUDGET, OWNER_VIEW_TICK, STEP_TIMEOUT,
    WATCH_CATCH_UP,
};
use crate::owner_address::{Address, owner_address};
use crate::owner_view::{self, OwnerView};
use crate::screen::Screen;
use crate::surface::{Seen, Surface};
use crate::surface_steps::{self, PlanFor};
use crate::{
    BrowserTabs, FitId, PartError, ScreenHolder, ScreenSize, TabHold, Viewer, ViewerGeometry, Xvnc,
};
use browseros_core::{TabId, TargetId};
use neo_trace::EpochMillis;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::watch;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

/// A fit as a host reads it: geometry for the owner's page, never values, labels or text.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    /// Its id.
    pub id: FitId,
    /// The fitted tab.
    pub tab: TabId,
    /// The physical screen the tab is fitted to, including external viewer resizes.
    pub screen: ScreenSize,
    /// The logical device and preview scale currently shared by every active fit.
    pub viewer: ViewerGeometry,
    /// Mobile or desktop, from the logical viewer width.
    pub mode: FitMode,
    /// How its last application landed.
    pub fitted: Fitted,
    /// Active, or restored.
    pub state: FitState,
    /// The latest keyboard boxes, while they were measured at `screen`.
    pub keyboard: Option<Keyboard>,
    /// The tab's place, on a phone.
    pub bar: Option<BrowserBar>,
    /// The pages the tab opened that stay where they are.
    pub popups: Vec<TargetId>,
    /// When it was created.
    pub created_at: EpochMillis,
}

/// Where a fit stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitState {
    /// It holds its tab, and follows the screen.
    Active,
    /// It put its tab back, at this time.
    Restored(EpochMillis),
}

/// How a fit's last application landed: the contract's `fit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fitted {
    /// Every step landed, and the page reported the mode and size asked of it.
    Full,
    /// A step failed, or the page did not report the mode and size asked of it. Neo does not
    /// retry.
    Partial,
}

/// The boxes of the fields a phone keyboard opens for: a field's own box, or for a field with
/// no size that the page draws with its own elements, the drawing's box, where the owner taps
/// (`owner_view`'s script).
#[derive(Clone, Debug, PartialEq)]
pub struct Keyboard {
    /// The screen size they were measured at. The owner's viewer uses them only at that size.
    pub screen: ScreenSize,
    /// `[x, y, width, height]` in screen px; at most 128.
    pub boxes: Vec<[f64; 4]>,
    /// When they were measured. The owner's viewer drops boxes older than 1.5 s.
    pub at: EpochMillis,
}

/// The fitted tab's place, for the owner page's browser bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserBar {
    /// The tab's current URL, as reported by browser navigation history.
    pub url: String,
    /// Whether there is history to go back to.
    pub back: bool,
    /// Whether there is history to go forward to.
    pub forward: bool,
}

/// One command from the owner's browser bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BarAction {
    /// History back.
    Back,
    /// History forward.
    Forward,
    /// Reload.
    Reload,
    /// What the owner typed: a web address opens (with `https://` added when it names no
    /// scheme), other text is searched, and a non-http(s) scheme or credentials are refused.
    Open(String),
}

/// What a browser bar command did. Neither a refusal nor a failure ends the owner's view; the
/// owner page shows the notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarOutcome {
    /// The tab moved.
    Completed,
    /// The typed text named a scheme other than http(s), or credentials.
    Refused,
    /// The tab did not move, or not within 20 s.
    Failed,
}

impl BarOutcome {
    /// A sentence for the owner when the bar did not move.
    #[must_use]
    pub fn notice(self) -> Option<&'static str> {
        match self {
            Self::Completed => None,
            Self::Refused => {
                Some("That is not a web address. Type another one, or words to search for.")
            }
            Self::Failed => Some("The page did not move. Try again."),
        }
    }
}

struct Entry {
    fit: Fit,
    /// The pages that stayed where they are at the last owner view read; `None` before the
    /// fit's first read.
    known_popups: Option<Vec<TargetId>>,
    standing: Standing,
}

/// A fit holds the screen for its owner exactly while it is active.
enum Standing {
    /// It holds its tab, and the screen.
    Active { _held: ScreenHoldGuard },
    /// It put its tab back at this instant, and is forgotten an hour later.
    Restored(Instant),
}

impl Entry {
    /// When it was restored; `None` while it is active.
    fn restored(&self) -> Option<Instant> {
        match self.standing {
            Standing::Active { .. } => None,
            Standing::Restored(at) => Some(at),
        }
    }
}

/// Every fit in the process, Neo's one screen and window, and the loop that keeps active fits
/// fitted. Fits live in memory only: a Neo restart drops them, and `Service::start` puts the
/// screen and window home (decision 30).
pub struct Fits {
    browser: BrowserTabs,
    screen: Screen,
    table: Mutex<HashMap<FitId, Entry>>,
    surface: tokio::sync::Mutex<Surface>,
    /// Held through each round of owner view reads, and by each release until its fit reads as
    /// restored, so no read of the fit starts after it.
    owner_views: tokio::sync::Mutex<()>,
    /// The screen's size as the watcher last reported it; `None` while it does not run.
    watched: watch::Sender<Option<ScreenSize>>,
    /// Stops the fits loop and its watcher; `Some` while they run.
    looping: Mutex<Option<CancellationToken>>,
}

impl Fits {
    /// Builds viewport fits over the browser and its shared screen.
    #[must_use]
    pub fn new(browser: BrowserTabs, xvnc: Xvnc) -> Arc<Self> {
        Arc::new(Self {
            browser,
            screen: Screen::new(xvnc),
            table: Mutex::default(),
            surface: tokio::sync::Mutex::default(),
            owner_views: tokio::sync::Mutex::default(),
            watched: watch::channel(None).0,
            looping: Mutex::default(),
        })
    }

    /// Fits `tab` to `viewer`: brings the tab to the front, resizes Neo's screen, then resets
    /// zoom and sets the window and the viewport for the logical viewer. Returns once that ran,
    /// with `fitted` saying whether all of it landed. The first fit starts the fits loop and
    /// the screen watcher. From the start of its claim to the end of its restore, the fit holds
    /// the screen for its owner.
    ///
    /// # Errors
    /// `InvalidViewer` when a refit's inherited scale requires a phone or its resolved physical
    /// sides exceed 4096; `UnusableViewer` under 100 px a side; `TabNotFound`; `Browser` when
    /// the tab's window cannot be read; `Screen` when Xvnc cannot be resized, which leaves the
    /// fits and the surface as they were; `TimedOut` when the tab lookup or an in-flight viewport
    /// change does not finish within the browser-step budget, before any fit is claimed.
    pub async fn fit(self: &Arc<Self>, tab: TabId, viewer: Viewer) -> Result<Fit, PartError> {
        let mut surface = self.surface.lock().await;
        // Scale inheritance and its bounds are resolved by the surface before browser work.
        // Its lock also prevents another fit or watcher from changing that choice mid-plan.
        let id = self.active_fit(tab).unwrap_or_default();
        let layout = surface.resolve_request(id, viewer)?;
        // Before browser work. Without the hold an agent move could pass the front rule
        // while this claim is reading the window, then land on top of its activation.
        let held = self.browser.hold_screen(ScreenHolder::Fit);
        // The lookup shares Surface with every restore. Bound it like a plan step so a stalled
        // browser cannot keep another takeover fitted past its give-back budget.
        let page = tokio::time::timeout(STEP_TIMEOUT, self.browser.page(tab))
            .await
            .map_err(|_| PartError::TimedOut("looking up the tab to fit"))??;
        let seen = Seen {
            window: self.browser.window(&page).await?,
            viewport: self.browser.viewport(&page).await?,
        };
        let before = surface.clone();
        let plan = surface.claim(id, tab, layout, seen);
        let refitted = plan.refitted();
        let fitted =
            match surface_steps::run(plan, &self.browser, &self.screen, PlanFor::Claim).await {
                Ok(fitted) => fitted,
                Err(error) => {
                    *surface = before;
                    return Err(error);
                }
            };
        let fit = {
            let mut table = self.table();
            table.entry(id).or_insert_with(|| Entry {
                fit: Fit {
                    id,
                    tab,
                    screen: layout.screen,
                    viewer: layout.viewer,
                    mode: layout.shape.mode(),
                    fitted,
                    state: FitState::Active,
                    keyboard: None,
                    bar: None,
                    popups: Vec::new(),
                    created_at: EpochMillis::now(),
                },
                known_popups: None,
                standing: Standing::Active { _held: held },
            });
            apply(&mut table, layout, &refitted, fitted);
            table.get(&id).map(|entry| entry.fit.clone())
        };
        if self.start_loop() {
            // The watcher may still hold the size from before this resize; until it reports
            // this one, the fits loop could take the older report for the owner's resize and
            // refit to it. The loop reads the latest size under the surface lock held here.
            let mut sizes = self.watched.subscribe();
            let caught_up = sizes.wait_for(|size| *size == Some(layout.screen));
            if tokio::time::timeout(WATCH_CATCH_UP, caught_up)
                .await
                .is_err()
            {
                tracing::debug!(screen = %layout.screen, "the screen watcher did not report the new size in time");
            }
        }
        drop(surface);
        tracing::info!(fit = %id, tab = tab.0, screen = %layout.screen, mode = ?layout.shape.mode(), ?fitted, "fitted");
        fit.ok_or(PartError::NotFound)
    }

    /// One fit as the fits loop last saw it. Never waits on the browser.
    #[must_use]
    pub fn view(&self, id: FitId) -> Option<Fit> {
        self.table().get(&id).map(|entry| entry.fit.clone())
    }

    /// Whether an active fit is on `tab` now, or when its last fit was restored: the idle-tab
    /// janitor keeps a fitted tab, which is how a host shows it to the owner, and restarts its
    /// idle clock from the restore. `None` when no fit named the tab within the last hour
    /// (`limits::KEEP_RESTORED_FIT`). Never waits on the browser.
    #[must_use]
    pub fn hold(&self, tab: TabId) -> Option<TabHold> {
        self.table()
            .values()
            .filter(|entry| entry.fit.tab == tab)
            .map(|entry| entry.restored().map_or(TabHold::Now, TabHold::Until))
            .reduce(TabHold::latest)
    }

    /// The owner's browser bar, on the fitted tab. It is the host's own call, not the agent's,
    /// so no lock refuses it.
    ///
    /// # Errors
    /// `NotFound`; `FitRestored` once the fit is restored; `TabNotFound` and `Browser` when the
    /// tab cannot be reached. A move that fails is `BarOutcome::Failed`, not an error.
    pub async fn navigate(&self, id: FitId, action: BarAction) -> Result<BarOutcome, PartError> {
        let tab = {
            let table = self.table();
            let entry = table.get(&id).ok_or(PartError::NotFound)?;
            if entry.restored().is_some() {
                return Err(PartError::FitRestored);
            }
            entry.fit.tab
        };
        let page = self.browser.page(tab).await?;
        let navigation = self.browser.navigation(&page);
        let moved = match action {
            BarAction::Back => tokio::time::timeout(NAVIGATE_TIMEOUT, navigation.back()).await,
            BarAction::Forward => {
                tokio::time::timeout(NAVIGATE_TIMEOUT, navigation.forward()).await
            }
            BarAction::Reload => {
                tokio::time::timeout(NAVIGATE_TIMEOUT, async {
                    navigation.reload().await.map(|()| true)
                })
                .await
            }
            BarAction::Open(typed) => match owner_address(&typed) {
                Address::Refused => return Ok(BarOutcome::Refused),
                Address::Open(url) => {
                    tokio::time::timeout(NAVIGATE_TIMEOUT, async {
                        navigation.goto(&url).await.map(|_| true)
                    })
                    .await
                }
            },
        };
        Ok(match moved {
            Ok(Ok(true)) => BarOutcome::Completed,
            Ok(Ok(false)) => BarOutcome::Failed,
            Ok(Err(error)) => {
                tracing::debug!(%error, fit = %id, "the owner's browser bar did not move the tab");
                BarOutcome::Failed
            }
            Err(_) => BarOutcome::Failed,
        })
    }

    /// Puts the fit's tab back as the fit found it: Ctrl+0 and its recorded viewport. When no
    /// other fit is active, the screen goes back to its home size and the window to its home
    /// state first, and the fits loop and the watcher stop. Idempotent: a restored fit is
    /// returned as it is.
    ///
    /// An owner view read that is running finishes first, so a popup it is pulling into the tab
    /// lands before the tab is the agent's again.
    ///
    /// # Errors
    /// `NotFound`.
    pub async fn restore(&self, id: FitId) -> Result<Fit, PartError> {
        let mut surface = self.surface.lock().await;
        let restored = {
            let table = self.table();
            let entry = table.get(&id).ok_or(PartError::NotFound)?;
            entry.restored().map(|_| entry.fit.clone())
        };
        match restored {
            Some(fit) => Ok(fit),
            None => self.release(&mut surface, id).await,
        }
    }

    /// The fitted tab closed: its fit is restored, so a fit nobody restores cannot leave the
    /// screen at a phone's size. Its own tab's steps fail quietly.
    pub async fn tab_closed(&self, tab: TabId) {
        let mut surface = self.surface.lock().await;
        if let Some(id) = self.active_fit(tab) {
            // Release never fails; `NotFound` cannot happen for a fit just read as active.
            let _ = self.release(&mut surface, id).await;
        }
    }

    /// The browser link reconnected with `open` tabs: every fit of another tab is restored, as
    /// if its tab closed.
    pub async fn retain_tabs(&self, open: &[TabId]) {
        let mut surface = self.surface.lock().await;
        let gone = self
            .table()
            .values()
            .filter(|entry| entry.restored().is_none() && !open.contains(&entry.fit.tab))
            .map(|entry| entry.fit.id)
            .collect::<Vec<_>>();
        for id in gone {
            let _ = self.release(&mut surface, id).await;
        }
    }

    /// Puts the screen at its home size and takes every fullscreen window out of fullscreen,
    /// unless a fit holds them already (decision 30). A Neo restart in the middle of a fit
    /// drops the fit but would leave the owner's size, and nothing else would put it back.
    /// Best-effort: each failure is logged.
    pub async fn put_surface_home(&self) {
        let surface = self.surface.lock().await;
        if surface.is_claimed() {
            return;
        }
        let home = self.screen.home();
        if let Err(error) = self.screen.resize(home).await {
            tracing::warn!(%error, %home, "could not put Neo's screen at its home size");
        }
        let windows = match self.browser.windows().await {
            Ok(windows) => windows,
            Err(error) => {
                tracing::warn!(%error, "could not read the browser's windows to put them home");
                return;
            }
        };
        for (window, state) in windows {
            if state != WindowState::Fullscreen {
                continue;
            }
            let normal = self
                .browser
                .set_window_state(window, WindowState::Maximized);
            match tokio::time::timeout(STEP_TIMEOUT, normal).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, window = window.0, "could not take a window out of fullscreen");
                }
                Err(_) => {
                    tracing::warn!(
                        window = window.0,
                        "a window did not leave fullscreen in time"
                    );
                }
            }
        }
    }

    /// Releases `id`'s claim under the held surface lock, then marks it restored and lets go of
    /// its hold on the screen, so no agent move lands while its plan runs. The last release
    /// stops the fits loop and its watcher.
    ///
    /// It first waits for a running round of owner view reads to end, rather than cancel it: a
    /// navigation a read has sent goes on in the browser whether or not the read is dropped. So
    /// only a read that outlives its 5 s budget can still move the tab after this returns.
    async fn release(&self, surface: &mut Surface, id: FitId) -> Result<Fit, PartError> {
        let _views = self.owner_views.lock().await;
        let plan = surface.release(id, self.screen.home());
        // A release never fails: every step is logged and the next one runs.
        let _ = surface_steps::run(plan, &self.browser, &self.screen, PlanFor::Change).await;
        if !surface.is_claimed() {
            self.stop_loop();
        }
        let (fit, standing) = {
            let mut table = self.table();
            let entry = table.get_mut(&id).ok_or(PartError::NotFound)?;
            let standing =
                std::mem::replace(&mut entry.standing, Standing::Restored(Instant::now()));
            entry.fit.state = FitState::Restored(EpochMillis::now());
            entry.fit.keyboard = None;
            entry.fit.bar = None;
            (entry.fit.clone(), standing)
        };
        drop(standing);
        tracing::info!(fit = %id, tab = fit.tab.0, "restored");
        Ok(fit)
    }

    fn active_fit(&self, tab: TabId) -> Option<FitId> {
        self.table()
            .values()
            .find(|entry| entry.restored().is_none() && entry.fit.tab == tab)
            .map(|entry| entry.fit.id)
    }

    /// Starts the fits loop and its watcher unless they run. Whether they were running.
    fn start_loop(self: &Arc<Self>) -> bool {
        let mut looping = self.looping.lock().unwrap_or_else(PoisonError::into_inner);
        if looping.is_some() {
            return true;
        }
        let stop = CancellationToken::new();
        *looping = Some(stop.clone());
        tokio::spawn(Arc::clone(self).run_loop(stop));
        false
    }

    fn stop_loop(&self) {
        let stopped = self
            .looping
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(stop) = stopped {
            stop.cancel();
        }
        // Read under the surface lock by the next first fit's loop, so an old size from a
        // watcher that just stopped cannot stand for the screen's size.
        self.watched.send_replace(None);
    }

    /// The fits loop: one per process while any fit is active. It follows the watched screen
    /// size, refitting every claim when the owner's viewer resizes the screen, and reads each
    /// active fit's owner view every 750 ms. The watcher runs in its own task, so a slow read
    /// never leaves Xvnc's reports unread.
    async fn run_loop(self: Arc<Self>, stop: CancellationToken) {
        tokio::spawn({
            let fits = Arc::clone(&self);
            let stop = stop.clone();
            async move {
                tokio::select! {
                    biased;
                    () = stop.cancelled() => {}
                    () = fits.screen.watch(&fits.watched) => {}
                }
            }
        });
        let mut sizes = self.watched.subscribe();
        let mut tick = tokio::time::interval_at(Instant::now() + OWNER_VIEW_TICK, OWNER_VIEW_TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                () = stop.cancelled() => return,
                changed = sizes.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    self.follow_screen().await;
                }
                _ = tick.tick() => self.read_owner_views().await,
            }
        }
    }

    /// Refits every claim to the screen's size as the watcher last saw it.
    async fn follow_screen(&self) {
        let mut surface = self.surface.lock().await;
        // Read under the lock: a fit that resized the screen held it until the watcher
        // reported that size, so the size read here is never older than the fit's.
        let watched = *self.watched.borrow();
        let Some(size) = watched else {
            return;
        };
        // Unusable logical dimensions, below the minimum or above the scaled mobile limit,
        // leave the last usable layout in place until the next observation.
        let Some(layout) = surface.observed_layout(size) else {
            return;
        };
        let Some(plan) = surface.follow_screen(layout) else {
            return;
        };
        let refitted = plan.refitted();
        let fitted = surface_steps::run(plan, &self.browser, &self.screen, PlanFor::Change)
            .await
            .unwrap_or(Fitted::Partial);
        apply(&mut self.table(), layout, &refitted, fitted);
        tracing::info!(screen = %size, mode = ?layout.shape.mode(), ?fitted, "fits followed the screen");
    }

    /// Reads each active fit's owner view, one after another. A failed read keeps the last
    /// good one; the next tick tries again.
    async fn read_owner_views(&self) {
        let _round = self.owner_views.lock().await;
        // Listed under the lock, so a fit restored while this waited for it is not read.
        let active = self
            .table()
            .values()
            .filter(|entry| entry.restored().is_none())
            .map(|entry| {
                (
                    entry.fit.id,
                    entry.fit.tab,
                    entry.fit.screen,
                    entry.fit.viewer,
                    entry.fit.mode,
                    entry.known_popups.clone(),
                )
            })
            .collect::<Vec<_>>();
        for (id, tab, screen, viewer, mode, known) in active {
            let read = owner_view::read(&self.browser, tab, mode, screen, viewer, known.as_deref());
            match tokio::time::timeout(OWNER_VIEW_BUDGET, read).await {
                Ok(Ok(view)) => self.store(id, screen, viewer, mode, view),
                Ok(Err(error)) => tracing::debug!(%error, fit = %id, "an owner view read failed"),
                Err(_) => tracing::debug!(fit = %id, "an owner view read ran out of time"),
            }
        }
    }

    /// Keeps a reading, unless the fit was restored or refitted while it was read: then it
    /// describes a tab that is no longer fitted that way.
    fn store(
        &self,
        id: FitId,
        screen: ScreenSize,
        viewer: ViewerGeometry,
        mode: FitMode,
        view: OwnerView,
    ) {
        let mut table = self.table();
        let Some(entry) = table.get_mut(&id) else {
            return;
        };
        // Equal physical screens can show different logical devices, such as 600x1000 at
        // 1x and 300x500 at 2x. Every geometry component belongs to the read's identity.
        if entry.restored().is_some()
            || entry.fit.screen != screen
            || entry.fit.viewer != viewer
            || entry.fit.mode != mode
        {
            return;
        }
        entry.fit.keyboard = view.keyboard;
        entry.fit.bar = view.bar;
        entry.known_popups = Some(view.popups.clone());
        entry.fit.popups = view.popups;
    }

    /// The fits, with restored ones forgotten an hour after they were restored. Never held
    /// across an await.
    fn table(&self) -> MutexGuard<'_, HashMap<FitId, Entry>> {
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        table.retain(|_, entry| {
            entry
                .restored()
                .is_none_or(|restored| restored.elapsed() < KEEP_RESTORED_FIT)
        });
        table
    }
}

/// Records a plan that showed `layout` on every active fit: each follows the screen's size and
/// mode, and the tabs the plan refitted take its outcome. Keyboard boxes measured at another
/// logical geometry or scale are dropped even when physical dimensions match, since they
/// would open the keyboard in the wrong place.
fn apply(table: &mut HashMap<FitId, Entry>, layout: Layout, refitted: &[TabId], fitted: Fitted) {
    for entry in table.values_mut() {
        if entry.restored().is_some() {
            continue;
        }
        if entry.fit.screen != layout.screen
            || entry.fit.viewer != layout.viewer
            || entry.fit.mode != layout.shape.mode()
        {
            entry.fit.keyboard = None;
        }
        entry.fit.screen = layout.screen;
        entry.fit.viewer = layout.viewer;
        entry.fit.mode = layout.shape.mode();
        if refitted.contains(&entry.fit.tab) {
            entry.fit.fitted = fitted;
        }
    }
}
