//! Screenshot jobs, completeness and storage through the crate's public API, on a real SQLite
//! file. The test plays the camera: it reads the writer's jobs, writes JPEGs where the jobs say,
//! and answers.

use browseros_core::TabId;
use neo_trace::{
    Delivery, Effect, EpochMillis, ErrorOwner, ErrorRecord, OperationEnded, OperationId,
    OperationKind, OperationStarted, Outcome, PageState, RedactionLimits, Redactor,
    RetentionPolicy, RunEnded, RunId, RunStarted, RunStatus, Shot, ShotJob, ShotJobs, ShotRead,
    ShotState, TraceRecordEvent, TraceSink, TraceStore, WriterConfig, screenshot_queue,
    spawn_writer,
};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::broadcast;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Harness {
    dir: tempfile::TempDir,
    store: TraceStore,
    sink: TraceSink,
    jobs: ShotJobs,
}

async fn harness() -> Result<Harness, Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let store = TraceStore::open(&dir.path().join("claw-sdk/trace.db")).await?;
    let (queue, jobs) = screenshot_queue();
    let sink = spawn_writer(&store, redactor(), WriterConfig::default(), Some(queue));
    Ok(Harness {
        dir,
        store,
        sink,
        jobs,
    })
}

fn redactor() -> Redactor {
    Redactor::new(RedactionLimits::default())
}

fn at(millis: i64) -> EpochMillis {
    EpochMillis::from_millis(1_800_000_000_000 + millis)
}

const DAY: i64 = 24 * 60 * 60 * 1000;

fn start_run(sink: &TraceSink, millis: i64) -> RunId {
    let run = RunId::new();
    sink.run_started(RunStarted {
        id: run,
        code: "x".into(),
        request_id: None,
        task_id: None,
        host_data: None,
        at: at(millis),
    });
    run
}

fn end_run(sink: &TraceSink, run: RunId, millis: i64) {
    sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Ok,
        error: None,
        result: None,
        replay: None,
        at: at(millis),
    });
}

/// Records one operation, start to end: `effect` on `tab`, failing with `owner` when given.
fn operation(
    sink: &TraceSink,
    id: OperationId,
    effect: Effect,
    tab: Option<i64>,
    owner: Option<ErrorOwner>,
) {
    sink.op_started(OperationStarted {
        site: None,
        id,
        parent: None,
        name: "locator.click".into(),
        kind: OperationKind::Builtin,
        effect,
        tab: tab.map(TabId),
        args: serde_json::Value::Null,
        at: at(i64::from(id.seq)),
    });
    sink.op_ended(OperationEnded {
        id,
        outcome: if owner.is_some() {
            Outcome::Error
        } else {
            Outcome::Ok
        },
        delivery: Delivery::Done,
        sent_at: None,
        result: None,
        error: owner.map(|owner| ErrorRecord {
            owner,
            code: "no_match".into(),
            message: "nothing matched".into(),
            host_code: None,
            retryable: false,
            operation_id: None,
        }),
        page_after: None,
        main_world: false,
        frames_incomplete: false,
        at: at(i64::from(id.seq) + 1),
    });
}

async fn next_job(jobs: &mut ShotJobs) -> Result<ShotJob, Box<dyn std::error::Error>> {
    Ok(tokio::time::timeout(Duration::from_secs(5), jobs.recv())
        .await?
        .ok_or("the writer stopped")?)
}

/// The jobs waiting now, after a flush has sent everything committed.
async fn waiting_jobs(h: &mut Harness) -> Result<Vec<ShotJob>, Box<dyn std::error::Error>> {
    h.sink.flush().await?;
    let mut jobs = Vec::new();
    while let Some(job) = h.jobs.try_recv() {
        jobs.push(job);
    }
    Ok(jobs)
}

/// What the camera does with a picture: writes it where the job says, then answers `Taken`.
async fn take(job: ShotJob, jpeg: &[u8]) -> TestResult {
    if let Some(dir) = job.path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    tokio::fs::write(&job.path, jpeg).await?;
    job.answer.send(Shot::Taken {
        bytes_len: u64::try_from(jpeg.len())?,
    });
    Ok(())
}

/// Waits until `run` is announced complete.
async fn completed(events: &mut broadcast::Receiver<TraceRecordEvent>, run: RunId) -> TestResult {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(TraceRecordEvent::RunComplete { run: done }) if done == run => return Ok(()),
                Ok(_) => {}
                Err(error) => return Err(error),
            }
        }
    })
    .await??;
    Ok(())
}

