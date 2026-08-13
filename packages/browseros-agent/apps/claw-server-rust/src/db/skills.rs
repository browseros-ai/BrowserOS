use crate::{
    db::{
        Database,
        entities::{
            prelude::{SkillRuns, Skills},
            skill_runs, skills,
        },
    },
    error::AppResult,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
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

    pub async fn exists(&self, name: &str) -> AppResult<bool> {
        Ok(Skills::find_by_id(name.to_owned())
            .one(self.db.connection())
            .await?
            .is_some())
    }

    pub async fn insert(&self, model: skills::Model) -> AppResult<()> {
        Skills::insert(into_active(model))
            .exec_without_returning(self.db.connection())
            .await?;
        Ok(())
    }

    pub async fn update(&self, model: skills::Model) -> AppResult<()> {
        into_active(model).update(self.db.connection()).await?;
        Ok(())
    }

    pub async fn delete(&self, name: &str) -> AppResult<u64> {
        let result = Skills::delete_by_id(name.to_owned())
            .exec(self.db.connection())
            .await?;
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
