//! Keeps a tab, or a task's current tab, in front of Neo's one screen while a view display
//! watches (D12's follow). It replaces zuzu's `watch`, `follow` and `toFront` and the observer
//! task they ran in: Neo already sees every operation, so a host only names what to follow.
//!
//! ```text
//!  admission ─ subscribe: T's ops, Neo's moves ─ look up the target ─ bring it forward
//!  followTab=N ──────────────────────────────────────┐
//!  followTask=T ─ TraceStore::task_last_act(T), then ├─▶ FollowMemory ─▶ next_activation
//!                 OperationStarted { task: T, tab }  │     (pure)            (pure)
//!  Neo's front moves, read before T's ops ───────────┤                         │ due
//!  1 s tick: the owner holds the screen? the tab     │      the front turn ◀───┘
//!            locked? the tab private? ───────────────┘      ─ decide again ─▶ Browser.activateTab
//! ```
//!
//! **It reads everything after it looks.** A follower subscribes to its task's operations and
//! to Neo's front moves as its view is admitted, before it looks up what it follows and brings
//! that forward (decision 41). The view's upgrade and the follower's first pass come later, and
//! whatever happens meanwhile waits in those subscriptions.
//!
//! **It never fights the owner.** It pauses while the owner holds the screen (the screen rule: a
//! takeover, a control display or a fit) and while a lock holds the followed tab: the owner may
//! be switching tabs on the same screen. A pause forgets what the follower showed and Neo's last
//! move, so when the pause ends it brings its tab back even if the screen moved meanwhile, and
//! its task's operations still retarget it while it is paused. A follower decides, takes the
//! front turn, then decides again before it activates, so a holder that began while it waited
//! keeps the front where it is.
//!
//! **It adopts and never undoes** (decision 47). Every activation Neo makes is a `FrontMove`
//! that every follower reads. A `followTask` follower adopts its own task's `bringToFront`; every
//! other move is seen and not undone. A follower brings its target back only on its own edges:
//! its admission (decision 41), a pause's end, and its task acting on a tab other than the one
//! Neo last put in front, in an operation that began after that move. An operation's start
//! reaches the follower only after the trace writer's commit, while a move arrives once it
//! lands, so an operation read after a move may have begun before it. That operation is old
//! news: acting on it would undo the move. Whenever both are waiting, the follower reads the
//! move first. An operation read before a newer move landed is old news by the time the
//! follower acts, too: a move it reads while it waits for the turn, and that began at or after
//! the operation, retires the edge. A follower that lost some of its task's operations (the
//! event stream lagged) re-reads the task's last act from the store, with when the task acted,
//! and judges it the same way. It marks its own activation seen while it still holds the
//! turn, so its own move never makes its task's operations old news. So no follow view undoes
//! the agent's `bringToFront`, and two followers never answer each other's moves. Op starts come
//! before the app's door decides, so a task follower shows work as it starts; it cannot carry a
//! refusal for the owner's screen or lock onto the screen, because it pauses on the same facts.
//!
//! **It never brings a private tab to the front** (decision 34). The views watching would show
//! its pixels until the front-tab watch closed them (front_tab.rs).

use crate::browser_tabs::{FrontMove, Mover};
use crate::limits::FOLLOW_TICK;
use crate::{Displays, Follow};
use browseros_core::TabId;
use neo_trace::{EpochMillis, TaskAct, TaskId, TraceRecordEvent};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};
use tokio::time::MissedTickBehavior;

/// What a follower knows at one moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FollowState {
    /// The tab to keep in front, once known.
    pub(crate) target: Option<TabId>,
    /// The owner holds the screen (a takeover, a control display or a fit), or a lock holds the
    /// target.
    pub(crate) paused: bool,
    /// The target is private.
    pub(crate) private: bool,
    /// The tab this follower brought to the front, and has not been paused since.
    pub(crate) shown: Option<TabId>,
}

