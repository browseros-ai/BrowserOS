//! One task owns the write connection and the OS file lock. It commits FIFO batches, then
//! publishes heads, then releases permits. Failed batches retain their payloads and permits;
//! database outages can exhaust the queue but can never make a producer wait.
//!
//! Actor shape follows cs/services/audit.rs:149-221; its blocking submit is deliberately absent.

use std::collections::{BTreeSet, HashMap};
use std::mem;
use std::slice;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use neo_runtime::{RunEvent, RunId};
use sea_orm::{DatabaseConnection, DbErr, TransactionTrait};
use tokio::sync::mpsc;
use tokio::time::{self, Instant};
use tracing::warn;

use crate::db;
use crate::ingest::{self, Command, Entry, State};
use crate::live::Heads;
use crate::{Retention, StoreError, apply, retention};

const BATCH: usize = 512;
const SWEEP: Duration = Duration::from_secs(1);
const HOURLY: Duration = Duration::from_hours(1);

/// Writer-owned state is never consulted from emit. The per-run map records isolated bad
/// events only; regular capture loss belongs to each `RunCapture`'s atomics.
pub(crate) struct Writer {
    pub(crate) db: DatabaseConnection,
    pub(crate) rx: mpsc::UnboundedReceiver<Entry>,
    pub(crate) state: Arc<State>,
    pub(crate) heads: Arc<Heads>,
    pub(crate) retention: Retention,
}

impl Writer {
    pub(crate) async fn run(mut self) {
        let mut sweep = time::interval(SWEEP);
        let mut next_retention = Instant::now() + HOURLY;
        let mut next_settle = Instant::now() + SWEEP;
        let mut cleaning = false;
        let mut losses = HashMap::new();
        loop {
            let next = tokio::select! {
                biased;
                entry = self.rx.recv() => entry,
                _ = sweep.tick() => None,
            };
            if let Some(entry) = next {
                let mut batch = vec![entry];
                while batch.len() < BATCH && !barrier(&batch[batch.len() - 1].command) {
                    let Ok(entry) = self.rx.try_recv() else {
                        break;
                    };
                    batch.push(entry);
                }
                if !self.persist(&mut batch, &mut losses).await {
                    break;
                }
            } else if self.rx.is_closed() {
                break;
            }
            // Check deadlines between batches as well as on an idle tick. A permanently busy
            // FIFO must not starve settlement or hourly retention.
            if Instant::now() >= next_settle {
                match apply::sweep(&self.db, now()).await {
                    Ok(keys) => self.publish(keys.into_iter().collect()).await,
                    Err(error) => self.failed(&error),
                }
                next_settle = Instant::now() + SWEEP;
            }
            if Instant::now() >= next_retention {
                cleaning = true;
                next_retention = Instant::now() + HOURLY;
            }
            if cleaning {
                match retention::step(&self.db, self.retention, now()).await {
                    Ok(more) => {
                        cleaning = more;
                        if let Err(error) = self.heads.initialize(&self.db, now()).await {
                            self.failed(&error);
                        }
                    }
                    Err(error) => {
                        self.failed(&error);
                        cleaning = false;
                    }
                }
            }
        }
        if let Err(error) = self.db.close().await {
            warn!(%error, "closing trace writer");
        }
    }

