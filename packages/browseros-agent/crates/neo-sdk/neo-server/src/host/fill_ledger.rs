//! The fill's own record of request ids: a fill with a `requestId` never types twice, across a Neo
//! restart too (protected-fill design, section 4.6; rulings R6 and R9).
//!
//! ```text
//!  admit(id, task) ─ running ──────────────────────▶ Join: wait for the first call's answer
//!                  ├ answered ─────────────────────▶ Answer: the stored answer, nothing typed
//!                  ├ started, never answered ──────▶ Interrupted: `fill_interrupted`, nothing typed
//!                  └ unknown ─ claim ──────────────▶ Fresh(ticket)
//!  ticket.started() ─ {"state":"started"} appended and synced, before the op is submitted,
//!                     so before the first key
//!  ticket.finish(answer) ─ {"state":"answered", answer} appended and synced; joiners get it
//!  open (startup) ─ the file read back; a start with no answer is an interrupted fill; lines
//!                   older than the trace's retention are dropped and the file rewritten
//! ```
//!
//! Why not `run`'s ledger: after a restart it answers from the trace, which commits in batches,
//! so a request id from before the restart can run again. For a script that risks a second run;
//! for a fill a secret typed twice, or a one-time code used twice. It also shares ids with
//! scripts, replays only a script's reply, and frees the id of a refused call. This record is a
//! file of its own, beside the trace, written before the first key. It holds request ids, task
//! ids, run ids and answers; never a body, so never a value.
//!
//! A fill that answered is spent, whatever it answered: a repeat gets the same answer. Only a
//! call refused before it started (a request error: no tab, no browser, a released lock) leaves
//! its id free, as nothing ran.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use neo_sdk_api as api;
use neo_trace::{EpochMillis, RequestId, RunId, TaskId};
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio::{fs as async_fs, task};

use super::fill_answer::{Refused, interrupted};

/// The record's file name, in the trace's folder.
pub(crate) const LEDGER_FILE: &str = "fill-requests.jsonl";

/// What a call's request id decided.
pub(crate) enum Admission {
    /// Run it: the ticket holds the id's place until the call answers.
    Fresh(FillTicket),
    /// The same request is running: wait for its answer.
    Join(FillWaiter),
    /// The same request answered: its answer, nothing typed.
    Answer(Arc<api::FillResult>),
    /// The record could not be read at startup, so Neo cannot tell whether this id ran: refused,
    /// nothing typed.
    Refused(Refused),
}

/// The fill's record of request ids. One per process, shared by every fill.
pub(crate) struct FillLedger {
    path: PathBuf,
    /// How long an answer is kept: the trace's retention, read at each prune (a host can change
    /// it), so a repeat gets the stored answer for as long as the trace keeps the fill's run.
    keep: Box<dyn Fn() -> Duration + Send + Sync>,
    /// One std lock, never held across an await.
    entries: Mutex<HashMap<RequestId, Vec<Entry>>>,
    /// The appender, opened on first use. One writer at a time keeps lines whole.
    file: Arc<Mutex<Option<File>>>,
    /// A prune forgot answers the file still holds: the next append compacts the file first,
    /// under the appender's lock, so the record keeps no more than the trace does.
    compact_due: Arc<AtomicBool>,
    /// The file existed and could not be read at startup. Its history is unknown, so every
    /// fill with a `requestId` is refused rather than risk typing a spent id again, and the file
    /// is left as it is for someone to look at.
    unreadable: bool,
}

struct Entry {
    task: Option<TaskId>,
    run: RunId,
    at: EpochMillis,
    state: State,
}

enum State {
    /// Running: a repeat joins it.
    Running(watch::Receiver<Option<Settled>>),
    /// Answered: a repeat gets this.
    Answered(Arc<api::FillResult>),
    /// Started and never answered: Neo restarted in the middle.
    Interrupted,
}

impl State {
    /// Which of several fills with one id a repeat follows: the one running, else one that
    /// answered, else one Neo restarted in the middle of (ruling R36). A start line written
    /// before its sync failed never typed and freed its id, so a later call with that id may have
    /// typed and answered: that answer is what happened, and after a restart a repeat gets it.
    const fn precedence(&self) -> u8 {
        match self {
            Self::Running(_) => 0,
            Self::Answered(_) => 1,
            Self::Interrupted => 2,
        }
    }
}

/// How the first call ended for those who joined it.
type Settled = Result<Arc<api::FillResult>, Refused>;

