//! How an op ends, decided once.

use std::sync::Arc;
use std::thread;

use browseros_core::TabId;
use browseros_trace::{
    Delivery, Effect, EpochMillis, OperationEnded, OperationKind, OperationStarted, Outcome,
    RedactionPolicy, SecretSet, TargetSensitivity, TraceSink,
};
use serde_json::Value;
use tokio::sync::oneshot;

use super::op_ctx::OpState;
use super::{
    Answer, CallSite, Caller, ErrorCode, LeafError, Lifetime, MethodSpec, OpFailure, OpLine,
    OpReply, OperationError, PageFact, RunCtx, StopReason,
};
use crate::browser::PageRef;

/// Ends one op exactly once: the trace's `op_ended`, the run's ledger, and the caller's reply.
///
/// [`OperationGuard::finish`] is the normal path and consumes the guard. `Drop` is the
/// backstop for an op whose future went away without an answer: its task was aborted at
/// teardown, never polled after a closed run refused it, or unwound from a panic. Drop settles
/// it with the run's stop reason, or as a panic, with delivery still derived from the sent
/// flag. Nothing here touches the JS runtime: the guard lives in an op task, never inside
/// QuickJS, so it is safe wherever the op's future is dropped.
pub(super) struct OperationGuard {
    open: Option<Open>,
}

struct Open {
    state: Arc<OpState>,
    spec: MethodSpec,
    site: Option<CallSite>,
    caller: Caller,
    run: Arc<RunCtx>,
    trace: TraceSink,
    secrets: Arc<SecretSet>,
    reply: oneshot::Sender<OpReply>,
}

/// What opening an op records. A plain data bag.
pub(super) struct OpStart {
    pub(super) state: Arc<OpState>,
    /// The name as the caller wrote it, so an unknown name is on the record as typed.
    pub(super) name: String,
    pub(super) spec: MethodSpec,
    pub(super) redaction: Arc<RedactionPolicy>,
    /// The raw input. The trace writer redacts it.
    pub(super) args: Value,
    /// The tab the call named, before the door checks it.
    pub(super) tab: Option<TabId>,
    pub(super) site: Option<CallSite>,
    pub(super) caller: Caller,
    pub(super) reply: oneshot::Sender<OpReply>,
}

/// How the run step ended, as the door saw it.
#[derive(Debug)]
pub(super) enum Ending {
    /// The method returned this value.
    Returned(Value),
    /// The method failed, with or without saying whether the browser answered.
    Failed(LeafError),
    /// The door refused before or after the run step: bad input, a gate, a rule, a dead target.
    Refused(OperationError),
    /// The run stopped under it, whether the door's race cut it or its task was aborted.
    Stopped(StopReason),
    /// Its task panicked.
    Panicked,
    /// It was dropped with no stop and no panic: the runtime shut down under it.
    Lost,
}

/// The decided end of an op: a pure function of whether it was sent, its spec (effect and
/// lifetime) and its ending. The trace, the reply and the run's effects all read this one value,
/// so they cannot disagree (skeleton D7 rule 5). A plain data bag.
#[derive(Debug, PartialEq)]
pub(super) struct Settlement {
    pub(super) outcome: Outcome,
    pub(super) delivery: Delivery,
    pub(super) retryable: bool,
    pub(super) result: Settled,
}

/// What an op ended with, as its trace record and its reply carry it.
#[derive(Debug, PartialEq)]
pub(super) enum Settled {
    /// The method's value.
    Value(Value),
    /// The op failed: the error is on its record and in its reply.
    Failed(OperationError),
    /// A background pull (sdk-playwright's event pump) that the run's stop cut. A pull has no
    /// end of its own, so this is how every pull ends, not a failure: its record carries no
    /// error (skeleton D7 rule 15). A caller still waiting on it gets the stop's error, as any
    /// cancelled call does; in practice the script is gone by then.
    Retired(StopReason),
}

