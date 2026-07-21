use crate::{
    db::{AuditLog as DbAuditLog, Database, SessionTabLedger, entities::session_tabs},
    error::AppResult,
};
use std::{path::Path, sync::Arc};

pub use crate::db::audit_log::*;

#[derive(Clone)]
pub struct AuditService {
    audit_log: Arc<DbAuditLog>,
    session_tabs: Arc<SessionTabLedger>,
}

impl AuditService {
    pub async fn open(path: impl AsRef<Path>) -> AppResult<Self> {
        let database = Database::open(path).await?;
        Ok(Self::new(database))
    }

    pub fn new(database: Database) -> Self {
        Self {
            audit_log: Arc::new(DbAuditLog::new(database.clone())),
            session_tabs: Arc::new(SessionTabLedger::new(database)),
        }
    }

    #[cfg(test)]
    pub(crate) fn connection(&self) -> &sea_orm::DatabaseConnection {
        self.audit_log.connection()
    }

    pub async fn record_tool_dispatch(&self, input: RecordToolDispatchInput) -> AppResult<i64> {
        self.audit_log.record_tool_dispatch(input).await
    }

    pub async fn mark_screenshot(&self, dispatch_id: i64) -> AppResult<()> {
        self.audit_log.mark_screenshot(dispatch_id).await
    }

    pub async fn record_session_start(
        &self,
        session_id: &str,
        agent_id: &str,
        slug: &str,
        agent_label: &str,
        client_name: &str,
        client_version: &str,
    ) -> AppResult<()> {
        self.audit_log
            .record_session_start(
                session_id,
                agent_id,
                slug,
                agent_label,
                client_name,
                client_version,
            )
            .await
    }

    pub async fn record_session_end(
        &self,
        session_id: &str,
        kind: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        self.audit_log
            .record_session_end(session_id, kind, reason)
            .await
    }

    pub async fn release_claims_for_target(&self, target_id: &str) -> AppResult<u64> {
        self.session_tabs.release_claims_for_target(target_id).await
    }

    pub async fn claim_target_for_session(
        &self,
        target_id: &str,
        session_id: &str,
        agent_id: &str,
        claimed_at: i64,
    ) -> AppResult<i64> {
        self.session_tabs
            .claim_target_for_session(target_id, session_id, agent_id, claimed_at)
            .await
    }

    pub async fn release_target_for_session(
        &self,
        target_id: &str,
        session_id: &str,
    ) -> AppResult<u64> {
        self.session_tabs
            .release_target_for_session(target_id, session_id)
            .await
    }

    pub async fn release_claims_for_session(&self, session_id: &str) -> AppResult<u64> {
        self.session_tabs
            .release_claims_for_session(session_id)
            .await
    }

    pub fn enqueue_claim_target_for_session(
        &self,
        target_id: String,
        session_id: String,
        agent_id: String,
        claimed_at: i64,
    ) {
        self.session_tabs
            .enqueue_claim_target_for_session(target_id, session_id, agent_id, claimed_at);
    }

    pub fn enqueue_release_target_for_session(&self, target_id: String, session_id: String) {
        self.session_tabs
            .enqueue_release_target_for_session(target_id, session_id);
    }

    pub fn enqueue_release_claims_for_session(&self, session_id: String) {
        self.session_tabs
            .enqueue_release_claims_for_session(session_id);
    }

    pub fn enqueue_release_claims_for_target(&self, target_id: String) {
        self.session_tabs
            .enqueue_release_claims_for_target(target_id);
    }

    pub fn enqueue_claim_tab_for_session(
        &self,
        tab_id: i64,
        opened_target_id: Option<String>,
        session_id: String,
        agent_id: String,
        claimed_at: i64,
    ) {
        self.session_tabs.enqueue_claim_tab_for_session(
            tab_id,
            opened_target_id,
            session_id,
            agent_id,
            claimed_at,
        );
    }

    pub fn enqueue_inherit_tab_ownership(
        &self,
        opener_tab_id: i64,
        tab_id: i64,
        opened_target_id: String,
        claimed_at: i64,
    ) {
        self.session_tabs.enqueue_inherit_tab_ownership(
            opener_tab_id,
            tab_id,
            opened_target_id,
            claimed_at,
        );
    }

    pub fn enqueue_release_tab_for_session(&self, tab_id: i64, session_id: String) {
        self.session_tabs
            .enqueue_release_tab_for_session(tab_id, session_id);
    }

    pub async fn drain_claim_writes(&self) {
        self.session_tabs.drain_writes().await;
    }

    pub async fn release_all_open_claims(&self) -> AppResult<u64> {
        self.session_tabs.release_all_open().await
    }

    pub async fn list_dispatches(
        &self,
        query: ListDispatchesQuery,
    ) -> AppResult<ListDispatchesResult> {
        self.audit_log.list_dispatches(query).await
    }

    pub async fn list_tasks(&self, query: ListTasksQuery) -> AppResult<ListTasksResult> {
        self.audit_log.list_tasks(query).await
    }

    pub async fn get_task_summary(&self, session_id: &str) -> AppResult<Option<TaskSummary>> {
        self.audit_log.get_task_summary(session_id).await
    }

    pub async fn list_open_session_tabs(
        &self,
        session_ids: &[String],
    ) -> AppResult<Vec<session_tabs::Model>> {
        self.session_tabs.list_open_session_tabs(session_ids).await
    }

    pub async fn open_session_tab(
        &self,
        session_id: &str,
        tab_id: i64,
    ) -> AppResult<Option<session_tabs::Model>> {
        self.session_tabs.open_session_tab(session_id, tab_id).await
    }

    pub async fn get_task(&self, session_id: &str) -> AppResult<Option<TaskDetail>> {
        self.audit_log.get_task(session_id).await
    }
}
