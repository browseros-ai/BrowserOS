//! Recovering request-id history from a broken trace file into its replacement.
//!
//! A lost request binding could let a retry run twice. Salvage copies the request runs it can
//! recover and reports whether the history is complete, so admission can judge an unknown id.
//! It runs before the new writer starts; the old file stays untouched for later inspection.
//!
//! ```text
//!  trace.db fails to open → move it and its WAL aside → open a fresh store
//!    → salvage(aside) → request runs and replay data committed together
//! ```
//!
//! How the old file is read:
//! - Read-only, never repaired: it is what a human will look at, and a write to a damaged b-tree
//!   can spread the damage. SQLite may still create an index file (`-shm`) beside it.
//! - Through its write-ahead log when the log holds frames: a crash leaves the latest commits,
//!   new request runs among them, only there. A read-only connection cannot open a WAL file whose log is
//!   gone, so without frames the file is read alone (`immutable`). A part the log view cannot
//!   read at all is read from the file alone too, and then is not whole: the commits still in
//!   the log are missing from it.
//! - The log is checked first, as SQLite's recovery reads it: its header (magic, version, page
//!   size, its own checksum), then each frame's place in the checksum chain. SQLite skips a log
//!   whose header fails, and stops at the first frame that fails, without an error, so a read
//!   through the log ends and passes `quick_check` while the commits past the damage are
//!   missing. A damaged log leaves every part not whole, even when the failed frame may be a
//!   write a crash cut short, with no commit in it: an incomplete history cannot prove that an
//!   unknown request never ran. A frame whose salts are not the header's was written before the log last
//!   restarted, which is normal, and is not judged.
//! - Table b-trees only (`NOT INDEXED`), so a damaged index cannot hide rows.
//! - Only the columns a part needs, so an older schema reads too: a missing optional column is
//!   NULL. A table that is missing, or lacks a column every row needs, is unreadable.
//! - `writable_schema` on. A file cut short claims more pages than it has, and SQLite refuses
//!   all of it unless the schema is writable; with it, the pages that are there still read.
//! - A scan that reaches its end counts only once SQLite's `quick_check` passes the table and its
//!   indexes: a page cut short reads as zeros, which SQLite can return as rows without an error.

use crate::storage::{SalvagedRun, TraceBatch};
use crate::{
    EpochMillis, RequestId, RunEffects, RunId, RunStatus, RunTabsScope, TaskId, TraceError,
    TraceStore,
};
use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteRow};
use sea_orm::sqlx::{self, Row, SqliteConnection};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::BufReader;

/// A write-ahead log's header. A log no longer than this holds no frames.
const LOG_HEADER: usize = 32;

/// A frame's header, before the page it carries.
const FRAME_HEADER: usize = 24;

/// The one log format SQLite reads (`WAL_MAX_VERSION`).
const LOG_VERSION: u32 = 3_007_000;

/// What [`TraceStore::salvage`] carried into the fresh store, part by part. A part that did not
/// come across whole leaves rows missing from the fresh store, so its absence there proves
/// nothing; each field says what the caller does then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "a part that did not come across whole must be acted on, or Neo fails open"]
pub struct Salvage {
    /// The runs that carried a `requestId`, which admission looks up
    /// ([`TraceStore::run_by_request`]) to refuse a repeat. When they are not complete, a repeat
    /// of a lost request would run again. Whether to refuse a `requestId` it cannot vouch for,
    /// or to accept that risk, is the caller's call; a repeat of a run that came across is
    /// refused, or replayed, as before.
    pub requests: Recovery,
}

/// How much of one part of a broken trace file reached the fresh store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// All of it, `rows` rows: none when the part was empty.
    Complete {
        /// Rows now in the fresh store.
        rows: u64,
    },
    /// Some of it.
    Partial {
        /// Rows now in the fresh store.
        rows: u64,
        /// The rows that did not come across.
        lost: Loss,
    },
    /// None of it: the file, the part's table, or a column every row needs could not be read.
    Unreadable,
}

impl Recovery {
    /// Whether every row of the part reached the fresh store.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete { .. })
    }
}

/// The rows of a part that did not reach the fresh store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loss {
    /// Exactly these: every row was read, and these did not parse, such as a run whose id is
    /// not a run id.
    Rows(NonZeroU64),
    /// An unknown number: the read stopped at a damaged page, SQLite found the table damaged,
    /// or not every commit in the log could be read: the log is damaged, or only the file
    /// could be read.
    Unknown {
        /// Rows that were read and did not parse.
        skipped: u64,
    },
}

