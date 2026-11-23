//! `conversations`: owns every conversation. It is the single writer of the `conversations`
//! table and of run state; `http` (and one day the cron job) only call it. A run belongs to
//! the server, not to a request: subscribers come and go, and only `stop`, `release`,
//! `delete` and `shutdown` cancel.
//!
//! ```text
//!   send ──▶ Run { log: Transcript + status } ◀── one task writes the row
//!              │                                   at admission, ToolInput, ToolDone and the end
//!              ├──▶ Subscription (cursor 0..)  the POST /chat response
//!              └──▶ Subscription (cursor 0..)  a second panel, replaying then live
//! ```
//!
//! Replay and live are one code path: a subscriber is a cursor into the run's append-only log,
//! and the log and its wake-up change under one lock, so "replay, then live" has no gap.
//!
//! Locks, always taken in this order and never held across an await: the slot map, then the
//! panel table, then one run's log.

mod run;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::browser::Browser;
use crate::store::{ConversationSummary, Store, StoreError};
use crate::turn::{
    AgentKind, Attachment, ChatError, ChatErrorCode, ConversationId, MessageId, Role, RunId,
    Target, TurnContext, TurnInput, TurnRunner, UiMessage, UiPart,
};

pub use run::Subscription;
use run::{Persist, Run, TurnJob, run_task, subscribe};

/// The run registry and the panel table. Cheap to clone.
#[derive(Clone)]
pub struct Conversations {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    runner: Arc<dyn TurnRunner>,
    /// Asked for the tabs a run's task opened after every `ToolDone` and before the terminal
    /// state is published, so a tab's `autoOpenId` reaches the panel table while the run is
    /// still `running` (the extension opens a panel only then).
    browser: Browser,
    slots: Mutex<HashMap<ConversationId, Slot>>,
    /// Numbers `Slot::Starting` and `Slot::Closing`, so a holder knows the slot is still its own.
    tickets: AtomicU64,
    /// Tab id → the run its side panel shows. A finished run stays assigned until the tab is
    /// forgotten, the conversation is released, or its next turn takes the tab over.
    panels: Mutex<BTreeMap<i64, PanelEntry>>,
    /// Every table published, in order, to every `/chat/panels` stream. A broadcast, not a
    /// watch: the extension opens a panel only when it sees a tab `running` with a new
    /// `autoOpenId`, and a watch keeps only the latest value, so a run that opens a tab and
    /// ends at once would replace that table with its terminal one before any reader saw it.
    panel_table: broadcast::Sender<Arc<PanelTable>>,
    /// The last table sent: the first event of a new stream, and its repair after a lag.
    current_table: Mutex<Arc<PanelTable>>,
    /// Set by `shutdown`, under the slot map's lock: no turn is installed after it, so the runs
    /// `shutdown` collected are all the runs there will be.
    closed: AtomicBool,
    /// Fired by `shutdown` one second before its deadline: every backend still running is
    /// dropped, so its run can seal and write its row in time.
    abandon: CancellationToken,
}

/// One lifecycle state per conversation, taken under the map's lock before any await.
/// `send` reserves `Starting` before it loads history, so two POSTs cannot both pass the
/// check; `release` and `delete` hold `Closing` until the run has ended (and, for delete, the
/// row is gone and the agent's session ended), so a POST in that window is `Busy` and cannot
/// start a second writer or bring a deleted row back.
enum Slot {
    Starting(u64),
    /// The conversation's current run, running or finished.
    Run(Arc<Run>),
    Closing {
        ticket: u64,
        /// Fired when the slot is given up, for a `release` or `delete` that waits its turn.
        done: CancellationToken,
        /// The run being stopped, so `busy` and `shutdown` still see it until it has ended.
        run: Option<Arc<Run>>,
    },
}

struct PanelEntry {
    run: Arc<Run>,
    auto_open_id: Option<String>,
}

/// Where a run is. Every status but `Running` is terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Completed,
    Aborted,
    Failed,
}

