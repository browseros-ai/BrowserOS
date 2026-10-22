use crate::{
    db::{
        Database,
        entities::{
            prelude::{SkillRunMarks, SkillRuns, Skills, ToolDispatches},
            skill_run_marks, skill_runs, skills, tool_dispatches,
        },
    },
    error::AppResult,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait, sea_query::OnConflict,
};

/// Database boundary for user skills and their run history.
#[derive(Clone)]
pub struct SkillsRepository {
    db: Database,
}

impl SkillsRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn all(&self) -> AppResult<Vec<skills::Model>> {
        Ok(Skills::find()
            .order_by_desc(skills::Column::UpdatedAt)
            .all(self.db.connection())
            .await?)
    }

    pub async fn get(&self, name: &str) -> AppResult<Option<skills::Model>> {
        Ok(Skills::find_by_id(name.to_owned())
            .one(self.db.connection())
            .await?)
    }

    /// Insert a skill only if its name is free. Returns `true` when the row was
    /// created and `false` when the name was already taken. The unique primary
    /// key makes this an atomic reservation, including across processes sharing
    /// the database.
    pub async fn try_insert(&self, model: skills::Model) -> AppResult<bool> {
        let inserted = Skills::insert(into_active(model))
            .on_conflict(
                OnConflict::column(skills::Column::Name)
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(self.db.connection())
            .await?;
        Ok(inserted == 1)
    }

    pub async fn update(&self, model: skills::Model) -> AppResult<()> {
        into_active(model).update(self.db.connection()).await?;
        Ok(())
    }

    /// Deletes a skill and everything keyed to its name: its run history and any
    /// pending run marks. Otherwise a task recreated with the same name would
    /// inherit the deleted task's runs, counts, token history, and run
    /// numbering, contradicting the delete dialog's "run history is discarded".
    /// Atomic so a partial failure cannot orphan runs without their skill.
    pub async fn delete(&self, name: &str) -> AppResult<u64> {
        let txn = self.db.connection().begin().await?;
        let result = Skills::delete_by_id(name.to_owned()).exec(&txn).await?;
        SkillRuns::delete_many()
            .filter(skill_runs::Column::SkillName.eq(name))
            .exec(&txn)
            .await?;
        SkillRunMarks::delete_many()
            .filter(skill_run_marks::Column::SkillName.eq(name))
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(result.rows_affected)
    }

    /// A cursor-paginated page of a skill's runs, newest run first. `cursor` is
    /// an exclusive upper bound on `run_number`; `next_cursor` is the last
    /// returned run number when the page is full.
    pub async fn list_runs(
        &self,
        name: &str,
        cursor: Option<i64>,
        limit: u64,
    ) -> AppResult<(Vec<skill_runs::Model>, Option<i64>)> {
        let mut query = SkillRuns::find()
            .filter(skill_runs::Column::SkillName.eq(name))
            .order_by_desc(skill_runs::Column::RunNumber);
        if let Some(cursor) = cursor {
            query = query.filter(skill_runs::Column::RunNumber.lt(cursor));
        }
        let rows = query.limit(limit).all(self.db.connection()).await?;
        let next_cursor = (rows.len() as u64 == limit)
            .then(|| rows.last().map(|row| row.run_number))
            .flatten();
        Ok((rows, next_cursor))
    }

    /// Every run across all skills, used to project per-skill list stats.
    pub async fn all_runs(&self) -> AppResult<Vec<skill_runs::Model>> {
        Ok(SkillRuns::find()
            .order_by_asc(skill_runs::Column::RunNumber)
            .all(self.db.connection())
            .await?)
    }

    pub async fn runs_for(&self, name: &str) -> AppResult<Vec<skill_runs::Model>> {
        Ok(SkillRuns::find()
            .filter(skill_runs::Column::SkillName.eq(name))
            .order_by_asc(skill_runs::Column::RunNumber)
            .all(self.db.connection())
            .await?)
    }

    pub async fn max_run_number(&self, name: &str) -> AppResult<Option<i64>> {
        Ok(SkillRuns::find()
            .filter(skill_runs::Column::SkillName.eq(name))
            .order_by_desc(skill_runs::Column::RunNumber)
            .one(self.db.connection())
            .await?
            .map(|row| row.run_number))
    }

    /// Insert a run row unless the session already recorded one. The unique
    /// session index makes this idempotent; returns whether this call inserted.
    pub async fn insert_run_if_absent(&self, model: skill_runs::Model) -> AppResult<bool> {
        let inserted = SkillRuns::insert(skill_runs::ActiveModel {
            id: Set(model.id),
            skill_name: Set(model.skill_name),
            session_id: Set(model.session_id),
            run_number: Set(model.run_number),
            agent_id: Set(model.agent_id),
            tokens: Set(model.tokens),
            duration_ms: Set(model.duration_ms),
            tool_count: Set(model.tool_count),
            clean: Set(model.clean),
            errored_tool: Set(model.errored_tool),
            created_at: Set(model.created_at),
        })
        .on_conflict(
            OnConflict::column(skill_runs::Column::SessionId)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(self.db.connection())
        .await?;
        Ok(inserted == 1)
    }

    /// The first tool in a session whose dispatch recorded an error (used for a
    /// not-clean run's `errored_tool`).
    pub async fn first_errored_tool(&self, session_id: &str) -> AppResult<Option<String>> {
        let rows = ToolDispatches::find()
            .filter(tool_dispatches::Column::SessionId.eq(session_id))
            .order_by_asc(tool_dispatches::Column::Id)
            .all(self.db.connection())
            .await?;
        Ok(rows
            .into_iter()
            .find(|row| dispatch_is_error(row.result_meta.as_deref()))
            .map(|row| row.tool_name))
    }

    pub async fn upsert_mark(&self, session_id: &str, skill_name: &str, now: i64) -> AppResult<()> {
        SkillRunMarks::insert(skill_run_marks::ActiveModel {
            session_id: Set(session_id.to_owned()),
            skill_name: Set(skill_name.to_owned()),
            created_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(skill_run_marks::Column::SessionId)
                .update_column(skill_run_marks::Column::SkillName)
                .to_owned(),
        )
        .exec_without_returning(self.db.connection())
        .await?;
        Ok(())
    }

    pub async fn get_mark(&self, session_id: &str) -> AppResult<Option<skill_run_marks::Model>> {
        Ok(SkillRunMarks::find_by_id(session_id.to_owned())
            .one(self.db.connection())
            .await?)
    }

    pub async fn all_marks(&self) -> AppResult<Vec<skill_run_marks::Model>> {
        Ok(SkillRunMarks::find().all(self.db.connection()).await?)
    }

    pub async fn delete_mark(&self, session_id: &str) -> AppResult<()> {
        SkillRunMarks::delete_by_id(session_id.to_owned())
            .exec(self.db.connection())
            .await?;
        Ok(())
    }
}

fn dispatch_is_error(result_meta: Option<&str>) -> bool {
    result_meta
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|value| {
            value.get("isError").and_then(serde_json::Value::as_bool) == Some(true)
                && value.get("cancelled").and_then(serde_json::Value::as_bool) != Some(true)
        })
}

fn into_active(model: skills::Model) -> skills::ActiveModel {
    skills::ActiveModel {
        name: Set(model.name),
        description: Set(model.description),
        site: Set(model.site),
        origin: Set(model.origin),
        source_session_id: Set(model.source_session_id),
        version: Set(model.version),
        body_path: Set(model.body_path),
        linked_agents_json: Set(model.linked_agents_json),
        created_at: Set(model.created_at),
        updated_at: Set(model.updated_at),
    }
}
