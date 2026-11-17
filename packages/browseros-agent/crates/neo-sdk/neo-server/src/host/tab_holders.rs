//! Which task holds which tab, kept by target id in a file beside the trace, so a restart of
//! Neo's server while the browser keeps its tabs gives every task its tabs back: the tabs it
//! opened, their popups, the tabs of the tasks its runs created, and the tab that was its page.
//!
//! ```text
//!  the task book changes ─ BookSink, under the registry's lock: a channel send, nothing else
//!    ─ the writer task: the tab's target from core's cache (a Seen ticket), one JSON line
//!      appended, no sync; the file is compacted once dead lines outnumber live ones
//!  Neo starts, the browser connects ─ restore: the browser's own tab list, unfiltered ─ the
//!    saved holders of the targets it still has, per task ─ each tab registered ─
//!    TabRegistry::restore_book ─ the file rewritten with what the book took ─ ready: runs start
//! ```
//!
//! Why target ids: a tab id names another tab once the browser restarts, a target id never does
//! (the filled-tabs record relies on the same fact). So a browser restart restores nothing, and
//! its tabs, with tab numbers that may repeat, hold no task: the safe direction.
//!
//! Why no sync per line: a lost line leaves a tab with no task after a restart, hidden from runs
//! bound to a task, never shown to another task. A task's page lost the same way falls back to its
//! newest tab. Neither is worth a disk flush on every open, popup and act.
//!
//! The line format: one self-contained JSON object a line, and the latest line for a target wins.
//!
//! ```text
//!  {"target":"<id>","task":"S","family":"S","origin":"host"}   the tab is held
//!  {"target":"<id>","page":true}                                it became its task's page
//!  {"target":"<id>","gone":true}                                it left its task
//! ```
//!
//! Each line is written after a newline of its own, so one an earlier write left half done costs
//! only itself: a torn line is not JSON, and is skipped.
//!
//! Limits, named: a tab whose target changes while it lives (if the browser ever does that) is
//! saved under its new target only at its next holder change, so a restart before that leaves it
//! with no task; and the tabs a task gets back are the owner's to the idle-tab janitor
//! (`TabOrigin::Found`), as every tab from before a restart is.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use browseros_core::{TabId, TargetId};
use neo_trace::TaskId;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{SetOnce, mpsc, oneshot};
use tokio::{fs as async_fs, task, time};

use super::LiveTab;
use crate::browser::{
    BookChange, BookSink, BrowserLink, Holder, PageRef, SavedTask, TabRegistry, TaskOrigin,
};

/// The record's file name, in the trace's folder.
pub(crate) const TAB_HOLDERS_FILE: &str = "tab-holders";

/// Dead lines (superseded, gone or torn) a rewrite waits for, however few lines are live, so a
/// small book is not rewritten every few changes.
const COMPACT_FLOOR: usize = 256;

/// How often the start asks the browser for its tabs, how long each ask may take, and the pause
/// between asks. Together they bound how long a run's start waits for the restore.
const LISTING_ATTEMPTS: u32 = 3;
const LISTING_WAIT: Duration = Duration::from_secs(3);
const LISTING_RETRY: Duration = Duration::from_millis(500);

/// The record. One per process in the sidecar: the start restores from it, then one writer task
/// appends the task book's changes. The Node embedding keeps none.
pub(crate) struct TabHolders {
    path: PathBuf,
    /// The fold of the file and the appender under one std lock, never held across an await: an
    /// append or a rewrite holds it through its write, on a blocking thread, so it writes the
    /// file the fold describes at that moment.
    state: Arc<Mutex<State>>,
    /// The file existed and could not be read at this start. Nothing is restored, and nothing is
    /// written: the file stays as it is for a person to look at, and the next start tries again.
    unreadable: bool,
    /// Set once the start's restore ended, however it ended. A run's start waits for it.
    ready: SetOnce<()>,
    /// How the start's restore ended, set before [`TabHolders::ready`] opens.
    outcome: OnceLock<RestoreState>,
}

/// How the start's restore of the record ended, as `GET /system/health` reports it in
/// `tabHolders.state`. Under task scope a lost record shows only as bound sessions losing their
/// tabs after a restart of Neo's server, so a host reads here whether that was Neo's file. It
/// names no tab and no task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RestoreState {
    /// The restore has not ended yet; runs wait for it.
    Pending,
    /// It ran: every saved tab the browser still has went back to its task. Zero after a
    /// browser restart, whose tabs are all new, is still a restore.
    Restored,
    /// The file could not be read: nothing was restored, nothing is saved for this process's
    /// life, and the file is left for a person to look at.
    Unreadable,
    /// The browser's tabs could not be listed at start: nothing was restored, and the file is
    /// left as it was.
    Unlisted,
}

/// What the lock holds: the file as the latest line for each target says, and this process's
/// tabs whose target the writer knows.
#[derive(Default)]
struct State {
    /// Each held target, as its latest line says.
    held: HashMap<TargetId, Saved>,
    /// Each task's page, as the latest page line among its held targets says.
    pages: HashMap<TaskId, TargetId>,
    /// The target of each tab this process restored or wrote a holder for, so a later page mark
    /// or departure names the target the tab had, even once core forgot the closed tab.
    tabs: HashMap<TabId, TargetId>,
    /// Lines in the file, live, superseded and torn alike.
    lines: usize,
    /// The order the next newly held target takes.
    next_order: u64,
    /// The appender, opened on first use.
    file: Option<File>,
}

/// One held target, as the file has it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Saved {
    task: TaskId,
    family: TaskId,
    origin: TaskOrigin,
    /// When the target was first held, in file order: a task's tabs come back in the order they
    /// joined it, so its newest is still its newest.
    order: u64,
}