/// A turn as the domain sees it; `http` builds it from the request.
#[derive(Debug, Clone)]
pub struct NewTurn {
    pub conversation: ConversationId,
    /// The client's id for this user message. If the stored transcript already holds a
    /// message with this id, everything after it is dropped and it is answered again, so
    /// Retry and a repeated POST never store the question twice. `None`: the server makes one.
    pub user_message_id: Option<MessageId>,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub target: Target,
    pub context: TurnContext,
    pub history: History,
    /// The tab whose side panel asked; it is assigned to the run in the panel table.
    pub panel_tab: Option<i64>,
}

/// Incognito turns are never written; their history comes from the request instead.
#[derive(Debug, Clone, PartialEq)]
pub enum History {
    Stored,
    Ephemeral(Vec<UiMessage>),
}

/// A turn is already running for this conversation (`409`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

/// What `GET /chat/{id}/stream?runId=` finds.
pub enum Attach {
    /// No run is known for the conversation, or its replay log is gone (`204`).
    NoRun,
    /// A newer run replaced the one asked for (`409`).
    Changed,
    Run(Subscription),
}

/// The current run of a conversation, for a panel that did not start it.
#[derive(Debug, Clone)]
pub struct RunState {
    pub conversation: ConversationId,
    pub run: RunId,
    pub status: RunStatus,
    /// Running: the transcript up to and including this turn's user message (the stream
    /// supplies the answer). Finished: that plus the sealed answer, because the extension
    /// reads the stream only while `status` is `running`.
    pub messages: Arc<[UiMessage]>,
    pub frame_count: usize,
}

/// Which tab's side panel shows which run. Sent whole on every change and every 5 s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PanelTable {
    pub assignments: Vec<PanelAssignment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelAssignment {
    pub tab_id: i64,
    pub conversation: ConversationId,
    pub run: RunId,
    pub status: RunStatus,
    /// Set once for a tab the agent opened; tells the extension to open its panel there.
    pub auto_open_id: Option<String>,
}

/// A stored conversation with its transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    pub summary: ConversationSummary,
    pub messages: Vec<UiMessage>,
}

