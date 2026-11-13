//! Salvaging a trace file that would not open, on real SQLite files the real writer wrote: how
//! a file that will not open fails, what reaches the fresh store from a readable file and from
//! damaged ones (moved without its log, with a log that cannot be read, a log restarted over
//! older frames, a log whose header or a committed frame fails its checksum, cut short, garbage
//! over the header, a damaged runs table, a run id that does not parse, an older schema), and
//! what the report says each time.

use browseros_core::TabId;
use neo_trace::{
    Delivery, Effect, EpochMillis, IdError, Loss, OperationEnded, OperationId, OperationKind,
    OperationStarted, Outcome, Recovery, RedactionLimits, Redactor, RequestId, RunEffects,
    RunEnded, RunId, RunStarted, RunStatus, Salvage, TaskId, TraceError, TraceSink, TraceStore,
    WriterConfig,
};
use sea_orm::sqlx;
use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{self, SeekFrom};
use std::num::TryFromIntError;
use std::path::{Path, PathBuf};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

/// Where a write-ahead log's header keeps its own checksum, in its last 8 bytes.
const LOG_HEADER_CHECKSUM: u64 = 24;

/// Where the log's first frame carries its page: after the log's 32-byte header and the frame's
/// own 24-byte header.
const FIRST_FRAME_PAGE: u64 = 32 + 24;

fn at(millis: i64) -> EpochMillis {
    EpochMillis::from_millis(1_800_000_000_000 + millis)
}

fn request(name: &str) -> Result<RequestId, IdError> {
    RequestId::parse(name)
}

fn task(name: &str) -> Result<TaskId, IdError> {
    TaskId::parse(name)
}

/// The reply a repeat of `call-1` gets while its window lasts.
fn reply() -> Value {
    json!({"content": [{"type": "text", "text": "Done"}]})
}

fn writer(store: &TraceStore) -> TraceSink {
    neo_trace::spawn_writer(
        store,
        Redactor::new(RedactionLimits::default()),
        WriterConfig::default(),
        None,
    )
}

/// A Neo still running over its trace, with three runs.
struct Old {
    path: PathBuf,
    /// Keep the store open, so its latest commits stay in the write-ahead log.
    _store: TraceStore,
    sink: TraceSink,
    /// `call-1` in task `checkout`: ended ok at 1 000 with a reply to replay.
    ended: RunId,
    /// `call-2` with no task: still running when Neo stopped.
    running: RunId,
    /// No request id, so nothing to salvage; its operations fill pages past the schema.
    untracked: RunId,
}

fn start(
    sink: &TraceSink,
    request_id: Option<&str>,
    task_id: Option<&str>,
    millis: i64,
) -> Result<RunId, IdError> {
    let run = RunId::new();
    sink.run_started(RunStarted {
        id: run,
        code: "await page.goto('https://x.example')".into(),
        request_id: request_id.map(request).transpose()?,
        task_id: task_id.map(task).transpose()?,
        host_data: None,
        at: at(millis),
    });
    Ok(run)
}

async fn old_trace(dir: &Path) -> TestResult<Old> {
    let path = dir.join("old/trace.db");
    let store = TraceStore::open(&path).await?;
    let sink = writer(&store);
    let ended = start(&sink, Some("call-1"), Some("checkout"), 0)?;
    sink.run_ended(RunEnded {
        id: ended,
        status: RunStatus::Ok,
        error: None,
        result: Some(json!({"title": "Done"})),
        replay: Some(reply()),
        at: at(1_000),
    });
    let running = start(&sink, Some("call-2"), None, 2_000)?;
    let untracked = start(&sink, None, None, 3_000)?;
    // Forty ended operations of a run still running: their inputs, stored with their ends, fill
    // the pages after the schema.
    for seq in 1..=40 {
        sink.op_started(OperationStarted {
            site: None,
            id: OperationId::new(untracked, seq),
            parent: None,
            name: "page.title".into(),
            kind: OperationKind::Builtin,
            effect: Effect::Read,
            tab: Some(TabId(4)),
            args: json!({"note": "x".repeat(300)}),
            at: at(3_000 + i64::from(seq)),
        });
        sink.op_ended(OperationEnded {
            id: OperationId::new(untracked, seq),
            outcome: Outcome::Ok,
            delivery: Delivery::Done,
            sent_at: None,
            result: None,
            error: None,
            page_after: None,
            main_world: false,
            frames_incomplete: false,
            at: at(3_000 + i64::from(seq)),
        });
    }
    sink.flush().await?;
    Ok(Old {
        path,
        _store: store,
        sink,
        ended,
        running,
        untracked,
    })
}