/// The right to run one fill, held by the call. Every path publishes: `finish` with the answer,
/// `release` for a call refused before it started, and `Drop` (the call's task unwound) as an
/// interrupted fill once it started, or a refusal before.
#[must_use]
pub(crate) struct FillTicket {
    ledger: Arc<FillLedger>,
    request: RequestId,
    task: Option<TaskId>,
    run: RunId,
    sender: watch::Sender<Option<Settled>>,
    started: bool,
    published: bool,
}

/// A repeat waiting for the first call's answer.
pub(crate) struct FillWaiter {
    receiver: watch::Receiver<Option<Settled>>,
    first: RunId,
}

/// One line of the record file. Keys it does not know are ignored, so a newer Neo's line that
/// adds one still reads here.
#[derive(Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Line {
    /// Written and synced before the first key.
    Started {
        #[serde(rename = "requestId")]
        request: String,
        #[serde(rename = "taskId", default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        #[serde(rename = "runId")]
        run: String,
        at: i64,
    },
    /// Written when the fill answered.
    Answered {
        #[serde(rename = "requestId")]
        request: String,
        #[serde(rename = "taskId", default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        #[serde(rename = "runId")]
        run: String,
        at: i64,
        answer: Box<api::FillResult>,
    },
}

/// What every line of the record names, read leniently: a newer Neo's line, of a state this one
/// does not know, still names its request id and its run, so this Neo keeps that id spent.
#[derive(Deserialize)]
struct Stamp {
    #[serde(rename = "requestId")]
    request: String,
    #[serde(rename = "taskId", default)]
    task: Option<String>,
    #[serde(rename = "runId")]
    run: String,
    at: i64,
}

impl FillLedger {
    /// Reads the record at `folder/fill-requests.jsonl` back, drops what is older than `keep`, and
    /// rewrites the file with what is left. A line that does not read costs only itself. A file
    /// that exists and cannot be read at all is left alone, and every fill with a `requestId` is
    /// refused until Neo starts with a readable one; one that cannot be rewritten is appended to
    /// as it is.
    pub(crate) async fn open(
        folder: &Path,
        keep: impl Fn() -> Duration + Send + Sync + 'static,
    ) -> Arc<Self> {
        let path = folder.join(LEDGER_FILE);
        let (bytes, unreadable) = match async_fs::read(&path).await {
            Ok(bytes) => (bytes, false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Vec::new(), false),
            Err(error) => {
                tracing::error!(%error, file = %path.display(), "cannot read the fill's request record; fills with a requestId are refused until it reads");
                (Vec::new(), true)
            }
        };
        let (entries, kept) = replay(&bytes, oldest(keep()));
        if !unreadable
            && let Err(error) = {
                let target = path.clone();
                task::spawn_blocking(move || rewrite(&target, &kept))
                    .await
                    .map_err(io::Error::other)
                    .and_then(|done| done)
            }
        {
            tracing::warn!(%error, file = %path.display(), "cannot compact the fill's request record; appending to it as it is");
        }
        Arc::new(Self {
            path,
            keep: Box::new(keep),
            entries: Mutex::new(entries),
            file: Arc::default(),
            compact_due: Arc::default(),
            unreadable,
        })
    }

