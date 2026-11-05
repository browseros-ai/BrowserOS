//! The lock rule: whether the locks refuse one agent operation, now ([`decide`]) or at a past
//! instant ([`decide_at`]). Pure, with no clock of its own: the caller passes the instant, so the
//! decided table is a unit test. The two differ only in which locks were in force at their
//! instant; what a lock in force refuses is one rule, shared.

use crate::locks::Held;
use crate::{Lock, LockTarget, OwnerHasControl, Reads};
use browseros_trace::{Effect, EpochMillis};
use tokio::time::Instant;

/// Whether a lock with `reads` refuses an operation with `effect`: a write always, a read only
/// under `Block`.
pub(crate) const fn refuses(reads: Reads, effect: Effect) -> bool {
    matches!(
        (effect, reads),
        (Effect::Write, _) | (Effect::Read, Reads::Block)
    )
}

/// The check over every held lock as the locks stand at `now`, for [`crate::Locks::check`].
///
/// A lock whose deadline has passed refuses nothing, whatever its timer did: the deadline, not
/// the timer, ends a lock.
pub(crate) fn decide<'a>(
    held: impl IntoIterator<Item = &'a Held>,
    target: LockTarget,
    effect: Effect,
    now: Instant,
) -> Result<(), OwnerHasControl> {
    let in_force = held
        .into_iter()
        .filter(|held| held.deadline > now)
        .map(|held| &held.lock);
    oldest_refusal(in_force, target, effect)
}

/// The check over every lock, held or ended, as the locks stood at `at`, for
/// [`crate::Locks::check_at`].
///
/// A lock was in force from its `created_at` through its end, both inclusive: `ended.at` once it
/// ended, `expires_at` while it is held. An expired lock's `ended.at` is its `expires_at`, so a
/// held lock past its deadline answers as it will once a read or its timer ends it.
pub(crate) fn decide_at<'a>(
    locks: impl IntoIterator<Item = &'a Lock>,
    target: LockTarget,
    effect: Effect,
    at: EpochMillis,
) -> Result<(), OwnerHasControl> {
    let in_force = locks.into_iter().filter(|lock| {
        let end = lock.ended.map_or(lock.expires_at, |ended| ended.at);
        (lock.created_at..=end).contains(&at)
    });
    oldest_refusal(in_force, target, effect)
}