impl TraceStore {
    /// Copies the request-id history of a trace file that would not open into its replacement.
    /// Call before [`spawn_writer`](crate::spawn_writer) starts accepting records.
    ///
    /// Move `old`'s write-ahead log with it as `<old>-wal`: a crash may leave the newest
    /// request bindings only there. Recovered runs keep their ids, request/task bindings,
    /// timestamps, status, effects and stored replies. They have no operations or code; code
    /// reads `unavailable`. A previously running run becomes interrupted with unknown effects.
    ///
    /// Idempotent: a run already in this store is left alone, so a salvage interrupted by a
    /// crash can run again over the same old files.
    ///
    /// # Errors
    /// When this store could not commit the recovered rows. Treat the history as unreadable.
    pub async fn salvage(&self, old: &Path) -> Result<Salvage, TraceError> {
        let log = read_log(old).await;
        let first = if log == Log::Empty {
            View::FileOnly
        } else {
            View::WithLog
        };
        let mut requests = scan_view(old, first, log).await;
        if log != Log::Empty && requests.is_nothing() {
            let file = scan_view(old, View::FileOnly, log).await;
            requests = requests.or(file);
        }
        let requests = carry(requests, request_run);
        let tx = TraceBatch::begin(self.db()).await?;
        for (id, run) in &requests.rows {
            tx.insert_salvaged_run(*id, run).await?;
        }
        tx.commit().await?;
        Ok(Salvage {
            requests: requests.recovery,
        })
    }
}

/// How salvage reads the old file.
#[derive(Clone, Copy, Debug)]
enum View {
    /// With its write-ahead log applied, as SQLite reads a WAL file, but read-only.
    WithLog,
    /// The file alone (`immutable`): no locks, no log, nothing created beside it.
    FileOnly,
}

impl View {
    /// Whether a read through this view sees every commit: the file alone does when the log
    /// holds no frames, and the log applied does when SQLite reads all of it.
    fn sees_every_commit(self, log: Log) -> bool {
        matches!(
            (self, log),
            (Self::FileOnly, Log::Empty) | (Self::WithLog, Log::Sound)
        )
    }
}

/// What the old file's write-ahead log holds, judged from its bytes before SQLite reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Log {
    /// No frames: no log, or one no longer than its header. The file alone holds every commit.
    Empty,
    /// A header and frames SQLite reads: a read through the log sees every commit in it.
    Sound,
    /// Frames SQLite may skip without an error: the header fails its checks, a frame of this
    /// log fails its checksum, or the log could not be read.
    Damaged,
}

/// One part of the trace file: its table, the columns salvage reads, and which rows.
struct Part {
    table: &'static str,
    /// The columns every row needs. A table that lacks one is unreadable.
    required: &'static [&'static str],
    /// The columns an older schema may lack. A missing one reads as NULL.
    optional: &'static [&'static str],
    /// The part's rows, as a `WHERE` clause; empty for all of the table's.
    filter: &'static str,
}

const REQUESTS: Part = Part {
    table: "runs",
    required: &["id", "request_id", "started_at"],
    optional: &[
        "task_id",
        "status",
        "ended_at",
        "effects",
        "replay",
        "replay_until",
        "tabs_scope",
    ],
    filter: "WHERE request_id IS NOT NULL",
};

/// What one view read of one part.
enum Scan {
    /// Nothing: the part's table, or a column every row needs, could not be read, or the read
    /// failed before its first row.
    Nothing,
    /// Rows, and whether they are the whole table: the read reached its end, the view saw every
    /// commit, and `quick_check` found the table sound.
    Rows { rows: Vec<SqliteRow>, whole: bool },
}

impl Scan {
    fn is_nothing(&self) -> bool {
        matches!(self, Self::Nothing)
    }

    /// This scan, or `other` when this one read nothing.
    fn or(self, other: Self) -> Self {
        match self {
            Self::Nothing => other,
            scan @ Self::Rows { .. } => scan,
        }
    }
}

