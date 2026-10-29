use crate::{
    db::{
        AuditLog, SkillsRepository,
        audit_log::{TaskStatus, TaskSummary},
        entities::{skill_run_marks, skill_runs},
        skills::RunMode,
    },
    error::{AppError, AppResult},
};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tracing::warn;
use ulid::Ulid;

/// Records a "skill run" when a session that was tagged as running a skill
/// completes, by projecting the session's audit stats into a `skill_runs` row.
/// The mark ties a session to a skill; the projection runs from the session
/// completion hook, with a startup reconcile sweep as the durability backstop.
pub struct SkillRunService {
    repo: SkillsRepository,
    audit_log: Arc<AuditLog>,
    /// Serializes projection so concurrent finalizers of the same skill cannot
    /// read the same max run number and assign duplicate run numbers.
    finalize_lock: Mutex<()>,
}

impl SkillRunService {
    #[must_use]
    pub fn new(repo: SkillsRepository, audit_log: Arc<AuditLog>) -> Self {
        Self {
            repo,
            audit_log,
            finalize_lock: Mutex::new(()),
        }
    }

    /// Tie the current session to a skill; completion turns this into a run row.
    /// Rejects a name that does not resolve to a saved skill so a typo or stale
    /// name never records a run for a skill that does not exist.
    pub async fn mark(&self, session_id: &str, skill_name: &str) -> AppResult<()> {
        if self.repo.get(skill_name).await?.is_none() {
            return Err(AppError::not_found("skill not found"));
        }
        // A run_skill outcome already marked this session for the same skill; the
        // agent finishing by hand must not erase that it was handed back.
        if let Some(existing) = self.repo.get_mark(session_id).await?
            && existing.skill_name == skill_name
            && existing.mode != "agent"
        {
            return Ok(());
        }
        self.repo
            .upsert_mark(session_id, skill_name, now_ms())
            .await
    }

    /// Tie the session to a skill with the way `run_skill` carried it out.
    pub async fn mark_as(
        &self,
        session_id: &str,
        skill_name: &str,
        mode: &RunMode,
    ) -> AppResult<()> {
        self.repo
            .upsert_mark_as(session_id, skill_name, now_ms(), mode)
            .await
    }

    /// Fire-and-forget projection for a completed session, safe to call from the
    /// session completion hook. Any miss is caught by `reconcile` at startup.
    pub fn spawn_finalize(self: &Arc<Self>, session_id: String) {
        let service = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(error) = service.finalize(&session_id).await {
                warn!(error = %error, "skill run projection failed");
            }
        });
    }

    /// Project one completed, marked session into a run row. Idempotent: a
    /// session records at most one run. Returns whether a new run was recorded.
    pub async fn finalize(&self, session_id: &str) -> AppResult<bool> {
        let _guard = self.finalize_lock.lock().await;
        let Some(mark) = self.repo.get_mark(session_id).await? else {
            return Ok(false);
        };
        // The skill may have been deleted between the mark and completion; drop
        // the mark rather than record a run for a skill that no longer exists.
        if self.repo.get(&mark.skill_name).await?.is_none() {
            self.repo.delete_mark(session_id).await?;
            return Ok(false);
        }
        let Some(task) = self.audit_log.get_task_summary(session_id).await? else {
            return Ok(false);
        };
        if matches!(task.status, TaskStatus::Live) {
            // Not terminal yet; leave the mark for a later completion.
            return Ok(false);
        }
        let recorded = self.record_run(&mark, session_id, &task).await?;
        // The run is durable now (this call inserted it or a prior one did), so
        // the mark has served its purpose.
        self.repo.delete_mark(session_id).await?;
        Ok(recorded)
    }

    /// Sweep every outstanding mark on startup, projecting the terminal ones.
    pub async fn reconcile(&self) -> AppResult<usize> {
        let mut recorded = 0;
        for mark in self.repo.all_marks().await? {
            if self.finalize(&mark.session_id).await? {
                recorded += 1;
            }
        }
        Ok(recorded)
    }

    async fn record_run(
        &self,
        mark: &skill_run_marks::Model,
        session_id: &str,
        task: &TaskSummary,
    ) -> AppResult<bool> {
        let skill_name = mark.skill_name.as_str();
        let run_number = self.repo.max_run_number(skill_name).await?.unwrap_or(0) + 1;
        let tokens = task
            .tokens_measured
            .then(|| task.tool_input_token_estimate + task.tool_output_token_estimate);
        let clean = task.error_count == 0
            && !matches!(task.status, TaskStatus::Failed | TaskStatus::Cancelled);
        let errored_tool = if clean {
            None
        } else {
            self.repo.first_errored_tool(session_id).await?
        };
        let row = skill_runs::Model {
            id: Ulid::new().to_string(),
            skill_name: skill_name.to_owned(),
            session_id: session_id.to_owned(),
            run_number,
            agent_id: task.agent_id.clone(),
            tokens,
            duration_ms: Some(task.duration_ms),
            tool_count: Some(task.dispatch_count),
            clean,
            errored_tool,
            created_at: now_ms(),
            mode: mark.mode.clone(),
            handover_step: mark.handover_step,
            handover_cause: mark.handover_cause.clone(),
        };
        self.repo.insert_run_if_absent(row).await
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}
