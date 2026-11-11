//! `Locks` as its callers see it, over a real trace store and writer: the dispatcher's check and
//! its past-instant form, the settle, the TTL, the tab's end, and the takeover binding.
//!
//! The settle reads the store through sqlx, whose pool does not run under paused time, so these
//! tests use real time with the shortest TTL (1 s) where a TTL has to pass.

use browseros_core::TabId;
use neo_takeover::{
    Lock, LockBinding, LockEnd, LockId, LockTarget, LockTtl, Locks, OwnerHasControl, PartError,
    Reads, TakeoverId,
};
use neo_trace::{
    Delivery, Effect, EpochMillis, OperationEnded, OperationId, OperationKind, OperationStarted,
    Outcome, RedactionLimits, Redactor, RunId, RunStarted, TraceSink, TraceStore, WriterConfig,
    spawn_writer,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::{Instant, timeout};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

struct Harness {
    _dir: tempfile::TempDir,
    sink: TraceSink,
    locks: Arc<Locks>,
}

async fn harness() -> Result<Harness, Box<dyn std::error::Error + Send + Sync>> {
    harness_with(WriterConfig::default()).await
}

async fn harness_with(
    config: WriterConfig,
) -> Result<Harness, Box<dyn std::error::Error + Send + Sync>> {
    let dir = tempfile::tempdir()?;
    let store = TraceStore::open(&dir.path().join("trace.db")).await?;
    let sink = spawn_writer(
        &store,
        Redactor::new(RedactionLimits::default()),
        config,
        None,
    );
    let locks = Locks::new(store, sink.clone());
    Ok(Harness {
        _dir: dir,
        sink,
        locks,
    })
}

fn ttl(seconds: u64) -> Result<LockTtl, &'static str> {
    LockTtl::new(Duration::from_secs(seconds)).ok_or("a TTL outside 1 s to 20 minutes")
}

/// Starts a run with one write on `tab` that has not ended: the agent's click in flight.
fn write_in_flight(sink: &TraceSink, tab: i64) -> OperationId {
    let run = RunId::new();
    sink.run_started(RunStarted {
        id: run,
        code: "await page.locator('#pay').click()".into(),
        request_id: None,
        task_id: None,
        host_data: None,
        at: EpochMillis::now(),
    });
    let op = OperationId::new(run, 1);
    sink.op_started(OperationStarted {
        site: None,
        id: op,
        parent: None,
        name: "locator.click".into(),
        kind: OperationKind::Builtin,
        effect: Effect::Write,
        tab: Some(TabId(tab)),
        args: serde_json::json!({ "selector": "#pay" }),
        at: EpochMillis::now(),
    });
    op
}

fn write_done(sink: &TraceSink, op: OperationId) {
    sink.op_ended(OperationEnded {
        id: op,
        outcome: Outcome::Ok,
        delivery: Delivery::Done,
        sent_at: Some(EpochMillis::now()),
        result: None,
        error: None,
        page_after: None,
        at: EpochMillis::now(),
    });
}

async fn next_change(changes: &mut broadcast::Receiver<Lock>) -> Result<Lock, String> {
    match timeout(Duration::from_secs(5), changes.recv()).await {
        Ok(Ok(lock)) => Ok(lock),
        Ok(Err(error)) => Err(format!("no lock change: {error}")),
        Err(_) => Err("no lock change within 5 s".into()),
    }
}

fn refused_by(lock: &Lock) -> Result<(), OwnerHasControl> {
    Err(OwnerHasControl { lock: lock.id })
}

