//! Neo: hook-only host requests. Each callback runs on the run's tracker, outside QuickJS,
//! so ending its invocation drops the callback future even while the realm is frozen.

use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio::time;
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, DropGuard};
use tokio_util::task::TaskTracker;

use crate::deadline::Deadline;
use crate::run::{HostCallback, HostRequest};

/// Neo: the maximum JSON byte length of either side of a hook host exchange.
const JSON_LIMIT: usize = 1024 * 1024;

/// The callback and run lifetime, plus the invocation currently allowed to ask it.
#[derive(Clone)]
pub(crate) struct Host {
    callback: Option<HostCallback>,
    deadline: Deadline,
    tasks: TaskTracker,
    job: Arc<Mutex<Option<Job>>>,
}

#[derive(Clone)]
struct Job {
    until: Instant,
    cancelled: CancellationToken,
}

impl Job {
    fn check(&self) -> Result<(), String> {
        if self.cancelled.is_cancelled() || Instant::now() >= self.until {
            Err("the hook invocation ended".to_string())
        } else {
            Ok(())
        }
    }
}

impl Host {
    pub(crate) fn new(
        callback: Option<HostCallback>,
        deadline: Deadline,
        tasks: TaskTracker,
    ) -> Self {
        Self {
            callback,
            deadline,
            tasks,
            job: Arc::default(),
        }
    }

    /// The actor holds this guard for exactly one job, including its synchronous JS. Dropping
    /// it cancels even calls that JS never awaited. Closing the sandbox also cancels them.
    pub(crate) fn arm(&self, until: Instant, close: &CancellationToken) -> DropGuard {
        let cancelled = close.child_token();
        if let Ok(mut job) = self.job.lock() {
            *job = Some(Job {
                until,
                cancelled: cancelled.clone(),
            });
        }
        cancelled.drop_guard()
    }

    /// Starts synchronously at the bridge call, before QuickJS first polls its promise: this
    /// captures the sending job, never whichever job later polls the frozen realm.
    pub(crate) fn call(&self, text: &str) -> BoxFuture<'static, Result<String, String>> {
        let pending = self.start(text);
        let deadline = self.deadline.clone();
        Box::pin(async move {
            let (job, reply) = pending?;
            let answer = reply.await.map_err(|_| "the host task ended".to_string())?;
            // An answer buffered before the job ended must not resolve in a later job.
            job.check()?;
            if deadline.stop().is_some() {
                return Err("the run stopped".to_string());
            }
            answer
        })
    }

    fn start(
        &self,
        text: &str,
    ) -> Result<(Job, oneshot::Receiver<Result<String, String>>), String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Request {
            name: String,
            #[serde(default)]
            args: Value,
        }
        if self.deadline.stop().is_some() {
            return Err("the run stopped".to_string());
        }
        let job = self
            .job
            .lock()
            .map_err(|_| "the hook invocation is unavailable".to_string())?
            .clone()
            .ok_or_else(|| "no hook invocation is running".to_string())?;
        job.check()?;
        if text.len() > JSON_LIMIT {
            return Err("the host request exceeds the JSON limit".to_string());
        }
        let request: Request = serde_json::from_str(text)
            .map_err(|_| "the host request must contain a name and JSON args".to_string())?;
        if request.name.is_empty() {
            return Err("the host request needs a non-empty name".to_string());
        }
        let callback = self
            .callback
            .clone()
            .ok_or_else(|| "no host callback was supplied".to_string())?;
        let deadline = self.deadline.clone();
        let running = job.clone();
        let (send, reply) = oneshot::channel();
        self.tasks.spawn(async move {
            let answer = tokio::select! {
                biased;
                _ = deadline.reached() => Err("the run stopped".to_string()),
                () = running.cancelled.cancelled() => Err("the hook invocation ended".to_string()),
                () = time::sleep_until(running.until) => Err("the hook budget ended".to_string()),
                // Construct the host future only once the task has checked every stop.
                answer = async { callback(HostRequest { name: request.name, args: request.args }).await } => {
                    answer.and_then(|value| {
                        let text = value.to_string();
                        if text.len() > JSON_LIMIT {
                            Err("the host answer exceeds the JSON limit".to_string())
                        } else {
                            Ok(text)
                        }
                    })
                }
            };
            let _sent = send.send(answer);
        });
        Ok((job, reply))
    }
}