/// Checkpoints every commit `old` made into its file, so its writer restarts the log, then
/// starts `call-3`: that commit is only in the log's first frames, and
/// frames left from before the restart follow them.
async fn restart_the_log(old: &Old) -> TestResult {
    use sea_orm::sqlx::Connection;

    let log = beside(&old.path, "-wal");
    let mut conn =
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&old.path)).await?;
    let (busy, _, _): (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(RESTART)")
        .fetch_one(&mut conn)
        .await?;
    conn.close().await?;
    assert_eq!(
        busy, 0,
        "no reader held the log, so the next write restarts it"
    );
    let before = fs::metadata(&log)?.len();
    start(&old.sink, Some("call-3"), None, 4_000)?;
    old.sink.flush().await?;
    assert_eq!(
        fs::metadata(&log)?.len(),
        before,
        "the restart wrote over the log's first frames, so frames from before it follow them"
    );
    Ok(())
}

fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The file and its write-ahead log as a crash leaves them, moved aside to `aside`: the store is
/// still open, so the latest commits are only in the log. SQLite rebuilds the log's index.
fn crash_copy(old: &Path, aside: &Path) -> TestResult {
    fs::copy(old, aside)?;
    fs::copy(beside(old, "-wal"), beside(aside, "-wal"))?;
    Ok(())
}

/// A crash copy at `aside` with its log folded into the file, so the file alone holds every
/// commit and damage to the file is damage to the data.
async fn folded_copy(old: &Path, aside: &Path) -> TestResult {
    use sea_orm::sqlx::Connection;

    crash_copy(old, aside)?;
    let mut conn =
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(aside)).await?;
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    for suffix in ["-wal", "-shm"] {
        match fs::remove_file(beside(aside, suffix)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Where a file's tables start, read before it is damaged.
struct Layout {
    page_size: u64,
    pages: u64,
    roots: HashMap<String, u64>,
}

impl Layout {
    async fn of(path: &Path) -> TestResult<Self> {
        use sea_orm::sqlx::Connection;

        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .immutable(true);
        let mut conn = SqliteConnection::connect_with(&options).await?;
        let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
            .fetch_one(&mut conn)
            .await?;
        let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(&mut conn)
            .await?;
        let roots: Vec<(String, i64)> =
            sqlx::query_as("SELECT name, rootpage FROM sqlite_schema WHERE type = 'table'")
                .fetch_all(&mut conn)
                .await?;
        conn.close().await?;
        Ok(Self {
            page_size: u64::try_from(page_size)?,
            pages: u64::try_from(pages)?,
            roots: roots
                .into_iter()
                .map(|(name, root)| Ok((name, u64::try_from(root)?)))
                .collect::<Result<_, TryFromIntError>>()?,
        })
    }

    /// The byte offset of `table`'s root page.
    fn root(&self, table: &str) -> TestResult<u64> {
        let page = self
            .roots
            .get(table)
            .ok_or_else(|| format!("no table {table}"))?;
        Ok((page - 1) * self.page_size)
    }
}

/// Writes garbage over `len` bytes of `path` at `offset`.
/// Cuts the last `bytes` off the file at `path`, as a crash in the middle of a write leaves it.
fn cut_short(path: &Path, bytes: u64) -> TestResult {
    let file = OpenOptions::new().write(true).open(path)?;
    let len = file.metadata()?.len();
    file.set_len(len - bytes)?;
    Ok(())
}

fn overwrite(path: &Path, offset: u64, len: u64) -> TestResult {
    use std::io::{Seek, Write};

    let mut file = OpenOptions::new().write(true).open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&vec![0xa5; usize::try_from(len)?])?;
    Ok(())
}

async fn fresh(dir: &Path) -> TestResult<TraceStore> {
    Ok(TraceStore::open(&dir.join("new/trace.db")).await?)
}

#[tokio::test]
async fn a_trace_whose_schema_does_not_parse_fails_to_open_with_every_connection_closed()
-> TestResult {
    use sea_orm::sqlx::Connection;

    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    let mut conn =
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&old.path)).await?;
    sqlx::raw_sql(
        "PRAGMA writable_schema = ON; \
         UPDATE sqlite_schema SET sql = 'not a table' WHERE name = 'runs';",
    )
    .execute(&mut conn)
    .await?;
    conn.close().await?;
    let broken = dir.path().join("old/trace.db.copy");
    crash_copy(&old.path, &broken)?;

    assert!(
        TraceStore::open(&broken).await.is_err(),
        "the schema in the log does not parse"
    );
    assert!(
        !beside(&broken, "-wal").try_exists()?,
        "the last connection closed before open returned, and took the log with it: one that \
         closed later would delete by name the log of the fresh store the caller opens here"
    );
    Ok(())
}