/// Judges the write-ahead log of the trace file at `old` before SQLite reads it. SQLite skips a
/// log whose header fails its checks, and stops at the first frame that fails its own, without
/// an error, so a read through a damaged log ends and passes `quick_check` while the commits past
/// the damage are missing from it. A log that cannot be read may hold commits.
/// [`TraceStore::open`] asks too, before it connects.
pub(crate) async fn read_log(old: &Path) -> Log {
    let mut log = old.as_os_str().to_owned();
    log.push("-wal");
    match check_log(&PathBuf::from(log)).await {
        Ok(log) => log,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Log::Empty,
        Err(error) => {
            tracing::warn!(%error, trace = %old.display(), "the trace's log cannot be read, and may hold commits");
            Log::Damaged
        }
    }
}

/// Reads the log at `path` as SQLite's recovery does (`walIndexRecover`): a header SQLite reads,
/// then each whole frame. A frame that carries the header's salts is one of this log's own and
/// must continue its checksum chain. One with other salts was left from before the log last
/// restarted, and SQLite rightly stops reading there. A log is whole frames after its header;
/// bytes past the last whole frame are a frame cut short, which SQLite skips without an error,
/// and it may be the commit frame of the newest transaction.
async fn check_log(path: &Path) -> io::Result<Log> {
    use tokio::io::AsyncReadExt;

    let file = File::open(path).await?;
    let len = usize::try_from(file.metadata().await?.len()).unwrap_or(usize::MAX);
    if len <= LOG_HEADER {
        return Ok(Log::Empty);
    }
    let mut log = BufReader::new(file);
    let mut header = [0; LOG_HEADER];
    log.read_exact(&mut header).await?;
    let Some(mut chain) = Chain::start(&header) else {
        return Ok(Log::Damaged);
    };
    let mut frame = vec![0; chain.frame_len];
    let frames = len - LOG_HEADER;
    if !frames.is_multiple_of(frame.len()) {
        return Ok(Log::Damaged);
    }
    for _ in 0..frames / frame.len() {
        log.read_exact(&mut frame).await?;
        if chain.refuses(&frame) {
            return Ok(Log::Damaged);
        }
    }
    Ok(Log::Sound)
}

/// The byte order of the 32-bit words a log's checksums add up, which its magic number names.
/// Every other field of the log is big-endian.
#[derive(Clone, Copy)]
enum WordOrder {
    Big,
    Little,
}

impl WordOrder {
    fn word(self, bytes: [u8; 4]) -> u32 {
        match self {
            Self::Big => u32::from_be_bytes(bytes),
            Self::Little => u32::from_le_bytes(bytes),
        }
    }
}

/// A log header SQLite reads, and the checksum chain its frames continue.
struct Chain {
    order: WordOrder,
    /// Every frame of this log carries them. A frame that does not was written before the log
    /// last restarted.
    salts: [[u8; 4]; 2],
    /// The checksum of the header and of every frame of this log so far.
    sum: [u32; 2],
    /// Each frame's length: its header and a page.
    frame_len: usize,
}

impl Chain {
    /// The chain `header` starts, when SQLite reads the log it heads: a magic number SQLite
    /// knows, its one format version, a page size it accepts, and the header's own checksum.
    fn start(header: &[u8; LOG_HEADER]) -> Option<Self> {
        let (words, _) = header.as_chunks::<4>();
        let &[magic, version, size, sequence, salt1, salt2, sum1, sum2] = words else {
            return None;
        };
        let order = match u32::from_be_bytes(magic) {
            0x377f_0682 => WordOrder::Little,
            0x377f_0683 => WordOrder::Big,
            _ => return None,
        };
        let page_size = u32::from_be_bytes(size);
        if u32::from_be_bytes(version) != LOG_VERSION
            || !page_size.is_power_of_two()
            || !(512..=65_536).contains(&page_size)
        {
            return None;
        }
        let sum = checksum(
            order,
            &[magic, version, size, sequence, salt1, salt2],
            [0, 0],
        );
        if sum != [u32::from_be_bytes(sum1), u32::from_be_bytes(sum2)] {
            return None;
        }
        Some(Self {
            order,
            salts: [salt1, salt2],
            sum,
            frame_len: FRAME_HEADER + usize::try_from(page_size).ok()?,
        })
    }