impl Conversations {
    /// Also starts one housekeeping task: the 5 s panel heartbeat and the retention rule.
    /// `browser` is asked for the tabs each run's task opened.
    pub fn new(store: Store, runner: Arc<dyn TurnRunner>, browser: Browser) -> Self {
        let inner = Arc::new(Inner {
            store,
            runner,
            browser,
            slots: Mutex::new(HashMap::new()),
            tickets: AtomicU64::new(0),
            panels: Mutex::new(BTreeMap::new()),
            panel_table: broadcast::channel(PANEL_BACKLOG).0,
            current_table: Mutex::new(Arc::new(PanelTable::default())),
            closed: AtomicBool::new(false),
            abandon: CancellationToken::new(),
        });
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(housekeeping(Arc::downgrade(&inner)));
        }
        Self { inner }
    }

    /// Starts a turn: refuses if one is running, loads the stored transcript (only when the
    /// row's target matches), places the user message, registers the run, spawns its task,
    /// and returns a subscription at frame 0. The spawned task owns every write for the run.
    pub async fn send(&self, turn: NewTurn) -> Result<Subscription, Busy> {
        let inner = &self.inner;
        let reservation = Reservation::take(inner, turn.conversation)?;

        let ephemeral = matches!(turn.history, History::Ephemeral(_));
        let (persist, prefix, refused) = match &turn.history {
            History::Ephemeral(previous) => (Persist::Never, previous.clone(), None),
            History::Stored => {
                let (target_type, agent_id) = row_target(&turn.target);
                match inner.store.conversations().load(turn.conversation).await {
                    Ok(row) => {
                        // Only a row this target owns is continued; another target's history
                        // must not bleed into this one (the old server's rule).
                        let stored = row
                            .filter(|row| {
                                row.target_type == target_type
                                    && (agent_id.is_none() || row.agent_id == agent_id)
                            })
                            .map(|row| row.messages)
                            .unwrap_or_default();
                        let persist = Persist::Row {
                            target_type,
                            agent_id,
                        };
                        (persist, stored, None)
                    }
                    Err(error) => {
                        tracing::warn!(conversation = %turn.conversation, %error, "could not read the conversation");
                        (Persist::Never, Vec::new(), Some(unreadable()))
                    }
                }
            }
        };
        let history = place_user_message(prefix, &turn);
        let run = Arc::new(Run::new(turn.conversation, history));
        reservation.install(run.clone())?;

        inner.attach_panels(&run, turn.panel_tab);
        let subscription = subscribe(run.clone());
        let input = TurnInput {
            conversation: turn.conversation,
            history: run.history.clone(),
            context: turn.context,
            attachments: turn.attachments,
            ephemeral,
        };
        let job = TurnJob {
            target: turn.target,
            input,
            persist,
            refused,
        };
        tokio::spawn(run_task(inner.clone(), run, job));
        Ok(subscription)
    }

    pub fn state(&self, id: ConversationId) -> Option<RunState> {
        let run = self.inner.current(id)?;
        let log = lock(&run.log);
        Some(RunState {
            conversation: id,
            run: run.id,
            status: log.status,
            messages: log.messages.clone(),
            frame_count: log.frame_count(),
        })
    }

    /// `run: None` attaches to the conversation's current run (today's behaviour when the
    /// query is absent). `Some(id)` that is not the current run is `Attach::Changed`. A run
    /// whose frame log is gone is `Attach::NoRun`.
    pub fn attach(&self, id: ConversationId, run: Option<RunId>) -> Attach {
        let Some(current) = self.inner.current(id) else {
            return Attach::NoRun;
        };
        if run.is_some_and(|asked| asked != current.id) {
            return Attach::Changed;
        }
        if !lock(&current.log).has_frames() {
            return Attach::NoRun;
        }
        Attach::Run(subscribe(current))
    }

    /// Fires the run's cancel token. Returns false when nothing was running.
    pub fn stop(&self, id: ConversationId) -> bool {
        match self.inner.current(id) {
            Some(run) if run.is_running() => {
                run.cancel.cancel();
                true
            }
            _ => false,
        }
    }

    /// `DELETE /chat/{id}`: stop, wait for the task, drop the run and its panels. Keeps the row.
    pub async fn release(&self, id: ConversationId) {
        let inner = self.inner.clone();
        // Its own task: a caller that goes away must not leave the slot `Closing`.
        let closing = tokio::spawn(async move {
            let Some(claim) = inner.claim(id).await else {
                return;
            };
            inner.end_run(id, &claim).await;
            inner.give_up(id, &claim);
        });
        if let Err(error) = closing.await {
            tracing::error!(conversation = %id, %error, "releasing a conversation failed");
        }
    }

    pub async fn list(&self) -> Result<Vec<ConversationSummary>, StoreError> {
        self.inner.store.conversations().list().await
    }

    /// The stored row as of its last write; a live run's newer frames are not in it.
    pub async fn get(&self, id: ConversationId) -> Result<Option<Conversation>, StoreError> {
        Ok(self
            .inner
            .store
            .conversations()
            .load_with_summary(id)
            .await?
            .map(|(summary, messages)| Conversation { summary, messages }))
    }

    /// `release`, then delete the row, then end the agent's session, in that order, so a
    /// late checkpoint cannot bring the row back.
    pub async fn delete(&self, id: ConversationId) -> Result<bool, StoreError> {
        let inner = self.inner.clone();
        let deleting = tokio::spawn(async move {
            let claim = inner.claim(id).await;
            if let Some(claim) = &claim {
                inner.end_run(id, claim).await;
            }
            let deleted = inner.store.conversations().delete(id).await;
            inner.runner.end_session(id).await;
            if let Some(claim) = &claim {
                inner.give_up(id, claim);
            }
            deleted
        });
        deleting.await.unwrap_or_else(|error| {
            Err(StoreError::Unreadable(format!(
                "deleting conversation {id} failed: {error}"
            )))
        })
    }

    /// Every panel table published from now on, in order. Subscribe first, then read
    /// `current_panels` for the first event: a table published in between then arrives twice,
    /// which is harmless because each event is the whole table.
    pub fn panels(&self) -> broadcast::Receiver<Arc<PanelTable>> {
        let _ordered = lock(&self.inner.panels);
        self.inner.panel_table.subscribe()
    }

    /// The table as last published.
    pub fn current_panels(&self) -> Arc<PanelTable> {
        lock(&self.inner.current_table).clone()
    }

    /// `DELETE /chat/panels/{tabId}`: a tab closed. A finished run that loses its last panel
    /// is forgotten with it; a running one stays.
    pub fn forget_panel(&self, tab_id: i64) {
        let inner = &self.inner;
        {
            let mut slots = lock(&inner.slots);
            let mut panels = lock(&inner.panels);
            let Some(entry) = panels.remove(&tab_id) else {
                return;
            };
            let orphaned = !entry.run.is_running()
                && !panels
                    .values()
                    .any(|other| Arc::ptr_eq(&other.run, &entry.run));
            if orphaned
                && matches!(slots.get(&entry.run.conversation), Some(Slot::Run(current)) if Arc::ptr_eq(current, &entry.run))
            {
                slots.remove(&entry.run.conversation);
            }
        }
        inner.publish_panels();
    }

    /// `GET /status` reports `can_update: !busy()`.
    pub fn busy(&self) -> bool {
        lock(&self.inner.slots).values().any(|slot| match slot {
            Slot::Starting(_) => true,
            Slot::Run(run) => run.is_running(),
            Slot::Closing { run, .. } => run.as_ref().is_some_and(|run| run.is_running()),
        })
    }

    /// Process shutdown: fire every run's cancel at once, drop a backend that has not returned
    /// by `deadline - 1 s`, write each sealed row, return by `deadline`. `app` passes 2.5 s of
    /// its 4 s total, because the launcher kills the process 5 s after SIGTERM.
    pub async fn shutdown(&self, deadline: Duration) {
        let started = tokio::time::Instant::now();
        let inner = &self.inner;
        let runs: Vec<Arc<Run>> = {
            let slots = lock(&inner.slots);
            inner.closed.store(true, Ordering::SeqCst);
            // A `send` still loading history (`Starting`) is refused at `install`.
            slots
                .values()
                .filter_map(|slot| match slot {
                    Slot::Run(run) => Some(run.clone()),
                    Slot::Closing { run, .. } => run.clone(),
                    Slot::Starting(_) => None,
                })
                .collect()
        };
        for run in &runs {
            run.cancel.cancel();
        }
        let ended = futures_util::future::join_all(runs.iter().map(|run| run.ended.cancelled()));
        let mut ended = std::pin::pin!(ended);
        let soft = started + deadline.saturating_sub(Duration::from_secs(1));
        if tokio::time::timeout_at(soft, &mut ended).await.is_err() {
            inner.abandon.cancel();
            let _ = tokio::time::timeout_at(started + deadline, ended).await;
        }
    }
}