impl Settlement {
    /// The delivery table. Not sent is `not_sent`, whatever happened. Sent and answered is
    /// `done`; sent and unanswered is `unknown`. Stops, panics and drops are always unanswered,
    /// and so is a leaf error that did not say. Retrying is safe when nothing was sent or the
    /// op only reads. A stop retires a background pull rather than failing it.
    #[must_use]
    pub(super) fn decide(sent: bool, spec: &MethodSpec, ending: Ending) -> Self {
        let (outcome, answer, result) = match ending {
            Ending::Returned(value) => (Outcome::Ok, Answer::Definite, Settled::Value(value)),
            Ending::Failed(leaf) => (
                Outcome::Error,
                leaf.answer.unwrap_or(Answer::Ambiguous),
                Settled::Failed(leaf.error),
            ),
            Ending::Refused(error) => (Outcome::Error, Answer::Definite, Settled::Failed(error)),
            Ending::Stopped(reason) => {
                let result = match spec.lifetime {
                    Lifetime::Operation => Settled::Failed(reason.error()),
                    Lifetime::Background => Settled::Retired(reason),
                };
                (Outcome::Cancelled, Answer::Ambiguous, result)
            }
            Ending::Panicked => (
                Outcome::Error,
                Answer::Ambiguous,
                Settled::Failed(OperationError::neo(
                    ErrorCode::INTERNAL_ERROR,
                    "the operation failed inside Neo",
                )),
            ),
            Ending::Lost => (
                Outcome::Cancelled,
                Answer::Ambiguous,
                Settled::Failed(OperationError::neo(
                    ErrorCode::INTERNAL_ERROR,
                    "the operation was dropped before it finished",
                )),
            ),
        };
        let delivery = match (sent, answer) {
            (false, Answer::Definite | Answer::Ambiguous) => Delivery::NotSent,
            (true, Answer::Definite) => Delivery::Done,
            (true, Answer::Ambiguous) => Delivery::Unknown,
        };
        let retryable = match (delivery, spec.effect) {
            (Delivery::NotSent, Effect::Read | Effect::Write)
            | (Delivery::Done | Delivery::Unknown, Effect::Read) => true,
            (Delivery::Done | Delivery::Unknown, Effect::Write) => false,
        };
        Self {
            outcome,
            delivery,
            retryable,
            result,
        }
    }
}

impl OperationGuard {
    /// Opens the op: records `op_started` before any rule runs, so refusals are on the record
    /// too (skeleton D7 rule 3). A channel send; it never waits on disk.
    pub(super) fn start(
        trace: &TraceSink,
        secrets: &Arc<SecretSet>,
        run: &Arc<RunCtx>,
        start: OpStart,
    ) -> Self {
        let OpStart {
            state,
            name,
            spec,
            redaction,
            args,
            tab,
            site,
            caller,
            reply,
        } = start;
        trace.op_started(OperationStarted {
            id: state.id(),
            parent: state.parent_id(),
            name,
            kind: OperationKind::Builtin,
            effect: spec.effect,
            tab,
            args,
            policy: redaction,
            at: EpochMillis::now(),
        });
        Self {
            open: Some(Open {
                state,
                spec,
                site,
                caller,
                run: Arc::clone(run),
                trace: trace.clone(),
                secrets: Arc::clone(secrets),
                reply,
            }),
        }
    }

    /// Settles the op and answers its caller.
    pub(super) fn finish(
        mut self,
        ending: Ending,
        pages: Vec<PageFact>,
        default_page: Option<PageRef>,
    ) {
        if let Some(open) = self.open.take() {
            settle(open, ending, pages, default_page);
        }
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        if let Some(open) = self.open.take() {
            let ending = if thread::panicking() {
                Ending::Panicked
            } else {
                open.run.stop_reason().map_or(Ending::Lost, Ending::Stopped)
            };
            settle(open, ending, Vec::new(), None);
        }
    }
}

/// Shared by `finish` and `Drop`, so it is sync: channel sends and std locks only.
fn settle(open: Open, ending: Ending, pages: Vec<PageFact>, default_page: Option<PageRef>) {
    let Open {
        state,
        spec,
        site,
        caller,
        run,
        trace,
        secrets,
        reply,
    } = open;
    let id = state.id();
    let sent_at = state.sent_at();
    let settlement = Settlement::decide(sent_at.is_some(), &spec, ending);
    for mut note in state.take_notes() {
        secrets.mask_str(&mut note);
        tracing::debug!(operation = %id, method = spec.name, %note, "op note");
    }
    let (result, error) = match &settlement.result {
        Settled::Value(value) => (Some(value.clone()), None),
        Settled::Failed(error) => (None, Some(error.record(settlement.retryable, Some(id)))),
        Settled::Retired(_) => (None, None),
    };
    trace.op_ended(OperationEnded {
        id,
        outcome: settlement.outcome,
        delivery: settlement.delivery,
        sent_at,
        result,
        error,
        page_after: pages.first().map(PageFact::page_state),
        target: if state.is_sensitive() {
            TargetSensitivity::Sensitive
        } else {
            TargetSensitivity::Ordinary
        },
        at: EpochMillis::now(),
    });
    let line = OpLine {
        operation: id,
        name: spec.name,
        site,
    };
    run.settled(line, &spec, &settlement);
    let Settlement {
        delivery,
        retryable,
        result,
        ..
    } = settlement;
    let outcome = match result {
        Settled::Value(value) => Ok(value),
        Settled::Failed(error) => Err(error),
        Settled::Retired(reason) => Err(reason.error()),
    };
    let mut answer = OpReply {
        outcome: outcome.map_err(|error| OpFailure {
            error,
            delivery,
            retryable,
            operation: id,
        }),
        pages,
        default_page,
    };
    // A child's result stays inside Neo, and its parent's reply is masked on the way out.
    match caller {
        Caller::Script => answer.mask(&secrets),
        Caller::Neo => {}
    }
    // The caller may be gone (teardown dropped the bridge); nobody is owed the answer then.
    let _ = reply.send(answer);
}