    /// Admits one call with `request`, which runs as `run` if it runs. The match is `run`'s: with
    /// a `taskId`, an earlier fill of that task; without, any fill with that id.
    pub(crate) fn admit(
        self: &Arc<Self>,
        request: &RequestId,
        task: Option<&TaskId>,
        run: RunId,
    ) -> Admission {
        if self.unreadable {
            return Admission::Refused(Refused::RecordUnavailable);
        }
        let mut entries = self.lock();
        self.prune(&mut entries);
        let found = entries.get(request).and_then(|runs| {
            runs.iter()
                .filter(|entry| task.is_none_or(|task| entry.task.as_ref() == Some(task)))
                .min_by_key(|entry| entry.state.precedence())
        });
        if let Some(entry) = found {
            return match &entry.state {
                State::Running(receiver) => Admission::Join(FillWaiter {
                    receiver: receiver.clone(),
                    first: entry.run,
                }),
                State::Answered(answer) => Admission::Answer(Arc::clone(answer)),
                State::Interrupted => Admission::Answer(Arc::new(interrupted(entry.run))),
            };
        }
        let (sender, receiver) = watch::channel(None);
        entries.entry(request.clone()).or_default().push(Entry {
            task: task.cloned(),
            run,
            at: EpochMillis::now(),
            state: State::Running(receiver),
        });
        Admission::Fresh(FillTicket {
            ledger: Arc::clone(self),
            request: request.clone(),
            task: task.cloned(),
            run,
            sender,
            started: false,
            published: false,
        })
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<RequestId, Vec<Entry>>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Forgets answers older than the trace keeps; a running fill is never older than a call.
    fn prune(&self, entries: &mut HashMap<RequestId, Vec<Entry>>) {
        let oldest = oldest((self.keep)());
        let before: usize = entries.values().map(Vec::len).sum();
        entries.retain(|_, runs| {
            runs.retain(|entry| matches!(entry.state, State::Running(_)) || entry.at > oldest);
            !runs.is_empty()
        });
        if entries.values().map(Vec::len).sum::<usize>() < before {
            self.compact_due.store(true, Ordering::Release);
        }
    }

    /// Appends one line and syncs it to disk before returning. On a blocking thread: a sync is a
    /// disk wait, and the handle's lock is std's, never held across an await.
    ///
    /// Each record starts on a line of its own: a line an earlier failed or crashed write left
    /// half done can then cost only itself, never the start record written after it.
    async fn append(&self, line: &Line) -> io::Result<()> {
        let mut text = String::from("\n");
        text.push_str(&serde_json::to_string(line).map_err(io::Error::other)?);
        text.push('\n');
        let file = Arc::clone(&self.file);
        let path = self.path.clone();
        let compact_due = Arc::clone(&self.compact_due);
        let oldest = oldest((self.keep)());
        task::spawn_blocking(move || {
            let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
            if compact_due.swap(false, Ordering::AcqRel) {
                // From the file's own lines, so a running fill's synced start is kept. The
                // rename replaces the file the handle points at, so the handle opens afresh.
                match fs::read(&path).map(|bytes| replay(&bytes, oldest).1) {
                    Ok(kept) => {
                        if let Err(error) = rewrite(&path, &kept) {
                            tracing::warn!(%error, "cannot compact the fill's request record; appending to it as it is");
                        }
                        *file = None;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "cannot read the fill's request record to compact it");
                    }
                }
            }
            if file.is_none() {
                let created = !path.exists();
                *file = Some(OpenOptions::new().create(true).append(true).open(&path)?);
                // A new file's name lives in its folder: synced too, or a power cut can lose
                // the whole file, and the start line with it.
                if created {
                    sync_folder(&path)?;
                }
            }
            let Some(open) = file.as_mut() else {
                return Err(io::Error::other("the record file did not open"));
            };
            let written = open
                .write_all(text.as_bytes())
                .and_then(|()| open.sync_data());
            if written.is_err() {
                // A half-written line may be in the file; the next append opens a fresh handle.
                *file = None;
            }
            written
        })
        .await
        .map_err(io::Error::other)?
    }

    fn settle(&self, request: &RequestId, run: RunId, state: State) {
        let mut entries = self.lock();
        if let Some(entry) = entries
            .get_mut(request)
            .and_then(|runs| runs.iter_mut().find(|entry| entry.run == run))
        {
            entry.state = state;
        }
    }

    fn forget(&self, request: &RequestId, run: RunId) {
        let mut entries = self.lock();
        if let Some(runs) = entries.get_mut(request) {
            runs.retain(|entry| entry.run != run);
            if runs.is_empty() {
                entries.remove(request);
            }
        }
    }
}

impl FillTicket {
    /// Records, synced to disk, that this fill starts. Call it right before the op is submitted:
    /// from then on a repeat never types, even after a restart. `Err` means nothing may be typed.
    pub(crate) async fn started(&mut self) -> io::Result<()> {
        self.ledger
            .append(&Line::Started {
                request: self.request.as_str().to_owned(),
                task: self.task.as_ref().map(|task| task.as_str().to_owned()),
                run: self.run.to_string(),
                at: EpochMillis::now().millis(),
            })
            .await?;
        self.started = true;
        Ok(())
    }

