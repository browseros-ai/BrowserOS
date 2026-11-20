//! The tab ledger: which tabs are open, by the origin each was born with; when each opened, why
//! it closed and after how long; which renderers crashed. The one place that decides Neo's tab
//! telemetry (`neo_tabs_open`, `neo_tabs_opened_total`, `neo_tabs_closed_total`,
//! `neo_tab_lifetime_seconds`, `neo_renderer_crashes_total`).
//!
//! ```text
//!  tab_watch (the only writer of browser facts) ── listed / crashed / recovered ───┐
//!  page.close wrapper, janitor ── intend_close ─▶ CloseIntent ──────────────────────┤
//!  newPage wrapper, NeoTab     ── expect_open  ─▶ OpenIntent ── claim ──────────────┼─▶ TabLife
//!                                                                                   │   Mutex<TabLedger>, pure
//!  neo_tabs_open, on the collecting thread ◀── open_counts ─────────────────────────┤     → Vec<TabFact>
//!  the facade's tab hooks ◀── each TabFact, once the lock is released ◀─────────────┘
//! ```
//!
//! Kept apart from `TabRegistry`'s admission map on purpose (architect arch-tabs, REBUILD): that
//! map holds only the tabs an op registered, its origin is janitor and sight policy that changes
//! over a tab's life, and its removal hook fires when a tab merely navigates to an excluded URL.
//! This ledger holds every tab the browser lists, and only the tab watch tells it what the
//! browser did; Neo's own code only says what it is about to do (an intent).
//!
//! Counting holds by construction: a tab enters `open` only where `TabFact::Opened` is produced
//! and leaves only where `TabFact::Closed` is, in the same locked step, and its origin never
//! changes once it is born. So per origin `opened − closed == open` for this process, the gauge
//! is a map count that cannot go negative, and a tab closes at most once: a second close finds
//! no entry. Facts are sent after the lock is released, so two threads' facts may reach the
//! counters in either order for a moment; the ledger itself never disagrees with them.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use browseros_core::{TabId, TargetId};
use neo_telemetry::label::{CloseReason, TabOrigin};
use neo_telemetry::{OpenTabCounts, Telemetry};
use tokio::time::{Duration, Instant};

/// Claims kept for tabs an open intent named before any listing showed them. Each waits for the
/// listing that shows its tab; one whose tab never shows (it closed first, or sits on an excluded
/// page) is evicted oldest first. A tab id never names another tab within one browser, so a stale
/// claim matches nothing, and a browser change drops them all.
const CLAIMS: usize = 64;

/// How far a held popup's chain of held openers is followed. Real openers form short chains; the
/// bound ends a loop a bad announcement could make (as `TabRegistry`'s own opener bound does).
const OPENER_HOPS: usize = 16;

/// Tab telemetry's shell: the ledger behind its own lock, and the facade it reports to.
///
/// Owned by `TabRegistry` (`TabRegistry::life`), so the tab watch, the janitor, the ops and
/// `NeoTab`, which all hold the registry, reach it with no new plumbing. Its lock is separate
/// from the registry's and is never held across an await or while telemetry runs: each call
/// takes the ledger's facts under the lock and sends them after it.
pub(crate) struct TabLife {
    ledger: Mutex<TabLedger>,
    telemetry: Telemetry,
}

impl TabLife {
    /// The ledger for `telemetry`, registered as its `neo_tabs_open` source. Built with the tab
    /// registry, before the browser link starts, so no tab event comes before it can count it.
    #[must_use]
    pub(crate) fn new(telemetry: Telemetry) -> Arc<Self> {
        let life = Arc::new(Self {
            ledger: Mutex::default(),
            telemetry,
        });
        // Weak: the telemetry handle the ledger holds keeps the source, which must not keep the
        // ledger alive in turn.
        let source = Arc::downgrade(&life);
        life.telemetry
            .set_open_tabs(Arc::new(move || open_counts_of(&source)));
        life
    }

    /// The live open tabs by birth origin: a count of the ledger's own map, never a running sum.
    #[must_use]
    pub(crate) fn open_counts(&self) -> OpenTabCounts {
        self.lock().open_counts()
    }

    /// The tab watch settled `listing`: births, closes, and a browser change on a new socket.
    pub(crate) fn listed(&self, listing: Listing<'_>) {
        let facts = self.lock().listed(listing);
        self.emit(facts);
    }

    /// The tab watch read `Target.targetCrashed` for `target`, the page target it maps to `tab`:
    /// one crash, whether or not an op was using the tab. Nothing else in Neo counts crashes, so
    /// an op that fails on a crashed tab adds none.
    pub(crate) fn crashed(&self, tab: TabId, target: &TargetId) {
        let facts = self.lock().crashed(tab, target);
        self.emit(facts);
    }

    /// The tab watch read `Target.targetInfoChanged` for `target` after it crashed: a page loaded
    /// in it again, so its renderer is back and a later close is not the crash's.
    pub(crate) fn recovered(&self, tab: TabId, target: &TargetId) {
        self.lock().recovered(tab, target);
    }

    /// Before Neo asks the browser to close `tab` (see [`CloseIntent`]).
    pub(crate) fn intend_close(self: &Arc<Self>, tab: TabId, reason: IntentReason) -> CloseIntent {
        let id = self.lock().intend_close(tab, reason);
        CloseIntent {
            life: Arc::clone(self),
            tab,
            id,
        }
    }

    /// Before Neo asks the browser to open a tab for `open` (see [`OpenIntent`]).
    pub(crate) fn expect_open(self: &Arc<Self>, open: OpenFor) -> OpenIntent {
        let id = self.lock().expect_open(open);
        OpenIntent {
            life: Arc::clone(self),
            id: Some(id),
        }
    }