/// The tab to bring to the front now: the target, unless the follower is paused, has no target
/// yet, the target is private, or the follower already showed it.
pub(crate) const fn next_activation(state: FollowState) -> Option<TabId> {
    match state {
        FollowState { paused: true, .. }
        | FollowState { private: true, .. }
        | FollowState { target: None, .. } => None,
        FollowState {
            target: Some(target),
            shown: Some(shown),
            ..
        } if target.0 == shown.0 => None,
        FollowState { target, .. } => target,
    }
}

/// What one follower remembers between its passes: the pure half of follow. Its transitions are
/// the follow rules of decision 47, so a table test states them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FollowMemory {
    /// The tab to keep in front, once known.
    target: Option<TabId>,
    /// The tab this follower brought or adopted to the front, and has not been paused since.
    /// `None` makes the target due.
    shown: Option<TabId>,
    /// Neo's last move of the front as this follower saw it, whoever made it. Never the owner's
    /// or Chromium's own moves, which Neo does not see, so a pause forgets it.
    front: Option<TabId>,
    /// When Neo made the last move this follower read ([`FrontMove::at`]). An operation of the
    /// task that began at or before it is old news. The follower's own moves leave it as it is,
    /// and a pause forgets it with `front`.
    moved_at: Option<EpochMillis>,
    /// When the task's operation that last set the target began. A move read later that began
    /// at or after it makes that operation old news too. `None` once a pause made the target
    /// due again: a pause's end is the follower's own edge.
    acted_at: Option<EpochMillis>,
}

impl FollowMemory {
    /// A follower starts from what its admission looked up and brought forward. It has read no
    /// move yet: the ones after its admission's look-up wait in its subscription.
    pub(crate) fn start(target: Option<TabId>, shown: Option<TabId>) -> Self {
        Self {
            target,
            shown,
            front: shown,
            moved_at: None,
            acted_at: None,
        }
    }

    /// The followed task started an operation on `tab` at `at`, or the task's last tab was read
    /// again after a lag or a newPage: it is the target. When Neo last put another tab in front,
    /// the follower brings `tab` back, even if it showed `tab` before: the task works there
    /// again.
    ///
    /// An operation that began at or before Neo's last move changes nothing. It reached the
    /// follower after that move only because the trace writer publishes an operation's start
    /// after its commit, and acting on it would undo the move.
    pub(crate) fn task_acted(&mut self, tab: TabId, at: EpochMillis) {
        if self.moved_at.is_some_and(|moved| at <= moved) {
            return;
        }
        self.target = Some(tab);
        self.shown = (self.front == Some(tab)).then_some(tab);
        self.acted_at = Some(at);
    }

    /// Neo moved the front. A task follower adopts its own task's move: the agent chose that tab
    /// for the task, and the view shows it. Every other move is only seen. Re-asserting would
    /// make `bringToFront` a no-op under any follow view, and two followers would ping-pong.
    ///
    /// A move that began at or after the operation that made the target due retires that edge.
    /// The follower read the operation first and is waiting for the turn this move held, or for
    /// its next pass; acting on the operation now would undo a newer move.
    pub(crate) fn front_moved(&mut self, follow: &Follow, moved: &FrontMove) {
        self.front = Some(moved.tab);
        self.moved_at = Some(moved.at);
        if let (Follow::Task(followed), Mover::Agent { task: Some(task) }) = (follow, &moved.by)
            && followed == task
        {
            self.target = Some(moved.tab);
            self.shown = Some(moved.tab);
        } else if self.acted_at.is_some_and(|acted| acted <= moved.at) {
            self.shown = self.target;
        }
    }

    /// The follower is paused. While the owner holds the screen or the tab, the front may move
    /// where Neo cannot see it, so the pause forgets what the follower showed and Neo's last
    /// move, its time included, and its end brings the target back. With no move left to
    /// protect, an operation of the task retargets the follower while it is paused, even one
    /// that began before a fit's own activation.
    pub(crate) fn paused(&mut self) {
        self.shown = None;
        self.front = None;
        self.moved_at = None;
        self.acted_at = None;
    }

    /// This follower brought `tab` forward. Its own move leaves `moved_at` as it is: it never
    /// makes its task's later operations old news.
    pub(crate) fn activated(&mut self, tab: TabId) {
        self.shown = Some(tab);
        self.front = Some(tab);
    }

