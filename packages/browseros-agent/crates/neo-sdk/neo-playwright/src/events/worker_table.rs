//! Dedicated worker endpoints, owned by the target watcher and shared with evaluation ops.
//! A worker's target identity survives duplicate auto-attaches; its chosen session and
//! execution context belong to one connection epoch. Closing wakes every in-flight call.

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

/// The execution context is unavailable during startup and permanently gone after detach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerContext {
    Starting {
        context: Option<i64>,
        script_loaded: bool,
    },
    Ready(i64),
    Closed,
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

    pub(crate) fn rebind(&self, session: ProtocolSession, wait_for_script: bool) {
        *self.session.lock().unwrap_or_else(PoisonError::into_inner) = session;
        self.context.send_replace(WorkerContext::Starting {
            context: None,
            script_loaded: !wait_for_script,
        });
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
        let (context, _) = watch::channel(WorkerContext::Starting {
            context: None,
            script_loaded: !wait_for_script,
        });
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

    pub(crate) fn context(&self, session: &SessionId, context: i64) {
        for worker in self
            .workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            if worker.session().session_id() == Some(session) {
                worker.context.send_modify(|state| {
                    *state = match *state {
                        WorkerContext::Starting {
                            script_loaded: false,
                            ..
                        } => WorkerContext::Starting {
                            context: Some(context),
                            script_loaded: false,
                        },
                        WorkerContext::Starting {
                            script_loaded: true,
                            ..
                        }
                        | WorkerContext::Ready(_) => WorkerContext::Ready(context),
                        WorkerContext::Closed => WorkerContext::Closed,
                    };
                });
            }
        }
    }

    /// Chromium 143+ announces when the worker's startup script has finished. Evaluating
    /// at executionContextCreated alone can run before the script defines its globals.
    pub(crate) fn script_loaded(&self, session: &SessionId) {
        for worker in self
            .workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            if worker.session().session_id() == Some(session) {
                worker.context.send_modify(|state| {
                    *state = match *state {
                        WorkerContext::Starting {
                            context: Some(id), ..
                        }
                        | WorkerContext::Ready(id) => WorkerContext::Ready(id),
                        WorkerContext::Starting { context: None, .. } => WorkerContext::Starting {
                            context: None,
                            script_loaded: true,
                        },
                        WorkerContext::Closed => WorkerContext::Closed,
                    };
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
                    WorkerContext::Ready(id) => {
                        return Ok((retained.session().session_id().cloned(), id));
                    }
                    WorkerContext::Closed => return Err("a live worker was treated as closed"),
                    WorkerContext::Starting { .. } => {
                        if context.changed().await.is_err() {
                            return Err("startup channel closed");
                        }
                    }
                }
            }
        });
        let recovered = SessionId::from("recovered-session");
        endpoint.rebind(browser.protocol_session(recovered.clone()), true);
        table.context(&old, 1);
        table.context(&recovered, 42);
        table.script_loaded(&recovered);
        let answered = time::timeout(Duration::from_secs(1), waiter).await???;
        assert_eq!(
            answered,
            (Some(recovered), 42),
            "the original waiting call uses the recovered session and its context"
        );
        Ok(())
    }
}
