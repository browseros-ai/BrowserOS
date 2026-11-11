//! The Rust side of the script's three natives.

use std::collections::{HashMap, VecDeque};
use std::mem;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use neo_trace::OperationId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{self, Duration, Instant};

use super::{LogLevel, RunLogs};
use crate::browser::PageRef;
use crate::dispatch::{
    Call, CallSite, Dispatcher, Observation, OpAbort, OpFailure, OpReply, PageFact, PendingReply,
    RunCtx, StopReason,
};

/// How many failed ops a run remembers for its own error. A script's uncaught `NeoError` is
/// nearly always its latest failure; one from further back is reported as Neo's.
const FAILURES_REMEMBERED: usize = 1024;

/// The name the bootstrap aborts an op under (`bootstrap.js`, Playwright's `signal`), with input
/// `{"key", "reason"}`. It never reaches the door: the bridge answers it itself, and `$` starts
/// no method's name. Upstream's client sends its `__abort__` the same way, down the channel its
/// calls take, so an abort needs no native of its own.
const ABORT: &str = "$neo.abort";

/// The brand the bootstrap wraps an abortable call's input in: `{"$neo": "abortable", "key",
/// "input"}`. The key is the realm's own name for the call, the only one it has before the
/// reply brings the op id; the door gets `input` alone.
const ABORTABLE: &str = "abortable";

/// What the natives do for one run. Takes and returns only strings and numbers and never
/// touches the runtime: an op runs as its own task on the run's set and answers over a
/// channel, so a slow op never holds the runtime's lock, and a panicking one never unwinds
/// through QuickJS.
pub(crate) struct Bridge {
    run: Arc<RunCtx>,
    dispatcher: Arc<Dispatcher>,
    logs: Mutex<RunLogs>,
    /// The run's latest failed ops. An escaped `NeoError` names its op, and the run's error is
    /// the failure recorded here, never fields the script could have edited.
    failures: Mutex<VecDeque<OpFailure>>,
    /// The abort switch of each abortable op still in flight, by the realm's key for it. An
    /// entry leaves when its op answers, so a late abort finds nothing and changes nothing.
    aborts: Mutex<HashMap<String, OpAbort>>,
    /// Finished navigation replies can reach JS out of order. Retain their receipts until
    /// the client acknowledges seeing them; storage follows unacknowledged producer batches.
    navigation_replies: Mutex<HashMap<u64, NavigationReceipt>>,
}

/// What one `__neo_invoke` started: an op the door admitted, or an abort the bridge already
/// answered. [`Bridge::answer`] turns either into the string the native resolves with.
pub(crate) struct Submitted(Started);

enum Started {
    Op {
        pending: PendingReply,
        /// The realm's key for an abortable op, released with its answer.
        abort_key: Option<String>,
    },
    Navigation {
        reply: NavigationReply,
        abort_key: Option<String>,
    },
    Answered(String),
}

/// Sharing only the reply receiver lets another completion poll readiness without joining
/// browser work or depending on when the producer's native callback task is scheduled.
type NavigationReply = Shared<BoxFuture<'static, Arc<OpReply>>>;

/// A receipt retains only readiness once complete; the producer's own future keeps its
/// answer alive. Long runs do not retain every completed navigation's frame payload.
enum NavigationReceipt {
    Pending(NavigationReply),
    Complete,
}

impl NavigationReceipt {
    fn ready(&mut self) -> bool {
        match self {
            Self::Complete => true,
            Self::Pending(reply) if reply.clone().now_or_never().is_some() => {
                *self = Self::Complete;
                true
            }
            Self::Pending(_) => false,
        }
    }
}

/// The JS wire form of an `OpReply`: `{"ok": value, "pages": […]}` or `{"err": {…}, "pages":
/// […]}`, plus `defaultPage` once the call opened the run's own page and `pumped` once an op
/// ordered its answer after event-pump replies. Protocol translation only: the door decided
/// every value in it.
#[derive(Serialize)]
#[serde(untagged)]
enum Envelope<'a> {
    Ok {
        ok: &'a Value,
        pages: &'a [PageFact],
        #[serde(skip_serializing_if = "Option::is_none")]
        observed: Option<&'a Observation>,
        #[serde(rename = "defaultPage", skip_serializing_if = "Option::is_none")]
        default_page: Option<DefaultPage>,
        /// `OpReply::pumped`: the prelude dispatches that many event-pump replies before it
        /// hands the script this one.
        #[serde(skip_serializing_if = "is_zero")]
        pumped: u64,
        #[serde(
            rename = "completedNavigations",
            skip_serializing_if = "<[u64]>::is_empty"
        )]
        completed_navigations: &'a [u64],
    },
    Err {
        err: ErrBody<'a>,
        pages: &'a [PageFact],
        #[serde(skip_serializing_if = "Option::is_none")]
        observed: Option<&'a Observation>,
        #[serde(rename = "defaultPage", skip_serializing_if = "Option::is_none")]
        default_page: Option<DefaultPage>,
        #[serde(skip_serializing_if = "is_zero")]
        pumped: u64,
        #[serde(
            rename = "completedNavigations",
            skip_serializing_if = "<[u64]>::is_empty"
        )]
        completed_navigations: &'a [u64],
    },
}