    /// What the pure rule reads, with this pass's pause and privacy.
    pub(crate) fn state(&self, paused: bool, private: bool) -> FollowState {
        FollowState {
            target: self.target,
            paused,
            private,
            shown: self.shown,
        }
    }

    pub(crate) fn target(&self) -> Option<TabId> {
        self.target
    }
}

/// A view's follower as its admission leaves it, for [`run`] to go on from.
pub(crate) struct Follower {
    follow: Follow,
    /// What the admission looked up: the followed tab, or the task's last tab.
    target: Option<TabId>,
    /// The tab the admission brought forward, if it did.
    shown: Option<TabId>,
    /// The task's operations from just before the look-up; `None` for a tab follower.
    events: Option<broadcast::Receiver<TraceRecordEvent>>,
    /// Neo's front moves from just before the look-up, the admission's own marked seen.
    moves: watch::Receiver<Option<FrontMove>>,
}

/// Starts a follow view's follower as the view is admitted, before the view reads what is in
/// front. It subscribes to the task's operations and to Neo's front moves first, then looks up
/// what it follows and brings that forward (`bring_forward`), so [`run`] reads every operation
/// and every move after the look-up, however long the view's upgrade takes.
pub(crate) async fn admit(follow: Follow, displays: &Displays) -> Follower {
    let events = match follow {
        Follow::Task(_) => Some(displays.sink.subscribe()),
        Follow::Tab(_) => None,
    };
    let mut moves = displays.browser.front_moves();
    let target = match &follow {
        Follow::Tab(tab) => Some(*tab),
        Follow::Task(task) => last_act(displays, task).await.map(|act| act.tab),
    };
    let shown = bring_forward(target, displays, &mut moves).await;
    Follower {
        follow,
        target,
        shown,
        events,
        moves,
    }
}

/// Brings a follow view's target to the front as the view is admitted, before the view reads what
/// is in front, when the follower would bring it there: the tab is known and not private, the
/// owner does not hold the screen, it is not locked, and views are not held out (a takeover's
/// hold, or the agent's private switch). It decides, takes the front turn, then decides again
/// before it activates, so a holder that began while the task's tab was looked up or while
/// it waited for the turn keeps the front where it is. Over the agent's own move it still brings
/// its tab forward: the host's newer request wins (decision 48).
///
/// A private tab can be left in front with no view in sight: a control display's first input
/// makes every open tab private, the flag outlives the display, and once the tab the owner worked
/// in closes, the one behind it comes forward. Read first, that tab would refuse the view, its
/// follower would never start, and the screen would stay on it.
///
/// Returns the tab it brought forward, which the follower then starts from as shown. Its own
/// move is marked seen in `moves` while it holds the turn, so the follower never reads it.
async fn bring_forward(
    target: Option<TabId>,
    displays: &Displays,
    moves: &mut watch::Receiver<Option<FrontMove>>,
) -> Option<TabId> {
    let decide = || {
        next_activation(FollowState {
            target,
            paused: displays.screen_held()
                || displays.views_paused()
                || target.is_some_and(|tab| displays.locks.is_locked(tab)),
            private: target.is_some_and(|tab| displays.privacy.is_private(tab).is_some()),
            shown: None,
        })
    };
    decide()?;
    let turn = displays.browser.front_turn().await;
    let tab = decide()?;
    match turn.activate(tab, Mover::Follow).await {
        Ok(()) => {
            moves.mark_unchanged();
            Some(tab)
        }
        Err(error) => {
            tracing::debug!(tab = tab.0, %error, "a new view could not bring its tab to the front");
            None
        }
    }
}