async fn is_complete(store: &TraceStore, run: RunId) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(store.run_summary(run).await?.ok_or("no such run")?.complete)
}

fn jobs_of(jobs: &[ShotJob]) -> Vec<(u32, TabId)> {
    jobs.iter().map(|job| (job.op.seq, job.tab)).collect()
}

#[tokio::test]
async fn writes_and_page_failures_get_a_job_and_each_touched_tab_one_at_run_end() -> TestResult {
    let mut h = harness().await?;
    let run = start_run(&h.sink, 0);
    let op = |seq| OperationId::new(run, seq);
    operation(&h.sink, op(1), Effect::Write, Some(3), None);
    operation(&h.sink, op(2), Effect::Read, Some(3), None);
    operation(
        &h.sink,
        op(3),
        Effect::Read,
        Some(4),
        Some(ErrorOwner::Website),
    );
    operation(
        &h.sink,
        op(4),
        Effect::Read,
        Some(4),
        Some(ErrorOwner::Host),
    );
    operation(&h.sink, op(5), Effect::Write, None, None);
    operation(&h.sink, op(6), Effect::Write, Some(5), None);
    let after = waiting_jobs(&mut h).await?;
    assert_eq!(
        jobs_of(&after),
        vec![(1, TabId(3)), (3, TabId(4)), (6, TabId(5))],
        "a write on a tab and a website failure on a tab"
    );
    // Answered before the run ends, so no job is out on a tab to fold its run-end shot into.
    for job in after {
        job.answer.send(Shot::Dropped);
    }
    end_run(&h.sink, run, 100);
    let at_end = waiting_jobs(&mut h).await?;
    assert_eq!(
        jobs_of(&at_end),
        vec![(2, TabId(3)), (4, TabId(4))],
        "tab 5's last operation already has one"
    );
    let root = h.dir.path().join("claw-sdk/screenshots");
    assert_eq!(
        at_end
            .iter()
            .map(|job| job.path.clone())
            .collect::<Vec<_>>(),
        vec![
            root.join(format!("{run}/2.jpg")),
            root.join(format!("{run}/4.jpg"))
        ]
    );
    Ok(())
}

/// Records one operation named `name`, start to end: `effect` on `tab`, answered ok.
fn named(sink: &TraceSink, id: OperationId, name: &str, effect: Effect, tab: i64) {
    sink.op_started(OperationStarted {
        site: None,
        id,
        parent: None,
        name: name.into(),
        kind: OperationKind::Builtin,
        effect,
        tab: Some(TabId(tab)),
        args: serde_json::Value::Null,
        at: at(i64::from(id.seq)),
    });
    sink.op_ended(OperationEnded {
        id,
        outcome: Outcome::Ok,
        delivery: Delivery::Done,
        sent_at: None,
        result: None,
        error: None,
        page_after: None,
        main_world: false,
        frames_incomplete: false,
        at: at(i64::from(id.seq) + 1),
    });
}

#[tokio::test]
async fn neos_own_listeners_make_no_tab_the_runs_for_its_run_end_shots() -> TestResult {
    // A task tab the script acts on, and three more open tabs it never touches. The prelude's
    // page.on (events.listen) and event pump (events.next) name every known page.
    let mut h = harness().await?;
    let run = start_run(&h.sink, 0);
    let op = |seq| OperationId::new(run, seq);
    for (seq, tab) in [(1, 4), (2, 5), (3, 6)] {
        named(&h.sink, op(seq), "events.listen", Effect::Read, tab);
    }
    named(&h.sink, op(4), "locator.click", Effect::Write, 3);
    named(&h.sink, op(5), "locator.textContent", Effect::Read, 3);
    named(&h.sink, op(6), "events.listen", Effect::Read, 3);
    named(&h.sink, op(7), "events.next", Effect::Read, 5);
    // A tab the script opened and left alone is the run's, so it gets its picture too.
    h.sink.op_started(OperationStarted {
        site: None,
        id: op(8),
        parent: None,
        name: "context.newPage".into(),
        kind: OperationKind::Builtin,
        effect: Effect::Write,
        tab: None,
        args: serde_json::Value::Null,
        at: at(8),
    });
    h.sink.op_ended(OperationEnded {
        id: op(8),
        outcome: Outcome::Ok,
        delivery: Delivery::Done,
        sent_at: None,
        result: None,
        error: None,
        page_after: Some(PageState {
            tab_id: Some(TabId(7)),
            url: Some("about:blank".into()),
            title: None,
        }),
        main_world: false,
        frames_incomplete: false,
        at: at(9),
    });
    let after = waiting_jobs(&mut h).await?;
    assert_eq!(
        jobs_of(&after),
        vec![(4, TabId(3))],
        "the click on the task tab"
    );
    for job in after {
        job.answer.send(Shot::Dropped);
    }
    end_run(&h.sink, run, 100);
    let at_end = waiting_jobs(&mut h).await?;
    // The task tab's last operation of the script's (the read) gets the run-end shot; a listener
    // after it does not move it, and the untouched tabs get none: the same tabs as the run's.
    assert_eq!(jobs_of(&at_end), vec![(5, TabId(3)), (8, TabId(7))]);
    let summary = h.store.run_summary(run).await?.ok_or("no such run")?;
    assert_eq!(summary.tabs, vec![TabId(3), TabId(7)]);
    Ok(())
}