/// One line of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    Held {
        target: TargetId,
        task: TaskId,
        family: TaskId,
        origin: TaskOrigin,
    },
    Page {
        target: TargetId,
    },
    Gone {
        target: TargetId,
    },
}

/// A line as read: every field the three shapes use, so a line is one shape or is skipped.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLine {
    target: String,
    task: Option<TaskId>,
    family: Option<TaskId>,
    origin: Option<TaskOrigin>,
    #[serde(default)]
    page: bool,
    #[serde(default)]
    gone: bool,
}

impl Line {
    /// The line `text` holds, or `None` for a torn, empty or unknown one.
    fn parse(text: &str) -> Option<Self> {
        let raw: RawLine = serde_json::from_str(text).ok()?;
        let target = TargetId::from(raw.target);
        match (raw.task, raw.family, raw.origin, raw.page, raw.gone) {
            (Some(task), Some(family), Some(origin), false, false) => Some(Self::Held {
                target,
                task,
                family,
                origin,
            }),
            (None, None, None, true, false) => Some(Self::Page { target }),
            (None, None, None, false, true) => Some(Self::Gone { target }),
            _ => None,
        }
    }

    fn to_json(&self) -> String {
        match self {
            Self::Held {
                target,
                task,
                family,
                origin,
            } => json!({ "target": target.0, "task": task, "family": family, "origin": origin }),
            Self::Page { target } => json!({ "target": target.0, "page": true }),
            Self::Gone { target } => json!({ "target": target.0, "gone": true }),
        }
        .to_string()
    }
}

impl State {
    /// Folds one line in: the latest line for a target wins.
    fn apply(&mut self, line: &Line) {
        match line {
            Line::Held {
                target,
                task,
                family,
                origin,
            } => {
                if let Some(saved) = self.held.get_mut(target) {
                    if saved.task != *task && self.pages.get(&saved.task) == Some(target) {
                        self.pages.remove(&saved.task);
                    }
                    saved.task = task.clone();
                    saved.family = family.clone();
                    saved.origin = *origin;
                } else {
                    self.held.insert(
                        target.clone(),
                        Saved {
                            task: task.clone(),
                            family: family.clone(),
                            origin: *origin,
                            order: self.next_order,
                        },
                    );
                    self.next_order += 1;
                }
            }
            Line::Page { target } => {
                if let Some(saved) = self.held.get(target) {
                    self.pages.insert(saved.task.clone(), target.clone());
                }
            }
            Line::Gone { target } => {
                if let Some(saved) = self.held.remove(target)
                    && self.pages.get(&saved.task) == Some(target)
                {
                    self.pages.remove(&saved.task);
                }
            }
        }
    }

    /// The lines one book change adds, folded in already. `target` is the tab's target as the
    /// writer resolved it, for a [`BookChange::Held`] only.
    fn lines_for(&mut self, change: &BookChange, target: Option<TargetId>) -> Vec<Line> {
        let lines = match change {
            BookChange::Held {
                tab,
                holder: Holder { task, family },
                origin,
            } => {
                let Some(target) = target else {
                    return Vec::new();
                };
                let mut lines = Vec::new();
                // A tab whose target changed while it lived: its old target leaves.
                if let Some(old) = self.tabs.insert(*tab, target.clone())
                    && old != target
                    && self.held.contains_key(&old)
                {
                    lines.push(Line::Gone { target: old });
                }
                let same = self.held.get(&target).is_some_and(|saved| {
                    saved.task == *task && saved.family == *family && saved.origin == *origin
                });
                if !same {
                    lines.push(Line::Held {
                        target,
                        task: task.clone(),
                        family: family.clone(),
                        origin: *origin,
                    });
                }
                lines
            }
            BookChange::Page { tab } => {
                let Some(target) = self.tabs.get(tab) else {
                    return Vec::new();
                };
                match self.held.get(target) {
                    Some(saved) if self.pages.get(&saved.task) != Some(target) => {
                        vec![Line::Page {
                            target: target.clone(),
                        }]
                    }
                    Some(_) | None => Vec::new(),
                }
            }
            BookChange::Gone { tab } => match self.tabs.remove(tab) {
                Some(target) if self.held.contains_key(&target) => vec![Line::Gone { target }],
                Some(_) | None => Vec::new(),
            },
        };
        for line in &lines {
            self.apply(line);
        }
        lines
    }

    fn live_lines(&self) -> usize {
        self.held.len() + self.pages.len()
    }

    /// Whether a rewrite is due: dead lines outnumber live ones, and are at least the floor.
    fn compaction_due(&self) -> bool {
        let dead = self.lines.saturating_sub(self.live_lines());
        dead > self.live_lines() && dead >= COMPACT_FLOOR
    }

    /// The live lines, held targets in the order they were first held, then the pages, which
    /// name a target already held above them.
    fn compacted(&self) -> Vec<Line> {
        let mut held: Vec<(&TargetId, &Saved)> = self.held.iter().collect();
        held.sort_by_key(|(_, saved)| saved.order);
        let mut lines: Vec<Line> = held
            .into_iter()
            .map(|(target, saved)| Line::Held {
                target: target.clone(),
                task: saved.task.clone(),
                family: saved.family.clone(),
                origin: saved.origin,
            })
            .collect();
        let mut pages: Vec<(&TaskId, &TargetId)> = self.pages.iter().collect();
        pages.sort_by_key(|(_, target)| self.held.get(*target).map(|saved| saved.order));
        lines.extend(pages.into_iter().map(|(_, target)| Line::Page {
            target: target.clone(),
        }));
        lines
    }
}

