//! The Rust side of the script's three natives.

use std::collections::VecDeque;
use std::mem;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_trace::{OperationId, SecretSet};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::time::{self, Duration, Instant};

use super::{LogLevel, RunLogs};
use crate::browser::PageRef;
use crate::dispatch::{
    Call, CallSite, Dispatcher, OpFailure, OpReply, PageFact, RunCtx, StopReason,
};

/// How many failed ops a run remembers for its own error. A script's uncaught `NeoError` is
/// nearly always its latest failure; one from further back is reported as Neo's.
const FAILURES_REMEMBERED: usize = 1024;

/// What the natives do for one run. Takes and returns only strings and numbers and never
/// touches the runtime: an op runs as its own task on the run's set and answers over a
/// channel, so a slow op never holds the runtime's lock, and a panicking one never unwinds
/// through QuickJS.
pub(crate) struct Bridge {
    run: Arc<RunCtx>,
    dispatcher: Arc<Dispatcher>,
    /// The process's known secrets: console lines are masked as they are kept.
    secrets: Arc<SecretSet>,
    logs: Mutex<RunLogs>,
    /// The run's latest failed ops. An escaped `NeoError` names its op, and the run's error is
    /// the failure recorded here, never fields the script could have edited.
    failures: Mutex<VecDeque<OpFailure>>,
}

/// The JS wire form of an `OpReply`: `{"ok": value, "pages": […]}` or `{"err": {…}, "pages":
/// […]}`, plus `defaultPage` once the call opened the run's own page. Protocol translation
/// only: the door decided and masked every value in it.
#[derive(Serialize)]
#[serde(untagged)]
enum Envelope<'a> {
    Ok {
        ok: &'a Value,
        pages: &'a [PageFact],
        #[serde(rename = "defaultPage", skip_serializing_if = "Option::is_none")]
        default_page: Option<DefaultPage>,
    },
    Err {
        err: ErrBody<'a>,
        pages: &'a [PageFact],
        #[serde(rename = "defaultPage", skip_serializing_if = "Option::is_none")]
        default_page: Option<DefaultPage>,
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
    id: u32,
    tab: i64,
}

impl Bridge {
    /// The natives of one run.
    #[must_use]
    pub(crate) fn new(run: Arc<RunCtx>, dispatcher: Arc<Dispatcher>) -> Self {
        Self {
            secrets: Arc::clone(dispatcher.secrets()),
            run,
            dispatcher,
            logs: Mutex::default(),
            failures: Mutex::default(),
        }
    }

    /// `__neo_invoke(name, inputJson, site)`: one SDK call through the door. Always resolves to
    /// an envelope, never rejects, so every failure reaches the script as a `NeoError` with its
    /// owner and delivery.
    pub(crate) async fn invoke(&self, name: String, input_json: String, site: String) -> String {
        let call = Call::script(name, decode_input(&input_json), CallSite::parse(&site));
        let reply = self.dispatcher.submit(&self.run, call).wait().await;
        if let Err(failure) = &reply.outcome {
            let mut failures = lock(&self.failures);
            if failures.len() == FAILURES_REMEMBERED {
                failures.pop_front();
            }
            failures.push_back(failure.clone());
        }
        envelope(&reply)
    }

    /// `__neo_log(level, text)`: one console line, kept within the run's limits.
    pub(crate) fn log(&self, level: &str, text: String) {
        lock(&self.logs).push(LogLevel::parse(level), text, &self.secrets);
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

/// The bootstrap's codec always sends JSON. Text that is not (a script calling the native by
/// hand) becomes the codec's invalid brand, which the door records and refuses as
/// `script / invalid_input`.
fn decode_input(input_json: &str) -> Value {
    serde_json::from_str(input_json).unwrap_or_else(
        |error| json!({ "$neo": "invalid", "reason": format!("the input is not JSON: {error}") }),
    )
}

fn envelope(reply: &OpReply) -> String {
    let default_page = reply.default_page.as_ref().map(DefaultPage::from);
    let envelope = match &reply.outcome {
        Ok(ok) => Envelope::Ok {
            ok,
            pages: &reply.pages,
            default_page,
        },
        Err(failure) => Envelope::Err {
            err: ErrBody::from(failure),
            pages: &reply.pages,
            default_page,
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
        Self {
            id: page.id.0,
            tab: page.tab.0,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