    /// Publishes the fill's answer: the record keeps it, and every joined repeat gets it. A
    /// record that cannot be written leaves the answer in memory, and after a restart the start
    /// line alone answers `fill_interrupted`, which never types.
    pub(crate) async fn finish(mut self, answer: Arc<api::FillResult>) {
        let line = Line::Answered {
            request: self.request.as_str().to_owned(),
            task: self.task.as_ref().map(|task| task.as_str().to_owned()),
            run: self.run.to_string(),
            at: EpochMillis::now().millis(),
            answer: Box::new((*answer).clone()),
        };
        if let Err(error) = self.ledger.append(&line).await {
            tracing::warn!(%error, run = %self.run, "cannot record the fill's answer; after a restart a repeat answers fill_interrupted");
        }
        self.publish(Ok(answer));
    }

    /// A call refused before it started: nothing ran, so the id is free again, and every joined
    /// repeat gets the same refusal.
    pub(crate) fn release(mut self, refused: Refused) {
        self.publish(Err(refused));
    }

    fn publish(&mut self, settled: Settled) {
        self.published = true;
        match &settled {
            Ok(answer) => {
                self.ledger
                    .settle(&self.request, self.run, State::Answered(Arc::clone(answer)));
            }
            Err(_) => self.ledger.forget(&self.request, self.run),
        }
        self.sender.send_replace(Some(settled));
    }
}

impl Drop for FillTicket {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        // The call's task unwound. Once started, what reached the page is unknown, so the id
        // answers as an interrupted fill for good; before, nothing ran.
        if self.started {
            let answer = Arc::new(interrupted(self.run));
            self.ledger
                .settle(&self.request, self.run, State::Interrupted);
            self.sender.send_replace(Some(Ok(answer)));
        } else {
            self.ledger.forget(&self.request, self.run);
            self.sender.send_replace(Some(Err(Refused::Internal(
                "the fill stopped inside Neo before it started".to_owned(),
            ))));
        }
        self.published = true;
    }
}

impl FillWaiter {
    /// The first call's answer, or its refusal. A first call that vanished without a word
    /// answers as interrupted, which never types.
    pub(crate) async fn answer(mut self) -> Result<Arc<api::FillResult>, Refused> {
        match self.receiver.wait_for(Option::is_some).await {
            Ok(settled) => settled
                .clone()
                .unwrap_or_else(|| Ok(Arc::new(interrupted(self.first)))),
            Err(_) => Ok(Arc::new(interrupted(self.first))),
        }
    }
}

/// The cut for a record kept for `keep`: a line is kept while it is younger than `keep`, so one
/// from this moment or before it is gone, and with no retention nothing is kept, even a line
/// written in the same millisecond.
fn oldest(keep: Duration) -> EpochMillis {
    EpochMillis::now().plus_millis(-i64::try_from(keep.as_millis()).unwrap_or(i64::MAX))
}

/// The record's lines back into memory, and the lines worth keeping, as they are in the file. A
/// start with no answer is interrupted.
///
/// A line that is not whole JSON (a torn line after a crash, or bytes that are not text) is
/// skipped and dropped alone: a torn start line was never synced, so its fill never reached its
/// first key. A whole line this Neo cannot read is kept as it is (ruling R36): after a rollback
/// from a newer Neo, deleting its lines would free ids that Neo spent. If it still names its id
/// and run, that id stays spent here too, as an interrupted fill, which never types; one that
/// names neither is kept, past retention too, for the Neo that wrote it.
fn replay(bytes: &[u8], oldest: EpochMillis) -> (HashMap<RequestId, Vec<Entry>>, Vec<Vec<u8>>) {
    let mut entries: HashMap<RequestId, Vec<Entry>> = HashMap::new();
    let mut kept = Vec::new();
    for raw in bytes
        .split(|byte| *byte == b'\n')
        .filter(|raw| !raw.trim_ascii().is_empty())
    {
        let Ok(stamp) = serde_json::from_slice::<Stamp>(raw) else {
            if serde_json::from_slice::<IgnoredAny>(raw).is_ok() {
                tracing::warn!(
                    "keeping a line of the fill's request record that this Neo cannot read"
                );
                kept.push(raw.to_vec());
            } else {
                tracing::warn!("skipping a line of the fill's request record that does not read");
            }
            continue;
        };
        if EpochMillis::from_millis(stamp.at) <= oldest {
            continue;
        }
        let (Ok(request), Ok(run)) = (RequestId::parse(&stamp.request), stamp.run.parse::<RunId>())
        else {
            // An id this Neo would refuse at admission: nothing here can match it.
            kept.push(raw.to_vec());
            continue;
        };
        let task = stamp
            .task
            .as_deref()
            .and_then(|task| TaskId::parse(task).ok());
        let state = match serde_json::from_slice::<Line>(raw) {
            Ok(Line::Answered { answer, .. }) => State::Answered(Arc::new(*answer)),
            Ok(Line::Started { .. }) | Err(_) => State::Interrupted,
        };
        let runs = entries.entry(request).or_default();
        match runs.iter_mut().find(|entry| entry.run == run) {
            Some(entry) => {
                // An answer replaces its start; a second start never undoes an answer.
                if matches!(state, State::Answered(_)) {
                    entry.state = state;
                }
            }
            None => runs.push(Entry {
                task,
                run,
                at: EpochMillis::from_millis(stamp.at),
                state,
            }),
        }
        kept.push(raw.to_vec());
    }
    (entries, kept)
}