/// The follow half of a view display's life, from where its admission left it. It runs until
/// the display drops it.
pub(crate) async fn run(follower: Follower, displays: &Displays) {
    let Follower {
        follow,
        target,
        shown,
        mut events,
        mut moves,
    } = follower;
    let mut memory = FollowMemory::start(target, shown);
    let mut tick = tokio::time::interval(FOLLOW_TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        if due(&mut memory, displays).is_some() {
            let turn = displays.browser.front_turn().await;
            // Decided again under the turn, with any move made while this waited: a holder that
            // began meanwhile keeps the front, and the task's own move is adopted, not undone.
            if moves.has_changed().unwrap_or(false) {
                see_last_move(&mut memory, &follow, &mut moves);
            }
            if let Some(tab) = due(&mut memory, displays) {
                match turn.activate(tab, Mover::Follow).await {
                    Ok(()) => {
                        // Under the turn the last move is this one, so marking it seen skips
                        // only the follower's own move, which never makes its task's
                        // operations old news.
                        moves.mark_unchanged();
                        memory.activated(tab);
                    }
                    Err(error) => {
                        tracing::debug!(tab = tab.0, %error, "follow could not bring its tab to the front");
                    }
                }
            }
        }
        tokio::select! {
            // A move is read before an operation that waits beside it: a move arrives once it
            // lands, an operation's start only after the writer's commit, so the operation is
            // judged against the move and not the other way round.
            biased;
            // The record lives in the displays this follower borrows, so it outlives the loop.
            Ok(()) = moves.changed() => see_last_move(&mut memory, &follow, &mut moves),
            event = next_event(&mut events) => {
                let Follow::Task(task) = &follow else {
                    continue;
                };
                match event {
                    Ok(TraceRecordEvent::OperationStarted {
                        task: Some(of),
                        tab: Some(tab),
                        at,
                        ..
                    }) if &of == task => memory.task_acted(tab, at),
                    // An operation with no target when it started: a newPage, whose tab exists
                    // only at its end, or one that never had a page (a context-level call, one
                    // refused before it had a page). The store's last act says which tab the
                    // task works in and when it acted there: a newPage's from its end, so the
                    // tab its task just opened comes forward; any other's from its own start,
                    // so this operation's end never makes an older tab newer than Neo's last
                    // move.
                    Ok(TraceRecordEvent::OperationEnded {
                        task: Some(of),
                        tab: None,
                        ..
                    }) if &of == task => {
                        if let Some(act) = last_act(displays, task).await {
                            memory.task_acted(act.tab, act.at);
                        }
                    }
                    Ok(_) => {}
                    // The operations it dropped are gone, and the store's last act carries when
                    // the task acted: one older than Neo's last move stays old news, as its
                    // event would have been, rather than undoing that move.
                    Err(RecvError::Lagged(_)) => {
                        if let Some(act) = last_act(displays, task).await {
                            memory.task_acted(act.tab, act.at);
                        }
                    }
                    Err(RecvError::Closed) => events = None,
                }
            }
            _ = tick.tick() => {}
        }
    }
}

/// The tab this follower would bring forward now, with this instant's pause and privacy. A pause
/// forgets what the follower showed.
fn due(memory: &mut FollowMemory, displays: &Displays) -> Option<TabId> {
    let target = memory.target();
    let paused = displays.screen_held() || target.is_some_and(|tab| displays.locks.is_locked(tab));
    if paused {
        memory.paused();
    }
    let private = target.is_some_and(|tab| displays.privacy.is_private(tab).is_some());
    next_activation(memory.state(paused, private))
}

/// Reads Neo's last front move into `memory`, and marks it seen.
fn see_last_move(
    memory: &mut FollowMemory,
    follow: &Follow,
    moves: &mut watch::Receiver<Option<FrontMove>>,
) {
    let moved = moves.borrow_and_update().clone();
    if let Some(moved) = moved {
        memory.front_moved(follow, &moved);
    }
}

/// The next trace event, or never once the stream closed (`None`).
pub(crate) async fn next_event(
    events: &mut Option<broadcast::Receiver<TraceRecordEvent>>,
) -> Result<TraceRecordEvent, RecvError> {
    match events {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    }
}

