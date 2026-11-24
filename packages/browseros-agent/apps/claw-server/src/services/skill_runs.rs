use crate::{
    analytics::{AnalyticsSink, NoopAnalyticsSink, events},
    db::{
        AuditLog, SkillsRepository,
        audit_log::{TaskStatus, TaskSummary},
        entities::{skill_run_marks, skill_runs},
        skills::RunMode,
    },
    error::{AppError, AppResult},
};
use serde_json::json;
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
    analytics: Arc<dyn AnalyticsSink>,
}

impl SkillRunService {
    #[must_use]
    pub fn new(repo: SkillsRepository, audit_log: Arc<AuditLog>) -> Self {
        Self::new_with_analytics(repo, audit_log, Arc::new(NoopAnalyticsSink))
    }

    #[must_use]
    pub fn new_with_analytics(
        repo: SkillsRepository,
        audit_log: Arc<AuditLog>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> Self {
        Self {
            repo,
            audit_log,
            finalize_lock: Mutex::new(()),
            analytics,
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
        let finished = match task.status {
            TaskStatus::Failed => false,
            // People press Stop to close a task that already finished, so a Stop only counts
            // against the run when it cut off a call that was still running.
            TaskStatus::Cancelled => !self.repo.has_cancelled_dispatch(session_id).await?,
            _ => true,
        };
        // A session is only marked ultrafast when its flow returned done.
        let clean = mark.mode == RunMode::ultrafast().mode || (task.error_count == 0 && finished);
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
        let recorded = self.repo.insert_run_if_absent(row.clone()).await?;
        if recorded {
            self.report_run(&row).await?;
        }
        Ok(recorded)
    }

    async fn report_run(&self, run: &skill_runs::Model) -> AppResult<()> {
        let Some(skill) = self.repo.get(&run.skill_name).await? else {
            return Ok(());
        };
        if run.run_number == 1 {
            let days = (now_ms() - skill.created_at).max(0) / DAY_MS;
            self.analytics.capture(
                events::SKILL_FIRST_RUN,
                json!({ "origin": skill.origin, "days_since_saved": days }),
            );
        }
        let mut properties = json!({
            "mode": run.mode,
            "origin": skill.origin,
            "run_number_bucket": events::run_number_bucket(run.run_number),
            "duration_bucket": events::p50_bucket(run.duration_ms.unwrap_or(0)),
            "tool_count": run.tool_count.unwrap_or(0).max(0),
            "clean": run.clean,
        });
        if let Some(step) = run.handover_step {
            properties["handover_step"] = json!(step.max(0));
        }
        if let Some(cause) = run.handover_cause.as_deref() {
            properties["handover_cause"] = json!(match cause {
                "site_changed" | "input_only" | "transient" => cause,
                _ => "unknown",
            });
        }
        self.analytics.capture(events::SKILL_RUN, properties);
        Ok(())
    }
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        analytics::events::EventDefinition,
        db::{DATABASE_FILENAME, Database, entities::skills},
    };
    use serde_json::Value;

    #[derive(Default)]
    struct RecordingAnalytics {
        events: std::sync::Mutex<Vec<(&'static str, Value)>>,
    }

    impl AnalyticsSink for RecordingAnalytics {
        fn capture(&self, event: EventDefinition, properties: Value) {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((event.name(), properties));
        }
    }

    impl RecordingAnalytics {
        fn names(&self) -> Vec<(&'static str, Value)> {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    fn skill(name: &str, origin: &str) -> skills::Model {
        skills::Model {
            name: name.to_string(),
            description: "Find phones under a budget".to_string(),
            site: None,
            origin: origin.to_string(),
            source_session_id: None,
            version: 1,
            body_path: format!("{name}/SKILL.md"),
            linked_agents_json: "[]".to_string(),
            inputs_json: None,
            created_at: now_ms() - 2 * DAY_MS,
            updated_at: now_ms(),
        }
    }

    async fn finished_run(
        audit_log: &AuditLog,
        service: &SkillRunService,
        session_id: &str,
        skill_name: &str,
    ) -> AppResult<bool> {
        audit_log
            .record_session_start(session_id, "convo", "codex", "codex/run", "codex", "1")
            .await?;
        audit_log
            .record_session_end(session_id, "closed", None)
            .await?;
        service.mark(session_id, skill_name).await?;
        service.finalize(session_id).await
    }

    #[tokio::test]
    async fn only_a_skills_first_recorded_run_is_counted() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        let repo = SkillsRepository::new(db.clone());
        repo.try_insert(skill("neo-phones", "history")).await?;
        let audit_log = Arc::new(AuditLog::new(db.clone()));
        let analytics = Arc::new(RecordingAnalytics::default());
        let service =
            SkillRunService::new_with_analytics(repo, audit_log.clone(), analytics.clone());

        assert!(finished_run(&audit_log, &service, "run-1", "neo-phones").await?);
        assert!(finished_run(&audit_log, &service, "run-2", "neo-phones").await?);

        let first_runs: Vec<_> = analytics
            .names()
            .into_iter()
            .filter(|(name, _)| *name == events::SKILL_FIRST_RUN.name())
            .collect();
        assert_eq!(
            first_runs,
            vec![(
                events::SKILL_FIRST_RUN.name(),
                json!({ "origin": "history", "days_since_saved": 2 })
            )]
        );
        Ok(())
    }

    fn runs(analytics: &RecordingAnalytics) -> Vec<Value> {
        analytics
            .names()
            .into_iter()
            .filter(|(name, _)| *name == events::SKILL_RUN.name())
            .map(|(_, properties)| properties)
            .collect()
    }

    #[tokio::test]
    async fn every_recorded_run_is_reported_with_how_it_ran() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        let repo = SkillsRepository::new(db.clone());
        repo.try_insert(skill("neo-phones", "agent")).await?;
        let audit_log = Arc::new(AuditLog::new(db.clone()));
        let analytics = Arc::new(RecordingAnalytics::default());
        let service =
            SkillRunService::new_with_analytics(repo, audit_log.clone(), analytics.clone());

        assert!(finished_run(&audit_log, &service, "run-1", "neo-phones").await?);
        audit_log
            .record_session_start("run-2", "convo", "codex", "codex/run", "codex", "1")
            .await?;
        audit_log
            .record_session_end("run-2", "closed", None)
            .await?;
        service
            .mark_as(
                "run-2",
                "neo-phones",
                &RunMode::handed_back(3, "site_changed"),
            )
            .await?;
        assert!(service.finalize("run-2").await?);
        // Finalizing again records nothing and reports nothing.
        assert!(!service.finalize("run-2").await?);

        assert_eq!(
            runs(&analytics),
            vec![
                json!({
                    "mode": "agent",
                    "origin": "agent",
                    "run_number_bucket": "1",
                    "duration_bucket": "under_2s",
                    "tool_count": 0,
                    "clean": true,
                }),
                json!({
                    "mode": "handed_back",
                    "origin": "agent",
                    "run_number_bucket": "2_to_5",
                    "duration_bucket": "under_2s",
                    "tool_count": 0,
                    "clean": true,
                    "handover_step": 3,
                    "handover_cause": "site_changed",
                }),
            ]
        );
        for properties in runs(&analytics) {
            assert!(
                events::SKILL_RUN.sanitize(&properties).as_ref() == Some(&properties),
                "{properties} should pass the catalog unchanged"
            );
        }
        Ok(())
    }
}
