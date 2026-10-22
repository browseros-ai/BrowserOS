//! Persists browser selection for a logical task independently of transport lifetimes.
//! Successor handles alias one canonical record, so an old handle observes later rebinds.
//! Only intent and correlation are stored here; native profiles, views and ownership stay elsewhere.

use crate::{
    error::{AppError, AppResult},
    ids::SessionId,
    storage::JsonStore,
};
use browseros_core::target::{TargetBinding, TargetIntent};
use browseros_mcp::TargetStore;
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;

const FILE: &str = "target-bindings.json";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Records {
    tasks: BTreeMap<SessionId, Record>,
    aliases: BTreeMap<SessionId, SessionId>,
    /// A group is an existing task-correlation handle on legacy transports.
    groups: BTreeMap<String, SessionId>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    current_session: SessionId,
    binding: Option<TargetBinding>,
    revision: u64,
    stopped: bool,
    updated_at_ms: i64,
}

impl Record {
    fn new(session: &SessionId) -> Self {
        Self {
            current_session: session.clone(),
            binding: None,
            revision: 0,
            stopped: false,
            updated_at_ms: crate::clock::now_epoch_ms(),
        }
    }

    fn require_live(&self) -> AppResult<()> {
        if self.stopped {
            return Err(AppError::Internal(
                "BrowserOS neo session was stopped and will not resume".into(),
            ));
        }
        Ok(())
    }

    fn next_revision(&self) -> AppResult<u64> {
        self.revision.checked_add(1).ok_or_else(|| {
            AppError::Internal("target_reselect_required: target revision exhausted".into())
        })
    }
}

impl Records {
    fn canonical<'a>(&'a self, session: &'a SessionId) -> &'a SessionId {
        self.aliases.get(session).unwrap_or(session)
    }

    fn record(&self, session: &SessionId) -> Option<&Record> {
        self.tasks.get(self.canonical(session))
    }

    fn ensure_record(&mut self, session: &SessionId) -> &mut Record {
        let canonical = self.canonical(session).clone();
        self.aliases
            .entry(session.clone())
            .or_insert(canonical.clone());
        self.tasks
            .entry(canonical)
            .or_insert_with(|| Record::new(session))
    }
}

/// Atomic host adapter for saved task intent and canonical successor correlation.
/// Records are deliberately small and retained indefinitely: age must never turn a
/// previously targeted or stopped handle into an unknown task that has lost its saved
/// selection (falling back to the default profile) or its stop.
pub struct TargetBindings {
    records: Mutex<Records>,
    store: JsonStore,
}

impl TargetBindings {
    pub async fn load(store: JsonStore) -> AppResult<Arc<Self>> {
        let records = match store.read_json(FILE).await {
            Ok(records) => records,
            Err(AppError::StorageNotFound(_)) => Records::default(),
            Err(error) => return Err(error),
        };
        Ok(Arc::new(Self {
            records: Mutex::new(records),
            store,
        }))
    }

    pub async fn get(&self, session: &SessionId) -> Option<TargetBinding> {
        self.records
            .lock()
            .await
            .record(session)
            .and_then(|record| record.binding.clone())
    }

    pub(crate) async fn binding_for_execution(
        &self,
        session: &SessionId,
    ) -> AppResult<Option<TargetBinding>> {
        let records = self.records.lock().await;
        let Some(record) = records.record(session) else {
            return Ok(None);
        };
        record.require_live()?;
        Ok(record.binding.clone())
    }

    pub(crate) async fn is_stopped(&self, session: &SessionId) -> bool {
        self.records
            .lock()
            .await
            .record(session)
            .is_some_and(|record| record.stopped)
    }

    /// The latest successor is a live-map lookup hint, never authority to forget intent.
    pub(crate) async fn current_session(&self, session: &SessionId) -> Option<SessionId> {
        self.records
            .lock()
            .await
            .record(session)
            .map(|record| record.current_session.clone())
    }