/// `send`'s hold on a conversation while it loads history. Dropping it before `install` (the
/// POST went away mid-load, or shutdown began) puts back what the slot held, so the
/// conversation is never stuck `Busy` and a finished run a panel shows is not lost.
struct Reservation<'a> {
    inner: &'a Inner,
    conversation: ConversationId,
    ticket: u64,
    /// The finished run this turn replaces, restored if the turn never starts.
    previous: Option<Arc<Run>>,
    installed: bool,
}

impl<'a> Reservation<'a> {
    fn take(inner: &'a Inner, conversation: ConversationId) -> Result<Self, Busy> {
        let ticket = inner.tickets.fetch_add(1, Ordering::SeqCst);
        let mut slots = lock(&inner.slots);
        if inner.closed.load(Ordering::SeqCst) {
            return Err(Busy);
        }
        let previous = match slots.get(&conversation) {
            Some(Slot::Starting(_) | Slot::Closing { .. }) => return Err(Busy),
            Some(Slot::Run(run)) if run.is_running() => return Err(Busy),
            // A finished run is replaced: its panels move to the new run in `attach_panels`.
            Some(Slot::Run(run)) => Some(run.clone()),
            None => None,
        };
        slots.insert(conversation, Slot::Starting(ticket));
        Ok(Self {
            inner,
            conversation,
            ticket,
            previous,
            installed: false,
        })
    }