#[tokio::test]
async fn a_readable_trace_carries_its_request_history_into_the_fresh_store() -> TestResult {
    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    let aside = dir.path().join("old/trace.db.broken");
    crash_copy(&old.path, &aside)?;
    let store = fresh(dir.path()).await?;

    let salvage = store.salvage(&aside).await?;
    assert_eq!(
        salvage,
        Salvage {
            requests: Recovery::Complete { rows: 2 },
        },
        "the run without a request id stays behind"
    );

    let first = store
        .run_by_request(&request("call-1")?, Some(&task("checkout")?))
        .await?;
    assert_eq!(first.map(|run| run.id), Some(old.ended));
    let global = store.run_by_request(&request("call-1")?, None).await?;
    assert_eq!(
        global.map(|run| run.id),
        Some(old.ended),
        "a repeat without a task matches it across Neo"
    );
    let other_task = store
        .run_by_request(&request("call-1")?, Some(&task("search")?))
        .await?;
    assert_eq!(other_task, None, "a repeat in another task does not");
    assert_eq!(
        store.replay(old.ended, at(61_000)).await?,
        Some(reply()),
        "a repeat inside the 10 minute window still replays"
    );
    assert_eq!(store.replay(old.ended, at(601_000)).await?, None);
    let detail = store
        .run(old.ended)
        .await?
        .ok_or("call-1's run did not come across")?;
    assert_eq!(
        (detail.summary.status, detail.summary.effects),
        (RunStatus::Ok, Some(RunEffects::None))
    );
    assert_eq!(detail.code_withheld.as_deref(), Some("unavailable"));

    let crashed = store
        .run_by_request(&request("call-2")?, None)
        .await?
        .ok_or("call-2's run did not come across")?;
    assert_eq!(
        (crashed.id, crashed.status, crashed.effects),
        (
            old.running,
            RunStatus::Interrupted,
            Some(RunEffects::Unknown)
        ),
        "a run the old Neo never ended ended with it"
    );
    assert_eq!(store.run(old.untracked).await?, None);

    assert_eq!(
        store.salvage(&aside).await?,
        salvage,
        "salvaging the same file again changes nothing"
    );
    Ok(())
}