    pub async fn set(&self, session: &SessionId, intent: TargetIntent) -> AppResult<TargetBinding> {
        let mut records = self.records.lock().await;
        let mut next = records.clone();
        let record = next.ensure_record(session);
        record.require_live()?;
        let binding = TargetBinding {
            intent,
            revision: record.next_revision()?,
        };
        record.revision = binding.revision;
        record.binding = Some(binding.clone());
        record.updated_at_ms = crate::clock::now_epoch_ms();
        // Serialize writers through rename and publish memory only after persistence.
        // A failed write must not acknowledge a selection or invalidate the last good one.
        self.store.write_json(FILE, &next).await?;
        *records = next;
        Ok(binding)
    }

    pub async fn clear(&self, session: &SessionId) -> AppResult<()> {
        let mut records = self.records.lock().await;
        let mut next = records.clone();
        let record = next.ensure_record(session);
        record.require_live()?;
        record.revision = record.next_revision()?;
        record.binding = None;
        record.updated_at_ms = crate::clock::now_epoch_ms();
        self.store.write_json(FILE, &next).await?;
        *records = next;
        Ok(())
    }

    /// CAS successor adoption. Aliases point directly at the original task, never at a
    /// copied binding: copying would let an old alias resurrect a superseded selection.
    pub async fn inherit(
        &self,
        from: &SessionId,
        to: &SessionId,
    ) -> AppResult<Option<TargetBinding>> {
        let mut records = self.records.lock().await;
        if let Some(record) = records.record(from) {
            record.require_live()?;
        }
        if let Some(record) = records.record(to) {
            record.require_live()?;
            return Ok(record.binding.clone());
        }
        let mut next = records.clone();
        next.ensure_record(from);
        let canonical = next.canonical(from).clone();
        next.aliases.insert(to.clone(), canonical.clone());
        let record = next
            .tasks
            .get_mut(&canonical)
            .ok_or_else(|| AppError::Internal("target task missing".into()))?;
        record.current_session = to.clone();
        record.updated_at_ms = crate::clock::now_epoch_ms();
        let binding = record.binding.clone();
        self.store.write_json(FILE, &next).await?;
        *records = next;
        Ok(binding)
    }

    /// Stop releases the native selection but keeps a durable marker for every alias.
    /// No profile unloading, window closure or ownership mutation occurs in this module.
    pub async fn forget(&self, session: &SessionId) -> AppResult<()> {
        let mut records = self.records.lock().await;
        let mut next = records.clone();
        let record = next.ensure_record(session);
        record.binding = None;
        record.stopped = true;
        record.updated_at_ms = crate::clock::now_epoch_ms();
        self.store.write_json(FILE, &next).await?;
        *records = next;
        Ok(())
    }

    pub async fn sweep(&self, _now_ms: i64) -> AppResult<usize> {
        // Views are per-call and never retained here. Keeping intent/aliases is what
        // makes a 30-day reconnect fail closed after native restart instead of unbinding.
        Ok(0)
    }

    pub(crate) async fn remember_group(&self, session: &SessionId, group: &str) -> AppResult<()> {
        let mut records = self.records.lock().await;
        let canonical = records.canonical(session).clone();
        if records.groups.get(group) == Some(&canonical) {
            return Ok(());
        }
        let mut next = records.clone();
        // A group already being created may finish during Stop teardown. Keep its
        // correlation pointed at the stopped task so reconnecting cannot revive it.
        next.ensure_record(session);
        next.groups.insert(group.to_string(), canonical);
        self.store.write_json(FILE, &next).await?;
        *records = next;
        Ok(())
    }

    pub(crate) async fn owns_group(&self, session: &SessionId, group: &str) -> bool {
        let records = self.records.lock().await;
        records
            .groups
            .get(group)
            .is_some_and(|owner| owner == records.canonical(session))
    }