    fn emit(&self, facts: Vec<TabFact>) {
        for fact in facts {
            match fact {
                TabFact::Opened { origin } => self.telemetry.tab_opened(origin),
                TabFact::Closed {
                    origin,
                    reason,
                    lifetime,
                } => self.telemetry.tab_closed(origin, reason, lifetime),
                TabFact::RendererCrashed => self.telemetry.renderer_crashed(),
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, TabLedger> {
        self.ledger.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The gauge's read, on whichever thread collects (a scrape's handler, the exporter's thread).
/// It cannot panic, so it needs no `catch_unwind` (decision D9, STYLE.md G16): a weak upgrade, a
/// lock whose poison is ignored as everywhere in the registry, and a saturating count. A ledger
/// already dropped reads as no tabs.
fn open_counts_of(life: &Weak<TabLife>) -> OpenTabCounts {
    life.upgrade()
        .map_or_else(OpenTabCounts::default, |life| life.open_counts())
}

/// A close Neo is about to ask the browser for. Taken before the close is sent, since the tab
/// watch may confirm the close before the op that asked for it answers.
///
/// It stays on the tab until the tab closes or its browser goes. The earliest live intent on a
/// tab names its close, so the janitor's, taken before it submits `page.close`, outranks that
/// op's own `script` one. Dropping the token is not a withdrawal: an op cancelled or timed out
/// after its send may still close the tab, and that close stays the one Neo asked for.
#[must_use = "bind it until the close settles; only `withdraw` takes it back"]
pub(crate) struct CloseIntent {
    life: Arc<TabLife>,
    tab: TabId,
    id: IntentId,
}

impl CloseIntent {
    /// The close definitely never reached the browser (refused, or failed before its send): the
    /// tab stays open, and its later close belongs to whoever makes it.
    pub(crate) fn withdraw(self) {
        self.life.lock().withdraw(self.tab, self.id);
    }
}

/// A tab Neo is about to open, for an agent or for itself. Taken before the browser is asked:
/// the tab watch usually lists the new tab before the open returns, and while an intent is
/// outstanding a new tab with no opener is held, counted nowhere, until an intent claims it or
/// none is left, when it is born `found`. Dropped unclaimed, it gives up its hold.
#[must_use = "claim the tab the open returned; dropped unclaimed, held tabs are born `found`"]
pub(crate) struct OpenIntent {
    life: Arc<TabLife>,
    /// `None` once claimed, so the drop does not undo what the claim settled.
    id: Option<IntentId>,
}

impl OpenIntent {
    /// The browser named `tab` as the tab this open made: it is born with this intent's origin,
    /// now if a listing already showed it, else at the first listing that does.
    pub(crate) fn claim(mut self, tab: TabId) {
        if let Some(id) = self.id.take() {
            let facts = self.life.lock().claim(id, tab, Instant::now());
            self.life.emit(facts);
        }
    }
}

impl Drop for OpenIntent {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let facts = self.life.lock().cancel_open(id);
            self.life.emit(facts);
        }
    }
}

/// Why Neo asks for a close: the only two closes it intends. `user`, `crash` and `shutdown` are
/// never intended; the ledger derives them ([`close_reason`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntentReason {
    /// A script's `page.close`, directly, through `task.close`, or as `page.runBeforeUnload`.
    Script,
    /// The idle-tab janitor's sweep.
    Janitor,
}

impl IntentReason {
    const fn close_reason(self) -> CloseReason {
        match self {
            Self::Script => CloseReason::Script,
            Self::Janitor => CloseReason::Janitor,
        }
    }
}

/// Whom Neo opens a tab for: the origins an open intent gives. `found` is never intended; it is
/// what a tab no intent claims is born as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenFor {
    /// `context.newPage`: a script's, a task's, or a run's lazy default page.
    Agent,
    /// `NeoTab`'s blank tab.
    Neo,
}

impl OpenFor {
    const fn origin(self) -> TabOrigin {
        match self {
            Self::Agent => TabOrigin::Agent,
            Self::Neo => TabOrigin::Neo,
        }
    }
}

/// One tab a listing showed: its current page target, and its opener tab when the browser
/// announced it as a popup. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListedTab {
    pub(crate) tab: TabId,
    pub(crate) target: TargetId,
    pub(crate) opener: Option<TabId>,
}

/// A live page target the tab watch knows, and its tab. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabTarget {
    pub(crate) tab: TabId,
    pub(crate) target: TargetId,
}

/// One listing the tab watch settled, with what the watch knows besides. A plain data bag.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Listing<'a> {
    /// Every tab core's listing showed; core leaves out its excluded pages (`chrome-extension://`,
    /// `devtools://`).
    pub(crate) tabs: &'a [ListedTab],
    /// Tabs whose page target was destroyed since the previous listing (the watch's close
    /// candidates).
    pub(crate) destroyed: &'a [TabId],
    /// Every live page target the watch knows, listed or not: a tab on an excluded page has one
    /// though the listing leaves it out, so it is not closed. Empty while the watch's map may
    /// hold a target whose destroy it never read, which would keep a closed tab counted.
    pub(crate) targeted: &'a [TabTarget],
    /// What the watch may have missed before this listing.
    pub(crate) gap: Gap,
    /// When the listing settled.
    pub(crate) now: Instant,
}

/// What the tab watch may have missed before a listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gap {
    /// Same socket, every target event read: an unlisted tab closed only if its page target was
    /// destroyed and no other target of it is alive.
    None,
    /// The first listing on a new socket (the process's first, a reconnect, a restarted browser):
    /// closes before it went unseen, so an unlisted tab with no live target closed. If no listed
    /// or announced (tab, target) is one the ledger knows, the browser is another one.
    Socket,
    /// Core's event channel dropped events since the previous listing: a destroy may be among
    /// them, so an unlisted tab with no live target closed.
    Lagged,
}

/// What the ledger decided, for the shell to send to telemetry once its lock is released.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabFact {
    /// A tab was born (`neo_tabs_opened_total`).
    Opened { origin: TabOrigin },
    /// A tab closed, once (`neo_tabs_closed_total`, `neo_tab_lifetime_seconds`).
    Closed {
        origin: TabOrigin,
        reason: CloseReason,
        lifetime: Duration,
    },
    /// A page target's renderer crashed (`neo_renderer_crashes_total`).
    RendererCrashed,
}

/// The ledger proper: pure, with no clock (callers pass `now`), no I/O and no telemetry. Every
/// input returns the facts it decided, so its tests feed browser facts in and read facts out.
#[derive(Default)]
struct TabLedger {
    /// Born tabs, each listed at least once in this browser, counted under its birth origin.
    open: HashMap<TabId, Life>,
    /// Listed but not born yet, so counted nowhere: a new tab with no opener seen while an open
    /// intent was outstanding, or a popup whose opener is such a tab.
    unclaimed: HashMap<TabId, Unclaimed>,
    /// Tabs an open intent claimed before any listing showed them, oldest first.
    claimed: VecDeque<Claim>,
    /// Open intents outstanding, with the origin each gives.
    opens: HashMap<IntentId, TabOrigin>,
    /// Close intents by tab, oldest first. They leave with their tab or its browser.
    closes: HashMap<TabId, Vec<Intent>>,
    /// Tabs whose renderer crashed and was not seen back.
    crashes: HashMap<TabId, Crash>,
    next_intent: u64,
    /// Orders close intents and crashes, which [`close_reason`] compares.
    step: u64,
}

/// A born tab.
struct Life {
    /// Frozen at birth (decision D7): an adopted blank tab stays `found`, a Neo tab the owner
    /// browsed in stays `neo`. The registry's own origin is janitor and sight policy.
    origin: TabOrigin,
    opened_at: Instant,
    /// The last listing that showed it, or that found its page target alive while leaving it
    /// out. A close found only after a gap, or with its browser, ends its lifetime here.
    last_seen: Instant,
    /// Its page target at that listing: what proves a new socket reached the same browser.
    target: TargetId,
}

/// A listed tab not born yet.
struct Unclaimed {
    target: TargetId,
    first_seen: Instant,
    last_seen: Instant,
    /// The opener of a popup held until its opener is born; `None` for a tab held for the open
    /// intents.
    opener: Option<TabId>,
}

/// An open intent's claim on a tab no listing showed yet.
struct Claim {
    tab: TabId,
    origin: TabOrigin,
    at: Instant,
}

/// A close intent on a tab.
struct Intent {
    id: IntentId,
    reason: IntentReason,
    taken: Step,
}

/// A renderer crash not seen recovered.
struct Crash {
    /// The page target that crashed: its own info change, or the tab listed under another
    /// target, means the renderer is back.
    target: TargetId,
    at: Step,
}

/// A point in the ledger's one sequence of close intents and crashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Step(u64);

/// Names one intent within its ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct IntentId(u64);

/// How a born tab's close was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gone {
    /// The tab went while its browser stayed.
    Tab,
    /// Its browser went: it exited or restarted while Neo was not connected.
    Browser,
}