impl TabHolders {
    /// The record at `folder/tab-holders`, read back. A file that exists and cannot be read
    /// restores nothing and is left for a person.
    pub(crate) async fn open(folder: &Path) -> Arc<Self> {
        let path = folder.join(TAB_HOLDERS_FILE);
        let (text, unreadable) = match async_fs::read(&path).await {
            Ok(bytes) => (String::from_utf8_lossy(&bytes).into_owned(), false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (String::new(), false),
            Err(error) => {
                tracing::error!(%error, file = %path.display(), "cannot read which task holds which tab; no task gets its tabs back at this start, and the file is left as it is");
                (String::new(), true)
            }
        };
        let mut state = State::default();
        for text in text.lines().filter(|line| !line.trim().is_empty()) {
            state.lines += 1;
            if let Some(line) = Line::parse(text) {
                state.apply(&line);
            }
        }
        Arc::new(Self {
            path,
            state: Arc::new(Mutex::new(state)),
            unreadable,
            ready: SetOnce::new(),
            outcome: OnceLock::new(),
        })
    }

    /// How the start's restore ended, or `Pending` until it does.
    #[must_use]
    pub(crate) fn state(&self) -> RestoreState {
        self.outcome.get().copied().unwrap_or(RestoreState::Pending)
    }

    /// Waits until the start's restore ended. Each task's tabs are in the book from then on, so a
    /// run's page default and listing see them. Bounded by the restore's own listing budget.
    pub(crate) async fn ready(&self) {
        self.ready.wait().await;
    }

    /// The tasks whose saved tabs are among `live` (the browser's own tab list, unfiltered), each
    /// with its tabs in the order they joined it and the tab the file last marked as its page.
    /// Remembers each matched tab's target. Writes nothing: [`TabHolders::keep_restored`] does,
    /// once the book took them.
    fn live(&self, live: &[LiveTab]) -> Vec<SavedTask> {
        if self.unreadable {
            return Vec::new();
        }
        let mut state = lock(&self.state);
        let mut matched: Vec<(&Saved, &LiveTab)> = live
            .iter()
            .filter_map(|tab| state.held.get(&tab.target).map(|saved| (saved, tab)))
            .collect();
        matched.sort_by_key(|(saved, _)| saved.order);
        let mut tasks: Vec<SavedTask> = Vec::new();
        for (saved, tab) in &matched {
            let page = state.pages.get(&saved.task) == Some(&tab.target);
            match tasks.iter_mut().find(|task| task.id == saved.task) {
                Some(task) => {
                    task.tabs.push(tab.tab);
                    // It only ever moves from an agent's to a host's.
                    if saved.origin == TaskOrigin::Host {
                        task.origin = TaskOrigin::Host;
                    }
                    if page {
                        task.page = Some(tab.tab);
                    }
                }
                None => tasks.push(SavedTask {
                    id: saved.task.clone(),
                    family: saved.family.clone(),
                    origin: saved.origin,
                    tabs: vec![tab.tab],
                    page: page.then_some(tab.tab),
                }),
            }
        }
        let tabs: HashMap<TabId, TargetId> = matched
            .iter()
            .map(|(_, tab)| (tab.tab, tab.target.clone()))
            .collect();
        state.tabs = tabs;
        tasks
    }

    /// Keeps only the targets of `restored`, the tabs the book took back, and rewrites the file
    /// with them alone: a target the browser no longer has, or a tab the book did not take (Neo's
    /// blank tab, a tab that closed meanwhile), is dropped.
    async fn keep_restored(&self, restored: &HashSet<TabId>) {
        {
            let mut state = lock(&self.state);
            state.tabs.retain(|tab, _| restored.contains(tab));
            let kept: HashSet<TargetId> = state.tabs.values().cloned().collect();
            state.held.retain(|target, _| kept.contains(target));
            let State { held, pages, .. } = &mut *state;
            pages.retain(|_, target| held.contains_key(target));
        }
        let _compacted = self.rewrite().await;
    }

    /// Restores the book from the file, once, at start: every task with a saved tab the browser
    /// still has gets it back, with its page. Opens [`TabHolders::ready`] however it ends. A
    /// listing that keeps failing restores nothing and leaves the file as it is: which saved
    /// tabs are still open cannot be told, and no tab is given to a task on a guess.
    pub(crate) async fn restore(&self, link: &BrowserLink, tabs: &TabRegistry) {
        let _ready = Ready(&self.ready);
        let state = self.take_back(link, tabs).await;
        let _ = self.outcome.set(state);
    }

    /// [`TabHolders::restore`]'s work, answering how it ended.
    async fn take_back(&self, link: &BrowserLink, tabs: &TabRegistry) -> RestoreState {
        if self.unreadable {
            return RestoreState::Unreadable;
        }
        let (nothing_held, lines) = {
            let state = lock(&self.state);
            (state.held.is_empty(), state.lines)
        };
        if nothing_held {
            // Nothing to give back; only dead lines, if any, to drop.
            if lines > 0 {
                let _compacted = self.rewrite().await;
            }
            return RestoreState::Restored;
        }
        let Some(live) = every_tab(link).await else {
            tracing::error!(
                "cannot list the browser's tabs at start; no task gets its tabs back, and the file of which task holds which tab is left as it is"
            );
            return RestoreState::Unlisted;
        };
        // Core's own listing next, so each live tab is in its cache before it is registered. A
        // tab it leaves out (an extension or devtools page) holds no task while Neo runs either:
        // core reports it closed. A listing that fails proves nothing, so nothing is dropped.
        let listed = match link.view() {
            Some(view) => matches!(
                time::timeout(LISTING_WAIT, view.view.list_pages()).await,
                Ok(Ok(_))
            ),
            None => false,
        };
        if !listed {
            tracing::error!(
                "core could not list the browser's tabs at start; no task gets its tabs back, and the file of which task holds which tab is left as it is"
            );
            return RestoreState::Unlisted;
        }
        let saved = self.live(&live);
        let mut tasks = Vec::with_capacity(saved.len());
        for mut task in saved {
            let mut registered = Vec::with_capacity(task.tabs.len());
            for tab in task.tabs {
                // Neo's blank tab may have taken one; it is no task's.
                if tabs.is_neo(tab) {
                    continue;
                }
                // Still the target it was saved under: if the browser restarted since the
                // listing, its number may name another tab.
                let saved_target = lock(&self.state).tabs.get(&tab).cloned();
                // Registered first: the book takes only a tab the registry has an entry for.
                let seen = tabs.seen();
                if link
                    .page(tab)
                    .await
                    .is_some_and(|info| Some(info.target_id) == saved_target)
                {
                    tabs.touched(&PageRef { tab }, seen);
                    registered.push(tab);
                }
            }
            task.page = task.page.filter(|page| registered.contains(page));
            task.tabs = registered;
            if !task.tabs.is_empty() {
                tasks.push(task);
            }
        }
        if let Err(error) = tabs.restore_book(tasks.clone()) {
            tracing::warn!(%error, "the task book could not take back every saved tab");
        }
        let restored: HashSet<TabId> = tasks
            .iter()
            .flat_map(|task| {
                task.tabs.iter().copied().filter(|tab| {
                    tabs.holder(*tab)
                        .is_some_and(|holder| holder.task == task.id)
                })
            })
            .collect();
        self.keep_restored(&restored).await;
        RestoreState::Restored
    }

    /// Appends the lines `changes` add, in order. A held tab's target is read from core's cache
    /// with a [`Seen`](crate::browser::Seen) ticket, so a tab that closed meanwhile is never
    /// written; its departure follows in the queue. Nothing while the file was unreadable.
    async fn record(&self, link: &BrowserLink, tabs: &TabRegistry, changes: Vec<BookChange>) {
        if self.unreadable {
            return;
        }
        let mut text = String::new();
        let mut added = 0;
        for change in changes {
            let target = match &change {
                BookChange::Held { tab, .. } => target_of(link, tabs, *tab).await,
                BookChange::Page { .. } | BookChange::Gone { .. } => None,
            };
            let lines = lock(&self.state).lines_for(&change, target);
            for line in lines {
                text.push('\n');
                text.push_str(&line.to_json());
                text.push('\n');
                added += 1;
            }
        }
        if added == 0 {
            return;
        }
        let compact = {
            let mut state = lock(&self.state);
            state.lines += added;
            state.compaction_due()
        };
        // A rewrite holds every line folded in, the ones above included; if it fails, they are
        // appended to the old file instead.
        if compact && self.rewrite().await {
            return;
        }
        if let Err(error) = self.append(text).await {
            tracing::warn!(%error, "cannot save which task holds which tab; after a restart of Neo's server those tabs hold no task");
        }
    }

    /// Appends `text` without a sync, opening the file on first use. Blocking work, on a blocking
    /// thread.
    async fn append(&self, text: String) -> io::Result<()> {
        let state = Arc::clone(&self.state);
        let path = self.path.clone();
        task::spawn_blocking(move || {
            let mut state = lock(&state);
            if state.file.is_none() {
                state.file = Some(OpenOptions::new().create(true).append(true).open(&path)?);
            }
            let Some(file) = state.file.as_mut() else {
                return Err(io::Error::other("the tab-holders file did not open"));
            };
            let written = file.write_all(text.as_bytes());
            if written.is_err() {
                // A half-written line may be in the file; it names no holder. The next append
                // opens a fresh handle, and starts on a line of its own.
                state.file = None;
            }
            written
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Rewrites the file with the live lines alone, synced, through a temporary file and a
    /// rename. Whether it did: a failure leaves the old file, which the next start folds the same
    /// way. Never while the file was unreadable at start.
    async fn rewrite(&self) -> bool {
        if self.unreadable {
            return false;
        }
        let state = Arc::clone(&self.state);
        let path = self.path.clone();
        let written = task::spawn_blocking(move || {
            let mut state = lock(&state);
            let lines = state.compacted();
            let mut text = String::new();
            for line in &lines {
                text.push_str(&line.to_json());
                text.push('\n');
            }
            let temporary = path.with_extension("tmp");
            let mut out = File::create(&temporary)?;
            out.write_all(text.as_bytes())?;
            out.sync_all()?;
            drop(out);
            fs::rename(&temporary, &path)?;
            state.lines = lines.len();
            // The rename replaced the file the appender held: the next append opens afresh, or
            // it would write to a file no longer in the folder.
            state.file = None;
            if let Err(error) = sync_folder(&path) {
                tracing::warn!(%error, "the compacted file of which task holds which tab is in place, but its folder was not synced");
            }
            Ok::<(), io::Error>(())
        })
        .await
        .map_err(io::Error::other)
        .and_then(|done| done);
        if let Err(error) = &written {
            tracing::warn!(%error, "cannot compact the file of which task holds which tab; the old one stays");
        }
        written.is_ok()
    }

    /// The writer: restores the book once the browser link first connects, then appends the
    /// book's changes in the order the book made them, a batch at a time, for the process's life.
    pub(crate) async fn keep(
        self: Arc<Self>,
        link: Arc<BrowserLink>,
        tabs: Arc<TabRegistry>,
        mut notes: mpsc::UnboundedReceiver<Note>,
    ) {
        link.linked().await;
        self.restore(&link, &tabs).await;
        while let Some(note) = notes.recv().await {
            let mut changes = Vec::new();
            let mut flushes = Vec::new();
            let mut next = Some(note);
            while let Some(note) = next {
                match note {
                    Note::Change(change) => changes.push(change),
                    Note::Flush(done) => flushes.push(done),
                }
                next = notes.try_recv().ok();
            }
            self.record(&link, &tabs, changes).await;
            for done in flushes {
                let _ = done.send(());
            }
        }
    }
}

/// What the writer task takes, in the order it was sent.
#[derive(Debug)]
pub(crate) enum Note {
    /// A change of the task book.
    Change(BookChange),
    /// Answered once every change sent before it is written.
    Flush(oneshot::Sender<()>),
}

/// The task book's sink: each change goes down a channel to the writer task, so the registry's
/// lock never waits on a disk.
pub(crate) struct HolderFeed {
    notes: mpsc::UnboundedSender<Note>,
}

impl HolderFeed {
    /// The feed, and the receiver its writer ([`TabHolders::keep`]) reads.
    pub(crate) fn channel() -> (Arc<Self>, mpsc::UnboundedReceiver<Note>) {
        let (notes, receiver) = mpsc::unbounded_channel();
        (Arc::new(Self { notes }), receiver)
    }

    /// Waits until every change sent so far is written (or the writer is gone).
    pub(crate) async fn flush(&self) {
        let (done, written) = oneshot::channel();
        if self.notes.send(Note::Flush(done)).is_ok() {
            let _ = written.await;
        }
    }
}

impl BookSink for HolderFeed {
    fn changed(&self, change: BookChange) {
        // No writer (it ended with the process): nothing to save to.
        let _ = self.notes.send(Note::Change(change));
    }
}

/// Opens [`TabHolders::ready`] when dropped, so a restore that ends early, or panics, never
/// leaves a run's start waiting.
#[must_use]
struct Ready<'a>(&'a SetOnce<()>);

impl Drop for Ready<'_> {
    fn drop(&mut self) {
        let _ = self.0.set(());
    }
}

/// The browser's own tab list, unfiltered (core's listing leaves out extension and devtools
/// pages, and a held tab can sit on one), within the start's listing budget.
async fn every_tab(link: &BrowserLink) -> Option<Vec<LiveTab>> {
    for attempt in 0..LISTING_ATTEMPTS {
        if attempt > 0 {
            time::sleep(LISTING_RETRY).await;
        }
        if let Ok(Some(tabs)) = time::timeout(LISTING_WAIT, link.every_tab()).await {
            return Some(
                tabs.into_iter()
                    .map(|(target, tab)| LiveTab { target, tab })
                    .collect(),
            );
        }
    }
    None
}

/// `tab`'s target, from core's cache, when no close of the tab came between the ticket and the
/// read: after a browser restart its number may name another tab.
async fn target_of(link: &BrowserLink, tabs: &TabRegistry, tab: TabId) -> Option<TargetId> {
    let seen = tabs.seen();
    let info = link.page(tab).await?;
    tabs.read_is_current(tab, seen).then_some(info.target_id)
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Syncs the folder that holds `path`, so a file renamed there survives a power cut.
fn sync_folder(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(folder) => File::open(folder)?.sync_all(),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_core::{TabId, TargetId};
    use neo_trace::TaskId;
    use serde_json::{Value, json};
    use tokio::time;

    use super::{HolderFeed, LiveTab, RestoreState, TAB_HOLDERS_FILE, TabHolders};
    use crate::browser::{
        BookChange, BookSink, Holder, PageRef, SavedTask, TabRegistry, TaskOrigin, TaskPage,
    };
    use crate::test_rig::{TestError, TestRig, on_neo_runtime, within};

    fn task(id: &str) -> Result<TaskId, TestError> {
        Ok(TaskId::parse(id)?)
    }

    fn held(tab: TabId, task: &TaskId, family: &TaskId, origin: TaskOrigin) -> BookChange {
        BookChange::Held {
            tab,
            holder: Holder {
                task: task.clone(),
                family: family.clone(),
            },
            origin,
        }
    }

    /// The file's lines, each as JSON; a line that is not JSON reads as a string.
    fn lines(folder: &Path) -> Result<Vec<Value>, TestError> {
        Ok(fs::read_to_string(folder.join(TAB_HOLDERS_FILE))?
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).unwrap_or_else(|_| json!(line)))
            .collect())
    }

    /// Opens a tab the owner's way and returns it with its target, as core's cache has it.
    async fn tab(rig: &TestRig, url: &str) -> Result<(TabId, String), TestError> {
        let page = rig.owner_tab(url).await?;
        let info = rig.link.page(page.tab).await.ok_or("core caches the tab")?;
        Ok((page.tab, info.target_id.0))
    }

    /// The browser's own tab list, as the start reads it.
    async fn browser_tabs(rig: &TestRig) -> Result<Vec<LiveTab>, TestError> {
        Ok(rig
            .link
            .every_tab()
            .await
            .ok_or("the fake lists its tabs")?
            .into_iter()
            .map(|(target, tab)| LiveTab { target, tab })
            .collect())
    }

    #[test]
    fn each_change_is_one_line_by_target_and_a_restart_reads_back_each_tasks_tabs_and_page()
    -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let (one, one_target) = tab(&rig, "https://one.example/").await?;
            let (two, two_target) = tab(&rig, "https://two.example/").await?;
            let (three, three_target) = tab(&rig, "https://three.example/").await?;
            let (four, four_target) = tab(&rig, "https://four.example/").await?;
            let host = task("host-task")?;
            let created = task("created-1")?;
            let holders = TabHolders::open(folder.path()).await;
            holders
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![
                        held(one, &host, &host, TaskOrigin::Host),
                        held(two, &host, &host, TaskOrigin::Host),
                        BookChange::Page { tab: two },
                        held(three, &created, &host, TaskOrigin::Agent),
                        BookChange::Page { tab: one },
                        // Already saved as it is: nothing to add.
                        held(one, &host, &host, TaskOrigin::Host),
                        BookChange::Page { tab: one },
                        held(four, &host, &host, TaskOrigin::Host),
                        BookChange::Gone { tab: four },
                        // A tab the writer never saw held: nothing to say.
                        BookChange::Gone { tab: TabId(9999) },
                    ],
                )
                .await;
            let hold = |target: &str, task: &str, origin: &str| json!({ "target": target, "task": task, "family": "host-task", "origin": origin });
            assert_eq!(
                lines(folder.path())?,
                vec![
                    hold(&one_target, "host-task", "host"),
                    hold(&two_target, "host-task", "host"),
                    json!({ "target": two_target, "page": true }),
                    hold(&three_target, "created-1", "agent"),
                    json!({ "target": one_target, "page": true }),
                    hold(&four_target, "host-task", "host"),
                    json!({ "target": four_target, "gone": true }),
                ],
                "one line per change, by target, never by tab number"
            );