/// The tab the task's latest operation acted on, and when, as the store has it.
async fn last_act(displays: &Displays, task: &TaskId) -> Option<TaskAct> {
    match displays.store.task_last_act(task).await {
        Ok(act) => act,
        Err(error) => {
            tracing::warn!(%error, "follow could not read the task's latest tab");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FollowMemory, FollowState, next_activation};
    use crate::Follow;
    use crate::browser_tabs::{FrontMove, Mover};
    use browseros_core::TabId;
    use neo_trace::{EpochMillis, TaskId};

    const FIVE: Option<TabId> = Some(TabId(5));
    const SIX: Option<TabId> = Some(TabId(6));

    #[test]
    fn the_target_is_brought_to_the_front_once_unless_paused_or_private() {
        let cases = [
            (FIVE, false, false, None, FIVE, "a target not yet shown"),
            (FIVE, false, false, FIVE, None, "already shown"),
            (SIX, false, false, FIVE, SIX, "the task moved on"),
            (
                FIVE,
                true,
                false,
                None,
                None,
                "the owner holds the screen, or the tab is locked",
            ),
            (SIX, true, false, FIVE, None, "paused, even on a new tab"),
            (None, false, false, None, None, "no operation yet"),
            (FIVE, false, true, None, None, "a private target"),
            (SIX, false, true, FIVE, None, "moved to a private tab"),
        ];
        for (target, paused, private, shown, activate, case) in cases {
            assert_eq!(
                next_activation(FollowState {
                    target,
                    paused,
                    private,
                    shown
                }),
                activate,
                "{case}"
            );
        }
    }

    /// One change to a follower's memory. Times are milliseconds on the records' clock.
    enum Step {
        /// Neo brought this tab forward, for this mover, at this time.
        Moved(i64, Mover, i64),
        /// The followed task started an operation on this tab at this time.
        Acted(i64, i64),
        /// A pass found the follower paused.
        Paused,
    }

    /// Replays `steps` on a follower of `follow` that shows tab 3, its target. Returns what is
    /// due once nothing pauses it, and its target.
    fn after(follow: &Follow, steps: Vec<Step>) -> (Option<TabId>, Option<TabId>) {
        let mut memory = FollowMemory::start(Some(TabId(3)), Some(TabId(3)));
        for step in steps {
            match step {
                Step::Moved(tab, by, at) => memory.front_moved(
                    follow,
                    &FrontMove {
                        tab: TabId(tab),
                        by,
                        at: EpochMillis::from_millis(at),
                    },
                ),
                Step::Acted(tab, at) => memory.task_acted(TabId(tab), EpochMillis::from_millis(at)),
                Step::Paused => memory.paused(),
            }
        }
        (next_activation(memory.state(false, false)), memory.target())
    }

    fn agent(task: &TaskId) -> Mover {
        Mover::Agent {
            task: Some(task.clone()),
        }
    }

    #[test]
    fn a_follower_adopts_its_own_tasks_move_sees_others_and_returns_when_its_task_acts()
    -> Result<(), Box<dyn std::error::Error>> {
        let (t, u) = (TaskId::parse("checkout")?, TaskId::parse("search")?);
        let three = Some(TabId(3));
        let four = Some(TabId(4));
        let cases = [
            (
                Follow::Task(t.clone()),
                vec![Step::Moved(4, agent(&u), 20)],
                (None, three),
                "another task's move is seen, not undone",
            ),
            (
                Follow::Task(t.clone()),
                vec![Step::Moved(4, Mover::Agent { task: None }, 20)],
                (None, three),
                "a move with no task is another task's",
            ),
            (
                Follow::Task(t.clone()),
                vec![Step::Moved(4, agent(&t), 20)],
                (None, four),
                "its own task's move is adopted: 4 is the target, and shown",
            ),
            (
                Follow::Tab(TabId(3)),
                vec![Step::Moved(4, agent(&t), 20)],
                (None, three),
                "a tab follower adopts no task's move",
            ),
            (
                Follow::Task(u.clone()),
                vec![Step::Moved(4, agent(&t), 20)],
                (None, three),
                "a follower of another task does not adopt it",
            ),
            (
                Follow::Task(t.clone()),
                vec![Step::Moved(4, agent(&u), 20), Step::Acted(3, 30)],
                (three, three),
                "its task acts on 3 after Neo put 4 in front: 3 comes back",
            ),
            (
                Follow::Task(t.clone()),
                vec![Step::Moved(4, agent(&u), 20), Step::Acted(4, 30)],
                (None, four),
                "its task acts on 4, already in front: nothing to do",
            ),
            (
                Follow::Task(t.clone()),
                vec![Step::Moved(4, agent(&u), 20), Step::Acted(5, 10)],
                (None, three),
                "an op that began before the move, read after it, is old news: the target stays",
            ),
            (
                Follow::Task(t.clone()),
                vec![Step::Moved(4, agent(&u), 20), Step::Acted(5, 20)],
                (None, three),
                "an op in the move's own millisecond is old news too",
            ),
            (
                Follow::Task(t.clone()),
                vec![Step::Moved(4, agent(&t), 20), Step::Acted(3, 10)],
                (None, four),
                "its task's op from before its own move does not take the target back",
            ),
            (
                Follow::Tab(TabId(3)),
                vec![Step::Moved(4, agent(&u), 20), Step::Paused],
                (three, three),
                "a pause's end brings the target back",
            ),
            (
                Follow::Task(t.clone()),
                vec![
                    Step::Moved(4, agent(&u), 20),
                    Step::Paused,
                    Step::Acted(4, 30),
                ],
                (four, four),
                "after a pause, the front Neo last set is unknown: its task's tab comes back",
            ),
            (
                Follow::Task(t.clone()),
                vec![
                    Step::Moved(4, Mover::Fit, 20),
                    Step::Paused,
                    Step::Acted(5, 10),
                ],
                (Some(TabId(5)), Some(TabId(5))),
                "a pause forgets the move's time too: its task's ops still retarget it",
            ),
            (
                Follow::Task(t.clone()),
                vec![
                    Step::Moved(4, agent(&u), 20),
                    Step::Acted(5, 30),
                    Step::Moved(6, agent(&u), 40),
                ],
                (None, Some(TabId(5))),
                "a move that began after the op that made 5 due, read before the follower acts, \
                 stands",
            ),
            (
                Follow::Task(t.clone()),
                vec![
                    Step::Moved(4, agent(&u), 20),
                    Step::Acted(5, 30),
                    Step::Moved(6, agent(&u), 25),
                ],
                (Some(TabId(5)), Some(TabId(5))),
                "a move that began before that op and landed late leaves 5 due",
            ),
            (
                Follow::Task(t.clone()),
                vec![
                    Step::Moved(4, agent(&u), 20),
                    Step::Acted(5, 30),
                    Step::Paused,
                    Step::Moved(6, agent(&u), 40),
                ],
                (Some(TabId(5)), Some(TabId(5))),
                "a pause's end is the follower's own edge: a move after the pause does not retire it",
            ),
        ];
        for (follow, steps, expected, case) in cases {
            assert_eq!(after(&follow, steps), expected, "{case}");
        }
        Ok(())
    }

    #[test]
    fn two_followers_never_answer_each_others_moves() -> Result<(), Box<dyn std::error::Error>> {
        let task = TaskId::parse("checkout")?;
        let (on_three, on_four) = (Follow::Tab(TabId(3)), Follow::Task(task));
        let mut three = FollowMemory::start(Some(TabId(3)), Some(TabId(3)));
        let mut four = FollowMemory::start(Some(TabId(4)), Some(TabId(4)));
        for (memory, follow, moved) in [(&mut three, &on_three, 4), (&mut four, &on_four, 3)] {
            memory.front_moved(
                follow,
                &FrontMove {
                    tab: TabId(moved),
                    by: Mover::Follow,
                    at: EpochMillis::from_millis(20),
                },
            );
        }
        assert_eq!(
            next_activation(three.state(false, false)),
            None,
            "the follower of 3 lets the other follower's 4 stand"
        );
        assert_eq!(
            next_activation(four.state(false, false)),
            None,
            "and the follower of 4 lets 3 stand"
        );
        Ok(())
    }
}