/// Where a closed tab's lifetime ends.
#[derive(Clone, Copy, Debug)]
enum Ended {
    /// At the listing that confirmed the close.
    At(Instant),
    /// At the last listing that saw it: it went unseen, during a gap.
    LastSeen,
}

impl TabLedger {
    /// Reconciles one listing: a browser change first (a new socket that reached another
    /// browser closes every known tab), then births before closes, as the watch orders them.
    fn listed(&mut self, listing: Listing<'_>) -> Vec<TabFact> {
        let mut facts = Vec::new();
        if listing.gap == Gap::Socket && self.browser_changed(listing) {
            self.lose_browser(&mut facts);
        }
        self.births(listing, &mut facts);
        self.closes(listing, &mut facts);
        self.resolve_unclaimed(&mut facts);
        facts
    }

    /// Whether a new socket reached another browser: no (tab, target) the ledger knows is listed
    /// or announced on it. Target ids are each browser's own GUIDs, so a restarted browser
    /// cannot match by chance; with nothing known there is nothing to lose either.
    fn browser_changed(&self, listing: Listing<'_>) -> bool {
        let shown = |tab: TabId, target: &TargetId| {
            listing
                .tabs
                .iter()
                .any(|listed| listed.tab == tab && listed.target == *target)
                || listing
                    .targeted
                    .iter()
                    .any(|alive| alive.tab == tab && alive.target == *target)
        };
        let mut known = self
            .open
            .iter()
            .map(|(tab, life)| (*tab, &life.target))
            .chain(
                self.unclaimed
                    .iter()
                    .map(|(tab, waiting)| (*tab, &waiting.target)),
            )
            .peekable();
        known.peek().is_some() && !known.any(|(tab, target)| shown(tab, target))
    }

    /// The browser went: every born tab closes at the last listing that saw it, and the held
    /// tabs, close intents and crashes keyed by its tab ids go, since the new browser numbers its
    /// tabs anew and a reused id must not inherit them. Claims stay: `NeoTab` opens its tab on the
    /// new socket's first news, so its claim on the new browser's tab can land before the listing
    /// that shows the browser changed. A claim left from the old browser matches only a tab id
    /// the new one reuses, and is evicted in time.
    fn lose_browser(&mut self, facts: &mut Vec<TabFact>) {
        let mut gone: Vec<TabId> = self.open.keys().copied().collect();
        gone.sort_unstable();
        for tab in gone {
            self.close(tab, Ended::LastSeen, Gone::Browser, facts);
        }
        self.unclaimed.clear();
        self.closes.clear();
        self.crashes.clear();
    }

    /// Every listed tab the ledger has not seen is born or held; known ones are seen again.
    fn births(&mut self, listing: Listing<'_>, facts: &mut Vec<TabFact>) {
        let mut fresh: Vec<&ListedTab> = Vec::new();
        let mut seen: HashSet<TabId> = HashSet::new();
        for listed in listing.tabs {
            if !seen.insert(listed.tab) {
                continue;
            }
            if let Some(life) = self.open.get_mut(&listed.tab) {
                life.last_seen = listing.now;
                life.target.clone_from(&listed.target);
            } else if let Some(waiting) = self.unclaimed.get_mut(&listed.tab) {
                waiting.last_seen = listing.now;
                waiting.target.clone_from(&listed.target);
            } else {
                fresh.push(listed);
            }
            // Listed under another target than the one that crashed: a new renderer has it.
            if self
                .crashes
                .get(&listed.tab)
                .is_some_and(|crash| crash.target != listed.target)
            {
                self.crashes.remove(&listed.tab);
            }
        }
        // An opener new in this listing is decided before its popups, whatever the order;
        // tabs whose openers only name each other are born `found`.
        while !fresh.is_empty() {
            let undecided: HashSet<TabId> = fresh.iter().map(|listed| listed.tab).collect();
            let before = fresh.len();
            fresh.retain(|listed| !self.birth(listed, &undecided, listing.now, facts));
            if fresh.len() == before {
                for listed in fresh {
                    let life = Life::found(listed, listing.now);
                    self.born(listed.tab, life, facts);
                }
                break;
            }
        }
    }

    /// Decides one tab the ledger has not seen: born now, or held. `false` while its opener is
    /// new in this same listing and not decided yet.
    fn birth(
        &mut self,
        listed: &ListedTab,
        undecided: &HashSet<TabId>,
        now: Instant,
        facts: &mut Vec<TabFact>,
    ) -> bool {
        if let Some(claim) = self.take_claim(listed.tab) {
            let life = Life {
                origin: claim.origin,
                opened_at: claim.at.min(now),
                last_seen: now,
                target: listed.target.clone(),
            };
            self.born(listed.tab, life, facts);
            return true;
        }
        match listed.opener.filter(|opener| *opener != listed.tab) {
            Some(opener) => {
                if let Some(origin) = self.open.get(&opener).map(|life| life.origin) {
                    let life = Life {
                        origin: popup_origin(origin),
                        ..Life::found(listed, now)
                    };
                    self.born(listed.tab, life, facts);
                } else if self.unclaimed.contains_key(&opener) {
                    self.hold(listed, Some(opener), now);
                } else if undecided.contains(&opener) {
                    return false;
                } else {
                    // Its opener closed first, or was never one of the listed tabs.
                    self.born(listed.tab, Life::found(listed, now), facts);
                }
            }
            None if self.opens.is_empty() => self.born(listed.tab, Life::found(listed, now), facts),
            None => self.hold(listed, None, now),
        }
        true
    }

    /// Holds a listed tab unborn until what it waits for is settled.
    fn hold(&mut self, listed: &ListedTab, opener: Option<TabId>, now: Instant) {
        self.unclaimed.insert(
            listed.tab,
            Unclaimed {
                target: listed.target.clone(),
                first_seen: now,
                last_seen: now,
                opener,
            },
        );
    }

    /// The only way into `open`, and it produces the tab's one `Opened`.
    fn born(&mut self, tab: TabId, life: Life, facts: &mut Vec<TabFact>) {
        if let Entry::Vacant(slot) = self.open.entry(tab) {
            facts.push(TabFact::Opened {
                origin: life.origin,
            });
            slot.insert(life);
        }
    }

    /// Closes what this listing shows gone. A tab is gone when the listing lacks it, no live
    /// page target of it is known (a tab on an excluded page has one), and, with no gap, its
    /// target was destroyed: an unlisted tab with no destroy only navigated somewhere core does
    /// not list.
    fn closes(&mut self, listing: Listing<'_>, facts: &mut Vec<TabFact>) {
        let listed: HashSet<TabId> = listing.tabs.iter().map(|listed| listed.tab).collect();
        let alive: HashSet<TabId> = listing
            .targeted
            .iter()
            .map(|alive| alive.tab)
            .filter(|tab| !listed.contains(tab))
            .collect();
        let destroyed: HashSet<TabId> = listing.destroyed.iter().copied().collect();
        let gone = |tab: &TabId| {
            !listed.contains(tab)
                && !alive.contains(tab)
                && (listing.gap != Gap::None || destroyed.contains(tab))
        };
        for tab in &alive {
            if let Some(life) = self.open.get_mut(tab) {
                life.last_seen = listing.now;
            }
        }
        let ended = match listing.gap {
            Gap::None => Ended::At(listing.now),
            Gap::Socket | Gap::Lagged => Ended::LastSeen,
        };
        let mut closing: Vec<TabId> = self.open.keys().copied().filter(|tab| gone(tab)).collect();
        closing.sort_unstable();
        for tab in closing {
            self.close(tab, ended, Gone::Tab, facts);
        }
        // Never born, so never closed: a held tab that went leaves no fact.
        self.unclaimed.retain(|tab, _| !gone(tab));
        // A destroyed tab that went before it was born leaves nothing behind.
        for tab in listing.destroyed.iter().filter(|tab| gone(tab)) {
            self.closes.remove(tab);
            self.crashes.remove(tab);
            self.claimed.retain(|claim| claim.tab != *tab);
        }
    }

