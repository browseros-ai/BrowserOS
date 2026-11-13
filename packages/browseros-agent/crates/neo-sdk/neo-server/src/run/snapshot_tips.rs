//! The run reply's tip about `page.ariaSnapshot({ delta: true })`.
//!
//! ```text
//!  neo-playwright  page.snapshotPending: a full whole-page snapshot found a stored baseline
//!                  (not a pending one) of its tab and document
//!                  → RunState::full_snapshot_repeats() lists the tab
//!  run service     after the ops drained, before the end record: SnapshotTips::decide
//!                    failed run → none
//!                    the host's taskId, else the run's Neo task → that task's count
//!                    no task at all → the tab's count (a TabRegistry slot)
//!                  under 3 → RunReply.tips gets the tip, and the count goes up
//!  run_tool text() the tip's line, after the value, the console and the notes
//! ```
//!
//! The decision is stored in the reply, so a `requestId` replay renders the same text, and the
//! counts only move for tips a caller was actually given. Under the spec harness's conformance
//! switch the prelude never sends the baseline ops, so no tip can arise there.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_core::TabId;
use neo_trace::TaskId;
use serde::{Deserialize, Serialize};

use crate::browser::{PageRef, TabRegistry};

/// At most this many tips per task, or per tab for runs with no task.
const TIPS_PER_TASK: u32 = 3;
/// How many tasks' counts memory keeps. A task evicted here may be shown the tip again, which
/// costs one line; keeping every host task id forever would not be bounded.
const TASKS_REMEMBERED: usize = 1024;

/// A tip a run's reply carries, one line each in the text the model reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ReplyTip {
    /// The run snapshotted a whole page in full where `{ delta: true }` would have printed only
    /// what changed.
    SnapshotDelta,
}

impl ReplyTip {
    /// The line the model reads.
    #[must_use]
    pub(crate) fn line(self) -> &'static str {
        match self {
            Self::SnapshotDelta => {
                "Tip: page.ariaSnapshot({ delta: true }) returns only what changed since your \
                 last snapshot."
            }
        }
    }
}

/// How many delta tips each task was given. One per process, on the run service.
#[derive(Default)]
pub(crate) struct SnapshotTips {
    tasks: Mutex<TaskCounts>,
}

/// Counts by task, the oldest task first in `order` so the bound evicts it.
#[derive(Default)]
struct TaskCounts {
    counts: HashMap<TaskId, u32>,
    order: VecDeque<TaskId>,
}

/// One tab's count, for runs with no task: kept with the tab (a `TabRegistry` slot), so it goes
/// when the tab closes, and every later agent on another tab still gets its three.
#[derive(Default)]
pub(crate) struct TabTips(AtomicU32);

impl SnapshotTips {
    /// The reply's tips. `repeats` are the tabs the run snapshotted in full while a delta was
    /// possible, in the order first seen; `task` is the host's task, else the run's Neo task.
    /// None on a failed run, and none once the task (or, without one, each tab) had its three.
    pub(crate) fn decide(
        &self,
        failed: bool,
        task: Option<&TaskId>,
        repeats: &[TabId],
        tabs: &TabRegistry,
    ) -> Vec<ReplyTip> {
        if failed || repeats.is_empty() {
            return Vec::new();
        }
        let given = match task {
            Some(task) => self.claim_for_task(task),
            None => repeats.iter().any(|&tab| {
                tabs.slot::<TabTips>(&PageRef { tab })
                    .is_some_and(|tips| tips.claim())
            }),
        };
        if given {
            vec![ReplyTip::SnapshotDelta]
        } else {
            Vec::new()
        }
    }

    /// One more tip for `task`, if it had fewer than three.
    fn claim_for_task(&self, task: &TaskId) -> bool {
        let mut tasks = self.lock();
        let TaskCounts { counts, order } = &mut *tasks;
        if let Some(count) = counts.get_mut(task) {
            if *count >= TIPS_PER_TASK {
                return false;
            }
            *count += 1;
            return true;
        }
        if order.len() >= TASKS_REMEMBERED
            && let Some(oldest) = order.pop_front()
        {
            counts.remove(&oldest);
        }
        counts.insert(task.clone(), 1);
        order.push_back(task.clone());
        true
    }

