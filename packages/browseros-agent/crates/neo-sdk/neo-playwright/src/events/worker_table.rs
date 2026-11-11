//! Dedicated worker endpoints, owned by the target watcher and shared with evaluation ops.
//! A worker's target identity survives duplicate auto-attaches; its chosen session and
//! execution context belong to one connection epoch. Closing wakes every in-flight call.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use browseros_core::{ProtocolSession, SessionId, TargetId};
use serde::Serialize;
use tokio::sync::watch;

/// A live dedicated worker, for the host's ordered page facts and the prelude's cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
    pub(crate) session: ProtocolSession,
    pub(crate) context: watch::Sender<WorkerContext>,
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
                session,
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
            if worker.session.session_id() == Some(session) {
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
            if worker.session.session_id() == Some(session) {
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