#[tokio::test]
async fn a_lock_with_nothing_in_flight_settles_at_once_and_refuses_the_agent() -> TestResult {
    let h = harness().await?;
    let mut changes = h.locks.subscribe();
    let (tab, other) = (LockTarget::Tab(TabId(12)), LockTarget::Tab(TabId(13)));
    assert_eq!(
        h.locks.check(LockTarget::UnmappedPage, Effect::Write),
        Ok(())
    );

    let started = Instant::now();
    let lock = h
        .locks
        .lock(TabId(12), Reads::Block, ttl(60)?, LockBinding::Host)
        .await;
    assert!(lock.settled, "nothing was in flight");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(lock.tab, TabId(12));
    assert_eq!(lock.ended, None);
    assert_eq!(
        lock.expires_at.millis() - lock.created_at.millis(),
        60_000,
        "the TTL, in the lock's own times"
    );
    assert_eq!(h.locks.get(lock.id), Some(lock.clone()));
    assert_eq!(
        next_change(&mut changes).await?,
        lock,
        "announced once settled"
    );

    assert_eq!(h.locks.check(tab, Effect::Write), refused_by(&lock));
    assert_eq!(h.locks.check(tab, Effect::Read), refused_by(&lock));
    assert_eq!(
        h.locks.check(LockTarget::UnmappedPage, Effect::Read),
        refused_by(&lock)
    );
    assert_eq!(h.locks.check(other, Effect::Write), Ok(()));
    assert_eq!(h.locks.check(LockTarget::NoPage, Effect::Write), Ok(()));
    assert!(h.locks.is_locked(TabId(12)));
    assert!(!h.locks.is_locked(TabId(13)));

    let reader = h
        .locks
        .lock(TabId(13), Reads::Allow, ttl(60)?, LockBinding::Host)
        .await;
    assert_eq!(h.locks.check(other, Effect::Write), refused_by(&reader));
    assert_eq!(h.locks.check(other, Effect::Read), Ok(()), "reads: allow");
    Ok(())
}

#[tokio::test]
async fn a_write_still_open_after_two_seconds_leaves_the_lock_unsettled() -> TestResult {
    let h = harness().await?;
    write_in_flight(&h.sink, 12);
    let started = Instant::now();
    let lock = h
        .locks
        .lock(TabId(12), Reads::Block, ttl(60)?, LockBinding::Host)
        .await;
    let waited = started.elapsed();
    assert!(!lock.settled);
    assert!(
        (Duration::from_secs(2)..Duration::from_secs(3)).contains(&waited),
        "the settle waits 2 s: {waited:?}"
    );
    assert_eq!(
        h.locks.check(LockTarget::Tab(TabId(12)), Effect::Write),
        refused_by(&lock),
        "an unsettled lock refuses all the same"
    );
    Ok(())
}

/// Under backpressure the trace drops operation events, so the store may not know a write that
/// is still in flight. The settle must not then report that nothing was in flight.
#[tokio::test]
async fn a_lock_taken_while_the_trace_lost_events_is_not_settled() -> TestResult {
    let h = harness_with(WriterConfig {
        max_backlog: 0,
        ..WriterConfig::default()
    })
    .await?;
    write_in_flight(&h.sink, 12);
    assert!(h.sink.lost_operation_events());
    let lock = h
        .locks
        .lock(TabId(12), Reads::Block, ttl(60)?, LockBinding::Host)
        .await;
    assert!(!lock.settled, "the dropped write cannot be ruled out");
    assert_eq!(
        h.locks.check(LockTarget::Tab(TabId(12)), Effect::Write),
        refused_by(&lock)
    );
    Ok(())
}

/// The writer commits in batches, so a write that started a moment before the lock may not be
/// in the store yet. The lock flushes the writer first, so it waits for that write.
#[tokio::test]
async fn the_settle_waits_for_a_write_that_started_just_before_the_lock() -> TestResult {
    let h = harness().await?;
    let op = write_in_flight(&h.sink, 12);
    let sink = h.sink.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        write_done(&sink, op);
    });
    let started = Instant::now();
    let lock = h
        .locks
        .lock(TabId(12), Reads::Block, ttl(60)?, LockBinding::Host)
        .await;
    assert!(lock.settled, "the write ended within 2 s");
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "the lock waited for the write: {:?}",
        started.elapsed()
    );
    Ok(())
}

#[tokio::test]
async fn a_caller_that_stops_waiting_still_leaves_the_lock_settled_and_announced() -> TestResult {
    let h = harness().await?;
    let mut changes = h.locks.subscribe();
    let op = write_in_flight(&h.sink, 12);
    let gave_up = timeout(
        Duration::from_millis(100),
        h.locks
            .lock(TabId(12), Reads::Block, ttl(60)?, LockBinding::Host),
    )
    .await;
    assert!(gave_up.is_err(), "the host hung up while the lock settled");
    write_done(&h.sink, op);

    let taken = next_change(&mut changes).await?;
    assert!(taken.settled);
    assert_eq!(taken.ended, None);
    assert_eq!(
        h.locks.check(LockTarget::Tab(TabId(12)), Effect::Write),
        refused_by(&taken)
    );
    Ok(())
}