#[tokio::test]
async fn a_tab_with_a_job_out_when_its_run_ends_gets_no_run_end_shot() -> TestResult {
    let mut h = harness().await?;
    let mut events = h.sink.subscribe();
    let run = start_run(&h.sink, 0);
    let op = |seq| OperationId::new(run, seq);
    operation(&h.sink, op(1), Effect::Write, Some(3), None);
    operation(&h.sink, op(2), Effect::Read, Some(3), None);
    operation(&h.sink, op(3), Effect::Write, Some(4), None);
    operation(&h.sink, op(4), Effect::Read, Some(4), None);
    let mut after = waiting_jobs(&mut h).await?.into_iter();
    let (Some(on_3), Some(on_4)) = (after.next(), after.next()) else {
        return Err("a job per write expected".into());
    };
    take(on_4, b"\xff\xd8 tab 4 \xff\xd9").await?;
    end_run(&h.sink, run, 100);
    let at_end = waiting_jobs(&mut h).await?;
    assert_eq!(
        jobs_of(&at_end),
        vec![(4, TabId(4))],
        "tab 3's write job is out, and pictures the tab as the run left it"
    );
    assert!(!is_complete(&h.store, run).await?, "two jobs out");

    take(on_3, b"\xff\xd8 tab 3 \xff\xd9").await?;
    for job in at_end {
        take(job, b"\xff\xd8 tab 4 at the end \xff\xd9").await?;
    }
    completed(&mut events, run).await?;
    assert_eq!(h.store.screenshot(op(2)).await?, ShotRead::None);
    assert_eq!(
        h.store.screenshot(op(1)).await?,
        ShotRead::Jpeg(b"\xff\xd8 tab 3 \xff\xd9".to_vec())
    );
    Ok(())
}

#[tokio::test]
async fn a_run_is_complete_only_once_every_job_is_answered() -> TestResult {
    let mut h = harness().await?;
    let mut events = h.sink.subscribe();
    let run = start_run(&h.sink, 0);
    let click = OperationId::new(run, 1);
    let title = OperationId::new(run, 2);
    operation(&h.sink, click, Effect::Write, Some(3), None);
    operation(&h.sink, title, Effect::Read, Some(4), None);
    end_run(&h.sink, run, 100);
    let mut jobs = waiting_jobs(&mut h).await?.into_iter();
    let (Some(after_click), Some(at_end)) = (jobs.next(), jobs.next()) else {
        return Err("two jobs expected".into());
    };
    let summary = h.store.run_summary(run).await?.ok_or("no run")?;
    assert_eq!(summary.status, RunStatus::Ok);
    assert!(!summary.complete, "ended, with two jobs out");

    take(after_click, b"\xff\xd8 click \xff\xd9").await?;
    h.sink.flush().await?;
    assert!(!is_complete(&h.store, run).await?, "one job still out");

    take(at_end, b"\xff\xd8 title \xff\xd9").await?;
    completed(&mut events, run).await?;
    assert!(is_complete(&h.store, run).await?);
    assert_eq!(
        h.store.screenshot(click).await?,
        ShotRead::Jpeg(b"\xff\xd8 click \xff\xd9".to_vec())
    );
    assert_eq!(
        h.store.screenshot(title).await?,
        ShotRead::Jpeg(b"\xff\xd8 title \xff\xd9".to_vec())
    );
    let detail = h.store.run(run).await?.ok_or("no run")?;
    assert_eq!(
        detail
            .operations
            .iter()
            .map(|op| op.screenshot)
            .collect::<Vec<_>>(),
        vec![Some(ShotState::Stored), Some(ShotState::Stored)]
    );
    Ok(())
}