/// The rule both checks apply to the locks in force at their instant.
///
/// Several locks on one tab add up, so any one of them can refuse. `NoPage` is never refused.
/// `UnmappedPage` is refused as if it were on every locked tab: the adapter could not say which
/// tab the page is, and letting it pass as "no page" would let a mapping gap skip the lock (fail
/// closed). The refusal names the oldest refusing lock (ids are time-ordered), whatever order the
/// locks come in: the past-instant check reads held locks before ended ones.
fn oldest_refusal<'a>(
    in_force: impl Iterator<Item = &'a Lock>,
    target: LockTarget,
    effect: Effect,
) -> Result<(), OwnerHasControl> {
    let covers = |lock: &Lock| match target {
        LockTarget::NoPage => false,
        LockTarget::Tab(tab) => lock.tab == tab,
        LockTarget::UnmappedPage => true,
    };
    match in_force
        .filter(|lock| covers(lock) && refuses(lock.reads, effect))
        .min_by_key(|lock| lock.id)
    {
        Some(lock) => Err(OwnerHasControl { lock: lock.id }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use crate::locks::Held;
    use crate::{
        Lock, LockBinding, LockEnd, LockEnded, LockId, LockTarget, OwnerHasControl, Reads,
    };
    use browseros_core::TabId;
    use browseros_trace::{Effect, EpochMillis};
    use std::time::Duration;
    use tokio::time::Instant;

    // Every lock here is taken at `TAKEN` with a 60 s TTL, so it expires at `EXPIRES` unless it
    // is released at `RELEASED_AT`. `DURING` is inside each of those lives.
    const TAKEN: EpochMillis = EpochMillis::from_millis(1_000_000);
    const DURING: EpochMillis = EpochMillis::from_millis(1_010_000);
    const RELEASED_AT: EpochMillis = EpochMillis::from_millis(1_030_000);
    const EXPIRES: EpochMillis = EpochMillis::from_millis(1_060_000);
    const HELD: Option<LockEnded> = None;
    const RELEASED: Option<LockEnded> = Some(LockEnded {
        at: RELEASED_AT,
        cause: LockEnd::Released,
    });
    const EXPIRED: Option<LockEnded> = Some(LockEnded {
        at: EXPIRES,
        cause: LockEnd::Expired,
    });

    fn lock(tab: i64, reads: Reads, ended: Option<LockEnded>) -> Lock {
        Lock {
            id: LockId::new(),
            tab: TabId(tab),
            reads,
            binding: LockBinding::Host,
            settled: true,
            created_at: TAKEN,
            expires_at: EXPIRES,
            ended,
        }
    }

    fn held(tab: i64, reads: Reads, deadline: Instant) -> Held {
        Held {
            lock: lock(tab, reads, HELD),
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
            let decided = super::decide([&lock], target, effect, now);
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
            assert_eq!(super::decide([&lock], target, Effect::Write, now), Ok(()));
        }
        let ahead = held(12, Reads::Block, now + Duration::from_millis(1));
        assert!(super::decide([&ahead], LockTarget::Tab(TabId(12)), Effect::Write, now).is_err());
    }

    #[test]
    fn an_unmapped_page_is_refused_while_any_lock_would_refuse_it() {
        let now = Instant::now();
        let later = now + Duration::from_secs(60);
        assert_eq!(
            super::decide([], LockTarget::UnmappedPage, Effect::Write, now),
            Ok(()),
            "no lock, nothing to refuse"
        );
        let allow = held(12, Reads::Allow, later);
        assert!(super::decide([&allow], LockTarget::UnmappedPage, Effect::Write, now).is_err());
        assert_eq!(
            super::decide([&allow], LockTarget::UnmappedPage, Effect::Read, now),
            Ok(()),
            "the read would pass on the locked tab too"
        );
        let block = held(40, Reads::Block, later);
        assert_eq!(
            super::decide(
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
            super::decide([&allow, &block], tab, Effect::Read, now),
            Err(OwnerHasControl {
                lock: block.lock.id
            }),
            "the block lock refuses the read the allow lock lets through"
        );
        assert_eq!(
            super::decide([&allow, &block], tab, Effect::Write, now),
            Err(OwnerHasControl {
                lock: allow.lock.id
            }),
            "the oldest lock that refuses is named"
        );
        let after_block = now + Duration::from_secs(5);
        assert_eq!(
            super::decide([&allow, &block], tab, Effect::Read, after_block),
            Ok(()),
            "once the block lock's deadline passes, the allow lock alone decides"
        );
        assert!(super::decide([&allow, &block], tab, Effect::Write, after_block).is_err());
    }

    #[test]
    fn a_lock_covers_the_instants_from_when_it_was_taken_through_its_end_both_included()
    -> Result<(), Box<dyn std::error::Error>> {
        let tab = LockTarget::Tab(TabId(12));
        for (ended, end, case) in [
            (HELD, EXPIRES, "a held lock ends at its expires_at"),
            (
                RELEASED,
                RELEASED_AT,
                "a released lock ends at its ended.at",
            ),
            (EXPIRED, EXPIRES, "an expired lock ends at its expires_at"),
        ] {
            let lock = lock(12, Reads::Block, ended);
            let refused = Err(OwnerHasControl { lock: lock.id });
            for (at, expected, when) in [
                (
                    TAKEN.plus_millis(-1),
                    Ok(()),
                    "a millisecond before it was taken",
                ),
                (TAKEN, refused, "the millisecond it was taken"),
                (DURING, refused, "while it held"),
                (end, refused, "the millisecond it ended"),
                (end.plus_millis(1), Ok(()), "a millisecond after it ended"),
            ] {
                assert_eq!(
                    super::decide_at([&lock], tab, Effect::Write, at),
                    expected,
                    "{case}; {when}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn at_an_instant_it_covered_a_lock_refuses_by_effect_and_reads_on_its_own_tab_only()
    -> Result<(), Box<dyn std::error::Error>> {
        let (tab, other) = (LockTarget::Tab(TabId(12)), LockTarget::Tab(TabId(13)));
        let cases = [
            (
                Reads::Block,
                tab,
                Effect::Read,
                false,
                "block refuses a read",
            ),
            (
                Reads::Allow,
                tab,
                Effect::Read,
                true,
                "allow lets a read through",
            ),
            (
                Reads::Allow,
                tab,
                Effect::Write,
                false,
                "allow still refuses a write",
            ),
            (Reads::Block, other, Effect::Write, true, "another tab runs"),
            (
                Reads::Block,
                LockTarget::NoPage,
                Effect::Write,
                true,
                "no page runs, a write included",
            ),
        ];
        for (reads, target, effect, passes, case) in cases {
            let lock = lock(12, reads, RELEASED);
            let expected = if passes {
                Ok(())
            } else {
                Err(OwnerHasControl { lock: lock.id })
            };
            assert_eq!(
                super::decide_at([&lock], target, effect, DURING),
                expected,
                "{case}"
            );
        }
        Ok(())
    }

    #[test]
    fn an_unmapped_page_is_refused_at_an_instant_any_lock_would_have_refused_it()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            super::decide_at([], LockTarget::UnmappedPage, Effect::Write, DURING),
            Ok(()),
            "no lock, nothing to refuse"
        );
        let allow = lock(12, Reads::Allow, HELD);
        assert_eq!(
            super::decide_at([&allow], LockTarget::UnmappedPage, Effect::Write, DURING),
            Err(OwnerHasControl { lock: allow.id })
        );
        assert_eq!(
            super::decide_at([&allow], LockTarget::UnmappedPage, Effect::Read, DURING),
            Ok(()),
            "the read would pass on the locked tab too"
        );
        let block = lock(40, Reads::Block, RELEASED);
        assert_eq!(
            super::decide_at(
                [&allow, &block],
                LockTarget::UnmappedPage,
                Effect::Read,
                DURING
            ),
            Err(OwnerHasControl { lock: block.id }),
            "any locked tab may be the page's"
        );
        assert_eq!(
            super::decide_at(
                [&allow, &block],
                LockTarget::UnmappedPage,
                Effect::Read,
                RELEASED_AT.plus_millis(1)
            ),
            Ok(()),
            "after the block lock's release, the allow lock alone decides"
        );
        Ok(())
    }

    #[test]
    fn the_oldest_lock_that_covered_the_instant_is_named_whatever_the_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let older = lock(12, Reads::Block, RELEASED);
        let newer = lock(12, Reads::Block, HELD);
        assert_eq!(
            super::decide_at(
                [&newer, &older],
                LockTarget::Tab(TabId(12)),
                Effect::Write,
                DURING
            ),
            Err(OwnerHasControl { lock: older.id }),
            "the ended lock is the older, though it comes second"
        );
        Ok(())
    }
}
