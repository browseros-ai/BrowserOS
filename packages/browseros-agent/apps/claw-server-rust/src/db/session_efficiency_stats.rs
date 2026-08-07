use crate::{
    db::{
        Database,
        entities::{
            agent_session_ends, agent_session_starts,
            prelude::{
                AgentSessionEnds, AgentSessionStarts, SessionEfficiencyStats, ToolDispatches,
            },
            session_efficiency_stats, tool_dispatches,
        },
    },
    error::AppResult,
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, QueryFilter,
    QueryOrder, Statement, sea_query::OnConflict,
};

/// Audit estimator version accepted by efficiency projections; version 0 is unmeasured legacy data.
pub(crate) const ELIGIBLE_TOKEN_ESTIMATOR_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub(crate) struct SessionEfficiencySource {
    pub session_id: String,
    pub start: Option<agent_session_starts::Model>,
    pub end: agent_session_ends::Model,
    pub dispatches: Vec<tool_dispatches::Model>,
}

/// Database boundary for insert-once efficiency projections and their source audit rows.
#[derive(Clone)]
pub struct SessionEfficiencyStatsRepository {
    db: Database,
}

impl SessionEfficiencyStatsRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub(crate) async fn source_for_session(
        &self,
        session_id: &str,
    ) -> AppResult<Option<SessionEfficiencySource>> {
        let Some(end) = AgentSessionEnds::find()
            .filter(agent_session_ends::Column::SessionId.eq(session_id))
            .order_by_asc(agent_session_ends::Column::Id)
            .one(self.db.connection())
            .await?
        else {
            return Ok(None);
        };
        let start = AgentSessionStarts::find()
            .filter(agent_session_starts::Column::SessionId.eq(session_id))
            .order_by_asc(agent_session_starts::Column::Id)
            .one(self.db.connection())
            .await?;
        let dispatches = ToolDispatches::find()
            .filter(tool_dispatches::Column::SessionId.eq(session_id))
            .order_by_asc(tool_dispatches::Column::Id)
            .all(self.db.connection())
            .await?;
        Ok(Some(SessionEfficiencySource {
            session_id: session_id.to_owned(),
            start,
            end,
            dispatches,
        }))
    }

    pub async fn insert_if_absent(&self, row: &session_efficiency_stats::Model) -> AppResult<bool> {
        let inserted = SessionEfficiencyStats::insert(session_efficiency_stats::ActiveModel {
            session_id: Set(row.session_id.clone()),
            ended_at: Set(row.ended_at),
            dispatch_count: Set(row.dispatch_count.max(0)),
            active_duration_ms: Set(row.active_duration_ms.max(0)),
            tool_input_token_estimate: Set(row.tool_input_token_estimate.max(0)),
            tool_output_token_estimate: Set(row.tool_output_token_estimate.max(0)),
            screenshot_baseline_token_estimate: Set(row.screenshot_baseline_token_estimate.max(0)),
            efficiency_estimator_version: Set(row.efficiency_estimator_version.max(0)),
            computed_at: Set(row.computed_at),
        })
        .on_conflict(
            OnConflict::column(session_efficiency_stats::Column::SessionId)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(self.db.connection())
        .await?;
        Ok(inserted == 1)
    }

    pub async fn reconciliation_candidates(&self) -> AppResult<Vec<String>> {
        let rows = self
            .db
            .connection()
            .query_all(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                r#"
                SELECT DISTINCT ended.session_id
                FROM agent_session_ends AS ended
                WHERE EXISTS (
                    SELECT 1
                    FROM tool_dispatches AS dispatch
                    WHERE dispatch.session_id = ended.session_id
                )
                AND NOT EXISTS (
                    SELECT 1
                    FROM tool_dispatches AS dispatch
                    WHERE dispatch.session_id = ended.session_id
                      AND dispatch.token_estimator_version != ?
                )
                AND NOT EXISTS (
                    SELECT 1
                    FROM session_efficiency_stats AS stats
                    WHERE stats.session_id = ended.session_id
                )
                ORDER BY ended.session_id
                "#
                .to_owned(),
                [ELIGIBLE_TOKEN_ESTIMATOR_VERSION.into()],
            ))
            .await?;
        rows.into_iter()
            .map(|row| row.try_get("", "session_id").map_err(Into::into))
            .collect()
    }

    pub(crate) async fn all_rows(&self) -> AppResult<Vec<session_efficiency_stats::Model>> {
        Ok(SessionEfficiencyStats::find()
            .all(self.db.connection())
            .await?)
    }

    #[cfg(test)]
    pub(crate) async fn find(
        &self,
        session_id: &str,
    ) -> AppResult<Option<session_efficiency_stats::Model>> {
        Ok(SessionEfficiencyStats::find_by_id(session_id)
            .one(self.db.connection())
            .await?)
    }
}