    /// Makes `run` the conversation's current run, unless a `release` or `delete` took the
    /// slot while history was loading, or shutdown began.
    fn install(mut self, run: Arc<Run>) -> Result<(), Busy> {
        let mut slots = lock(&self.inner.slots);
        let ours = matches!(slots.get(&self.conversation), Some(Slot::Starting(ticket)) if *ticket == self.ticket);
        if !ours || self.inner.closed.load(Ordering::SeqCst) {
            return Err(Busy);
        }
        slots.insert(self.conversation, Slot::Run(run));
        self.installed = true;
        Ok(())
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.installed {
            return;
        }
        let mut slots = lock(&self.inner.slots);
        if matches!(slots.get(&self.conversation), Some(Slot::Starting(ticket)) if *ticket == self.ticket)
        {
            match self.previous.take() {
                Some(previous) => slots.insert(self.conversation, Slot::Run(previous)),
                None => slots.remove(&self.conversation),
            };
        }
    }
}

/// A `release` or `delete` holding the slot as `Closing`, with the run it took from it.
struct Claim {
    ticket: u64,
    done: CancellationToken,
    run: Option<Arc<Run>>,
}

impl Inner {
    /// The conversation's current run, if it has one (none while `Starting` or `Closing`).
    fn current(&self, id: ConversationId) -> Option<Arc<Run>> {
        match lock(&self.slots).get(&id) {
            Some(Slot::Run(run)) => Some(run.clone()),
            _ => None,
        }
    }

    /// Takes the slot as `Closing`, waiting out another `release` or `delete` first. `None`
    /// when there was nothing to close and no row-level work is needed beyond the caller's.
    async fn claim(&self, id: ConversationId) -> Option<Claim> {
        loop {
            let waiting = {
                let mut slots = lock(&self.slots);
                match slots.get(&id) {
                    Some(Slot::Closing { done, .. }) => done.clone(),
                    current => {
                        let run = match current {
                            Some(Slot::Run(run)) => Some(run.clone()),
                            // A `send` still loading history: it sees the slot is not its own
                            // and refuses.
                            Some(Slot::Starting(_)) | None => None,
                            Some(Slot::Closing { .. }) => None,
                        };
                        let claim = Claim {
                            ticket: self.tickets.fetch_add(1, Ordering::SeqCst),
                            done: CancellationToken::new(),
                            run,
                        };
                        slots.insert(
                            id,
                            Slot::Closing {
                                ticket: claim.ticket,
                                done: claim.done.clone(),
                                run: claim.run.clone(),
                            },
                        );
                        return Some(claim);
                    }
                }
            };
            waiting.cancelled().await;
        }
    }

    /// Stops the claimed run, waits until its task has written its last row, and drops every
    /// panel of the conversation. Panels go even with no run claimed: a finished run that a
    /// `send` still loading history had taken out of the slot may still be assigned.
    async fn end_run(&self, id: ConversationId, claim: &Claim) {
        if let Some(run) = &claim.run {
            run.cancel.cancel();
            run.ended.cancelled().await;
        }
        let removed = {
            let mut panels = lock(&self.panels);
            let before = panels.len();
            panels.retain(|_, entry| entry.run.conversation != id);
            panels.len() != before
        };
        if removed {
            self.publish_panels();
        }
    }

    /// Gives the `Closing` slot back and wakes whoever waits for it.
    fn give_up(&self, id: ConversationId, claim: &Claim) {
        let mut slots = lock(&self.slots);
        if matches!(slots.get(&id), Some(Slot::Closing { ticket, .. }) if *ticket == claim.ticket) {
            slots.remove(&id);
        }
        claim.done.cancel();
    }

    /// A new run takes over every tab already showing its conversation, plus the tab that
    /// asked. A tab taken from another conversation loses that conversation's `autoOpenId`.
    fn attach_panels(&self, run: &Arc<Run>, panel_tab: Option<i64>) {
        {
            let mut panels = lock(&self.panels);
            let mut tabs: Vec<i64> = panels
                .iter()
                .filter(|(_, entry)| entry.run.conversation == run.conversation)
                .map(|(tab, _)| *tab)
                .collect();
            if let Some(tab) = panel_tab.filter(|tab| *tab >= 0)
                && !tabs.contains(&tab)
            {
                tabs.push(tab);
            }
            if tabs.is_empty() {
                return;
            }
            for tab in tabs {
                let auto_open_id = panels
                    .get(&tab)
                    .filter(|entry| entry.run.conversation == run.conversation)
                    .and_then(|entry| entry.auto_open_id.clone());
                panels.insert(
                    tab,
                    PanelEntry {
                        run: run.clone(),
                        auto_open_id,
                    },
                );
            }
        }
        self.publish_panels();
    }