#[cfg(test)]
mod tests {
    use browseros_trace::{Delivery, Effect, ErrorOwner, Outcome};
    use serde_json::json;

    use super::{Ending, Settled, Settlement};
    use crate::dispatch::{ErrorCode, LeafError, Lifetime, MethodSpec, OperationError, StopReason};

    fn error() -> OperationError {
        OperationError::website(ErrorCode::CDP_ERROR, "the page said no")
    }

    /// One row of the table: an ending and what it must decide. A plain data bag.
    struct Row {
        name: &'static str,
        ending: fn() -> Ending,
        outcome: Outcome,
        delivery_if_sent: Delivery,
    }

    fn rows() -> Vec<Row> {
        let row = |name, ending, outcome, delivery_if_sent| Row {
            name,
            ending,
            outcome,
            delivery_if_sent,
        };
        vec![
            row(
                "returned",
                || Ending::Returned(json!(1)),
                Outcome::Ok,
                Delivery::Done,
            ),
            row(
                "failed, definite",
                || Ending::Failed(LeafError::definite(error())),
                Outcome::Error,
                Delivery::Done,
            ),
            row(
                "failed, ambiguous",
                || Ending::Failed(LeafError::ambiguous(error())),
                Outcome::Error,
                Delivery::Unknown,
            ),
            row(
                "failed, unclassified",
                || Ending::Failed(LeafError::from(error())),
                Outcome::Error,
                Delivery::Unknown,
            ),
            row(
                "refused",
                || Ending::Refused(error()),
                Outcome::Error,
                Delivery::Done,
            ),
            row(
                "stopped",
                || Ending::Stopped(StopReason::Deadline),
                Outcome::Cancelled,
                Delivery::Unknown,
            ),
            row(
                "panicked",
                || Ending::Panicked,
                Outcome::Error,
                Delivery::Unknown,
            ),
            row(
                "lost",
                || Ending::Lost,
                Outcome::Cancelled,
                Delivery::Unknown,
            ),
        ]
    }

    #[test]
    fn the_delivery_table() {
        for Row {
            name,
            ending,
            outcome,
            delivery_if_sent,
        } in rows()
        {
            for effect in [Effect::Read, Effect::Write] {
                let spec = MethodSpec::new("test.op", effect);
                let unsent = Settlement::decide(false, &spec, ending());
                assert_eq!(unsent.outcome, outcome, "{name}");
                assert_eq!(
                    unsent.delivery,
                    Delivery::NotSent,
                    "{name}: nothing sent is not_sent"
                );
                assert!(
                    unsent.retryable,
                    "{name}: nothing sent is always safe to repeat"
                );

                let sent = Settlement::decide(true, &spec, ending());
                assert_eq!(sent.outcome, outcome, "{name}");
                assert_eq!(sent.delivery, delivery_if_sent, "{name}, sent");
                assert_eq!(
                    sent.retryable,
                    effect == Effect::Read,
                    "{name}, sent: only a read is safe to repeat"
                );
            }
        }
    }

    #[test]
    fn a_stopped_op_carries_the_stops_owner() {
        let spec = MethodSpec::new("test.op", Effect::Write);
        let settled = Settlement::decide(true, &spec, Ending::Stopped(StopReason::CallerLeft));
        let Settled::Failed(error) = settled.result else {
            panic!("a stopped operation fails: {:?}", settled.result);
        };
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Host, ErrorCode::CANCELLED),
            "the caller left, so the host owns the cut"
        );
    }

    #[test]
    fn a_stop_retires_a_background_pull_with_no_error() {
        let pull = MethodSpec {
            lifetime: Lifetime::Background,
            ..MethodSpec::new("test.pull", Effect::Read)
        };
        for reason in [
            StopReason::ScriptReturned,
            StopReason::Deadline,
            StopReason::CallerLeft,
            StopReason::Shutdown,
            StopReason::Crashed,
        ] {
            let settled = Settlement::decide(false, &pull, Ending::Stopped(reason));
            assert_eq!(
                (settled.outcome, settled.delivery, settled.result),
                (
                    Outcome::Cancelled,
                    Delivery::NotSent,
                    Settled::Retired(reason)
                ),
                "{reason:?}: the run's end is how a pull ends, so nothing failed"
            );
        }
        for (name, ending) in [("panicked", Ending::Panicked), ("lost", Ending::Lost)] {
            let settled = Settlement::decide(false, &pull, ending);
            assert!(
                matches!(settled.result, Settled::Failed(_)),
                "{name}: Neo lost the pull, which is no normal end"
            );
        }
    }
}