#[tokio::test]
async fn an_answer_that_reaches_the_writer_with_its_runs_end_completes_the_run() -> TestResult {
    let mut h = harness().await?;
    let mut events = h.sink.subscribe();
    let run = start_run(&h.sink, 0);
    operation(
        &h.sink,
        OperationId::new(run, 1),
        Effect::Write,
        Some(3),
        None,
    );
    let job = next_job(&mut h.jobs).await?;
    // Both are queued before the writer runs again, so one batch counts the answer and ends the
    // run.
    job.answer.send(Shot::Dropped);
    end_run(&h.sink, run, 100);
    completed(&mut events, run).await?;
    assert!(is_complete(&h.store, run).await?);
    Ok(())
}

#[tokio::test]
async fn an_operation_after_its_run_ended_never_holds_the_run_incomplete() -> TestResult {
    let mut h = harness().await?;
    let mut events = h.sink.subscribe();
    let run = start_run(&h.sink, 0);
    operation(
        &h.sink,
        OperationId::new(run, 1),
        Effect::Write,
        Some(3),
        None,
    );
    end_run(&h.sink, run, 100);
    let job = next_job(&mut h.jobs).await?;
    operation(
        &h.sink,
        OperationId::new(run, 2),
        Effect::Write,
        Some(3),
        None,
    );
    assert!(
        waiting_jobs(&mut h).await?.is_empty(),
        "no job for a run that has ended"
    );
    take(job, b"\xff\xd8\xff\xd9").await?;
    completed(&mut events, run).await?;
    Ok(())
}

#[tokio::test]
async fn without_a_camera_a_run_is_complete_when_it_ends() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = TraceStore::open(&dir.path().join("trace.db")).await?;
    let sink = spawn_writer(&store, redactor(), WriterConfig::default(), None);
    let mut events = sink.subscribe();
    let run = start_run(&sink, 0);
    operation(
        &sink,
        OperationId::new(run, 1),
        Effect::Write,
        Some(3),
        None,
    );
    sink.flush().await?;
    assert!(
        !is_complete(&store, run).await?,
        "a running run is never complete"
    );
    end_run(&sink, run, 100);
    completed(&mut events, run).await?;
    assert!(is_complete(&store, run).await?);
    assert_eq!(
        store.screenshot(OperationId::new(run, 1)).await?,
        ShotRead::None
    );
    Ok(())
}

#[tokio::test]
async fn a_job_dropped_unanswered_counts_as_dropped() -> TestResult {
    let mut h = harness().await?;
    let mut events = h.sink.subscribe();
    let run = start_run(&h.sink, 0);
    let click = OperationId::new(run, 1);
    operation(&h.sink, click, Effect::Write, Some(3), None);
    end_run(&h.sink, run, 100);
    let job = next_job(&mut h.jobs).await?;
    drop(job);
    completed(&mut events, run).await?;
    assert_eq!(h.store.screenshot(click).await?, ShotRead::None);
    Ok(())
}

#[tokio::test]
async fn jobs_for_a_camera_that_is_gone_are_dropped() -> TestResult {
    let Harness {
        dir: _dir,
        store,
        sink,
        jobs,
    } = harness().await?;
    drop(jobs);
    let mut events = sink.subscribe();
    let run = start_run(&sink, 0);
    operation(
        &sink,
        OperationId::new(run, 1),
        Effect::Write,
        Some(3),
        None,
    );
    end_run(&sink, run, 100);
    completed(&mut events, run).await?;
    assert!(is_complete(&store, run).await?);
    Ok(())
}

