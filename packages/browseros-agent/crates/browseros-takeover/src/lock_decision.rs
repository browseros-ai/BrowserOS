//! The lock rule: whether the held locks refuse one agent operation. Pure, with no clock of its
//! own: the caller passes the instant, so the decided table is a unit test.

use crate::locks::Held;
use crate::{LockTarget, OwnerHasControl, Reads};
use browseros_trace::Effect;
use tokio::time::Instant;

/// Whether a lock with `reads` refuses an operation with `effect`: a write always, a read only
/// under `Block`.
pub(crate) const fn refuses(reads: Reads, effect: Effect) -> bool {
    matches!(
        (effect, reads),
        (Effect::Write, _) | (Effect::Read, Reads::Block)
    )
}

/// The check over every held lock as the locks stand at `now`.
///
/// A lock whose deadline has passed refuses nothing, whatever its timer did: the deadline, not
/// the timer, ends a lock. Several locks on one tab add up, so any one of them can refuse.
/// `NoPage` is never refused. `UnmappedPage` is refused as if it were on every locked tab: the
/// adapter could not say which tab the page is, and letting it pass as "no page" would let a
/// mapping gap skip the lock (fail closed). The refusal names the first refusing lock in
/// `held`'s order.
pub(crate) fn decide<'a>(
    held: impl IntoIterator<Item = &'a Held>,
    target: LockTarget,
    effect: Effect,
    now: Instant,
) -> Result<(), OwnerHasControl> {
    let covers = |held: &Held| match target {
        LockTarget::NoPage => false,
        LockTarget::Tab(tab) => held.lock.tab == tab,
        LockTarget::UnmappedPage => true,
    };
    match held
        .into_iter()
        .find(|held| held.deadline > now && covers(held) && refuses(held.lock.reads, effect))
    {
        Some(held) => Err(OwnerHasControl { lock: held.lock.id }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::decide;
    use crate::locks::Held;
    use crate::{Lock, LockBinding, LockId, LockTarget, OwnerHasControl, Reads};
    use browseros_core::TabId;
    use browseros_trace::{Effect, EpochMillis};
    use std::time::Duration;
    use tokio::time::Instant;

    fn held(tab: i64, reads: Reads, deadline: Instant) -> Held {
        Held {
            lock: Lock {
                id: LockId::new(),
                tab: TabId(tab),
                reads,
                binding: LockBinding::Host,
                settled: true,
                created_at: EpochMillis::from_millis(0),
                expires_at: EpochMillis::from_millis(60_000),
                ended: None,
            },
            deadline,
        }
    }

    #[test]
    fn one_lock_refuses_by_effect_and_reads_on_its_own_tab_only() {
        let now = Instant::now();
        let later = now + Duration::from_secs(60);
        let (tab, other) = (LockTarget::Tab(TabId(12)), LockTarget::Tab(TabId(13)));
        let cases = [
            (
                Reads::Block,
                tab,
                Effect::Write,
                false,
                "a write is refused",
            ),
            (
                Reads::Block,
                tab,
                Effect::Read,
                false,
                "block refuses reads",
            ),
            (
                Reads::Allow,
                tab,
                Effect::Write,
                false,
                "allow still refuses writes",
            ),
            (
                Reads::Allow,
                tab,
                Effect::Read,
                true,
                "allow lets reads through",
            ),
            (Reads::Block, other, Effect::Write, true, "another tab runs"),
            (
                Reads::Block,
                LockTarget::NoPage,
                Effect::Write,
                true,
                "no page runs",
            ),
        ];
        for (reads, target, effect, passes, case) in cases {
            let lock = held(12, reads, later);
            let decided = decide([&lock], target, effect, now);
            let expected = if passes {
                Ok(())
            } else {
                Err(OwnerHasControl { lock: lock.lock.id })
            };
            assert_eq!(decided, expected, "{case}");
        }
    }

    #[test]
    fn a_lock_past_its_deadline_refuses_nothing() {
        let now = Instant::now();
        let lock = held(12, Reads::Block, now);
        for target in [LockTarget::Tab(TabId(12)), LockTarget::UnmappedPage] {
            assert_eq!(decide([&lock], target, Effect::Write, now), Ok(()));
        }
        let ahead = held(12, Reads::Block, now + Duration::from_millis(1));
        assert!(decide([&ahead], LockTarget::Tab(TabId(12)), Effect::Write, now).is_err());
    }

    #[test]
    fn an_unmapped_page_is_refused_while_any_lock_would_refuse_it() {
        let now = Instant::now();
        let later = now + Duration::from_secs(60);
        assert_eq!(
            decide([], LockTarget::UnmappedPage, Effect::Write, now),
            Ok(()),
            "no lock, nothing to refuse"
        );
        let allow = held(12, Reads::Allow, later);
        assert!(decide([&allow], LockTarget::UnmappedPage, Effect::Write, now).is_err());
        assert_eq!(
            decide([&allow], LockTarget::UnmappedPage, Effect::Read, now),
            Ok(()),
            "the read would pass on the locked tab too"
        );
        let block = held(40, Reads::Block, later);
        assert_eq!(
            decide(
                [&allow, &block],
                LockTarget::UnmappedPage,
                Effect::Read,
                now
            ),
            Err(OwnerHasControl {
                lock: block.lock.id
            }),
            "any locked tab may be the page's"
        );
    }

    #[test]
    fn several_locks_on_one_tab_add_up() {
        let now = Instant::now();
        let allow = held(12, Reads::Allow, now + Duration::from_secs(60));
        let block = held(12, Reads::Block, now + Duration::from_secs(5));
        let tab = LockTarget::Tab(TabId(12));
        assert_eq!(
            decide([&allow, &block], tab, Effect::Read, now),
            Err(OwnerHasControl {
                lock: block.lock.id
            }),
            "the block lock refuses the read the allow lock lets through"
        );
        assert_eq!(
            decide([&allow, &block], tab, Effect::Write, now),
            Err(OwnerHasControl {
                lock: allow.lock.id
            }),
            "the first lock that refuses is named"
        );
        let after_block = now + Duration::from_secs(5);
        assert_eq!(
            decide([&allow, &block], tab, Effect::Read, after_block),
            Ok(()),
            "once the block lock's deadline passes, the allow lock alone decides"
        );
        assert!(decide([&allow, &block], tab, Effect::Write, after_block).is_err());
    }
}