    /// The only way out of `open`, and it produces the tab's one `Closed`, with its one reason.
    fn close(&mut self, tab: TabId, ended: Ended, gone: Gone, facts: &mut Vec<TabFact>) {
        let Some(life) = self.open.remove(&tab) else {
            return;
        };
        let crashed = self.crashes.remove(&tab).map(|crash| crash.at);
        let intents = self.closes.remove(&tab).unwrap_or_default();
        let end = match ended {
            Ended::At(now) => now,
            Ended::LastSeen => life.last_seen,
        };
        facts.push(TabFact::Closed {
            origin: life.origin,
            reason: close_reason(crashed, &intents, gone),
            lifetime: end.saturating_duration_since(life.opened_at),
        });
    }

    /// Births every held tab that no longer waits: a popup whose opener was born (or went), and,
    /// once no open intent is outstanding, every held tab with no opener, as `found`.
    fn resolve_unclaimed(&mut self, facts: &mut Vec<TabFact>) {
        loop {
            let mut ready: Vec<(TabId, TabOrigin)> = self
                .unclaimed
                .iter()
                .filter_map(|(tab, waiting)| Some((*tab, self.ready_origin(waiting)?)))
                .collect();
            if ready.is_empty() {
                return;
            }
            ready.sort_unstable_by_key(|(tab, _)| *tab);
            for (tab, origin) in ready {
                if let Some(waiting) = self.unclaimed.remove(&tab) {
                    let life = Life {
                        origin,
                        opened_at: waiting.first_seen,
                        last_seen: waiting.last_seen,
                        target: waiting.target,
                    };
                    self.born(tab, life, facts);
                }
            }
        }
    }

    /// The origin a held tab is born with now, or `None` while it still waits.
    fn ready_origin(&self, waiting: &Unclaimed) -> Option<TabOrigin> {
        let Some(opener) = waiting.opener else {
            return self.opens.is_empty().then_some(TabOrigin::Found);
        };
        if let Some(life) = self.open.get(&opener) {
            return Some(popup_origin(life.origin));
        }
        (!self.chain_waits(opener)).then_some(TabOrigin::Found)
    }

    /// Whether held tab `tab`'s chain of held openers still waits: it ends at a tab held for the
    /// open intents, or at a tab born this round, within [`OPENER_HOPS`]. A chain that ends at a
    /// tab that went, loops, or runs on waits for nothing.
    fn chain_waits(&self, mut tab: TabId) -> bool {
        for _ in 0..OPENER_HOPS {
            match self.unclaimed.get(&tab) {
                None => return self.open.contains_key(&tab),
                Some(Unclaimed { opener: None, .. }) => return true,
                Some(Unclaimed {
                    opener: Some(next), ..
                }) => tab = *next,
            }
        }
        false
    }

    /// One crash per event. The first crash since the renderer was last seen alive dates the
    /// tab's death, for [`close_reason`].
    fn crashed(&mut self, tab: TabId, target: &TargetId) -> Vec<TabFact> {
        let at = self.next_step();
        self.crashes.entry(tab).or_insert_with(|| Crash {
            target: target.clone(),
            at,
        });
        vec![TabFact::RendererCrashed]
    }

    fn recovered(&mut self, tab: TabId, target: &TargetId) {
        if self
            .crashes
            .get(&tab)
            .is_some_and(|crash| crash.target == *target)
        {
            self.crashes.remove(&tab);
        }
    }

    fn intend_close(&mut self, tab: TabId, reason: IntentReason) -> IntentId {
        let id = self.next_intent_id();
        let taken = self.next_step();
        self.closes
            .entry(tab)
            .or_default()
            .push(Intent { id, reason, taken });
        id
    }

    fn withdraw(&mut self, tab: TabId, id: IntentId) {
        if let Some(intents) = self.closes.get_mut(&tab) {
            intents.retain(|intent| intent.id != id);
            if intents.is_empty() {
                self.closes.remove(&tab);
            }
        }
    }

    fn expect_open(&mut self, open: OpenFor) -> IntentId {
        let id = self.next_intent_id();
        self.opens.insert(id, open.origin());
        id
    }

    /// Open intent `id` named `tab`. A held tab is born with the intent's origin (from when it
    /// was first listed); a tab no listing showed yet is born at the listing that does; a tab
    /// already born keeps the origin it was born with.
    fn claim(&mut self, id: IntentId, tab: TabId, now: Instant) -> Vec<TabFact> {
        let mut facts = Vec::new();
        let Some(origin) = self.opens.remove(&id) else {
            return facts;
        };
        if let Some(waiting) = self.unclaimed.remove(&tab) {
            let life = Life {
                origin,
                opened_at: waiting.first_seen,
                last_seen: waiting.last_seen,
                target: waiting.target,
            };
            self.born(tab, life, &mut facts);
        } else if !self.open.contains_key(&tab) {
            self.claimed.retain(|claim| claim.tab != tab);
            if self.claimed.len() >= CLAIMS {
                self.claimed.pop_front();
            }
            self.claimed.push_back(Claim {
                tab,
                origin,
                at: now,
            });
        }
        self.resolve_unclaimed(&mut facts);
        facts
    }

    /// Open intent `id` ended with no tab named: what it held may be born now.
    fn cancel_open(&mut self, id: IntentId) -> Vec<TabFact> {
        let mut facts = Vec::new();
        if self.opens.remove(&id).is_some() {
            self.resolve_unclaimed(&mut facts);
        }
        facts
    }

    fn open_counts(&self) -> OpenTabCounts {
        let mut counts = OpenTabCounts::default();
        for life in self.open.values() {
            let count = match life.origin {
                TabOrigin::Agent => &mut counts.agent,
                TabOrigin::Found => &mut counts.found,
                TabOrigin::Neo => &mut counts.neo,
            };
            *count = count.saturating_add(1);
        }
        counts
    }

    fn take_claim(&mut self, tab: TabId) -> Option<Claim> {
        let index = self.claimed.iter().position(|claim| claim.tab == tab)?;
        self.claimed.remove(index)
    }

    fn next_step(&mut self) -> Step {
        self.step = self.step.saturating_add(1);
        Step(self.step)
    }

    fn next_intent_id(&mut self) -> IntentId {
        self.next_intent = self.next_intent.saturating_add(1);
        IntentId(self.next_intent)
    }
}

impl Life {
    /// A tab born `found` now: open before Neo saw it, or opened by someone else.
    fn found(listed: &ListedTab, now: Instant) -> Self {
        Self {
            origin: TabOrigin::Found,
            opened_at: now,
            last_seen: now,
            target: listed.target.clone(),
        }
    }
}