    async fn persist(&mut self, batch: &mut [Entry], losses: &mut HashMap<RunId, u64>) -> bool {
        let mut failures = 0_u32;
        let mut delay = Duration::from_millis(50);
        loop {
            match self.transaction(batch, losses).await {
                Ok(dirty) => {
                    self.publish(dirty).await;
                    break;
                }
                Err(error) => {
                    self.failed(&error);
                    failures = failures.saturating_add(1);
                    if failures >= 5 {
                        self.isolate(batch, losses).await;
                        break;
                    }
                    time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(5));
                }
            }
        }
        self.state.failing_since.store(0, Ordering::Release);
        for entry in batch {
            match mem::replace(&mut entry.command, Command::Stop) {
                Command::Flush(done) => {
                    let _ = done.send(());
                }
                Command::Forget(key, done) => {
                    let result = self.forget(key.as_str()).await;
                    let _ = done.send(result);
                }
                Command::Stop => return false,
                #[cfg(test)]
                Command::Pause(entered, resume) => {
                    let _ = entered.send(());
                    let _ = resume.await;
                }
                Command::Event {
                    event: RunEvent::RunSettled { run },
                    ..
                } => {
                    losses.remove(&run);
                }
                _ => {}
            }
        }
        true
    }

    async fn transaction(
        &self,
        batch: &[Entry],
        losses: &HashMap<RunId, u64>,
    ) -> Result<BTreeSet<String>, DbErr> {
        #[cfg(test)]
        {
            use sea_orm::ConnectionTrait;
            self.db
                .execute_unprepared(if self.state.test_readonly.load(Ordering::Acquire) {
                    "PRAGMA query_only=ON"
                } else {
                    "PRAGMA query_only=OFF"
                })
                .await?;
        }
        let txn = self.db.begin().await?;
        let mut dirty = BTreeSet::new();
        for entry in batch {
            match &entry.command {
                Command::Withhold(key) => {
                    apply::withhold(&txn, key.as_str()).await?;
                    dirty.insert(key.as_str().to_string());
                }
                Command::Event {
                    event: RunEvent::RunSettled { run },
                    stub,
                    loss,
                } => {
                    let loss = (
                        loss.0,
                        loss.1.saturating_add(losses.get(run).copied().unwrap_or(0)),
                    );
                    if let Some(key) = apply::command(
                        &txn,
                        &Command::Event {
                            event: RunEvent::RunSettled { run: *run },
                            stub: *stub,
                            loss,
                        },
                        now(),
                    )
                    .await?
                    {
                        dirty.insert(key);
                    }
                }
                command => {
                    if let Some(key) = apply::command(&txn, command, now()).await? {
                        dirty.insert(key);
                    }
                }
            }
        }
        txn.commit().await?;
        Ok(dirty)
    }

    async fn isolate(&self, batch: &[Entry], losses: &mut HashMap<RunId, u64>) {
        for entry in batch {
            let mut delay = Duration::from_millis(50);
            loop {
                match self.transaction(slice::from_ref(entry), losses).await {
                    Ok(dirty) => {
                        self.publish(dirty).await;
                        break;
                    }
                    Err(error) if transient(&error) => {
                        self.failed(&error);
                        time::sleep(delay).await;
                        delay = (delay * 2).min(Duration::from_secs(5));
                    }
                    Err(error) => {
                        self.failed(&error);
                        self.state.writes_dropped.fetch_add(1, Ordering::Relaxed);
                        self.state.dropped.fetch_add(1, Ordering::Relaxed);
                        if let Some(id) = ingest::run_id(&entry.command) {
                            *losses.entry(id).or_default() += 1;
                        }
                        // No error text or payload: SQL errors can embed rejected values.
                        warn!(run = ?ingest::run_id(&entry.command), kind = kind(&entry.command), number = number(&entry.command), "dropping an isolated invalid trace event");
                        break;
                    }
                }
            }
        }
    }

    async fn forget(&self, key: &str) -> Result<u64, StoreError> {
        let mut count = 0;
        loop {
            let txn = self.db.begin().await.map_err(StoreError::db)?;
            let (removed, more) = apply::forget_chunk(&txn, key)
                .await
                .map_err(StoreError::db)?;
            txn.commit().await.map_err(StoreError::db)?;
            count += removed;
            if !more {
                break;
            }
        }
        self.publish(BTreeSet::from([key.to_string()])).await;
        Ok(count)
    }

    async fn publish(&self, dirty: BTreeSet<String>) {
        for key in dirty {
            let mut delay = Duration::from_millis(50);
            loop {
                match self.heads.publish(&self.db, &key, now()).await {
                    Ok(()) => break,
                    Err(error) => {
                        self.failed(&error);
                        time::sleep(delay).await;
                        delay = (delay * 2).min(Duration::from_secs(5));
                    }
                }
            }
        }
    }

    fn failed(&self, _error: &DbErr) {
        let _ = self.state.failing_since.compare_exchange(
            0,
            now().max(1),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

fn barrier(command: &Command) -> bool {
    match command {
        Command::Flush(_) | Command::Forget(..) | Command::Stop => true,
        #[cfg(test)]
        Command::Pause(..) => true,
        _ => false,
    }
}

/// Storage outages are retried forever, including after batch isolation. Dropping every row
/// during `SQLITE_READONLY/BUSY/FULL` would turn a temporary outage into permanent trace loss.
fn transient(error: &DbErr) -> bool {
    db::sqlite_code(error).is_some_and(|code| matches!(code, 5 | 6 | 8 | 10 | 13 | 14))
        || matches!(error, DbErr::Conn(_) | DbErr::ConnectionAcquire(_))
}

fn kind(command: &Command) -> &'static str {
    match command {
        Command::Started(_) => "RunStarted",
        Command::Event { event, .. } => match event {
            RunEvent::BeforeCall { .. } => "BeforeCall",
            RunEvent::InputSent { .. } => "InputSent",
            RunEvent::AfterCall { .. } => "AfterCall",
            RunEvent::DeliverySettled { .. } => "DeliverySettled",
            RunEvent::PageOpened { .. } => "PageOpened",
            RunEvent::PageClosed { .. } => "PageClosed",
            RunEvent::Getter { .. } => "Getter",
            RunEvent::RunEnded { .. } => "RunEnded",
            RunEvent::RunSettled { .. } => "RunSettled",
        },
        Command::Artifact(..) => "Artifact",
        Command::Withhold(_) => "Withhold",
        Command::Forget(..) => "Forget",
        Command::Flush(_) => "Flush",
        Command::Stop => "Stop",
        #[cfg(test)]
        Command::Pause(..) => "Pause",
    }
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

fn number(command: &Command) -> Option<u64> {
    match command {
        Command::Event {
            event:
                RunEvent::BeforeCall { call, .. }
                | RunEvent::AfterCall { call, .. }
                | RunEvent::InputSent { call, .. }
                | RunEvent::DeliverySettled { call, .. },
            ..
        } => Some(*call),
        Command::Event {
            event: RunEvent::Getter { no, .. },
            ..
        } => Some(*no),
        Command::Artifact(call, _) => Some(call.no),
        _ => None,
    }
}
