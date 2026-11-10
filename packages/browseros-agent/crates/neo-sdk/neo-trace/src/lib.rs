//! Neo: Correlated live logs for one script and its actual Playwright calls.
//! The runtime owns call records and cancellation; this adapter owns run ids, rendering,
//! and the optional durable-write queue. Persistence never reaches back into execution.
//! Raw code and parameters are logged at info by default. `CLAW_LOG=info,neo_trace=off`
//! disables them in claw-server. Hosted operators must choose their payload logging policy
//! before deployment because stderr may be collected outside the local machine.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::time::Instant;

use neo_runtime::{
    CallEvent, CallRecord, CallResult, Delivery, RunOutcome, RunRequest, RunStatus, Runtime,
    StartPage,
};
use serde::{Serialize, Serializer};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tracing::{dispatcher, field, info, warn};
use ulid::Ulid;

mod store;

pub use store::{CallRow, RunDetail, RunRow, TraceError, TraceStore};

/// Neo: Code log payloads stop at 64 KiB; the typed request is never shortened.
const CODE_BYTES: usize = 64 * 1024;
/// Neo: Large call parameters and failure logs stop at 4 KiB each in the log adapter only.
const FIELD_BYTES: usize = 4 * 1024;

/// One traced run's identity, minted here and echoed by hosts. Display and serde use
/// the same 26-character ULID string so concurrent runs can be correlated in any transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunId(Ulid);