/// A popup's birth origin from its opener's: an agent's tab's popup is the agent's (the settings
/// design, decision 3, as the registry marks it), since nothing else would close it; every other
/// popup is `found`, a popup of Neo's own tab included.
const fn popup_origin(opener: TabOrigin) -> TabOrigin {
    match opener {
        TabOrigin::Agent => TabOrigin::Agent,
        TabOrigin::Found | TabOrigin::Neo => TabOrigin::Found,
    }
}

/// The one rule for a close's reason (the owner's decisions D5 and D6, 2026-11-20):
///
/// 1. its renderer crashed and was not seen back → `crash`, unless the earliest live intent was
///    taken before the crash: that close was already under way when the renderer died;
/// 2. else the earliest live intent → `script` or `janitor`;
/// 3. else `shutdown` when its browser went (it exited or restarted), otherwise `user`.
///
/// Pure, so one table test covers every combination.
fn close_reason(crashed: Option<Step>, intents: &[Intent], gone: Gone) -> CloseReason {
    let earliest = intents.iter().min_by_key(|intent| intent.taken);
    match (crashed, earliest) {
        (Some(crashed), Some(intent)) if intent.taken < crashed => intent.reason.close_reason(),
        (Some(_), _) => CloseReason::Crash,
        (None, Some(intent)) => intent.reason.close_reason(),
        (None, None) => match gone {
            Gone::Browser => CloseReason::Shutdown,
            Gone::Tab => CloseReason::User,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use browseros_core::{TabId, TargetId};
    use neo_telemetry::label::{CloseReason, TabOrigin};
    use neo_telemetry::{OpenTabCounts, Telemetry, TelemetryConfig};
    use tokio::time::{Duration, Instant};

    use super::{
        Gap, Gone, Intent, IntentId, IntentReason, ListedTab, Listing, OpenFor, Step, TabFact,
        TabLedger, TabTarget, close_reason,
    };
    use crate::browser::TabRegistry;
    use crate::telemetry_rig::Scrape;
    use crate::test_rig::TestError;

    const ORIGINS: [TabOrigin; 3] = [TabOrigin::Agent, TabOrigin::Found, TabOrigin::Neo];

    /// A tab as listed under its first target, `target-<n>`, with no opener.
    fn tab(n: i64) -> ListedTab {
        ListedTab {
            tab: TabId(n),
            target: target(n),
            opener: None,
        }
    }

    /// A popup `n` the browser announced with `opener` as its opener.
    fn popup(n: i64, opener: i64) -> ListedTab {
        ListedTab {
            opener: Some(TabId(opener)),
            ..tab(n)
        }
    }

    /// Tab `n` listed under another page target than its first: Chromium swapped its contents,
    /// or a restarted browser reused its number.
    fn swapped(n: i64) -> ListedTab {
        ListedTab {
            target: TargetId(format!("target-{n}-new")),
            ..tab(n)
        }
    }

    fn target(n: i64) -> TargetId {
        TargetId(format!("target-{n}"))
    }

    fn alive(listed: &ListedTab) -> TabTarget {
        TabTarget {
            tab: listed.tab,
            target: listed.target.clone(),
        }
    }

    fn opened(origin: TabOrigin) -> TabFact {
        TabFact::Opened { origin }
    }

    fn closed(origin: TabOrigin, reason: CloseReason, seconds: u64) -> TabFact {
        TabFact::Closed {
            origin,
            reason,
            lifetime: Duration::from_secs(seconds),
        }
    }

    /// A ledger under test that tallies every fact it produces and, after every input, checks
    /// the counting rules: per origin `opened − closed == open`, never below zero, and no tab
    /// both born and held.
    struct Book {
        ledger: TabLedger,
        start: Instant,
        opened: HashMap<TabOrigin, u64>,
        closed: HashMap<TabOrigin, u64>,
        crashes: u64,
    }

    impl Book {
        fn new() -> Self {
            Self {
                ledger: TabLedger::default(),
                start: Instant::now(),
                opened: HashMap::new(),
                closed: HashMap::new(),
                crashes: 0,
            }
        }

        fn at(&self, seconds: u64) -> Instant {
            self.start + Duration::from_secs(seconds)
        }

        /// A listing at `seconds` with no target known beyond the listed ones.
        fn listed(
            &mut self,
            seconds: u64,
            gap: Gap,
            tabs: &[ListedTab],
            destroyed: &[i64],
        ) -> Vec<TabFact> {
            self.listed_with(seconds, gap, tabs, destroyed, &[])
        }

        fn listed_with(
            &mut self,
            seconds: u64,
            gap: Gap,
            tabs: &[ListedTab],
            destroyed: &[i64],
            targeted: &[TabTarget],
        ) -> Vec<TabFact> {
            let destroyed: Vec<TabId> = destroyed.iter().map(|tab| TabId(*tab)).collect();
            let now = self.at(seconds);
            let facts = self.ledger.listed(Listing {
                tabs,
                destroyed: &destroyed,
                targeted,
                gap,
                now,
            });
            for listed in tabs {
                assert!(
                    self.ledger.open.contains_key(&listed.tab)
                        || self.ledger.unclaimed.contains_key(&listed.tab),
                    "every listed tab is born or held after its listing"
                );
            }
            self.settle(facts)
        }

        fn crashed(&mut self, tab: i64) -> Vec<TabFact> {
            let facts = self.ledger.crashed(TabId(tab), &target(tab));
            self.settle(facts)
        }

        fn claim(&mut self, intent: IntentId, tab: i64, seconds: u64) -> Vec<TabFact> {
            let now = self.at(seconds);
            let facts = self.ledger.claim(intent, TabId(tab), now);
            self.settle(facts)
        }

        fn cancel(&mut self, intent: IntentId) -> Vec<TabFact> {
            let facts = self.ledger.cancel_open(intent);
            self.settle(facts)
        }

        fn counts(&self) -> OpenTabCounts {
            self.ledger.open_counts()
        }

        fn settle(&mut self, facts: Vec<TabFact>) -> Vec<TabFact> {
            for fact in &facts {
                match fact {
                    TabFact::Opened { origin } => *self.opened.entry(*origin).or_default() += 1,
                    TabFact::Closed { origin, .. } => *self.closed.entry(*origin).or_default() += 1,
                    TabFact::RendererCrashed => self.crashes += 1,
                }
            }
            let counts = self.ledger.open_counts();
            for origin in ORIGINS {
                let opened = self.opened.get(&origin).copied().unwrap_or_default();
                let closed = self.closed.get(&origin).copied().unwrap_or_default();
                assert!(
                    opened >= closed,
                    "a tab closes only once it was born, so closes never outnumber opens ({origin:?})"
                );
                assert_eq!(
                    opened - closed,
                    counts.of(origin),
                    "the gauge counts the same map the counters come from ({origin:?})"
                );
            }
            assert!(
                self.ledger
                    .unclaimed
                    .keys()
                    .all(|tab| !self.ledger.open.contains_key(tab)),
                "a held tab is not born yet"
            );
            facts
        }
    }

    #[test]
    fn tabs_open_before_neo_attached_count_once_as_found() {
        let mut book = Book::new();
        assert_eq!(
            book.listed(0, Gap::Socket, &[tab(1), tab(2)], &[]),
            [opened(TabOrigin::Found), opened(TabOrigin::Found)],
            "the first listing births every tab it shows"
        );
        assert_eq!(
            book.listed(5, Gap::None, &[tab(1), tab(2)], &[]),
            [],
            "seeing them again counts nothing"
        );
        assert_eq!(
            book.counts(),
            OpenTabCounts {
                found: 2,
                ..OpenTabCounts::default()
            },
            "they are in the live count from the first listing"
        );
    }

    #[test]
    fn a_reconnect_to_the_same_browser_counts_no_open_again() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2), tab(3)], &[]);
        book.listed(10, Gap::None, &[tab(1), tab(2), tab(3)], &[]);
        // Tab 3 closed while the socket was down; tab 4 opened meanwhile.
        assert_eq!(
            book.listed(60, Gap::Socket, &[tab(1), tab(2), tab(4)], &[]),
            [
                opened(TabOrigin::Found),
                closed(TabOrigin::Found, CloseReason::User, 10)
            ],
            "only the new tab is born; the one that went closes as of its last listing"
        );
        assert_eq!(
            book.counts(),
            OpenTabCounts {
                found: 3,
                ..OpenTabCounts::default()
            },
            "a reconnect neither re-counts nor skews the live count"
        );
    }

    #[test]
    fn a_tab_on_an_excluded_page_outlives_a_reconnect() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2)], &[]);
        // Tab 2 sits on a `chrome-extension://` page: the new socket's discovery announced its
        // target, though core's listing leaves it out.
        assert_eq!(
            book.listed_with(
                30,
                Gap::Socket,
                &[tab(1)],
                &[],
                &[alive(&tab(1)), alive(&tab(2))]
            ),
            [],
            "a tab with a live target is open whatever core lists"
        );
        assert_eq!(book.counts().found, 2, "both still count");
    }

    #[test]
    fn another_browser_closes_every_known_tab_as_shutdown_and_births_its_own() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2)], &[]);
        book.listed(20, Gap::None, &[tab(1), tab(2)], &[]);
        // The browser restarted while Neo was away: new targets, and it reused tab number 1.
        assert_eq!(
            book.listed(90, Gap::Socket, &[swapped(1), tab(7)], &[]),
            [
                closed(TabOrigin::Found, CloseReason::Shutdown, 20),
                closed(TabOrigin::Found, CloseReason::Shutdown, 20),
                opened(TabOrigin::Found),
                opened(TabOrigin::Found),
            ],
            "the old browser's tabs close at their last listing; the new one's are born"
        );
        assert_eq!(book.counts().found, 2, "only the new browser's tabs count");
    }

    #[test]
    fn neos_tab_claimed_on_a_new_socket_before_its_listing_survives_a_browser_change() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2)], &[]);
        // The browser restarted. On the new socket's first news NeoTab opens its blank tab, and
        // the browser names it before the watch's first listing there settles.
        let neo = book.ledger.expect_open(OpenFor::Neo);
        assert_eq!(book.claim(neo, 9, 30), [], "not listed yet");
        assert_eq!(
            book.listed(31, Gap::Socket, &[tab(8), tab(9)], &[]),
            [
                closed(TabOrigin::Found, CloseReason::Shutdown, 0),
                closed(TabOrigin::Found, CloseReason::Shutdown, 0),
                opened(TabOrigin::Found),
                opened(TabOrigin::Neo),
            ],
            "the old browser's tabs close; Neo's new tab is born Neo all the same"
        );
    }

    #[test]
    fn a_reused_tab_id_inherits_no_intent_and_no_crash_from_the_old_browser() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2)], &[]);
        let _intent = book.ledger.intend_close(TabId(2), IntentReason::Script);
        book.crashed(1);
        assert_eq!(
            book.listed(30, Gap::Socket, &[swapped(1), swapped(2)], &[]),
            [
                closed(TabOrigin::Found, CloseReason::Crash, 0),
                closed(TabOrigin::Found, CloseReason::Script, 0),
                opened(TabOrigin::Found),
                opened(TabOrigin::Found),
            ],
            "an unrecovered crash and an intent name the old tabs' closes"
        );
        assert_eq!(
            book.listed(40, Gap::None, &[swapped(1)], &[2]),
            [closed(TabOrigin::Found, CloseReason::User, 10)],
            "the new tab 2 closes as nobody asked: the old intent went with its browser"
        );
        assert!(
            book.ledger.crashes.is_empty() && book.ledger.closes.is_empty(),
            "nothing keyed by the old browser's tab ids survives"
        );
    }

    #[test]
    fn agent_neo_and_popup_tabs_get_their_frozen_birth_origin() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1)], &[]);
        // context.newPage: the watch lists the tab before the open returns.
        let agent = book.ledger.expect_open(OpenFor::Agent);
        assert_eq!(
            book.listed(1, Gap::None, &[tab(1), tab(2)], &[]),
            [],
            "a new tab with no opener waits while an open is in flight"
        );
        assert_eq!(
            book.claim(agent, 2, 2),
            [opened(TabOrigin::Agent)],
            "the open names it: the agent's"
        );
        // NeoTab: the open returns before any listing shows the tab.
        let neo = book.ledger.expect_open(OpenFor::Neo);
        assert_eq!(
            book.claim(neo, 3, 3),
            [],
            "not listed yet: born at its listing"
        );
        assert_eq!(
            book.listed(4, Gap::None, &[tab(1), tab(2), tab(3)], &[]),
            [opened(TabOrigin::Neo)],
            "Neo's blank tab"
        );
        assert_eq!(
            book.listed(
                5,
                Gap::None,
                &[
                    tab(1),
                    tab(2),
                    tab(3),
                    popup(4, 2),
                    popup(5, 1),
                    popup(6, 3),
                    popup(7, 4)
                ],
                &[]
            ),
            [
                opened(TabOrigin::Agent),
                opened(TabOrigin::Found),
                opened(TabOrigin::Found),
                opened(TabOrigin::Agent),
            ],
            "an agent tab's popups, through popups of popups, are the agent's; others are found"
        );
        // An open intent naming a tab already born changes nothing: an adopted blank tab stays
        // `found`.
        let adopting = book.ledger.expect_open(OpenFor::Neo);
        assert_eq!(
            book.claim(adopting, 1, 6),
            [],
            "a born tab keeps its origin"
        );
        assert_eq!(
            book.counts(),
            OpenTabCounts {
                agent: 3,
                found: 3,
                neo: 1
            },
            "each tab counts under the origin it was born with"
        );
    }

    #[test]
    fn a_popup_listed_before_its_opener_waits_for_it() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1)], &[]);
        let agent = book.ledger.expect_open(OpenFor::Agent);
        assert_eq!(
            book.listed(1, Gap::None, &[tab(1), popup(3, 2), tab(2)], &[]),
            [],
            "the opener waits for the open, and its popup for the opener"
        );
        assert_eq!(
            book.claim(agent, 2, 2),
            [opened(TabOrigin::Agent), opened(TabOrigin::Agent)],
            "once the agent's tab is born, its popup follows"
        );
    }

    #[test]
    fn a_tab_held_for_an_open_that_names_another_is_found() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[], &[]);
        let agent = book.ledger.expect_open(OpenFor::Agent);
        book.listed(1, Gap::None, &[tab(1), tab(2)], &[]);
        assert_eq!(
            book.claim(agent, 2, 3),
            [opened(TabOrigin::Agent), opened(TabOrigin::Found)],
            "the owner's tab, opened meanwhile, is born found once no open is in flight"
        );
        let failed = book.ledger.expect_open(OpenFor::Agent);
        book.listed(4, Gap::None, &[tab(1), tab(2), tab(3)], &[]);
        assert_eq!(
            book.cancel(failed),
            [opened(TabOrigin::Found)],
            "an open that named nothing gives up what it held"
        );
        assert_eq!(
            book.listed(9, Gap::None, &[tab(1), tab(2)], &[3]),
            [closed(TabOrigin::Found, CloseReason::User, 5)],
            "a held tab's lifetime runs from its first listing"
        );
    }

    #[test]
    fn a_tab_that_opens_and_closes_while_held_is_never_counted() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[], &[]);
        let agent = book.ledger.expect_open(OpenFor::Agent);
        book.listed(1, Gap::None, &[tab(1)], &[]);
        assert_eq!(
            book.listed(2, Gap::None, &[], &[1]),
            [],
            "never born, never closed"
        );
        assert_eq!(book.cancel(agent), [], "nothing left to give up");
        assert_eq!(book.counts(), OpenTabCounts::default(), "nothing open");
    }

    #[test]
    fn script_janitor_and_user_closes_each_close_once_with_their_reason() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2), tab(3)], &[]);
        let _script = book.ledger.intend_close(TabId(1), IntentReason::Script);
        // The janitor takes its intent before it submits `page.close`, whose wrapper takes the
        // op's own `script` one: the earliest names the close.
        let _janitor = book.ledger.intend_close(TabId(2), IntentReason::Janitor);
        let _op = book.ledger.intend_close(TabId(2), IntentReason::Script);
        assert_eq!(
            book.listed(30, Gap::None, &[], &[1, 2, 3]),
            [
                closed(TabOrigin::Found, CloseReason::Script, 30),
                closed(TabOrigin::Found, CloseReason::Janitor, 30),
                closed(TabOrigin::Found, CloseReason::User, 30),
            ],
            "one close each, with the reason Neo intended, else the user's"
        );
        assert_eq!(
            book.listed(31, Gap::None, &[], &[1, 2, 3]),
            [],
            "a second confirmation finds no entry"
        );
        assert!(
            book.ledger.closes.is_empty(),
            "the intents left with their tabs"
        );
    }

    #[test]
    fn a_withdrawn_intent_leaves_the_close_to_whoever_makes_it() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1)], &[]);
        let refused = book.ledger.intend_close(TabId(1), IntentReason::Janitor);
        book.ledger.withdraw(TabId(1), refused);
        assert_eq!(
            book.listed(7, Gap::None, &[], &[1]),
            [closed(TabOrigin::Found, CloseReason::User, 7)],
            "a close Neo never sent is not Neo's"
        );
    }

    #[test]
    fn a_crash_counts_once_on_an_active_and_on_an_idle_tab() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2)], &[]);
        // Tab 1 is in use by a run, tab 2 idle: the ledger is told by the watch either way.
        assert_eq!(
            book.crashed(1),
            [TabFact::RendererCrashed],
            "the active tab's"
        );
        assert_eq!(
            book.crashed(2),
            [TabFact::RendererCrashed],
            "the idle tab's"
        );
        assert_eq!(book.crashes, 2, "one per crashed target");
        assert_eq!(
            book.listed(3, Gap::None, &[tab(1), tab(2)], &[]),
            [],
            "a crashed tab stays open until it closes"
        );
    }

    #[test]
    fn a_crashed_tab_closes_as_crash_exactly_once_whoever_removes_it() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2), tab(3)], &[]);
        let _before = book.ledger.intend_close(TabId(3), IntentReason::Script);
        book.crashed(1);
        book.crashed(2);
        book.crashed(3);
        // A script closes crashed tab 2 after the crash.
        let _after = book.ledger.intend_close(TabId(2), IntentReason::Script);
        assert_eq!(
            book.listed(4, Gap::None, &[], &[1, 2, 3]),
            [
                closed(TabOrigin::Found, CloseReason::Crash, 4),
                closed(TabOrigin::Found, CloseReason::Crash, 4),
                closed(TabOrigin::Found, CloseReason::Script, 4),
            ],
            "unrecovered crashes close as crash, unless the close was under way before the crash"
        );
        assert_eq!(
            book.listed(5, Gap::None, &[], &[1, 2, 3]),
            [],
            "a later targetDestroyed finds nothing to close"
        );
    }

    #[test]
    fn a_recovered_tab_closes_for_its_own_reason() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2)], &[]);
        book.crashed(1);
        book.ledger.recovered(TabId(1), &target(1));
        book.crashed(2);
        // Chromium gave tab 2 a new renderer under a new target.
        book.listed(2, Gap::None, &[tab(1), swapped(2)], &[2]);
        assert_eq!(
            book.listed(3, Gap::None, &[], &[1, 2]),
            [
                closed(TabOrigin::Found, CloseReason::User, 3),
                closed(TabOrigin::Found, CloseReason::User, 3),
            ],
            "a renderer seen back clears the crash"
        );
    }

    #[test]
    fn a_navigation_to_an_excluded_page_is_not_a_close() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2)], &[]);
        assert_eq!(
            book.listed(1, Gap::None, &[tab(1)], &[]),
            [],
            "unlisted with no destroy: it only went where core does not list"
        );
        // Tab 2 comes back; then Chromium swaps it to a new target on an excluded page.
        book.listed(2, Gap::None, &[tab(1), tab(2)], &[]);
        assert_eq!(
            book.listed_with(
                3,
                Gap::None,
                &[tab(1)],
                &[2],
                &[alive(&tab(1)), alive(&swapped(2))]
            ),
            [],
            "destroyed and unlisted, but another target of it is alive"
        );
        assert_eq!(
            book.listed(9, Gap::None, &[tab(1)], &[2]),
            [closed(TabOrigin::Found, CloseReason::User, 9)],
            "it closes once no target of it is left"
        );
    }

    #[test]
    fn after_events_were_dropped_an_unlisted_tab_is_closed() {
        let mut book = Book::new();
        book.listed(0, Gap::Socket, &[tab(1), tab(2)], &[]);
        book.listed(4, Gap::None, &[tab(1), tab(2)], &[]);
        assert_eq!(
            book.listed(10, Gap::Lagged, &[tab(1)], &[]),
            [closed(TabOrigin::Found, CloseReason::User, 4)],
            "its destroy may be among the dropped events; it ends at its last listing"
        );
    }

    #[test]
    fn the_close_reason_rule_covers_every_combination() {
        let intent = |reason, taken| Intent {
            id: IntentId(taken),
            reason,
            taken: Step(taken),
        };
        let cases = [
            (None, vec![], Gone::Tab, CloseReason::User),
            (None, vec![], Gone::Browser, CloseReason::Shutdown),
            (
                None,
                vec![intent(IntentReason::Script, 1)],
                Gone::Tab,
                CloseReason::Script,
            ),
            (
                None,
                vec![intent(IntentReason::Script, 1)],
                Gone::Browser,
                CloseReason::Script,
            ),
            (
                None,
                vec![
                    intent(IntentReason::Janitor, 1),
                    intent(IntentReason::Script, 2),
                ],
                Gone::Tab,
                CloseReason::Janitor,
            ),
            (Some(Step(5)), vec![], Gone::Tab, CloseReason::Crash),
            (Some(Step(5)), vec![], Gone::Browser, CloseReason::Crash),
            (
                Some(Step(5)),
                vec![intent(IntentReason::Script, 6)],
                Gone::Tab,
                CloseReason::Crash,
            ),
            (
                Some(Step(5)),
                vec![intent(IntentReason::Janitor, 4)],
                Gone::Tab,
                CloseReason::Janitor,
            ),
            (
                Some(Step(5)),
                vec![
                    intent(IntentReason::Script, 7),
                    intent(IntentReason::Janitor, 3),
                ],
                Gone::Browser,
                CloseReason::Janitor,
            ),
        ];
        for (crashed, intents, gone, reason) in cases {
            assert_eq!(
                close_reason(crashed, &intents, gone),
                reason,
                "crash {crashed:?}, intents {:?}, {gone:?}",
                intents
                    .iter()
                    .map(|intent| (intent.reason, intent.taken))
                    .collect::<Vec<_>>()
            );
        }
    }

    /// A deterministic generator for the property test: xorshift64*.
    struct Dice(u64);

    impl Dice {
        fn roll(&mut self, sides: u64) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) % sides
        }

        fn tab(&mut self) -> i64 {
            i64::try_from(self.roll(8)).unwrap_or_default() + 1
        }

        /// Tab `n` under one of its two targets, a popup of a random tab one time in four.
        fn listed(&mut self, n: i64) -> ListedTab {
            let listed = match self.roll(4) {
                0 => swapped(n),
                _ => tab(n),
            };
            match self.roll(4) {
                0 => ListedTab {
                    opener: Some(TabId(self.tab())),
                    ..listed
                },
                _ => listed,
            }
        }
    }

    /// Any order of any inputs keeps the counting rules (checked by `Book::settle` after every
    /// step): births, closes, gaps, browser changes, crashes, recoveries, intents, claims.
    #[test]
    fn the_counting_rules_hold_after_every_input_in_any_order() {
        for seed in 1..=300_u64 {
            let mut dice = Dice(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
            let mut book = Book::new();
            let mut opens: Vec<IntentId> = Vec::new();
            let mut closes: Vec<(TabId, IntentId)> = Vec::new();
            for step in 0..200_u64 {
                match dice.roll(8) {
                    0..=2 => {
                        let shown: HashSet<i64> = (0..dice.roll(6)).map(|_| dice.tab()).collect();
                        let mut tabs: Vec<ListedTab> =
                            shown.iter().map(|tab| dice.listed(*tab)).collect();
                        tabs.sort_by_key(|listed| listed.tab);
                        let destroyed: Vec<i64> = (0..dice.roll(4)).map(|_| dice.tab()).collect();
                        let targeted: Vec<TabTarget> = (0..dice.roll(3))
                            .map(|_| {
                                let n = dice.tab();
                                alive(&dice.listed(n))
                            })
                            .collect();
                        let gap = match dice.roll(6) {
                            0 => Gap::Socket,
                            1 => Gap::Lagged,
                            _ => Gap::None,
                        };
                        book.listed_with(step, gap, &tabs, &destroyed, &targeted);
                    }
                    3 => {
                        let tab = dice.tab();
                        let facts = book.crashed(tab);
                        assert_eq!(facts, [TabFact::RendererCrashed], "one crash per event");
                    }
                    4 => {
                        let tab = dice.tab();
                        book.ledger.recovered(TabId(tab), &target(tab));
                    }
                    5 => {
                        let tab = TabId(dice.tab());
                        let reason = if dice.roll(2) == 0 {
                            IntentReason::Script
                        } else {
                            IntentReason::Janitor
                        };
                        closes.push((tab, book.ledger.intend_close(tab, reason)));
                        if dice.roll(3) == 0
                            && let Some((tab, intent)) = closes.pop()
                        {
                            book.ledger.withdraw(tab, intent);
                        }
                    }
                    6 => {
                        let open = if dice.roll(2) == 0 {
                            OpenFor::Agent
                        } else {
                            OpenFor::Neo
                        };
                        opens.push(book.ledger.expect_open(open));
                    }
                    _ => {
                        if let Some(intent) = opens.pop() {
                            if dice.roll(2) == 0 {
                                let tab = dice.tab();
                                book.claim(intent, tab, step);
                            } else {
                                book.cancel(intent);
                            }
                        }
                    }
                }
            }
            for intent in opens {
                book.cancel(intent);
            }
            assert!(
                book.ledger
                    .unclaimed
                    .values()
                    .all(|waiting| waiting.opener.is_some()),
                "with no open in flight no tab waits for one (seed {seed})"
            );
        }
    }

    /// The shell over the live pipeline (`Telemetry::in_memory`), through the registry that owns
    /// it, read as the host API's scrape reads it: the registry registers the ledger as the
    /// gauge's source, `neo_tabs_open` is the ledger's own map at every scrape, and each fact
    /// becomes exactly one count, once the ledger's lock is released.
    #[test]
    fn the_ledger_s_facts_are_the_live_tab_metrics() -> Result<(), TestError> {
        let (telemetry, _probe) = Telemetry::in_memory(&TelemetryConfig::default());
        let tabs = TabRegistry::new_with_telemetry(telemetry.clone());
        let life = tabs.life();
        assert_eq!(
            Scrape::of(&telemetry)?.whole("neo_tabs_open", &[("origin", "found")]),
            Some(0),
            "the registry registers its ledger as the gauge's source"
        );
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        life.listed(Listing {
            tabs: &[tab(1), tab(2)],
            destroyed: &[],
            targeted: &[],
            gap: Gap::Socket,
            now: at(0),
        });
        let open = life.expect_open(OpenFor::Agent);
        life.listed(Listing {
            tabs: &[tab(1), tab(2), tab(3)],
            destroyed: &[],
            targeted: &[],
            gap: Gap::None,
            now: at(1),
        });
        open.claim(TabId(3));
        assert_eq!(
            Scrape::of(&telemetry)?.open_tabs(),
            (1, 2, 0),
            "the gauge reads the ledger now: (agent, found, neo)"
        );
        life.crashed(TabId(2), &target(2));
        let _close = life.intend_close(TabId(3), IntentReason::Script);
        life.listed(Listing {
            tabs: &[tab(1)],
            destroyed: &[TabId(2), TabId(3)],
            targeted: &[],
            gap: Gap::None,
            now: at(5),
        });

        let scrape = Scrape::of(&telemetry)?;
        assert_eq!(scrape.open_tabs(), (0, 1, 0), "one tab left open");
        assert_eq!(
            scrape.lines_of(&[
                "neo_tabs_opened_total",
                "neo_tabs_closed_total",
                "neo_tab_lifetime_seconds_sum",
                "neo_renderer_crashes_total",
            ]),
            [
                "neo_renderer_crashes_total 1",
                r#"neo_tab_lifetime_seconds_sum{close_reason="crash",origin="found"} 5"#,
                r#"neo_tab_lifetime_seconds_sum{close_reason="script",origin="agent"} 4"#,
                r#"neo_tabs_closed_total{close_reason="crash",origin="found"} 1"#,
                r#"neo_tabs_closed_total{close_reason="script",origin="agent"} 1"#,
                r#"neo_tabs_opened_total{origin="agent"} 1"#,
                r#"neo_tabs_opened_total{origin="found"} 2"#,
            ],
            "each fact counted once: births by frozen origin, one close each with its one \
             reason and lifetime, one crash"
        );
        Ok(())
    }
}