    /// Resolve the legacy group correlation before capture or tab creation. A caller
    /// that already chose a different task must explicitly reselect rather than borrow
    /// an old group's targeting as an accidental side effect of browser mutation.
    pub(crate) async fn correlate_group(&self, session: &SessionId, group: &str) -> AppResult<()> {
        let mut records = self.records.lock().await;
        let Some(canonical) = records.groups.get(group).cloned() else {
            return Ok(());
        };
        let source = records.tasks.get(&canonical).ok_or_else(|| {
            AppError::Internal("target_reselect_required: group task identity is missing".into())
        })?;
        if records.record(session).is_some() {
            if records.canonical(session) == &canonical {
                source.require_live()?;
                return Ok(());
            }
            if let Some(current) = records.record(session)
                && current.revision > 0
            {
                // Explicit set/clear selected this task's routing already. It may
                // borrow a group without adopting that group's task or its ownership.
                current.require_live()?;
                return Ok(());
            }
            return Err(AppError::Internal("target_reselect_required: group belongs to another task; select its profile explicitly".into()));
        }
        source.require_live()?;
        let mut next = records.clone();
        next.aliases.insert(session.clone(), canonical.clone());
        if let Some(record) = next.tasks.get_mut(&canonical) {
            record.current_session = session.clone();
            record.updated_at_ms = crate::clock::now_epoch_ms();
        }
        self.store.write_json(FILE, &next).await?;
        *records = next;
        Ok(())
    }
}

/// MCP management adapter pinned to the admitted logical task, not its transport.
pub struct ClawTargetStore {
    pub bindings: Arc<TargetBindings>,
    pub session: SessionId,
}

