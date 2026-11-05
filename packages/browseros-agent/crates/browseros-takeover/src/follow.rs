//! Keeps a tab, or a task's current tab, in front of Neo's one screen while a view display
//! watches (D12's follow). It replaces zuzu's `watch`, `follow` and `toFront` and the observer
//! task they ran in: Neo already sees every operation, so a host only names what to follow.
//!
//! ```text
//!  followTab=N ──────────────────────────────────────┐
//!  followTask=T ─ TraceStore::task_last_tab(T), then ├─▶ next_activation ─▶ Browser.activateTab
//!                 OperationStarted { task: T, tab }  │       (pure)
//!  1 s tick: a control display open? the tab locked? │
//!            the tab private? ───────────────────────┘
//! ```
//!
//! **It never fights the owner.** It pauses while any control display is open, and while a lock
//! holds the followed tab: the owner may be switching tabs on the same screen. A pause forgets
//! what the follower showed, so when the pause ends it brings its tab back even if the screen
//! moved meanwhile. The tick also re-reads the task's tab after the event stream lagged.
//!
//! **It never brings a private tab to the front** (decision 34). The views watching would show
//! its pixels until the front-tab watch closed them (front_tab.rs).

use crate::limits::FOLLOW_TICK;
use crate::{Displays, Follow};
use browseros_core::TabId;
use browseros_trace::{TaskId, TraceRecordEvent};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::MissedTickBehavior;

/// What a follower knows at one moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FollowState {
    /// The tab to keep in front, once known.
    pub(crate) target: Option<TabId>,
    /// A control display is open, or a lock holds the target.
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

/// The follow half of a view display's life. It runs until the display drops it.
pub(crate) async fn run(follow: &Follow, displays: &Displays) {
    // Subscribed before the task's tab is read, so an operation between the two is not missed.
    let mut events = match follow {
        Follow::Task(_) => Some(displays.sink.subscribe()),
        Follow::Tab(_) => None,
    };
    let mut target = match follow {
        Follow::Tab(tab) => Some(*tab),
        Follow::Task(task) => last_tab(displays, task).await,
    };
    let mut shown = None;
    let mut tick = tokio::time::interval(FOLLOW_TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        let paused =
            displays.control_open() || target.is_some_and(|tab| displays.locks.is_locked(tab));
        if paused {
            shown = None;
        }
        let private = target.is_some_and(|tab| displays.privacy.is_private(tab).is_some());
        if let Some(tab) = next_activation(FollowState {
            target,
            paused,
            private,
            shown,
        }) {
            match displays.browser.activate(tab).await {
                Ok(()) => shown = Some(tab),
                Err(error) => {
                    tracing::debug!(tab = tab.0, %error, "follow could not bring its tab to the front");
                }
            }
        }
        tokio::select! {
            _ = tick.tick() => {}
            event = next_event(&mut events) => {
                let Follow::Task(task) = follow else {
                    continue;
                };
                match event {
                    Ok(TraceRecordEvent::OperationStarted {
                        task: Some(of),
                        tab: Some(tab),
                        ..
                    }) if &of == task => target = Some(tab),
                    // A newPage has no target when it starts; the tab it opened is known once
                    // it ends, as the page the task works in next.
                    Ok(TraceRecordEvent::OperationEnded {
                        task: Some(of),
                        tab: None,
                        ..
                    }) if &of == task => target = last_tab(displays, task).await.or(target),
                    Ok(_) => {}
                    Err(RecvError::Lagged(_)) => target = last_tab(displays, task).await.or(target),
                    Err(RecvError::Closed) => events = None,
                }
            }
        }
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

/// The tab the task's latest operation acted on, as the store has it.
async fn last_tab(displays: &Displays, task: &TaskId) -> Option<TabId> {
    match displays.store.task_last_tab(task).await {
        Ok(tab) => tab,
        Err(error) => {
            tracing::warn!(%error, "follow could not read the task's latest tab");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FollowState, next_activation};
    use browseros_core::TabId;

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
                "a control display is open, or the tab is locked",
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
}