    /// Whether SQLite stops at `frame`, a frame header and its page, though it is one of this
    /// log's own: it carries the header's salts, but its page number is 0 or its checksum does
    /// not continue the chain. A frame of this log SQLite reads moves the chain on. A frame
    /// with other salts is skipped, so a frame of this log past one, which SQLite never reads,
    /// does not continue the chain either.
    fn refuses(&mut self, frame: &[u8]) -> bool {
        let (words, _) = frame.as_chunks::<4>();
        let &[page, commit, salt1, salt2, sum1, sum2, ref data @ ..] = words else {
            return true;
        };
        if [salt1, salt2] != self.salts {
            return false;
        }
        let sum = checksum(
            self.order,
            data,
            checksum(self.order, &[page, commit], self.sum),
        );
        if page == [0; 4] || sum != [u32::from_be_bytes(sum1), u32::from_be_bytes(sum2)] {
            return true;
        }
        self.sum = sum;
        false
    }
}

/// SQLite's log checksum (`walChecksumBytes`): for each pair of words `x0, x1` read in `order`,
/// `s0 += x0 + s1`, then `s1 += x1 + s0`, wrapping. SQLite sums whole pairs only.
fn checksum(order: WordOrder, words: &[[u8; 4]], [mut s0, mut s1]: [u32; 2]) -> [u32; 2] {
    let (pairs, _) = words.as_chunks::<2>();
    for &[x0, x1] in pairs {
        s0 = s0.wrapping_add(order.word(x0)).wrapping_add(s1);
        s1 = s1.wrapping_add(order.word(x1)).wrapping_add(s0);
    }
    [s0, s1]
}

/// Reads the request runs of `old` through `view`. A file that does not open reads as nothing.
async fn scan_view(old: &Path, view: View, log: Log) -> Scan {
    use sea_orm::sqlx::Connection;

    let options = SqliteConnectOptions::new()
        .filename(old)
        .read_only(true)
        .immutable(matches!(view, View::FileOnly))
        .pragma("writable_schema", "ON");
    let mut conn = match SqliteConnection::connect_with(&options).await {
        Ok(conn) => conn,
        Err(error) => {
            tracing::warn!(%error, ?view, old = %old.display(), "salvage cannot open the broken trace file");
            return Scan::Nothing;
        }
    };
    let requests = scan(&mut conn, &REQUESTS, view, log).await;
    // Closed rather than dropped: sqlx closes a dropped connection later, on its worker thread,
    // and the caller may move the file as soon as this returns.
    if let Err(error) = conn.close().await {
        tracing::warn!(%error, ?view, "salvage could not close the broken trace file");
    }
    requests
}

async fn scan(conn: &mut SqliteConnection, part: &Part, view: View, log: Log) -> Scan {
    use futures_util::StreamExt;

    let table = part.table;
    let present: Vec<String> = match sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
        .bind(table)
        .fetch_all(&mut *conn)
        .await
    {
        Ok(present) => present,
        Err(error) => {
            tracing::warn!(%error, table, ?view, "salvage cannot read the broken trace's schema");
            return Scan::Nothing;
        }
    };
    if present.is_empty() {
        tracing::warn!(table, ?view, "the broken trace has no such table");
        return Scan::Nothing;
    }
    let has = |column: &str| present.iter().any(|name| name == column);
    if let Some(missing) = part.required.iter().find(|column| !has(column)) {
        tracing::warn!(
            table,
            column = missing,
            ?view,
            "the broken trace lacks a column salvage needs"
        );
        return Scan::Nothing;
    }
    let columns: Vec<String> = part
        .required
        .iter()
        .chain(part.optional)
        .map(|column| {
            if has(column) {
                (*column).to_string()
            } else {
                format!("NULL AS {column}")
            }
        })
        .collect();
    let sql = format!(
        "SELECT {} FROM {table} NOT INDEXED {}",
        columns.join(", "),
        part.filter
    );
    let mut rows = Vec::new();
    let mut ended = true;
    let mut stream = sqlx::query(&sql).fetch(&mut *conn);
    while let Some(row) = stream.next().await {
        match row {
            Ok(row) => rows.push(row),
            Err(error) => {
                tracing::warn!(%error, table, ?view, read = rows.len(), "salvage stopped at a damaged page");
                ended = false;
                break;
            }
        }
    }
    drop(stream);
    if rows.is_empty() && !ended {
        return Scan::Nothing;
    }
    let whole = ended && view.sees_every_commit(log) && sound(conn, table, view).await;
    Scan::Rows { rows, whole }
}