impl FromStr for RunId {
    type Err = ulid::DecodeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for RunId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Runs a script and logs its code, live calls, and outcome on target `neo_trace`.
/// An optional store receives best-effort queued writes without database waits.
/// Cancel through `request.cancel` and await completion; do not drop this future.
pub async fn run(
    runtime: &Runtime,
    request: RunRequest,
    store: Option<&TraceStore>,
) -> (RunId, RunOutcome) {
    let id = RunId(Ulid::new());
    let started = Instant::now();
    // The runtime may create the first tab inside its deadline. Until it does, there is
    // no target id to log; subsequent call guids and the outcome provide that identity.
    let page = match &request.page {
        StartPage::Page(id) => Some(id.as_str()),
        StartPage::NewTab => None,
    };
    info!(
        target: "neo_trace", run = %id, page = page.map(field::debug),
        code = ?clip(&request.code, CODE_BYTES), "run.start"
    );
    if let Some(store) = store {
        store.start_run(id, &request);
    }
    let (sender, receiver) = mpsc::unbounded_channel();
    let (finished, completion) = oneshot::channel();
    // QuickJS can occupy its async worker until a synchronous script yields. The
    // collector therefore polls on the blocking pool, even on a one-worker runtime.
    // Its future uses only channels, so Handle::block_on needs no separate I/O driver.
    // Carry the caller's subscriber explicitly across this thread boundary.
    let collector = Handle::try_current().ok().map(|handle| {
        let store = store.cloned();
        let subscriber = dispatcher::get_default(Clone::clone);
        let collector_runtime = handle.clone();
        handle.spawn_blocking(move || {
            dispatcher::with_default(&subscriber, || {
                collector_runtime.block_on(collect(id, receiver, completion, store.as_ref()))
            })
        })
    });
    // No Tokio context keeps Runtime's existing error outcome rather than panicking
    // while starting a collector. Such a run cannot dispatch any browser calls.
    let outcome = runtime.run_observed(request, sender).await;
    let _sent = finished.send(());
    let deliveries = match collector {
        Some(collector) => match collector.await {
            Ok(deliveries) => deliveries,
            Err(error) => {
                warn!(target: "neo_trace", run = %id, error = ?error, "call collector failed");
                BTreeMap::new()
            }
        },
        None => BTreeMap::new(),
    };
    // The call's input acknowledgement can arrive after its result. The outcome is
    // authoritative, so reconcile that one mutable field before the final run line.
    for (call, delivery) in delivery_changes(&deliveries, &outcome.calls) {
        info!(
            target: "neo_trace", run = %id, call,
            delivery = delivery.map(delivery_name), "call.delivery"
        );
        if let Some(store) = store {
            store.update_delivery(id, call, delivery);
        }
    }
    let status = match outcome.status {
        RunStatus::Ok => "ok",
        RunStatus::Error => "error",
        RunStatus::Timeout => "timeout",
        RunStatus::Cancelled => "cancelled",
    };
    let error = outcome
        .error
        .as_ref()
        .map(|error| format!("{}: {}", error.name, error.message));
    // Awaiting the collector above puts every live observation before the final delivery
    // reconciliation and run end in the database queue, even across the blocking thread.
    if let Some(store) = store {
        store.finish_run(id, &outcome);
    }
    info!(
        target: "neo_trace", run = %id, status, error = error.as_deref().map(field::debug),
        ms = started.elapsed().as_millis(), calls = outcome.calls.len(),
        dropped = outcome.calls_dropped.as_ref().map_or(0, |dropped| dropped.count), "run.end"
    );
    (id, outcome)
}

/// Completion comes from the outcome, never sender closure. Dropping the run also
/// releases this waiter; ordinary cancellation still awaits all final observations.
async fn collect(
    id: RunId,
    mut receiver: mpsc::UnboundedReceiver<CallEvent>,
    mut completion: oneshot::Receiver<()>,
    store: Option<&TraceStore>,
) -> BTreeMap<u64, Option<Delivery>> {
    let mut deliveries = BTreeMap::new();
    loop {
        tokio::select! {
            _ = &mut completion => break,
            Some(event) = receiver.recv() => log_call(id, event, &mut deliveries, store),
        }
    }
    while let Ok(event) = receiver.try_recv() {
        log_call(id, event, &mut deliveries, store);
    }
    deliveries
}

fn log_call(
    id: RunId,
    event: CallEvent,
    deliveries: &mut BTreeMap<u64, Option<Delivery>>,
    store: Option<&TraceStore>,
) {
    match event {
        CallEvent::Started(call) => {
            let params = call.params.to_string();
            info!(
                target: "neo_trace", run = %id, call = call.call_id,
                line = call.location.map(|location| location.line),
                column = call.location.and_then(|location| location.column),
                title = ?call.title.as_deref().unwrap_or(&call.method), guid = ?call.guid,
                params = %clip(&params, FIELD_BYTES), "call.start"
            );
            if let Some(store) = store {
                store.start_call(id, call);
            }
        }
        CallEvent::Finished(call) => {
            let (result, error) = match &call.result {
                CallResult::Ok => ("ok", None),
                CallResult::Error { name, message } => {
                    ("error", Some(format!("{name}: {message}")))
                }
                CallResult::Unfinished { .. } => ("unfinished", None),
            };
            let log = (call.result != CallResult::Ok).then(|| call.log.join("\n"));
            info!(
                target: "neo_trace", run = %id, call = call.call_id,
                ms = call.duration.as_millis(), result, error = error.as_deref().map(field::debug),
                delivery = call.delivery.map(delivery_name),
                log = log.as_deref().map(|log| field::debug(clip(log, FIELD_BYTES))), "call.end"
            );
            deliveries.insert(call.call_id, call.delivery);
            if let Some(store) = store {
                store.finish_call(id, call);
            }
        }
    }
}

fn delivery_changes<'a>(
    logged: &'a BTreeMap<u64, Option<Delivery>>,
    calls: &'a [CallRecord],
) -> impl Iterator<Item = (u64, Option<Delivery>)> + 'a {
    calls.iter().filter_map(|call| {
        (logged.get(&call.call_id) != Some(&call.delivery)).then_some((call.call_id, call.delivery))
    })
}

fn delivery_name(delivery: Delivery) -> &'static str {
    match delivery {
        Delivery::NotSent => "notSent",
        Delivery::Done => "done",
        Delivery::Unknown => "unknown",
    }
}

fn clip(text: &str, limit: usize) -> Cow<'_, str> {
    if text.len() <= limit {
        return Cow::Borrowed(text);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Cow::Owned(format!("{}…[+{} bytes]", &text[..end], text.len() - end))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn delivery_diff_reports_only_acknowledgements_that_changed() {
        let record = |call_id, delivery| CallRecord {
            call_id,
            guid: "frame@tab".to_string(),
            method: "click".to_string(),
            title: None,
            params: serde_json::Value::Null,
            location: None,
            started_at_ms: 0,
            duration: Duration::ZERO,
            result: CallResult::Ok,
            delivery,
            log: Vec::new(),
        };
        let logged = BTreeMap::from([
            (1, Some(Delivery::Unknown)),
            (2, Some(Delivery::Done)),
            (3, None),
        ]);
        let calls = [
            record(1, Some(Delivery::Done)),
            record(2, Some(Delivery::Done)),
            record(3, None),
        ];
        assert_eq!(
            delivery_changes(&logged, &calls).collect::<Vec<_>>(),
            [(1, Some(Delivery::Done))]
        );
    }
}