#[tokio::test]
async fn reopening_completes_every_run_and_keeps_no_pending_shot() -> TestResult {
    let mut h = harness().await?;
    let ended = start_run(&h.sink, 0);
    operation(
        &h.sink,
        OperationId::new(ended, 1),
        Effect::Write,
        Some(3),
        None,
    );
    end_run(&h.sink, ended, 100);
    let running = start_run(&h.sink, 200);
    operation(
        &h.sink,
        OperationId::new(running, 1),
        Effect::Write,
        Some(4),
        None,
    );
    let unanswered = waiting_jobs(&mut h).await?;
    assert_eq!(unanswered.len(), 2);
    assert!(!is_complete(&h.store, ended).await?);

    // The process dies with both jobs unanswered.
    let reopened = TraceStore::open(&h.dir.path().join("claw-sdk/trace.db")).await?;
    assert!(is_complete(&reopened, ended).await?);
    let interrupted = reopened.run_summary(running).await?.ok_or("no run")?;
    assert_eq!(interrupted.status, RunStatus::Interrupted);
    assert!(interrupted.complete);
    assert_eq!(
        reopened.screenshot(OperationId::new(ended, 1)).await?,
        ShotRead::None
    );
    drop(unanswered);
    Ok(())
}

#[tokio::test]
async fn a_stored_screenshot_whose_file_is_gone_reads_as_expired() -> TestResult {
    let mut h = harness().await?;
    let run = start_run(&h.sink, 0);
    let click = OperationId::new(run, 1);
    operation(&h.sink, click, Effect::Write, Some(3), None);
    let job = next_job(&mut h.jobs).await?;
    job.answer.send(Shot::Taken { bytes_len: 10 });
    h.sink.flush().await?;
    assert_eq!(h.store.screenshot(click).await?, ShotRead::Expired);
    Ok(())
}

/// One finished run started at `millis`, with one stored screenshot of `bytes` bytes.
async fn run_with_a_screenshot(
    h: &mut Harness,
    millis: i64,
    bytes: usize,
) -> Result<(RunId, PathBuf), Box<dyn std::error::Error>> {
    let run = start_run(&h.sink, millis);
    operation(
        &h.sink,
        OperationId::new(run, 1),
        Effect::Write,
        Some(3),
        None,
    );
    end_run(&h.sink, run, millis + 100);
    let job = next_job(&mut h.jobs).await?;
    let path = job.path.clone();
    take(job, &vec![0xab; bytes]).await?;
    h.sink.flush().await?;
    Ok((run, path))
}

#[tokio::test]
async fn the_sweep_deletes_a_swept_runs_screenshots() -> TestResult {
    let mut h = harness().await?;
    let (old, old_file) = run_with_a_screenshot(&mut h, 0, 10).await?;
    let (recent, recent_file) = run_with_a_screenshot(&mut h, 8 * DAY, 10).await?;
    let swept = h
        .sink
        .sweep(RetentionPolicy::default(), at(8 * DAY + 1))
        .await?;
    assert_eq!((swept.runs, swept.screenshots), (1, 1));
    assert!(h.store.run(old).await?.is_none());
    assert!(!old_file.exists(), "the swept run's file is gone");
    assert!(
        old_file.parent().is_some_and(|dir| !dir.exists()),
        "and its directory"
    );
    assert!(recent_file.exists());
    assert_eq!(
        h.store.screenshot(OperationId::new(recent, 1)).await?,
        ShotRead::Jpeg(vec![0xab; 10])
    );
    Ok(())
}

#[tokio::test]
async fn over_the_cap_the_oldest_runs_screenshots_go_first() -> TestResult {
    let mut h = harness().await?;
    // The oldest run is still waiting for an answer, so the cap passes over it.
    let waiting = start_run(&h.sink, 0);
    let waiting_op = OperationId::new(waiting, 1);
    operation(&h.sink, waiting_op, Effect::Write, Some(3), None);
    operation(
        &h.sink,
        OperationId::new(waiting, 2),
        Effect::Write,
        Some(3),
        None,
    );
    let first = next_job(&mut h.jobs).await?;
    let unanswered = next_job(&mut h.jobs).await?;
    take(first, &[0xcd; 100]).await?;
    end_run(&h.sink, waiting, 50);
    let (a, a_file) = run_with_a_screenshot(&mut h, 10, 100).await?;
    let (b, b_file) = run_with_a_screenshot(&mut h, 20, 100).await?;
    let (c, c_file) = run_with_a_screenshot(&mut h, 30, 100).await?;

    let swept = h
        .sink
        .sweep(
            RetentionPolicy {
                screenshot_cap: 200,
                ..RetentionPolicy::default()
            },
            at(40),
        )
        .await?;
    assert_eq!((swept.runs, swept.screenshots), (0, 2));
    for (run, file, read) in [
        (a, &a_file, ShotRead::Expired),
        (b, &b_file, ShotRead::Expired),
        (c, &c_file, ShotRead::Jpeg(vec![0xab; 100])),
    ] {
        assert_eq!(
            h.store.screenshot(OperationId::new(run, 1)).await?,
            read,
            "{run}"
        );
        assert_eq!(file.exists(), read != ShotRead::Expired, "{run}");
    }
    let a_op = h
        .store
        .operation(OperationId::new(a, 1))
        .await?
        .ok_or("no op")?;
    assert_eq!(a_op.screenshot, Some(ShotState::Expired));
    assert!(
        matches!(h.store.screenshot(waiting_op).await?, ShotRead::Jpeg(_)),
        "a run still waiting for answers keeps its screenshots"
    );
    drop(unanswered);
    Ok(())
}