    fn lock(&self) -> MutexGuard<'_, TaskCounts> {
        self.tasks.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TabTips {
    /// One more tip for this tab, if it had fewer than three.
    fn claim(&self) -> bool {
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count < TIPS_PER_TASK).then_some(count + 1)
            })
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::TabId;
    use neo_trace::TaskId;

    use super::{ReplyTip, SnapshotTips, TASKS_REMEMBERED};
    use crate::browser::{PageRef, TabRegistry};

    fn open(tabs: &TabRegistry, tab: i64) -> TabId {
        let tab = TabId(tab);
        tabs.touched(&PageRef { tab }, tabs.seen());
        tab
    }

    #[test]
    fn tests_that_a_task_gets_the_tip_three_times() -> Result<(), Box<dyn Error>> {
        let (tips, tabs) = (SnapshotTips::default(), TabRegistry::new());
        let tab = open(&tabs, 1);
        let task = TaskId::parse("reply-to-mail")?;
        let shown: Vec<_> = (0..5)
            .map(|_| tips.decide(false, Some(&task), &[tab], &tabs))
            .collect();
        let once = vec![ReplyTip::SnapshotDelta];
        assert_eq!(
            shown,
            [once.clone(), once.clone(), once, Vec::new(), Vec::new()]
        );
        let other = TaskId::parse("book-flight")?;
        assert_eq!(
            tips.decide(false, Some(&other), &[tab], &tabs),
            [ReplyTip::SnapshotDelta],
            "another task on the same tab has its own three"
        );
        Ok(())
    }

    #[test]
    fn tests_that_runs_without_a_task_count_per_tab() {
        let (tips, tabs) = (SnapshotTips::default(), TabRegistry::new());
        let (first, second) = (open(&tabs, 1), open(&tabs, 2));
        for _ in 0..3 {
            assert_eq!(
                tips.decide(false, None, &[first], &tabs),
                [ReplyTip::SnapshotDelta]
            );
        }
        assert!(tips.decide(false, None, &[first], &tabs).is_empty());
        assert_eq!(
            tips.decide(false, None, &[first, second], &tabs),
            [ReplyTip::SnapshotDelta],
            "a tab that still has tips gives the reply its one line"
        );
        assert_eq!(
            tips.decide(false, None, &[second], &tabs),
            [ReplyTip::SnapshotDelta]
        );
        assert_eq!(
            tips.decide(false, None, &[second], &tabs),
            [ReplyTip::SnapshotDelta]
        );
        assert!(tips.decide(false, None, &[second], &tabs).is_empty());
        let closed = TabId(3);
        assert!(
            tips.decide(false, None, &[closed], &tabs).is_empty(),
            "a tab Neo no longer has keeps no count"
        );
    }

    #[test]
    fn tests_that_a_failed_run_or_no_repeat_gives_no_tip_and_spends_none()
    -> Result<(), Box<dyn Error>> {
        let (tips, tabs) = (SnapshotTips::default(), TabRegistry::new());
        let tab = open(&tabs, 1);
        let task = TaskId::parse("t")?;
        for _ in 0..5 {
            assert!(tips.decide(true, Some(&task), &[tab], &tabs).is_empty());
            assert!(tips.decide(true, None, &[tab], &tabs).is_empty());
            assert!(tips.decide(false, Some(&task), &[], &tabs).is_empty());
        }
        for _ in 0..3 {
            assert_eq!(
                tips.decide(false, Some(&task), &[tab], &tabs),
                [ReplyTip::SnapshotDelta]
            );
            assert_eq!(
                tips.decide(false, None, &[tab], &tabs),
                [ReplyTip::SnapshotDelta]
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_the_task_counts_stay_bounded() -> Result<(), Box<dyn Error>> {
        let (tips, tabs) = (SnapshotTips::default(), TabRegistry::new());
        let tab = open(&tabs, 1);
        for n in 0..=TASKS_REMEMBERED {
            tips.decide(
                false,
                Some(&TaskId::parse(&format!("t{n}"))?),
                &[tab],
                &tabs,
            );
        }
        assert_eq!(tips.lock().counts.len(), TASKS_REMEMBERED);
        assert!(!tips.lock().counts.contains_key(&TaskId::parse("t0")?));
        Ok(())
    }

    #[test]
    fn tests_the_tip_s_wording() {
        assert_eq!(
            ReplyTip::SnapshotDelta.line(),
            "Tip: page.ariaSnapshot({ delta: true }) returns only what changed since your last \
             snapshot."
        );
    }
}