#[tokio::test]
async fn a_trace_moved_without_its_write_ahead_log_loses_request_history() -> TestResult {
    use sea_orm::sqlx::Connection;

    let dir = tempfile::tempdir()?;
    fs::create_dir_all(dir.path().join("old"))?;
    let path = dir.path().join("old/trace.db");
    // A store that never checkpointed: its tables are only in its log. `TraceStore::open`
    // checkpoints the schema into the file, so a store it opened that loses its log loses only
    // the rows since, and nothing in the file says so.
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&options).await?;
    sqlx::raw_sql(
        "PRAGMA journal_mode = WAL; \
         CREATE TABLE runs (id TEXT PRIMARY KEY, started_at INTEGER NOT NULL, \
         status TEXT NOT NULL, request_id TEXT);",
    )
    .execute(&mut conn)
    .await?;
    let aside = dir.path().join("old/trace.db.broken");
    fs::copy(&path, &aside)?;
    conn.close().await?;
    let store = fresh(dir.path()).await?;

    assert_eq!(
        store.salvage(&aside).await?,
        Salvage {
            requests: Recovery::Unreadable,
        },
        "the tables are only in the log, so the log must move with the file"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn a_log_that_cannot_be_read_leaves_the_file_alone_and_request_history_incomplete()
-> TestResult {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    let aside = dir.path().join("old/trace.db.broken");
    folded_copy(&old.path, &aside).await?;
    // A log past its header that SQLite cannot open: it opens the log's index with O_NOFOLLOW,
    // so a symlink there fails it, as an index it cannot create would.
    fs::write(beside(&aside, "-wal"), [0xa5; 64])?;
    symlink(dir.path().join("elsewhere"), beside(&aside, "-shm"))?;
    let store = fresh(dir.path()).await?;

    let read_without_the_log = Recovery::Partial {
        rows: 2,
        lost: Loss::Unknown { skipped: 0 },
    };
    assert_eq!(
        store.salvage(&aside).await?,
        Salvage {
            requests: read_without_the_log,
        },
        "the file alone gives every row it holds, but the log may hold newer commits"
    );
    Ok(())
}

#[tokio::test]
async fn a_log_restarted_over_older_frames_carries_the_commits_only_it_holds() -> TestResult {
    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    restart_the_log(&old).await?;
    let aside = dir.path().join("old/trace.db.broken");
    crash_copy(&old.path, &aside)?;
    let store = fresh(dir.path()).await?;

    assert_eq!(
        store.salvage(&aside).await?,
        Salvage {
            requests: Recovery::Complete { rows: 3 },
        },
        "frames left from before the restart are not the log's own, so they cost nothing"
    );
    let latest = store.run_by_request(&request("call-3")?, None).await?;
    assert!(latest.is_some(), "call-3's run came across from the log");
    Ok(())
}

#[tokio::test]
async fn a_log_whose_header_fails_its_checksum_leaves_request_history_incomplete() -> TestResult {
    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    restart_the_log(&old).await?;
    let aside = dir.path().join("old/trace.db.broken");
    crash_copy(&old.path, &aside)?;
    overwrite(&beside(&aside, "-wal"), LOG_HEADER_CHECKSUM, 8)?;
    let store = fresh(dir.path()).await?;

    let without_the_log = Recovery::Partial {
        rows: 2,
        lost: Loss::Unknown { skipped: 0 },
    };
    assert_eq!(
        store.salvage(&aside).await?,
        Salvage {
            requests: without_the_log,
        },
        "SQLite skips a log whose header fails its checksum without an error, so the file alone \
         reads as whole"
    );
    Ok(())
}

#[tokio::test]
async fn a_log_cut_short_inside_its_last_frame_leaves_request_history_incomplete() -> TestResult {
    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    restart_the_log(&old).await?;
    let aside = dir.path().join("old/trace.db.broken");
    crash_copy(&old.path, &aside)?;
    cut_short(&beside(&aside, "-wal"), 1)?;
    let store = fresh(dir.path()).await?;

    let salvage = store.salvage(&aside).await?;
    assert!(
        !salvage.requests.is_complete(),
        "SQLite skips a frame cut short without an error, and it may hold the newest commit: \
         {salvage:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_store_whose_log_is_damaged_is_refused_and_keeps_its_log_for_salvage() -> TestResult {
    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    restart_the_log(&old).await?;
    let broken = dir.path().join("old/trace.db.broken");
    crash_copy(&old.path, &broken)?;
    let log = beside(&broken, "-wal");
    overwrite(&log, LOG_HEADER_CHECKSUM, 8)?;
    let damaged = fs::read(&log)?;

    let refused = TraceStore::open(&broken).await.err();
    assert!(
        matches!(refused, Some(TraceError::DamagedLog)),
        "a store over a damaged log does not open: {refused:?}"
    );
    assert_eq!(
        fs::read(&log)?,
        damaged,
        "no connection read past the damage, folded the rest into the file, or deleted the log"
    );
    let store = fresh(dir.path()).await?;
    let salvage = store.salvage(&broken).await?;
    assert!(
        !salvage.requests.is_complete(),
        "salvage still sees the damage: {salvage:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_damaged_page_in_a_committed_frame_leaves_request_history_incomplete() -> TestResult {
    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    restart_the_log(&old).await?;
    let aside = dir.path().join("old/trace.db.broken");
    crash_copy(&old.path, &aside)?;
    overwrite(&beside(&aside, "-wal"), FIRST_FRAME_PAGE + 64, 16)?;
    let store = fresh(dir.path()).await?;

    let without_the_log = Recovery::Partial {
        rows: 2,
        lost: Loss::Unknown { skipped: 0 },
    };
    assert_eq!(
        store.salvage(&aside).await?,
        Salvage {
            requests: without_the_log,
        },
        "SQLite stops reading the log at the first frame that fails its checksum, without an \
         error, so the file alone reads as whole"
    );
    assert_eq!(
        store.run_by_request(&request("call-3")?, None).await?,
        None,
        "call-3's run was only in the log"
    );
    Ok(())
}

#[tokio::test]
async fn a_trace_cut_short_keeps_the_pages_before_the_cut_and_reports_the_rest_lost() -> TestResult
{
    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    let aside = dir.path().join("old/trace.db.broken");
    folded_copy(&old.path, &aside).await?;
    let layout = Layout::of(&aside).await?;
    let cut_page = layout.root("private_tabs")?;
    assert!(
        layout.pages * layout.page_size > cut_page + layout.page_size,
        "pages written after the schema sit past the schema, so the cut leaves fewer pages \
         than the header claims"
    );
    let cut = cut_page + layout.page_size / 2;
    OpenOptions::new().write(true).open(&aside)?.set_len(cut)?;
    let store = fresh(dir.path()).await?;

    let salvage = store.salvage(&aside).await?;
    assert_eq!(
        salvage.requests,
        Recovery::Complete { rows: 2 },
        "the runs table lies before the cut"
    );
    let first = store.run_by_request(&request("call-1")?, None).await?;
    assert_eq!(first.map(|run| run.id), Some(old.ended));
    Ok(())
}

#[tokio::test]
async fn garbage_over_the_header_loses_request_history() -> TestResult {
    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    let aside = dir.path().join("old/trace.db.broken");
    folded_copy(&old.path, &aside).await?;
    overwrite(&aside, 0, 100)?;
    let store = fresh(dir.path()).await?;

    let salvage = store.salvage(&aside).await?;
    assert_eq!(
        salvage,
        Salvage {
            requests: Recovery::Unreadable,
        },
        "not a database at all"
    );
    assert_eq!(store.run_by_request(&request("call-1")?, None).await?, None);
    Ok(())
}

#[tokio::test]
async fn a_damaged_runs_table_loses_the_request_history_() -> TestResult {
    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    let aside = dir.path().join("old/trace.db.broken");
    folded_copy(&old.path, &aside).await?;
    let layout = Layout::of(&aside).await?;
    overwrite(&aside, layout.root("runs")?, layout.page_size)?;
    let store = fresh(dir.path()).await?;

    let salvage = store.salvage(&aside).await?;
    assert_eq!(
        salvage,
        Salvage {
            requests: Recovery::Unreadable,
        },
        "the damaged runs table cannot supply request history"
    );
    assert_eq!(store.run_by_request(&request("call-1")?, None).await?, None);
    Ok(())
}

#[tokio::test]
async fn a_run_whose_id_does_not_parse_is_counted_and_the_rest_come_across() -> TestResult {
    use sea_orm::sqlx::Connection;

    let dir = tempfile::tempdir()?;
    let old = old_trace(dir.path()).await?;
    let aside = dir.path().join("old/trace.db.broken");
    folded_copy(&old.path, &aside).await?;
    let mut conn =
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&aside)).await?;
    sqlx::query("UPDATE runs SET id = 'not-a-run-id' WHERE request_id = 'call-2'")
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    let store = fresh(dir.path()).await?;

    let salvage = store.salvage(&aside).await?;
    assert_eq!(
        salvage.requests,
        Recovery::Partial {
            rows: 1,
            lost: Loss::Rows(1.try_into()?),
        }
    );
    let first = store.run_by_request(&request("call-1")?, None).await?;
    assert_eq!(first.map(|run| run.id), Some(old.ended));
    Ok(())
}

#[tokio::test]
async fn an_older_schema_without_optional_columns_still_carries_over() -> TestResult {
    use sea_orm::sqlx::Connection;

    let dir = tempfile::tempdir()?;
    let aside = dir.path().join("old/trace.db.broken");
    fs::create_dir_all(dir.path().join("old"))?;
    let options = SqliteConnectOptions::new()
        .filename(&aside)
        .create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&options).await?;
    // No task_id, effects, replay or replay_until on runs.
    sqlx::raw_sql(
        "CREATE TABLE runs (id TEXT PRIMARY KEY, started_at INTEGER NOT NULL, ended_at INTEGER, \
         status TEXT NOT NULL, request_id TEXT);",
    )
    .execute(&mut conn)
    .await?;
    let ended = RunId::new();
    let running = RunId::new();
    let rows = [
        (ended, Some("call-1"), "ok", Some(at(10))),
        (running, Some("call-2"), "running", None),
        (RunId::new(), None, "ok", Some(at(30))),
    ];
    for (id, request_id, status, ended_at) in rows {
        sqlx::query(
            "INSERT INTO runs (id, started_at, ended_at, status, request_id) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(at(0).millis())
        .bind(ended_at.map(EpochMillis::millis))
        .bind(status)
        .bind(request_id)
        .execute(&mut conn)
        .await?;
    }
    conn.close().await?;
    let store = fresh(dir.path()).await?;

    let salvage = store.salvage(&aside).await?;
    assert_eq!(
        salvage,
        Salvage {
            requests: Recovery::Complete { rows: 2 },
        }
    );
    let first = store
        .run_by_request(&request("call-1")?, None)
        .await?
        .ok_or("call-1's run did not come across")?;
    assert_eq!(
        (first.id, first.task_id, first.status, first.effects),
        (ended, None, RunStatus::Ok, Some(RunEffects::Unknown)),
        "a file that never stored effects cannot vouch for them"
    );
    let in_task = store
        .run_by_request(&request("call-1")?, Some(&task("checkout")?))
        .await?;
    assert_eq!(in_task, None, "a run with no task is in no task");
    assert_eq!(store.replay(ended, at(20)).await?, None);
    let crashed = store
        .run_by_request(&request("call-2")?, None)
        .await?
        .map(|run| (run.id, run.status));
    assert_eq!(crashed, Some((running, RunStatus::Interrupted)));
    Ok(())
}