    /// Tabs the run's task opened since the last look each get the run and a fresh
    /// `autoOpenId`, which tells the extension to open the side panel there once. Only the
    /// conversation's current run, while running, routes panels.
    fn assign_opened_tabs(&self, run: &Arc<Run>) {
        let tabs = self.browser.take_opened_tabs(&run.conversation.to_string());
        if tabs.is_empty() {
            return;
        }
        let current = self
            .current(run.conversation)
            .is_some_and(|current| Arc::ptr_eq(&current, run));
        if !current || !run.is_running() {
            return;
        }
        let mut changed = false;
        {
            let mut panels = lock(&self.panels);
            for tab in tabs.into_iter().filter(|tab| *tab >= 0) {
                if panels
                    .get(&tab)
                    .is_some_and(|entry| Arc::ptr_eq(&entry.run, run))
                {
                    continue;
                }
                panels.insert(
                    tab,
                    PanelEntry {
                        run: run.clone(),
                        auto_open_id: Some(uuid::Uuid::new_v4().to_string()),
                    },
                );
                changed = true;
            }
        }
        if changed {
            self.publish_panels();
        }
    }

    fn has_panels(&self, run: &Arc<Run>) -> bool {
        lock(&self.panels)
            .values()
            .any(|entry| Arc::ptr_eq(&entry.run, run))
    }

    /// Sends the whole table to every `/chat/panels` stream. Built and sent under the panel
    /// lock, so two publishers cannot send their tables in the opposite order to the one they
    /// built them in.
    fn publish_panels(&self) {
        let panels = lock(&self.panels);
        let table = Arc::new(PanelTable {
            assignments: panels
                .iter()
                .map(|(tab, entry)| PanelAssignment {
                    tab_id: *tab,
                    conversation: entry.run.conversation,
                    run: entry.run.id,
                    status: entry.run.status(),
                    auto_open_id: entry.auto_open_id.clone(),
                })
                .collect(),
        });
        *lock(&self.current_table) = table.clone();
        // No stream connected is not an error: the next one starts from `current_table`.
        let _ = self.panel_table.send(table);
    }

    /// Retention: a finished run's frame log goes `KEEP_FINISHED` after it ended. The run
    /// record goes then too, unless a panel still shows it; then it stays until that panel is
    /// forgotten, the conversation is released, or its next turn replaces it.
    fn sweep(&self, now: Instant) {
        let mut slots = lock(&self.slots);
        let panels = lock(&self.panels);
        slots.retain(|_, slot| {
            let Slot::Run(run) = slot else {
                return true;
            };
            let mut log = lock(&run.log);
            let expired = log
                .ended_at
                .is_some_and(|ended| now.saturating_duration_since(ended) >= KEEP_FINISHED);
            if !expired {
                return true;
            }
            log.drop_frames();
            panels.values().any(|entry| Arc::ptr_eq(&entry.run, run))
        });
    }
}

/// The 5 s panel heartbeat and the retention sweep. Holds the registry weakly, so it ends when
/// the last handle is dropped.
async fn housekeeping(inner: Weak<Inner>) {
    let mut tick = tokio::time::interval(HEARTBEAT);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await;
    loop {
        tick.tick().await;
        let Some(inner) = inner.upgrade() else {
            return;
        };
        inner.sweep(Instant::now());
        inner.publish_panels();
    }
}

/// The stored row's `target_type` and `agent_id` for a target.
fn row_target(target: &Target) -> (&'static str, Option<String>) {
    match target {
        Target::Provider(_) => ("browseros", None),
        Target::Coding { kind, agent_id } => (
            match kind {
                AgentKind::Claude => "claude",
                AgentKind::Codex => "codex",
            },
            Some(agent_id.clone()),
        ),
    }
}