#[tokio::test]
async fn the_ttl_ends_the_lock_and_announces_it() -> TestResult {
    let h = harness().await?;
    let mut changes = h.locks.subscribe();
    let lock = h
        .locks
        .lock(TabId(12), Reads::Block, ttl(1)?, LockBinding::Host)
        .await;
    assert_eq!(next_change(&mut changes).await?, lock);

    let expired = next_change(&mut changes).await?;
    assert_eq!(expired.id, lock.id);
    let ended = expired.ended.ok_or("the second change is the end")?;
    assert_eq!(ended.cause, LockEnd::Expired);
    assert_eq!(ended.at, lock.expires_at, "it ended at its deadline");
    assert_eq!(h.locks.get(lock.id), Some(expired));
    assert_eq!(
        h.locks.check(LockTarget::Tab(TabId(12)), Effect::Write),
        Ok(())
    );
    assert!(!h.locks.is_locked(TabId(12)));
    Ok(())
}

#[tokio::test]
async fn a_lock_that_ends_during_its_settle_is_announced_only_as_ended() -> TestResult {
    let h = harness().await?;
    let mut changes = h.locks.subscribe();
    write_in_flight(&h.sink, 12);
    let lock = h
        .locks
        .lock(TabId(12), Reads::Block, ttl(1)?, LockBinding::Host)
        .await;
    assert_eq!(
        lock.ended.map(|ended| ended.cause),
        Some(LockEnd::Expired),
        "the 1 s TTL passed during the 2 s settle"
    );
    assert!(!lock.settled);
    let only = next_change(&mut changes).await?;
    assert!(only.ended.is_some());
    assert!(
        timeout(Duration::from_millis(200), changes.recv())
            .await
            .is_err(),
        "no `taken` after its end"
    );
    Ok(())
}

#[tokio::test]
async fn closing_a_tab_ends_every_lock_on_it() -> TestResult {
    let h = harness().await?;
    let reads = h
        .locks
        .lock(TabId(5), Reads::Allow, ttl(60)?, LockBinding::Host)
        .await;
    let writes = h
        .locks
        .lock(
            TabId(5),
            Reads::Block,
            ttl(60)?,
            LockBinding::Takeover(TakeoverId::new()),
        )
        .await;
    let neighbour = h
        .locks
        .lock(TabId(6), Reads::Block, ttl(60)?, LockBinding::Host)
        .await;
    let mut changes = h.locks.subscribe();

    h.locks.tab_closed(TabId(5));
    let mut ended = vec![
        next_change(&mut changes).await?,
        next_change(&mut changes).await?,
    ];
    ended.sort_by_key(|lock| lock.id);
    assert_eq!(
        ended.iter().map(|lock| lock.id).collect::<Vec<_>>(),
        [reads.id, writes.id]
    );
    for lock in &ended {
        assert_eq!(
            lock.ended.map(|ended| ended.cause),
            Some(LockEnd::TabClosed)
        );
    }
    assert!(!h.locks.is_locked(TabId(5)));
    assert_eq!(
        h.locks.check(LockTarget::Tab(TabId(5)), Effect::Write),
        Ok(())
    );
    assert_eq!(
        h.locks.check(LockTarget::Tab(TabId(6)), Effect::Write),
        refused_by(&neighbour)
    );

    h.locks.retain_tabs(&[TabId(5)]);
    let gone = next_change(&mut changes).await?;
    assert_eq!(gone.id, neighbour.id, "tab 6 closed while nobody watched");
    assert_eq!(
        gone.ended.map(|ended| ended.cause),
        Some(LockEnd::TabClosed)
    );
    assert!(!h.locks.is_locked(TabId(6)));
    Ok(())
}