/// The fields of the script's `NeoError`, named as the trace names them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrBody<'a> {
    code: &'static str,
    owner: &'static str,
    delivery: &'static str,
    retryable: bool,
    operation_id: OperationId,
    /// Makes the bootstrap throw a `TimeoutError`.
    timeout: bool,
    message: &'a str,
}

/// The tab of the run's own page, once the call that needed it opened it. Plain ids rather
/// than the page brand, so the page prelude can give the script's existing `page` its tab
/// instead of reviving a second object for it.
#[derive(Serialize)]
struct DefaultPage {
    tab: i64,
}

impl Bridge {
    /// The natives of one run.
    #[must_use]
    pub(crate) fn new(run: Arc<RunCtx>, dispatcher: Arc<Dispatcher>) -> Self {
        Self {
            run,
            dispatcher,
            logs: Mutex::default(),
            failures: Mutex::default(),
            aborts: Mutex::default(),
            navigation_replies: Mutex::default(),
        }
    }

    /// `__neo_invoke(name, inputJson, site)`, as the script calls it: one SDK call through the
    /// door, admitted now, so it has its op id and its start record before the realm runs on. A
    /// prelude's refusal comes this way too, as a refusal brand (`dispatch/refusal_brand.rs`).
    /// An abortable call registers its abort switch here, before the realm can fire it; the
    /// bootstrap's abort of one ([`ABORT`]) is answered here and never reaches the door.
    pub(crate) fn submit(&self, name: String, input_json: &str, site: &str) -> Submitted {
        let input = decode_input(input_json);
        if name == ABORT {
            self.abort(&input);
            return Submitted(Started::Answered(ABORTED.to_owned()));
        }
        let site = CallSite::parse(site);
        let publication;
        let (call, abort_key) = match abortable(input) {
            Ok((key, input)) => {
                let abort = OpAbort::default();
                lock(&self.aborts).insert(key.clone(), abort.clone());
                publication = navigation_publication(&name, &input);
                (Call::script(name, input, site).abortable(abort), Some(key))
            }
            Err(input) => {
                publication = navigation_publication(&name, &input);
                (Call::script(name, input, site), None)
            }
        };
        let pending = self.dispatcher.submit(&self.run, call);
        match publication {
            Some(publication) => {
                let reply = async move { Arc::new(pending.wait().await) }
                    .boxed()
                    .shared();
                let mut replies = lock(&self.navigation_replies);
                replies.retain(|id, reply| {
                    !publication.acknowledged.contains(id)
                        || !matches!(reply, NavigationReceipt::Complete)
                });
                replies.insert(publication.id, NavigationReceipt::Pending(reply.clone()));
                Submitted(Started::Navigation { reply, abort_key })
            }
            None => Submitted(Started::Op { pending, abort_key }),
        }
    }

    /// The answer `__neo_invoke`'s promise resolves to. Always an envelope, never a rejection, so
    /// every failure reaches the script as a `NeoError` with its owner and delivery.
    pub(crate) async fn answer(&self, submitted: Submitted) -> String {
        let (reply, abort_key) = match submitted.0 {
            Started::Op { pending, abort_key } => (Arc::new(pending.wait().await), abort_key),
            Started::Navigation { reply, abort_key } => (reply.await, abort_key),
            Started::Answered(answer) => return answer,
        };
        if let Some(key) = abort_key {
            lock(&self.aborts).remove(&key);
        }
        if let Err(failure) = &reply.outcome {
            let mut failures = lock(&self.failures);
            if failures.len() == FAILURES_REMEMBERED {
                failures.pop_front();
            }
            failures.push_back(failure.clone());
        }
        // Poll receiver readiness, not browser work: an earlier dispatcher may have sent
        // its reply while that producer's own callback task has not run yet. A ready shared
        // receiver retains the exact answer for that task; polling cannot consume it away.
        let mut completed: Vec<_> = lock(&self.navigation_replies)
            .iter_mut()
            .filter_map(|(id, reply)| reply.ready().then_some(*id))
            .collect();
        completed.extend(reply.completed_publications.iter().copied());
        completed.sort_unstable();
        completed.dedup();
        envelope(&reply, &completed)
    }

    /// The bootstrap's abort of the op it keyed `key`: `{"key", "reason"}`. An op that already
    /// answered has left the table, and an input the bootstrap never sends aborts nothing.
    fn abort(&self, input: &Value) {
        let (Some(key), Some(reason)) = (
            input.get("key").and_then(Value::as_str),
            input.get("reason").and_then(Value::as_str),
        ) else {
            return;
        };
        if let Some(abort) = lock(&self.aborts).get(key) {
            abort.abort(reason.to_owned());
        }
    }