/// Sets `dir`'s mode. Read and search only (`0o500`) stops anything in it being deleted, as
/// when another user owns the files. Root ignores the mode, so the tests that use it need an
/// ordinary user, as CI has.
#[cfg(unix)]
fn set_mode(dir: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))
}

#[cfg(unix)]
#[tokio::test]
async fn a_run_whose_files_cannot_be_deleted_keeps_its_rows_for_the_next_sweep() -> TestResult {
    let mut h = harness().await?;
    let (old, old_file) = run_with_a_screenshot(&mut h, 0, 10).await?;
    let old_dir = old_file.parent().ok_or("no run directory")?.to_path_buf();
    set_mode(&old_dir, 0o500)?;
    let stuck = h
        .sink
        .sweep(RetentionPolicy::default(), at(8 * DAY + 1))
        .await;
    set_mode(&old_dir, 0o700)?;
    let stuck = stuck?;
    assert_eq!((stuck.runs, stuck.screenshots), (0, 0));
    assert_eq!(
        h.store.screenshot(OperationId::new(old, 1)).await?,
        ShotRead::Jpeg(vec![0xab; 10]),
        "the run keeps its rows and its file"
    );

    let swept = h
        .sink
        .sweep(RetentionPolicy::default(), at(8 * DAY + 1))
        .await?;
    assert_eq!((swept.runs, swept.screenshots), (1, 1));
    assert!(h.store.run(old).await?.is_none());
    assert!(!old_dir.exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn over_the_cap_a_run_whose_files_cannot_be_deleted_still_counts() -> TestResult {
    let mut h = harness().await?;
    let (a, a_file) = run_with_a_screenshot(&mut h, 10, 100).await?;
    let (b, _) = run_with_a_screenshot(&mut h, 20, 100).await?;
    let (c, _) = run_with_a_screenshot(&mut h, 30, 100).await?;
    let a_dir = a_file.parent().ok_or("no run directory")?.to_path_buf();
    let cap = |bytes| RetentionPolicy {
        screenshot_cap: bytes,
        ..RetentionPolicy::default()
    };
    set_mode(&a_dir, 0o500)?;
    let capped = h.sink.sweep(cap(200), at(40)).await;
    set_mode(&a_dir, 0o700)?;
    assert_eq!(capped?.screenshots, 1);
    for (run, read) in [
        (a, ShotRead::Jpeg(vec![0xab; 100])),
        (b, ShotRead::Expired),
        (c, ShotRead::Jpeg(vec![0xab; 100])),
    ] {
        assert_eq!(
            h.store.screenshot(OperationId::new(run, 1)).await?,
            read,
            "the oldest run's files stay and still count, so the cap took the next run's"
        );
    }

    let capped = h.sink.sweep(cap(100), at(40)).await?;
    assert_eq!(capped.screenshots, 1);
    assert_eq!(
        h.store.screenshot(OperationId::new(a, 1)).await?,
        ShotRead::Expired
    );
    assert!(!a_file.exists());
    assert_eq!(
        h.store.screenshot(OperationId::new(c, 1)).await?,
        ShotRead::Jpeg(vec![0xab; 100])
    );
    Ok(())
}

/// The host's `trace.screenshots: off` reaches the writer as a policy in its own queue: from the
/// next operation that ends there is no job, after an operation or at a run's end, and the run is
/// complete when it ends. A job made before the change is still taken. `perOperation` brings
/// them back.
#[tokio::test]
async fn with_screenshots_off_no_job_is_made_from_the_next_operation() -> TestResult {
    use neo_trace::{RecordPolicy, Screenshots};

    let mut h = harness().await?;
    let mut events = h.sink.subscribe();
    let run = start_run(&h.sink, 0);
    let op = |seq| OperationId::new(run, seq);
    operation(&h.sink, op(1), Effect::Write, Some(3), None);
    h.sink.record_policy(RecordPolicy {
        screenshots: Screenshots::Off,
        ..RecordPolicy::default()
    });
    operation(&h.sink, op(2), Effect::Write, Some(4), None);
    operation(
        &h.sink,
        op(3),
        Effect::Read,
        Some(4),
        Some(ErrorOwner::Website),
    );
    let jobs = waiting_jobs(&mut h).await?;
    assert_eq!(
        jobs_of(&jobs),
        vec![(1, TabId(3))],
        "the job made before the change only"
    );
    for job in jobs {
        take(job, b"jpeg").await?;
    }
    end_run(&h.sink, run, 100);
    assert!(
        waiting_jobs(&mut h).await?.is_empty(),
        "no run-end shot of tab 4 either"
    );
    completed(&mut events, run).await?;

    h.sink.record_policy(RecordPolicy::default());
    let again = start_run(&h.sink, 200);
    operation(
        &h.sink,
        OperationId::new(again, 1),
        Effect::Write,
        Some(3),
        None,
    );
    assert_eq!(
        jobs_of(&waiting_jobs(&mut h).await?),
        vec![(1, TabId(3))],
        "perOperation takes them again"
    );
    Ok(())
}

/// The host's private typing (`OperationKind::Protected`, `host.fill`) is never pictured: its
/// operation gets no job, failing or not, and a tab it typed into gets no run-end shot from its
/// run, whatever ran there after it (protected-fill design, section 8). A write of the same run on
/// another tab is pictured as usual, so the rule cannot pass by taking no picture at all.
#[tokio::test]
async fn a_protected_operation_and_its_tab_get_no_picture() -> TestResult {
    let mut h = harness().await?;
    let run = start_run(&h.sink, 0);
    let op = |seq| OperationId::new(run, seq);
    for (seq, outcome, owner) in [
        (1, Outcome::Ok, None),
        (2, Outcome::Error, Some(ErrorOwner::Website)),
    ] {
        h.sink.op_started(OperationStarted {
            site: None,
            id: op(seq),
            parent: None,
            name: "host.fill".into(),
            kind: OperationKind::Protected,
            effect: Effect::Write,
            tab: Some(TabId(3)),
            args: serde_json::json!({ "fields": [{ "ref": "e5", "characters": 8 }] }),
            at: at(i64::from(seq)),
        });
        h.sink.op_ended(OperationEnded {
            id: op(seq),
            outcome,
            delivery: Delivery::Done,
            sent_at: None,
            result: None,
            error: owner.map(|owner| ErrorRecord {
                owner,
                code: "focus_lost".into(),
                message: "The focus left the box during the entry.".into(),
                host_code: None,
                retryable: false,
                operation_id: None,
            }),
            page_after: Some(PageState {
                tab_id: Some(TabId(3)),
                url: None,
                title: None,
            }),
            main_world: false,
            frames_incomplete: false,
            at: at(i64::from(seq) + 1),
        });
    }
    // A later read on the typed tab, and a write on another tab.
    named(&h.sink, op(3), "locator.textContent", Effect::Read, 3);
    named(&h.sink, op(4), "locator.click", Effect::Write, 4);
    let after = waiting_jobs(&mut h).await?;
    assert_eq!(
        jobs_of(&after),
        vec![(4, TabId(4))],
        "only the other tab's write"
    );
    for job in after {
        job.answer.send(Shot::Dropped);
    }
    end_run(&h.sink, run, 100);
    let at_end = waiting_jobs(&mut h).await?;
    assert!(
        jobs_of(&at_end).iter().all(|(_, tab)| *tab != TabId(3)),
        "no run-end picture of the typed tab: {:?}",
        jobs_of(&at_end)
    );
    let operation = h.store.operation(op(1)).await?.ok_or("no such operation")?;
    assert_eq!(operation.kind, OperationKind::Protected);
    Ok(())
}