#[tokio::test]
async fn the_host_ends_its_own_locks_but_never_a_takeovers() -> TestResult {
    let h = harness().await?;
    let takeover = h
        .locks
        .lock(
            TabId(12),
            Reads::Block,
            ttl(60)?,
            LockBinding::Takeover(TakeoverId::new()),
        )
        .await;
    assert!(matches!(
        h.locks.unlock_host(takeover.id),
        Err(PartError::TakeoverLock)
    ));
    assert_eq!(
        h.locks.check(LockTarget::Tab(TabId(12)), Effect::Write),
        refused_by(&takeover),
        "the takeover's lock still holds"
    );

    let host = h
        .locks
        .lock(TabId(13), Reads::Block, ttl(60)?, LockBinding::Host)
        .await;
    let released = h.locks.unlock_host(host.id)?;
    let ended = released.ended.ok_or("released")?;
    assert_eq!(ended.cause, LockEnd::Released);
    assert_eq!(
        h.locks.check(LockTarget::Tab(TabId(13)), Effect::Write),
        Ok(())
    );
    assert_eq!(
        h.locks.unlock_host(host.id)?,
        released,
        "releasing again changes nothing"
    );
    assert!(matches!(
        h.locks.unlock_host(LockId::new()),
        Err(PartError::NotFound)
    ));
    Ok(())
}

#[tokio::test]
async fn only_its_own_takeover_ends_a_takeovers_lock() -> TestResult {
    let h = harness().await?;
    let owner = TakeoverId::new();
    let lock = h
        .locks
        .lock(
            TabId(12),
            Reads::Block,
            ttl(60)?,
            LockBinding::Takeover(owner),
        )
        .await;
    assert!(matches!(
        h.locks.unlock_takeover(lock.id, TakeoverId::new()),
        Err(PartError::TakeoverLock)
    ));
    let host = h
        .locks
        .lock(TabId(13), Reads::Block, ttl(60)?, LockBinding::Host)
        .await;
    assert!(
        matches!(
            h.locks.unlock_takeover(host.id, owner),
            Err(PartError::NotFound)
        ),
        "a takeover never ends a host's lock"
    );
    assert_eq!(
        h.locks.check(LockTarget::Tab(TabId(12)), Effect::Write),
        refused_by(&lock)
    );

    let released = h.locks.unlock_takeover(lock.id, owner)?;
    assert_eq!(
        released.ended.map(|ended| ended.cause),
        Some(LockEnd::Released)
    );
    assert_eq!(
        h.locks.check(LockTarget::Tab(TabId(12)), Effect::Write),
        Ok(())
    );
    assert_eq!(
        h.locks.unlock_takeover(lock.id, owner)?,
        released,
        "releasing again changes nothing"
    );
    Ok(())
}

#[tokio::test]
async fn a_past_instant_is_judged_by_the_locks_that_covered_it_held_or_ended() -> TestResult {
    let h = harness().await?;
    let (tab, other) = (LockTarget::Tab(TabId(12)), LockTarget::Tab(TabId(13)));
    let taken = h
        .locks
        .lock(TabId(12), Reads::Block, ttl(60)?, LockBinding::Host)
        .await;
    let released = h.locks.unlock_host(taken.id)?;
    let end = released.ended.ok_or("released")?.at;
    assert_eq!(
        h.locks.check(tab, Effect::Write),
        Ok(()),
        "nothing is refused now"
    );
    for at in [released.created_at, end] {
        assert_eq!(
            h.locks.check_at(tab, Effect::Write, at),
            refused_by(&released),
            "the ended lock covered {}",
            at.millis()
        );
    }
    assert_eq!(
        h.locks.check_at(tab, Effect::Write, end.plus_millis(1)),
        Ok(()),
        "the release, not the TTL, ended it"
    );

    let held = h
        .locks
        .lock(TabId(13), Reads::Allow, ttl(60)?, LockBinding::Host)
        .await;
    assert_eq!(
        h.locks.check_at(other, Effect::Write, held.expires_at),
        refused_by(&held),
        "a held lock covers through its expires_at"
    );
    assert_eq!(
        h.locks.check_at(other, Effect::Read, held.expires_at),
        Ok(()),
        "reads: allow"
    );
    assert_eq!(
        h.locks
            .check_at(other, Effect::Write, held.created_at.plus_millis(-1)),
        Ok(())
    );
    Ok(())
}