/// Rewrites the record with `lines` only, each as it was read: written to a temporary file,
/// synced, then renamed over the record, so a crash leaves the old record or the new one, never
/// half of one. Blocking: call it on a blocking thread.
fn rewrite(path: &Path, lines: &[Vec<u8>]) -> io::Result<()> {
    let mut text = Vec::new();
    for line in lines {
        text.extend_from_slice(line);
        text.push(b'\n');
    }
    let temporary = path.with_extension("jsonl.tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(&text)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    // The rename is the folder's change: synced, or a power cut could bring the old file back
    // without the start lines appended since.
    sync_folder(path)
}

/// Syncs the folder that holds `path`, so a file created or renamed there survives a power cut.
fn sync_folder(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(folder) => File::open(folder)?.sync_all(),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    use std::{fs, mem};

    use neo_sdk_api as api;
    use neo_trace::{EpochMillis, RequestId, RunId, TaskId};

    use super::{Admission, FillLedger, LEDGER_FILE};
    use crate::host::fill_answer::Refused;

    const KEEP: Duration = Duration::from_hours(7 * 24);

    fn answer(run: RunId) -> Arc<api::FillResult> {
        Arc::new(api::FillResult {
            typed: Some(vec![8]),
            ..api::FillResult::new(run.to_string(), "entered".into(), "done".into())
        })
    }

    fn fresh(admission: Admission) -> Result<super::FillTicket, Box<dyn Error>> {
        match admission {
            Admission::Fresh(ticket) => Ok(ticket),
            Admission::Join(_) | Admission::Answer(_) | Admission::Refused(_) => {
                Err("expected a fresh admission".into())
            }
        }
    }

    #[tokio::test]
    async fn a_repeat_joins_while_running_and_replays_after() -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        let ledger = FillLedger::open(folder.path(), || KEEP).await;
        let id = RequestId::parse("pay-1")?;
        let first = RunId::new();
        let mut ticket = fresh(ledger.admit(&id, None, first))?;
        ticket.started().await?;
        let Admission::Join(waiter) = ledger.admit(&id, None, RunId::new()) else {
            return Err("a repeat while running joins".into());
        };
        ticket.finish(answer(first)).await;
        assert_eq!(waiter.answer().await.ok(), Some(answer(first)));
        let Admission::Answer(replayed) = ledger.admit(&id, None, RunId::new()) else {
            return Err("a repeat after it answered gets the stored answer".into());
        };
        assert_eq!(replayed, answer(first));
        Ok(())
    }

    #[tokio::test]
    async fn after_a_restart_a_start_with_no_answer_is_interrupted() -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        let id = RequestId::parse("code-6")?;
        let answered = RequestId::parse("pw-2")?;
        let first = RunId::new();
        let second = RunId::new();
        {
            let ledger = FillLedger::open(folder.path(), || KEEP).await;
            let mut ticket = fresh(ledger.admit(&id, None, first))?;
            ticket.started().await?;
            // Neo dies here: the ticket is forgotten without an answer.
            mem::forget(ticket);
            let mut done = fresh(ledger.admit(&answered, None, second))?;
            done.started().await?;
            done.finish(answer(second)).await;
        }
        let ledger = FillLedger::open(folder.path(), || KEEP).await;
        let Admission::Answer(after) = ledger.admit(&id, None, RunId::new()) else {
            return Err("an interrupted fill never runs again".into());
        };
        assert_eq!(after.outcome, "uncertain");
        assert_eq!(after.run_id, first.to_string());
        assert_eq!(
            after.error.as_ref().map(|error| error.code.as_str()),
            Some("fill_interrupted")
        );
        let Admission::Answer(kept) = ledger.admit(&answered, None, RunId::new()) else {
            return Err("an answer survives a restart".into());
        };
        assert_eq!(kept, answer(second));
        Ok(())
    }

    #[tokio::test]
    async fn a_refusal_before_the_start_frees_the_id_and_a_task_scopes_it()
    -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        let ledger = FillLedger::open(folder.path(), || KEEP).await;
        let id = RequestId::parse("r")?;
        let task = TaskId::parse("task-a")?;
        let other = TaskId::parse("task-b")?;
        let ticket = fresh(ledger.admit(&id, Some(&task), RunId::new()))?;
        let Admission::Join(waiter) = ledger.admit(&id, Some(&task), RunId::new()) else {
            return Err("joins".into());
        };
        // Another task's id of the same text is its own.
        let other_ticket = fresh(ledger.admit(&id, Some(&other), RunId::new()))?;
        ticket.release(Refused::TabNotFound);
        assert_eq!(waiter.answer().await.err(), Some(Refused::TabNotFound));
        drop(fresh(ledger.admit(&id, Some(&task), RunId::new()))?);
        drop(other_ticket);
        Ok(())
    }

    #[tokio::test]
    async fn a_torn_or_unreadable_line_costs_only_itself() -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        let kept = RequestId::parse("kept")?;
        let after = RequestId::parse("after-the-tear")?;
        {
            let ledger = FillLedger::open(folder.path(), || KEEP).await;
            let run = RunId::new();
            let mut ticket = fresh(ledger.admit(&kept, None, run))?;
            ticket.started().await?;
            ticket.finish(answer(run)).await;
        }
        // A crash left half a line, with bytes that are not text, and no newline.
        let path = folder.path().join(LEDGER_FILE);
        let mut torn = fs::read(&path)?;
        torn.extend_from_slice(b"{\"state\":\"started\",\"requestId\":\"r-\xff\xfe");
        fs::write(&path, &torn)?;
        let first = RunId::new();
        {
            let ledger = FillLedger::open(folder.path(), || KEEP).await;
            assert!(
                matches!(
                    ledger.admit(&kept, None, RunId::new()),
                    Admission::Answer(_)
                ),
                "the intact record survives the torn one"
            );
            // Torn again while this Neo runs, as a failed write leaves it, then a start.
            let mut live = fs::read(&path)?;
            live.extend_from_slice(b"{\"state\":\"answered\",\"req");
            fs::write(&path, &live)?;
            let mut ticket = fresh(ledger.admit(&after, None, first))?;
            ticket.started().await?;
            mem::forget(ticket);
        }
        // And the start written after the tear is whole: after a restart it never types again.
        let ledger = FillLedger::open(folder.path(), || KEEP).await;
        let Admission::Answer(interrupted) = ledger.admit(&after, None, RunId::new()) else {
            return Err("the start after a torn line was lost".into());
        };
        assert_eq!(interrupted.run_id, first.to_string());
        Ok(())
    }

    #[tokio::test]
    async fn an_answer_wins_over_a_start_that_never_typed() -> Result<(), Box<dyn Error>> {
        // A start line whose sync failed: that call refused and freed the id, and a repeat with
        // the id typed and answered. After a restart the repeat's answer is what happened.
        let folder = tempfile::tempdir()?;
        let request = RequestId::parse("sync-failed-then-typed")?;
        let (lost, typed) = (RunId::new(), RunId::new());
        let at = EpochMillis::now().millis();
        let line = |state: &str, run: RunId, tail: &str| {
            format!(
                r#"{{"state":"{state}","requestId":"{}","runId":"{run}","at":{at}{tail}}}"#,
                request.as_str()
            )
        };
        let answered = serde_json::to_string(&*answer(typed))?;
        let text = [
            line("started", lost, ""),
            line("started", typed, ""),
            line("answered", typed, &format!(r#","answer":{answered}"#)),
        ]
        .join("\n");
        fs::write(folder.path().join(LEDGER_FILE), text)?;
        let ledger = FillLedger::open(folder.path(), || KEEP).await;
        let Admission::Answer(stored) = ledger.admit(&request, None, RunId::new()) else {
            return Err("the id is spent".into());
        };
        assert_eq!(stored.run_id, typed.to_string());
        assert_eq!(
            stored.outcome,
            answer(typed).outcome,
            "the stored answer, not fill_interrupted"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_whole_line_this_neo_cannot_read_is_kept_and_its_id_stays_spent()
    -> Result<(), Box<dyn Error>> {
        // A newer Neo's lines, then a rollback: deleting them would free the ids that Neo spent.
        let folder = tempfile::tempdir()?;
        let path = folder.path().join(LEDGER_FILE);
        let newer = RunId::new();
        let at = EpochMillis::now().millis();
        let unknown_state = format!(
            r#"{{"state":"typed_and_checked","requestId":"from-a-newer-neo","runId":"{newer}","at":{at},"more":1}}"#
        );
        let unknown_shape = r#"{"version":9,"note":"a line of another shape"}"#;
        let torn = r#"{"state":"started","requestId":"torn"#;
        fs::write(&path, format!("{unknown_state}\n{unknown_shape}\n{torn}"))?;
        let ledger = FillLedger::open(folder.path(), || KEEP).await;
        let Admission::Answer(spent) =
            ledger.admit(&RequestId::parse("from-a-newer-neo")?, None, RunId::new())
        else {
            return Err("a newer Neo's id types again".into());
        };
        assert_eq!(spent.run_id, newer.to_string());
        assert_eq!(
            spent.error.as_ref().map(|error| error.code.as_str()),
            Some("fill_interrupted"),
            "spent, and never typed"
        );
        let after = fs::read_to_string(&path)?;
        assert!(
            after.lines().any(|line| line == unknown_state),
            "kept as it was: {after}"
        );
        assert!(
            after.lines().any(|line| line == unknown_shape),
            "kept as it was: {after}"
        );
        assert!(
            !after.contains("torn"),
            "a torn line costs only itself: {after}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_running_server_drops_expired_lines_from_the_file() -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        let keep = Arc::new(AtomicU64::new(KEEP.as_secs()));
        let reader = Arc::clone(&keep);
        let ledger = FillLedger::open(folder.path(), move || {
            Duration::from_secs(reader.load(Ordering::SeqCst))
        })
        .await;
        let old = RequestId::parse("old")?;
        let run = RunId::new();
        let mut ticket = fresh(ledger.admit(&old, None, run))?;
        ticket.started().await?;
        ticket.finish(answer(run)).await;
        // The host shortens the trace's retention: the next admission forgets the old answer,
        // and the next append compacts the file before it writes.
        keep.store(0, Ordering::SeqCst);
        let new = RequestId::parse("new")?;
        let next = RunId::new();
        let mut started = fresh(ledger.admit(&new, None, next))?;
        started.started().await?;
        let text = fs::read_to_string(folder.path().join(LEDGER_FILE))?;
        assert!(!text.contains(r#""requestId":"old""#), "{text}");
        assert!(text.contains(r#""requestId":"new""#), "{text}");
        started.finish(answer(next)).await;
        Ok(())
    }

    #[tokio::test]
    async fn a_record_that_cannot_be_read_refuses_every_request_id() -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        // A directory where the file should be: it exists and cannot be read.
        fs::create_dir(folder.path().join(LEDGER_FILE))?;
        let ledger = FillLedger::open(folder.path(), || KEEP).await;
        assert!(matches!(
            ledger.admit(&RequestId::parse("any")?, None, RunId::new()),
            Admission::Refused(Refused::RecordUnavailable)
        ));
        assert!(folder.path().join(LEDGER_FILE).is_dir(), "left as it was");
        Ok(())
    }

    #[tokio::test]
    async fn the_record_holds_ids_and_answers_only_and_drops_old_lines()
    -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        let id = RequestId::parse("old")?;
        {
            let ledger = FillLedger::open(folder.path(), || KEEP).await;
            let run = RunId::new();
            let mut ticket = fresh(ledger.admit(&id, None, run))?;
            ticket.started().await?;
            ticket.finish(answer(run)).await;
        }
        let text = fs::read_to_string(folder.path().join(LEDGER_FILE))?;
        assert_eq!(
            text.lines().filter(|line| !line.is_empty()).count(),
            2,
            "{text}"
        );
        assert!(!text.contains("value"), "{text}");
        // Kept for no time at all: the next open drops both lines.
        let ledger = FillLedger::open(folder.path(), || Duration::ZERO).await;
        assert!(matches!(
            ledger.admit(&id, None, RunId::new()),
            Admission::Fresh(_)
        ));
        let text = fs::read_to_string(folder.path().join(LEDGER_FILE))?;
        assert!(text.is_empty(), "{text}");
        Ok(())
    }
}
