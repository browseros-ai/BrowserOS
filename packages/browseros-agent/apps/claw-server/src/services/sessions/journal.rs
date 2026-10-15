//! claw's `SessionJournal`: session start/end and operator-cancel marks go to the audit
//! database, tab-claim release goes to the session tab ledger. An audit write failure
//! surfaces to `Sessions` unchanged, so a mint without its start row still aborts.

use crate::{
    db::{AuditLog, SessionTabLedger},
    ids::DispatchId,
};
use browseros_mcp_host::{
    HostResult,
    seams::{SessionJournal, SessionStart},
};
use futures_util::future::BoxFuture;
use std::sync::Arc;

pub struct ClawSessionJournal {
    audit_log: Arc<AuditLog>,
    session_tabs: Arc<SessionTabLedger>,
}

impl ClawSessionJournal {
    #[must_use]
    pub fn new(audit_log: Arc<AuditLog>, session_tabs: Arc<SessionTabLedger>) -> Arc<Self> {
        Arc::new(Self {
            audit_log,
            session_tabs,
        })
    }
}

impl SessionJournal for ClawSessionJournal {
    fn record_session_start<'a>(
        &'a self,
        start: SessionStart<'a>,
    ) -> BoxFuture<'a, HostResult<()>> {
        Box::pin(async move {
            self.audit_log
                .record_session_start(
                    start.session_id,
                    start.convo_id,
                    start.slug,
                    start.agent_label,
                    start.client_name,
                    start.client_version,
                )
                .await?;
            Ok(())
        })
    }

    fn record_session_end<'a>(
        &'a self,
        session_id: &'a str,
        kind: &'a str,
        reason: Option<&'a str>,
    ) -> BoxFuture<'a, HostResult<()>> {
        Box::pin(async move {
            self.audit_log
                .record_session_end(session_id, kind, reason)
                .await?;
            Ok(())
        })
    }

    fn mark_dispatch_operator_cancelled<'a>(
        &'a self,
        dispatch_id: &'a DispatchId,
    ) -> BoxFuture<'a, HostResult<()>> {
        Box::pin(async move {
            self.audit_log
                .mark_dispatch_operator_cancelled(dispatch_id)
                .await?;
            Ok(())
        })
    }

    fn release_session_tabs(&self, session_id: String) {
        self.session_tabs
            .enqueue_release_claims_for_session(session_id);
    }

    fn set_task_summary<'a>(
        &'a self,
        session_id: &'a str,
        summary: &'a str,
    ) -> BoxFuture<'a, HostResult<()>> {
        Box::pin(async move {
            self.audit_log.set_task_summary(session_id, summary).await?;
            Ok(())
        })
    }
}