/// The transcript the turn answers: `prefix` with the new user message at its end. A
/// `userMessageId` already in `prefix` is a retry or an edit: that message and everything after
/// it are replaced, so the question is stored once.
fn place_user_message(mut prefix: Vec<UiMessage>, turn: &NewTurn) -> Arc<[UiMessage]> {
    if let Some(id) = &turn.user_message_id
        && let Some(position) = prefix
            .iter()
            .position(|message| message.role == Role::User && message.id == *id)
    {
        prefix.truncate(position);
    }
    let id = turn
        .user_message_id
        .clone()
        .unwrap_or_else(|| MessageId(uuid::Uuid::new_v4().to_string()));
    // The parts the extension's own `sendMessage` gives the message on screen: the raw text
    // (context is wrapped around the model's copy only), then each attachment as a file part.
    let mut parts = Vec::new();
    if !turn.text.is_empty() || turn.attachments.is_empty() {
        parts.push(UiPart::Text {
            text: turn.text.clone(),
            state: None,
            extra: serde_json::Map::new(),
        });
    }
    for attachment in &turn.attachments {
        let url = if attachment.data_url.starts_with("data:") {
            attachment.data_url.clone()
        } else {
            format!(
                "data:{};base64,{}",
                attachment.media_type, attachment.data_url
            )
        };
        parts.push(UiPart::Other(json!({
            "type": "file",
            "mediaType": attachment.media_type,
            "url": url,
        })));
    }
    prefix.push(UiMessage {
        id,
        role: Role::User,
        parts,
        metadata: None,
    });
    Arc::from(prefix)
}

fn unreadable() -> ChatError {
    ChatError::new(
        ChatErrorCode::Unknown,
        "Could not load this conversation",
        "The server could not read this conversation's history, so it did not answer. Try again, or start a new conversation.",
    )
}

