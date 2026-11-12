//! Dedicated worker endpoints, owned by the target watcher and shared with evaluation ops.
//! A worker's target identity survives duplicate auto-attaches; its chosen session belongs to
//! one connection epoch. Closing wakes every in-flight call.
//!
//! Readiness comes from `Inspector.workerScriptLoaded`, which Chromium sends without any domain
//! enabled, never from `Runtime` context events: `Runtime` stays off on a worker session unless a
//! run listens for its console (`target_watcher.rs`), so scripts run in the worker's only
//! context, its global scope, as the session's default context.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use browseros_core::{ProtocolSession, SessionId, TargetId};
use serde::Serialize;
use tokio::sync::watch;
use ts_rs::TS;

/// A live dedicated worker, for the host's ordered page facts and the prelude's cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct WorkerInfo {
    /// The worker target's identity, stable until it closes.
    pub id: String,
    /// The script URL reported by Chromium, including blob URLs.
    pub url: String,
}

/// Whether a script may run in the worker's global scope: not before its startup script has run
/// (its globals are not defined yet), and never again after detach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerContext {
    Starting,
    Ready,
    Closed,
}

impl WorkerContext {
    /// A new endpoint's state. Without `Inspector.workerScriptLoaded` (Chromium before 143)
    /// nothing will report the script, so the worker counts as ready once attached, as
    /// upstream treats it there.
    fn attached(wait_for_script: bool) -> Self {
        if wait_for_script {
            Self::Starting
        } else {
            Self::Ready
        }
    }
}

#[derive(Debug)]
pub(crate) struct WorkerEndpoint {
    pub(crate) page: TargetId,
    pub(crate) info: WorkerInfo,
    session: Mutex<ProtocolSession>,
    pub(crate) context: watch::Sender<WorkerContext>,
}

impl WorkerEndpoint {
    /// Startup recovery may replace the protocol session while a call waits on `context`.
    /// A Ready endpoint is never rebound, so callers read this after their readiness wait.
    pub(crate) fn session(&self) -> ProtocolSession {
        self.session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Chromium reports `Inspector.workerScriptLoaded` again on the new session at attach when
    /// the script already ran, so a rebound endpoint starts over.
    pub(crate) fn rebind(&self, session: ProtocolSession, wait_for_script: bool) {
        *self.session.lock().unwrap_or_else(PoisonError::into_inner) = session;
        self.context
            .send_replace(WorkerContext::attached(wait_for_script));
    }
}

/// Retains only live workers. Callers keep an endpoint while evaluating, so removal can
/// signal closure without retaining dead workers in the process-wide table.
#[derive(Debug, Default)]
pub(crate) struct WorkerTable {
    workers: Mutex<HashMap<String, Arc<WorkerEndpoint>>>,
}

impl WorkerTable {
    pub(crate) fn insert(
        &self,
        page: TargetId,
        info: WorkerInfo,
        session: ProtocolSession,
        wait_for_script: bool,
    ) -> bool {
        let mut workers = self.workers.lock().unwrap_or_else(PoisonError::into_inner);
        if workers.contains_key(&info.id) {
            return false;
        }
        let (context, _) = watch::channel(WorkerContext::attached(wait_for_script));
        workers.insert(
            info.id.clone(),
            Arc::new(WorkerEndpoint {
                page,
                info,
                session: Mutex::new(session),
                context,
            }),
        );
        true
    }

    pub(crate) fn get(&self, page: &TargetId, id: &str) -> Option<Arc<WorkerEndpoint>> {
        self.workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .filter(|worker| worker.page == *page)
            .map(Arc::clone)
    }

    pub(crate) fn list(&self, page: &TargetId) -> Vec<WorkerInfo> {
        self.workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|worker| worker.page == *page)
            .map(|worker| worker.info.clone())
            .collect()
    }

    /// Chromium 143+ announces when the worker's startup script has finished, and again at
    /// attach to a worker whose script already ran. A script run before it could find the
    /// worker's globals undefined.
    pub(crate) fn script_loaded(&self, session: &SessionId) {
        for worker in self
            .workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            if worker.session().session_id() == Some(session) {
                worker.context.send_if_modified(|state| {
                    let starting = *state == WorkerContext::Starting;
                    if starting {
                        *state = WorkerContext::Ready;
                    }
                    starting
                });
            }
        }
    }

    pub(crate) fn remove(&self, id: &str) -> Option<Arc<WorkerEndpoint>> {
        let worker = self
            .workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)?;
        worker.context.send_replace(WorkerContext::Closed);
        Some(worker)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_core::{SessionId, TargetId};
    use tokio::time;

    use super::{WorkerContext, WorkerInfo, WorkerTable};
    use crate::test_env;

    #[tokio::test]
    async fn a_startup_waiter_follows_a_recovered_live_endpoint() -> Result<(), Box<dyn Error>> {
        let browser = test_env::offline_session();
        let table = WorkerTable::default();
        let page = TargetId("page".to_owned());
        let old = SessionId::from("old-session");
        assert!(table.insert(
            page.clone(),
            WorkerInfo {
                id: "worker".to_owned(),
                url: "worker.js".to_owned()
            },
            browser.protocol_session(old.clone()),
            true
        ));
        let endpoint = table.get(&page, "worker").ok_or("worker exists")?;
        let retained = Arc::clone(&endpoint);
        let mut context = endpoint.context.subscribe();
        let waiter = tokio::spawn(async move {
            loop {
                let state = *context.borrow_and_update();
                match state {
                    WorkerContext::Ready => return Ok(retained.session().session_id().cloned()),
                    WorkerContext::Closed => return Err("a live worker was treated as closed"),
                    WorkerContext::Starting => {
                        if context.changed().await.is_err() {
                            return Err("startup channel closed");
                        }
                    }
                }
            }
        });
        let recovered = SessionId::from("recovered-session");
        endpoint.rebind(browser.protocol_session(recovered.clone()), true);
        table.script_loaded(&old);
        assert_eq!(
            *endpoint.context.borrow(),
            WorkerContext::Starting,
            "the replaced session's report is not the recovered endpoint's"
        );
        table.script_loaded(&recovered);
        let answered = time::timeout(Duration::from_secs(1), waiter).await???;
        assert_eq!(
            answered,
            Some(recovered),
            "the original waiting call uses the recovered session"
        );
        Ok(())
    }
}
