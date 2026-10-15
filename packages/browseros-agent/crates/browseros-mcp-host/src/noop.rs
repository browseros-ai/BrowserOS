//! Default adapters for a host without an audit trail, analytics, cockpit or recordings.
//! Every method succeeds and records nothing, so MCP behaviour is unchanged.

use crate::{
    error::HostResult,
    identity::ProfileView,
    ids::DispatchId,
    seams::{
        BrowserSessionObserver, HostAnalytics, HostEvent, ProfileDirectory, SessionJournal,
        SessionStart, TabActivity, TabClaim, TabJournal,
    },
};
use browseros_core::BrowserSession;
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSessionJournal;

impl SessionJournal for NoopSessionJournal {
    fn record_session_start<'a>(
        &'a self,
        _start: SessionStart<'a>,
    ) -> BoxFuture<'a, HostResult<()>> {
        Box::pin(async { Ok(()) })
    }

    fn record_session_end<'a>(
        &'a self,
        _session_id: &'a str,
        _kind: &'a str,
        _reason: Option<&'a str>,
    ) -> BoxFuture<'a, HostResult<()>> {
        Box::pin(async { Ok(()) })
    }

    fn mark_dispatch_operator_cancelled<'a>(
        &'a self,
        _dispatch_id: &'a DispatchId,
    ) -> BoxFuture<'a, HostResult<()>> {
        Box::pin(async { Ok(()) })
    }

    fn release_session_tabs(&self, _session_id: String) {}

    fn set_task_summary<'a>(
        &'a self,
        _session_id: &'a str,
        _summary: &'a str,
    ) -> BoxFuture<'a, HostResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopAnalytics;

impl HostAnalytics for NoopAnalytics {
    fn capture(&self, _event: HostEvent, _properties: Value) {}
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopBrowserSessionObserver;

impl BrowserSessionObserver for NoopBrowserSessionObserver {
    fn observe_session(
        &self,
        _session: Arc<BrowserSession>,
        _epoch: u64,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }

    fn is_ready(&self, _epoch: u64) -> bool {
        true
    }

    fn reseed<'a>(
        &'a self,
        _session: &'a BrowserSession,
        _epoch: u64,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTabJournal;

impl TabJournal for NoopTabJournal {
    fn record_activity(&self, _activity: TabActivity) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn forget_incarnation<'a>(&'a self, _page_id: u32, _target_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn claim_tab(&self, _claim: TabClaim) {}

    fn release_tab(&self, _tab_id: i64, _session_id: String) {}
}

/// No saved agent profiles: every client resolves to an ephemeral identity.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopProfiles;

impl ProfileDirectory for NoopProfiles {
    fn profiles(&self) -> BoxFuture<'_, HostResult<Vec<ProfileView>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}