    /// `__neo_log(level, text)`: one console line, kept within the run's limits.
    pub(crate) fn log(&self, level: &str, text: String) {
        lock(&self.logs).push(LogLevel::parse(level), text);
    }

    /// `__neo_sleep(ms)`: the clock behind `setTimeout`. Capped by the run's deadline and cut
    /// short by its stop, so a pending timer never outlives the run. Waking at the deadline
    /// stops the run there, as the interrupt handler would.
    pub(crate) async fn sleep(&self, ms: f64) {
        let deadline = self.run.deadline();
        // NaN, negative and infinite delays clamp; `max` ignores a NaN.
        let wake = Duration::try_from_secs_f64(ms.max(0.0) / 1000.0)
            .ok()
            .and_then(|delay| Instant::now().checked_add(delay))
            .map_or(deadline, |wake| wake.min(deadline));
        tokio::select! {
            () = time::sleep_until(wake) => {}
            () = self.run.cancel_token().cancelled() => {}
        }
        if Instant::now() >= deadline {
            self.run.stop(StopReason::Deadline);
        }
    }

    /// The failure the door decided for `operation`, if the run still remembers it.
    #[must_use]
    pub(crate) fn failure(&self, operation: OperationId) -> Option<OpFailure> {
        lock(&self.failures)
            .iter()
            .rev()
            .find(|failure| failure.operation == operation)
            .cloned()
    }

    /// The run's console output. Called once the runtime is gone.
    pub(crate) fn take_logs(&self) -> RunLogs {
        mem::take(&mut *lock(&self.logs))
    }
}

/// What the bridge answers an abort with: an empty success, which the bootstrap ignores.
const ABORTED: &str = r#"{"ok":null,"pages":[]}"#;

/// An abortable call's key and its own input, or the input as it came when it is not one.
fn abortable(input: Value) -> Result<(String, Value), Value> {
    let is_brand = input.get("$neo").and_then(Value::as_str) == Some(ABORTABLE);
    let key = input.get("key").and_then(Value::as_str).map(str::to_owned);
    match (is_brand, key, input) {
        (true, Some(key), Value::Object(mut fields)) => match fields.remove("input") {
            Some(inner) => Ok((key, inner)),
            None => Err(Value::Object(fields)),
        },
        (_, _, input) => Err(input),
    }
}

/// The bootstrap's codec always sends JSON. Text that is not (a script calling the native by
/// hand) becomes the codec's invalid brand, which the door records and refuses as
/// `script / invalid_input`.
fn decode_input(input_json: &str) -> Value {
    serde_json::from_str(input_json).unwrap_or_else(
        |error| json!({ "$neo": "invalid", "reason": format!("the input is not JSON: {error}") }),
    )
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
fn is_zero(count: &u64) -> bool {
    *count == 0
}

/// Internal scheduling hints. They cannot claim a native failure or change input delivery.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NavigationPublication {
    id: u64,
    acknowledged: Vec<u64>,
}

fn navigation_publication(name: &str, input: &Value) -> Option<NavigationPublication> {
    if !matches!(
        name,
        "page.goto" | "page.reload" | "page.goBack" | "page.goForward"
    ) {
        return None;
    }
    serde_json::from_value(input.get("publication")?.clone()).ok()
}

fn envelope(reply: &OpReply, completed_navigations: &[u64]) -> String {
    let pumped = reply.pumped;
    let default_page = reply.default_page.as_ref().map(DefaultPage::from);
    let envelope = match &reply.outcome {
        Ok(ok) => Envelope::Ok {
            ok,
            pages: &reply.pages,
            observed: reply.observed.as_ref(),
            default_page,
            pumped,
            completed_navigations,
        },
        Err(failure) => Envelope::Err {
            err: ErrBody::from(failure),
            pages: &reply.pages,
            observed: reply.observed.as_ref(),
            default_page,
            pumped,
            completed_navigations,
        },
    };
    serde_json::to_string(&envelope).unwrap_or_else(|error| {
        // Unreachable for these types; still an answer, so the script never hangs on it.
        json!({
            "err": {
                "code": "internal_error", "owner": "neo", "delivery": "unknown",
                "retryable": false, "operationId": "", "timeout": false,
                "message": format!("Neo could not encode this operation's reply: {error}"),
            },
            "pages": [],
        })
        .to_string()
    })
}

impl<'a> From<&'a OpFailure> for ErrBody<'a> {
    fn from(failure: &'a OpFailure) -> Self {
        Self {
            code: failure.error.code().as_str(),
            owner: failure.error.owner().as_str(),
            delivery: failure.delivery.as_str(),
            retryable: failure.retryable,
            operation_id: failure.operation,
            timeout: failure.error.is_timeout(),
            message: failure.error.message(),
        }
    }
}

impl From<&PageRef> for DefaultPage {
    fn from(page: &PageRef) -> Self {
        Self { tab: page.tab.0 }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