/// A poisoned lock means a panic elsewhere already failed that operation; the data under it is
/// still consistent (every critical section here is a few assignments).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How long a backend may take to return after Stop before its future is dropped.
const GRACE: Duration = Duration::from_secs(5);
/// The longest one turn may run.
const TURN_BUDGET: Duration = Duration::from_secs(60 * 60);
/// How long a finished run's frame log stays replayable. The run record itself (id, status,
/// final messages) stays while a panel assignment points at it; a finished run with no panel
/// goes after this too.
const KEEP_FINISHED: Duration = Duration::from_secs(600);
/// How often `/chat/panels` re-sends the whole table, so the extension repairs a missed update.
const HEARTBEAT: Duration = Duration::from_secs(5);
/// Tables a slow `/chat/panels` stream may fall behind by before it lags; a lagging stream is
/// sent the current table and goes on.
const PANEL_BACKLOG: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::BrowserConfig;
    use crate::turn::{EventSink, FinishReason, Mode, TurnEnd, TurnEvent};
    use futures_util::StreamExt as _;

    /// Says one word and finishes.
    struct Says;

    #[async_trait::async_trait]
    impl TurnRunner for Says {
        async fn run_turn(
            &self,
            _target: &Target,
            _input: &TurnInput,
            sink: &mut dyn EventSink,
            _cancel: &CancellationToken,
        ) -> TurnEnd {
            sink.emit(TurnEvent::StepStart).await;
            sink.emit(TurnEvent::Text("Hi".into())).await;
            sink.emit(TurnEvent::StepEnd).await;
            TurnEnd::Done(FinishReason::Stop)
        }

        async fn end_session(&self, _conversation: ConversationId) {}
    }

    fn turn(conversation: ConversationId, panel_tab: Option<i64>) -> NewTurn {
        NewTurn {
            conversation,
            user_message_id: None,
            text: "Hello".into(),
            attachments: Vec::new(),
            target: Target::Provider(None),
            context: TurnContext {
                mode: Mode::Agent,
                window_id: None,
                active_tab: None,
                selected_tabs: Vec::new(),
                selected_text: None,
                user_system_prompt: None,
                working_dir: None,
                supports_reasoning: true,
            },
            history: History::Stored,
            panel_tab,
        }
    }

    /// A registry over a fresh database whose conversation has one finished run.
    async fn finished(
        panel_tab: Option<i64>,
    ) -> Result<(Conversations, ConversationId, tempfile::TempDir), Box<dyn std::error::Error>>
    {
        let state = tempfile::tempdir()?;
        let store = Store::open(state.path()).await?;
        let browser = Browser::start(
            BrowserConfig::Off { reason: "test" },
            0,
            CancellationToken::new(),
        )
        .await;
        let conversations = Conversations::new(store, Arc::new(Says), browser);
        let id = uuid::Uuid::new_v4();
        let Ok(subscription) = conversations.send(turn(id, panel_tab)).await else {
            return Err("busy".into());
        };
        // The stream ends only once the run's terminal status is published.
        subscription.collect::<Vec<_>>().await;
        Ok((conversations, id, state))
    }

    fn later(by: Duration) -> Instant {
        Instant::now() + by
    }

    #[tokio::test]
    async fn a_finished_run_without_a_panel_goes_after_keep_finished()
    -> Result<(), Box<dyn std::error::Error>> {
        let (conversations, id, _state) = finished(None).await?;
        conversations
            .inner
            .sweep(later(KEEP_FINISHED - Duration::from_secs(1)));
        assert!(matches!(conversations.attach(id, None), Attach::Run(_)));

        conversations
            .inner
            .sweep(later(KEEP_FINISHED + Duration::from_secs(1)));
        assert!(conversations.state(id).is_none());
        assert!(matches!(conversations.attach(id, None), Attach::NoRun));
        Ok(())
    }

    #[tokio::test]
    async fn a_run_a_panel_shows_keeps_its_transcript_until_the_panel_is_forgotten()
    -> Result<(), Box<dyn std::error::Error>> {
        let (conversations, id, _state) = finished(Some(3)).await?;
        conversations
            .inner
            .sweep(later(KEEP_FINISHED + Duration::from_secs(1)));

        let state = conversations
            .state(id)
            .ok_or("the record went with the log")?;
        assert_eq!(state.status, RunStatus::Completed);
        assert_eq!(
            state.messages.len(),
            2,
            "the question and the sealed answer"
        );
        assert!(
            matches!(conversations.attach(id, Some(state.run)), Attach::NoRun),
            "the log is gone, so the panel reads /state, not the stream"
        );

        conversations.forget_panel(3);
        assert!(conversations.state(id).is_none());
        assert_eq!(conversations.current_panels().assignments, []);
        Ok(())
    }

    /// A `send` that went away while loading history puts the finished run back, so `/state`
    /// keeps answering for the panel that shows it.
    #[tokio::test]
    async fn a_send_that_never_starts_restores_the_finished_run()
    -> Result<(), Box<dyn std::error::Error>> {
        let (conversations, id, _state) = finished(Some(3)).await?;
        let before = conversations.state(id).map(|state| state.run);
        let reservation = Reservation::take(&conversations.inner, id).map_err(|_| "busy")?;
        assert!(
            conversations.state(id).is_none(),
            "`Starting` while history loads"
        );
        drop(reservation);
        assert_eq!(conversations.state(id).map(|state| state.run), before);
        Ok(())
    }

    /// Shutdown collects the runs it stops under the slot lock; a `send` still loading history
    /// then cannot start a run it would never stop.
    #[tokio::test]
    async fn a_send_loading_history_does_not_start_after_shutdown()
    -> Result<(), Box<dyn std::error::Error>> {
        let (conversations, id, _state) = finished(None).await?;
        let reservation = Reservation::take(&conversations.inner, id).map_err(|_| "busy")?;
        conversations.shutdown(Duration::from_millis(100)).await;
        let run = Arc::new(Run::new(id, Arc::from(Vec::new())));
        assert_eq!(reservation.install(run), Err(Busy));
        assert!(!conversations.busy());
        Ok(())
    }

    /// A delete while a `send` loads history: the finished run that `send` took out of the slot
    /// still had a panel, and the delete must drop it with the conversation.
    #[tokio::test]
    async fn a_delete_while_a_send_loads_drops_the_conversations_panels()
    -> Result<(), Box<dyn std::error::Error>> {
        let (conversations, id, _state) = finished(Some(3)).await?;
        let reservation = Reservation::take(&conversations.inner, id).map_err(|_| "busy")?;
        assert!(conversations.delete(id).await?, "the row existed");
        assert_eq!(conversations.current_panels().assignments, []);
        let run = Arc::new(Run::new(id, Arc::from(Vec::new())));
        assert_eq!(reservation.install(run), Err(Busy), "the send is refused");
        assert!(conversations.state(id).is_none());
        Ok(())
    }
}