impl TargetStore for ClawTargetStore {
    fn get(&self) -> BoxFuture<'_, Result<Option<TargetBinding>, String>> {
        Box::pin(async move {
            self.bindings
                .binding_for_execution(&self.session)
                .await
                .map_err(|error| error.to_string())
        })
    }
    fn set(&self, intent: TargetIntent) -> BoxFuture<'_, Result<TargetBinding, String>> {
        Box::pin(async move {
            self.bindings
                .set(&self.session, intent)
                .await
                .map_err(|error| error.to_string())
        })
    }
    fn clear(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.bindings
                .clear(&self.session)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browseros_core::BrowserProfileId;

    fn intent(id: &str) -> TargetIntent {
        TargetIntent {
            browser_profile_id: BrowserProfileId::from(id.to_string()),
            directory: format!("Synthetic-{id}"),
            name: id.into(),
            window: None,
        }
    }

    #[tokio::test]
    async fn retained_group_correlation_recovers_intent_before_mutation_or_requires_selection()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = JsonStore::new(dir.path().to_path_buf());
        let bindings = TargetBindings::load(store.clone()).await?;
        let old = SessionId::new("old");
        let first = bindings.set(&old, intent("eng")).await?;
        bindings.remember_group(&old, "G1").await?;
        bindings.sweep(i64::MAX).await?;
        drop(bindings);
        let restarted = TargetBindings::load(store).await?;
        let reconnect = SessionId::new("reconnect");
        restarted.correlate_group(&reconnect, "G1").await?;
        assert_eq!(restarted.get(&reconnect).await, Some(first));
        let unrelated = SessionId::new("unrelated");
        assert_eq!(restarted.binding_for_execution(&unrelated).await?, None);
        // A previously adopted different unbound task is ambiguous, not a licence
        // to silently lose the targeted group's intent. Explicit selection recovers.
        let ambiguous = SessionId::new("ambiguous");
        restarted.inherit(&unrelated, &ambiguous).await?;
        let error = restarted
            .correlate_group(&ambiguous, "G1")
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("ambiguous task adopted a group"))?;
        assert!(error.to_string().contains("target_reselect_required"));
        restarted.set(&ambiguous, intent("eng")).await?;
        restarted.correlate_group(&ambiguous, "G1").await?;
        assert!(
            !restarted.owns_group(&ambiguous, "G1").await,
            "explicit selection permits borrowing without ownership transfer"
        );
        Ok(())
    }

    #[tokio::test]
    async fn aliases_observe_rebind_clear_and_restart() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = JsonStore::new(dir.path().to_path_buf());
        let bindings = TargetBindings::load(store.clone()).await?;
        let old = SessionId::new("old");
        let successor = SessionId::new("successor");
        let first = bindings.set(&old, intent("eng")).await?;
        assert_eq!(bindings.inherit(&old, &successor).await?, Some(first));
        let next = bindings.set(&successor, intent("ops")).await?;
        assert_eq!(next.revision, 2);
        assert_eq!(bindings.get(&old).await, Some(next.clone()));
        bindings.clear(&old).await?;
        assert_eq!(bindings.get(&successor).await, None);
        let fourth = bindings.set(&successor, intent("eng")).await?;
        assert_eq!(fourth.revision, 4);
        drop(bindings);
        let restarted = TargetBindings::load(store).await?;
        assert_eq!(restarted.get(&old).await, Some(fourth));
        assert_eq!(restarted.current_session(&old).await, Some(successor));
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_successors_share_intent_and_do_not_overwrite_existing_tasks()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let bindings = TargetBindings::load(JsonStore::new(dir.path().to_path_buf())).await?;
        let from = SessionId::new("from");
        let a = SessionId::new("a");
        let b = SessionId::new("b");
        bindings.set(&from, intent("eng")).await?;
        let (left, right) = tokio::join!(bindings.inherit(&from, &a), bindings.inherit(&from, &b));
        assert_eq!(left?, right?);
        let changed = bindings.set(&b, intent("ops")).await?;
        assert_eq!(bindings.get(&a).await, Some(changed.clone()));
        let other = SessionId::new("other");
        let other_binding = bindings.set(&other, intent("other")).await?;
        assert_eq!(bindings.inherit(&from, &other).await?, Some(other_binding));
        assert_eq!(bindings.get(&from).await, Some(changed));
        Ok(())
    }

    #[tokio::test]
    async fn old_target_and_stopped_aliases_survive_retention_and_restart() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = JsonStore::new(dir.path().to_path_buf());
        let bindings = TargetBindings::load(store.clone()).await?;
        let old = SessionId::new("old");
        let new = SessionId::new("new");
        let selected = bindings
            .set(&old, intent("process-lifetime-handle"))
            .await?;
        assert_eq!(bindings.inherit(&old, &new).await?, Some(selected));
        let latest = bindings.set(&new, intent("ops")).await?;
        bindings.remember_group(&old, "G1").await?;
        // Advance the retention clock beyond any 30-day horizon, first while the
        // task is bound and again after Stop. Neither state may become unknown.
        assert_eq!(bindings.sweep(i64::MAX).await?, 0);
        drop(bindings);
        let retained = TargetBindings::load(store.clone()).await?;
        assert_eq!(retained.get(&old).await, Some(latest.clone()));
        assert_eq!(retained.get(&new).await, Some(latest.clone()));
        assert_eq!(retained.current_session(&old).await, Some(new.clone()));
        let reconnect = SessionId::new("reconnect");
        retained.correlate_group(&reconnect, "G1").await?;
        assert_eq!(retained.get(&reconnect).await, Some(latest));
        retained.forget(&new).await?;
        assert_eq!(retained.sweep(i64::MAX).await?, 0);
        drop(retained);
        let restarted = TargetBindings::load(store).await?;
        assert!(restarted.is_stopped(&old).await);
        assert!(restarted.is_stopped(&new).await);
        assert!(restarted.is_stopped(&reconnect).await);
        assert!(
            restarted
                .inherit(&old, &SessionId::new("third"))
                .await
                .is_err()
        );
        assert!(
            restarted
                .correlate_group(&SessionId::new("fourth"), "G1")
                .await
                .is_err()
        );
        assert!(restarted.set(&new, intent("eng")).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn failed_persistence_leaves_last_good_binding_and_revision() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = JsonStore::new(dir.path().to_path_buf());
        let bindings = TargetBindings::load(store.clone()).await?;
        let session = SessionId::new("s1");
        let first = bindings.set(&session, intent("eng")).await?;
        tokio::fs::remove_file(store.resolve(FILE)?).await?;
        tokio::fs::create_dir(store.resolve(FILE)?).await?;
        assert!(bindings.set(&session, intent("ops")).await.is_err());
        assert!(bindings.clear(&session).await.is_err());
        assert!(bindings.forget(&session).await.is_err());
        assert!(!bindings.is_stopped(&session).await);
        let successor = SessionId::new("successor");
        assert!(bindings.inherit(&session, &successor).await.is_err());
        assert_eq!(
            bindings.current_session(&session).await,
            Some(session.clone())
        );
        assert_eq!(bindings.get(&session).await, Some(first));
        tokio::fs::remove_dir(store.resolve(FILE)?).await?;
        assert_eq!(bindings.set(&session, intent("eng")).await?.revision, 2);
        Ok(())
    }
}