/// SQLite's own check of `table` and its indexes. It can read a damaged page as rows without an
/// error, so a scan that ended counts as whole only when this finds nothing.
async fn sound(conn: &mut SqliteConnection, table: &str, view: View) -> bool {
    // PRAGMA arguments cannot be bound; `table` is one of this module's constants.
    let sql = format!("PRAGMA quick_check({table})");
    let findings: Result<Vec<String>, sqlx::Error> =
        sqlx::query_scalar(&sql).fetch_all(&mut *conn).await;
    match findings {
        Ok(findings) if findings == ["ok"] => true,
        Ok(findings) => {
            tracing::warn!(table, ?view, count = findings.len(), first = ?findings.first(), "SQLite finds the broken trace's table damaged");
            false
        }
        Err(error) => {
            tracing::warn!(%error, table, ?view, "SQLite cannot check the broken trace's table");
            false
        }
    }
}

/// One part's rows, parsed and keyed, and how much of the part they are.
struct Carried<K, V> {
    rows: BTreeMap<K, V>,
    recovery: Recovery,
}

fn carry<K: Ord, V>(scan: Scan, parse: impl Fn(&SqliteRow) -> Option<(K, V)>) -> Carried<K, V> {
    let Scan::Rows { rows, whole } = scan else {
        return Carried {
            rows: BTreeMap::new(),
            recovery: Recovery::Unreadable,
        };
    };
    let mut carried = BTreeMap::new();
    let mut skipped: u64 = 0;
    for row in &rows {
        match parse(row) {
            // Only damage makes two rows with one key: the first one read stands.
            Some((key, value)) => {
                carried.entry(key).or_insert(value);
            }
            None => skipped += 1,
        }
    }
    let count = u64::try_from(carried.len()).unwrap_or(u64::MAX);
    let recovery = match (whole, NonZeroU64::new(skipped)) {
        (true, None) => Recovery::Complete { rows: count },
        (true, Some(skipped)) => Recovery::Partial {
            rows: count,
            lost: Loss::Rows(skipped),
        },
        (false, _) => Recovery::Partial {
            rows: count,
            lost: Loss::Unknown { skipped },
        },
    };
    Carried {
        rows: carried,
        recovery,
    }
}

/// A run row, or `None` when a field that places it does not read: its id, its request or task
/// id, or its start. Those are what match a repeat to it.
fn request_run(row: &SqliteRow) -> Option<(RunId, SalvagedRun)> {
    let id: RunId = row.try_get::<String, _>("id").ok()?.parse().ok()?;
    let request = RequestId::parse(&row.try_get::<String, _>("request_id").ok()?).ok()?;
    let task = match row.try_get::<Option<String>, _>("task_id").ok()? {
        Some(task) => Some(TaskId::parse(&task).ok()?),
        None => None,
    };
    let started_at = EpochMillis::from_millis(row.try_get("started_at").ok()?);
    let text = |column: &str| row.try_get::<Option<String>, _>(column).ok().flatten();
    let time = |column: &str| {
        row.try_get::<Option<i64>, _>(column)
            .ok()
            .flatten()
            .map(EpochMillis::from_millis)
    };
    // A run the old file shows running ended with that Neo; one whose status does not read
    // happened, and what it did is unknown.
    let (status, effects) = match text("status").as_deref().and_then(RunStatus::parse) {
        Some(
            status @ (RunStatus::Ok | RunStatus::Error | RunStatus::Cancelled | RunStatus::Timeout),
        ) => {
            let effects = text("effects").as_deref().and_then(RunEffects::parse);
            (status, effects.unwrap_or(RunEffects::Unknown))
        }
        Some(RunStatus::Running | RunStatus::Interrupted) | None => {
            (RunStatus::Interrupted, RunEffects::Unknown)
        }
    };
    // Admission fails a repeat whose stored reply does not parse; without one, it refuses the
    // repeat as a duplicate instead.
    let replay = text("replay").filter(|reply| serde_json::from_str::<Value>(reply).is_ok());
    let run = SalvagedRun {
        request,
        task,
        started_at,
        ended_at: time("ended_at"),
        status,
        effects,
        replay,
        replay_until: time("replay_until"),
        // Absent before m0006, and for a name this build does not have: the listing then says
        // nothing, as for any run that predates the column.
        tabs_scope: text("tabs_scope").as_deref().and_then(RunTabsScope::parse),
    };
    Some((id, run))
}