            let reopened = TabHolders::open(folder.path()).await;
            assert_eq!(
                reopened.live(&browser_tabs(&rig).await?),
                vec![
                    SavedTask {
                        id: host.clone(),
                        family: host.clone(),
                        origin: TaskOrigin::Host,
                        tabs: vec![one, two],
                        page: Some(one),
                    },
                    SavedTask {
                        id: created,
                        family: host,
                        origin: TaskOrigin::Agent,
                        tabs: vec![three],
                        page: None,
                    },
                ],
                "each task's tabs in the order they joined it, and the page its file marked last"
            );
            Ok(())
        })
    }

    #[test]
    fn a_restart_gives_the_book_back_the_tabs_the_browser_kept_and_compacts_the_file()
    -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let (kept, kept_target) = tab(&rig, "https://kept.example/").await?;
            let (newest, newest_target) = tab(&rig, "https://newest.example/").await?;
            let (closed, _) = tab(&rig, "https://closed.example/").await?;
            let host = task("host-task")?;
            let holders = TabHolders::open(folder.path()).await;
            holders
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![
                        held(kept, &host, &host, TaskOrigin::Host),
                        held(newest, &host, &host, TaskOrigin::Host),
                        held(closed, &host, &host, TaskOrigin::Host),
                        BookChange::Page { tab: kept },
                        BookChange::Page { tab: newest },
                        BookChange::Page { tab: kept },
                    ],
                )
                .await;
            // Neo's server stops before the close reaches its file, and the browser lives on.
            assert!(rig.fake.close_tab(closed));

            let book = TabRegistry::new();
            let restarted = TabHolders::open(folder.path()).await;
            within(Duration::from_secs(5), restarted.restore(&rig.link, &book)).await?;
            within(Duration::from_secs(1), restarted.ready()).await?;
            assert_eq!(book.owner(kept), Some(host.clone()));
            assert_eq!(book.owner(newest), Some(host.clone()));
            assert_eq!(
                book.owner(closed),
                None,
                "a target the browser no longer has"
            );
            assert_eq!(
                book.task_page(&host),
                TaskPage::Tab(PageRef { tab: kept }),
                "its page is the one it last acted on, not its newest"
            );
            let hold = |target: &str| json!({ "target": target, "task": "host-task", "family": "host-task", "origin": "host" });
            assert_eq!(
                lines(folder.path())?,
                vec![
                    hold(&kept_target),
                    hold(&newest_target),
                    json!({ "target": kept_target, "page": true }),
                ],
                "rewritten with what the book took back, nothing else"
            );
            Ok(())
        })
    }

    #[test]
    fn a_popup_named_before_the_restore_joins_its_openers_task() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let (opener, _) = tab(&rig, "https://opener.example/").await?;
            let host = task("host-task")?;
            let holders = TabHolders::open(folder.path()).await;
            holders
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![held(opener, &host, &host, TaskOrigin::Host)],
                )
                .await;
            // While Neo's server was down, the task's page opened a popup, and the new process's
            // tab watch names it before the book gets the opener back.
            let popup = rig.fake.open_popup(opener, "https://popup.example/");
            let book = TabRegistry::new();
            book.popup(&PageRef { tab: popup }, opener, book.seen());
            assert_eq!(book.owner(popup), None, "its opener holds no task yet");
            let restarted = TabHolders::open(folder.path()).await;
            within(Duration::from_secs(5), restarted.restore(&rig.link, &book)).await?;
            assert_eq!(book.owner(opener), Some(host.clone()));
            assert_eq!(
                book.owner(popup),
                Some(host),
                "a popup follows its opener's task, also when the opener came back from the file"
            );
            Ok(())
        })
    }

    #[test]
    fn a_browser_restart_gives_nothing_back() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let (tab, _) = tab(&rig, "https://reused-number.example/").await?;
            // The tab numbers repeat in a new browser; its targets never do.
            fs::write(
                folder.path().join(TAB_HOLDERS_FILE),
                format!(
                    "{}\n{}\n",
                    json!({ "target": "an-old-browsers-target", "task": "host-task", "family": "host-task", "origin": "host" }),
                    json!({ "target": "an-old-browsers-target", "page": true }),
                ),
            )?;
            let book = TabRegistry::new();
            let holders = TabHolders::open(folder.path()).await;
            within(Duration::from_secs(5), holders.restore(&rig.link, &book)).await?;
            assert_eq!(book.owner(tab), None);
            assert_eq!(book.task_page(&task("host-task")?), TaskPage::Unknown);
            assert_eq!(lines(folder.path())?, Vec::<Value>::new());
            Ok(())
        })
    }

    #[test]
    fn a_listing_that_fails_gives_nothing_back_and_leaves_the_file_as_it_is()
    -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let (tab, _) = tab(&rig, "https://held.example/").await?;
            let host = task("host-task")?;
            let holders = TabHolders::open(folder.path()).await;
            holders
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![held(tab, &host, &host, TaskOrigin::Host)],
                )
                .await;
            let before = fs::read(folder.path().join(TAB_HOLDERS_FILE))?;
            rig.fake.answer("Browser.getTabs", json!({}));
            let book = TabRegistry::new();
            let restarted = TabHolders::open(folder.path()).await;
            within(Duration::from_secs(5), restarted.restore(&rig.link, &book)).await?;
            within(Duration::from_secs(1), restarted.ready()).await?;
            assert_eq!(
                book.owner(tab),
                None,
                "no tab is given on a guess: fails closed"
            );
            assert_eq!(fs::read(folder.path().join(TAB_HOLDERS_FILE))?, before);
            Ok(())
        })
    }

    #[test]
    fn a_file_that_cannot_be_read_gives_nothing_back_and_is_never_written() -> Result<(), TestError>
    {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let (tab, _) = tab(&rig, "https://held.example/").await?;
            // A directory where the file should be: it exists and cannot be read.
            fs::create_dir(folder.path().join(TAB_HOLDERS_FILE))?;
            let holders = TabHolders::open(folder.path()).await;
            let book = TabRegistry::new();
            within(Duration::from_secs(5), holders.restore(&rig.link, &book)).await?;
            within(Duration::from_secs(1), holders.ready()).await?;
            let host = task("host-task")?;
            holders
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![held(tab, &host, &host, TaskOrigin::Host)],
                )
                .await;
            assert_eq!(book.owner(tab), None);
            assert!(
                folder.path().join(TAB_HOLDERS_FILE).is_dir(),
                "left for a person to look at"
            );
            Ok(())
        })
    }

    #[test]
    fn a_torn_or_unknown_line_names_no_holder() -> Result<(), TestError> {
        on_neo_runtime(async {
            let folder = tempfile::tempdir()?;
            let hold = |target: &str, task: &str| {
                json!({ "target": target, "task": task, "family": task, "origin": "host" })
                    .to_string()
            };
            let torn = hold("target-2", "b");
            let text = [
                hold("target-1", "a"),
                torn.get(..torn.len() - 9).ok_or("a torn line")?.to_owned(),
                json!({ "target": "target-3", "task": "c", "family": "c", "origin": "host", "later": 1 })
                    .to_string(),
                json!({ "target": "target-4", "page": false }).to_string(),
                json!({ "target": "target-5", "task": "e", "family": "e", "origin": "elsewhere" })
                    .to_string(),
                hold("target-6", "f"),
            ]
            .join("\n");
            fs::write(folder.path().join(TAB_HOLDERS_FILE), text)?;
            let holders = TabHolders::open(folder.path()).await;
            let live: Vec<LiveTab> = (1..=6)
                .map(|tab| LiveTab {
                    target: TargetId::from(format!("target-{tab}")),
                    tab: TabId(tab),
                })
                .collect();
            let tasks: Vec<String> = holders
                .live(&live)
                .into_iter()
                .map(|task| task.id.to_string())
                .collect();
            assert_eq!(tasks, vec!["a", "f"]);
            Ok(())
        })
    }

    #[test]
    fn a_closed_tab_is_never_written_and_its_departure_names_the_target_it_had()
    -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let (saved, saved_target) = tab(&rig, "https://saved.example/").await?;
            let (late, _) = tab(&rig, "https://late.example/").await?;
            let host = task("host-task")?;
            let holders = TabHolders::open(folder.path()).await;
            holders
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![held(saved, &host, &host, TaskOrigin::Host)],
                )
                .await;
            assert!(rig.fake.close_tab(saved));
            assert!(rig.fake.close_tab(late));
            within(Duration::from_secs(5), async {
                while rig.link.page(saved).await.is_some() || rig.link.page(late).await.is_some() {
                    time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await?;
            holders
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![
                        // Its join reached the writer only after it closed.
                        held(late, &host, &host, TaskOrigin::Host),
                        BookChange::Page { tab: late },
                        BookChange::Gone { tab: late },
                        BookChange::Gone { tab: saved },
                    ],
                )
                .await;
            assert_eq!(
                lines(folder.path())?,
                vec![
                    json!({ "target": saved_target, "task": "host-task", "family": "host-task", "origin": "host" }),
                    json!({ "target": saved_target, "gone": true }),
                ],
                "core forgot the closed tab; the writer did not"
            );
            Ok(())
        })
    }

    #[test]
    fn a_task_moving_to_a_hosts_and_a_tab_changing_target_are_saved() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let (tab, old_target) = tab(&rig, "https://swapped.example/").await?;
            let created = task("created-1")?;
            let holders = TabHolders::open(folder.path()).await;
            holders
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![
                        held(tab, &created, &created, TaskOrigin::Agent),
                        // A host names the task an agent created.
                        held(tab, &created, &created, TaskOrigin::Host),
                    ],
                )
                .await;
            assert!(rig.fake.replace_target(tab));
            let new_target = within(Duration::from_secs(5), async {
                loop {
                    if let Some(info) = rig.link.page(tab).await
                        && info.target_id.0 != old_target
                    {
                        return info.target_id.0;
                    }
                    time::sleep(Duration::from_millis(5)).await;
                    // Core learns the new target at its next listing.
                    if let Some(view) = rig.link.view() {
                        let _ = view.view.list_pages().await;
                    }
                }
            })
            .await?;
            holders
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![held(tab, &created, &created, TaskOrigin::Host)],
                )
                .await;
            let hold = |target: &str, origin: &str| json!({ "target": target, "task": "created-1", "family": "created-1", "origin": origin });
            assert_eq!(
                lines(folder.path())?,
                vec![
                    hold(&old_target, "agent"),
                    hold(&old_target, "host"),
                    json!({ "target": old_target, "gone": true }),
                    hold(&new_target, "host"),
                ]
            );
            Ok(())
        })
    }

    #[test]
    fn dead_lines_are_compacted_away_once_they_outnumber_the_live_ones() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let (first, _) = tab(&rig, "https://first.example/").await?;
            let (second, second_target) = tab(&rig, "https://second.example/").await?;
            let host = task("host-task")?;
            let holders = TabHolders::open(folder.path()).await;
            holders
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![
                        held(first, &host, &host, TaskOrigin::Host),
                        held(second, &host, &host, TaskOrigin::Host),
                    ],
                )
                .await;
            // A script switching between its two tabs, one change a batch.
            for flip in 0..400 {
                let tab = if flip % 2 == 0 { first } else { second };
                holders
                    .record(&rig.link, &rig.tabs, vec![BookChange::Page { tab }])
                    .await;
            }
            let written = lines(folder.path())?;
            assert!(
                written.len() < 300,
                "rewritten at least once: {} lines",
                written.len()
            );
            assert_eq!(
                written.last(),
                Some(&json!({ "target": second_target, "page": true }))
            );
            let reopened = TabHolders::open(folder.path()).await;
            let tasks = reopened.live(&browser_tabs(&rig).await?);
            assert_eq!(
                tasks.first().map(|task| (task.tabs.clone(), task.page)),
                Some((vec![first, second], Some(second))),
                "the same book, in fewer lines"
            );
            Ok(())
        })
    }

    #[test]
    fn the_writer_restores_first_then_saves_each_change_in_order() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let (tab, target) = tab(&rig, "https://written.example/").await?;
            let host = task("host-task")?;
            let holders = TabHolders::open(folder.path()).await;
            let (feed, notes) = HolderFeed::channel();
            // Sent before the writer runs: kept in order until it does.
            feed.changed(held(tab, &host, &host, TaskOrigin::Host));
            feed.changed(BookChange::Page { tab });
            let writer = tokio::spawn(Arc::clone(&holders).keep(
                Arc::clone(&rig.link),
                Arc::clone(&rig.tabs),
                notes,
            ));
            within(Duration::from_secs(5), holders.ready()).await?;
            within(Duration::from_secs(5), feed.flush()).await?;
            assert_eq!(
                lines(folder.path())?,
                vec![
                    json!({ "target": target, "task": "host-task", "family": "host-task", "origin": "host" }),
                    json!({ "target": target, "page": true }),
                ]
            );
            writer.abort();
            Ok(())
        })
    }

    /// How each start's restore ended, as health reports it: the file unreadable, the browser's
    /// tabs unlisted, or restored, with zero tabs back after a whole browser restart too.
    #[test]
    fn each_restore_records_how_it_ended() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let host = task("host-task")?;

            let broken = tempfile::tempdir()?;
            fs::create_dir(broken.path().join(TAB_HOLDERS_FILE))?;
            let unreadable = TabHolders::open(broken.path()).await;
            within(
                Duration::from_secs(5),
                unreadable.restore(&rig.link, &TabRegistry::new()),
            )
            .await?;
            assert_eq!(unreadable.state(), RestoreState::Unreadable);

            let saved = tempfile::tempdir()?;
            let (kept, _) = tab(&rig, "https://kept.example/").await?;
            let (gone, _) = tab(&rig, "https://gone.example/").await?;
            let writer = TabHolders::open(saved.path()).await;
            writer
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![held(kept, &host, &host, TaskOrigin::Host)],
                )
                .await;
            let unlisted = TabHolders::open(saved.path()).await;
            assert_eq!(
                unlisted.state(),
                RestoreState::Pending,
                "before its restore"
            );
            rig.fake.answer("Browser.getTabs", json!({}));
            within(
                Duration::from_secs(15),
                unlisted.restore(&rig.link, &TabRegistry::new()),
            )
            .await?;
            rig.fake.unanswer("Browser.getTabs");
            assert_eq!(unlisted.state(), RestoreState::Unlisted);

            let elsewhere = tempfile::tempdir()?;
            let writer = TabHolders::open(elsewhere.path()).await;
            writer
                .record(
                    &rig.link,
                    &rig.tabs,
                    vec![held(gone, &host, &host, TaskOrigin::Host)],
                )
                .await;
            assert!(rig.fake.close_tab(gone), "the fake had the tab");
            let none_back = TabHolders::open(elsewhere.path()).await;
            let book = TabRegistry::new();
            within(Duration::from_secs(15), none_back.restore(&rig.link, &book)).await?;
            assert_eq!(
                none_back.state(),
                RestoreState::Restored,
                "the browser no longer has the saved tab: zero back is still a restore"
            );
            assert_eq!(book.owner(gone), None);
            Ok(())
        })
    }
}
