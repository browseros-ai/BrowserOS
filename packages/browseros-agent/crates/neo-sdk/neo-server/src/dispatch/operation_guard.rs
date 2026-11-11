//! How an op ends, decided once.

use std::sync::Arc;
use std::thread;

use browseros_core::TabId;
use neo_trace::{
    Delivery, Effect, EpochMillis, OperationEnded, OperationId, OperationKind, OperationStarted,
    Outcome, PageState, TraceSink,
};
use serde_json::Value;
use tokio::sync::oneshot;

use super::op_ctx::OpState;
use super::{
    Answer, CallSite, ErrorCode, LeafError, Lifetime, MethodSpec, Observation, OpFailure, OpLine,
    OpReply, OperationError, PageFact, RunCtx, StopReason,
};
use crate::browser::PageRef;

/// Ends one op exactly once: the trace's `op_ended`, the run's ledger, and the caller's reply.
/// The record and the reply carry the same outcome, as the method produced it.
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
    /// The operation's recorded tab, even when its fact is withheld at release.
    tab: Option<TabId>,
    /// The start record while it waits for the tab of the run's lazy page ([`StartTab::Lazy`]).
    /// Whatever ends the op first sends it before the end record.
    unsent: Option<Unsent>,
    spec: MethodSpec,
    site: Option<CallSite>,
    run: Arc<RunCtx>,
    trace: TraceSink,
    reply: oneshot::Sender<OpReply>,
}

/// What opening an op records. A plain data bag.
pub(super) struct OpStart {
    pub(super) state: Arc<OpState>,
    /// The name as the caller wrote it, so an unknown name is on the record as typed. The trace
    /// writer clips it.
    pub(super) name: String,
    pub(super) spec: MethodSpec,
    /// The raw input. The trace writer bounds it at the operation's end.
    pub(super) args: Value,
    /// The tab the call named, before the door checks it.
    pub(super) tab: StartTab,
    pub(super) site: Option<CallSite>,
    pub(super) reply: oneshot::Sender<OpReply>,
}

/// An op's start record as the door opened it, before its tab is known: a lazy page delays the
/// send ([`Unsent::sent`]).
struct Unsent {
    id: OperationId,
    parent: Option<OperationId>,
    name: String,
    effect: Effect,
    /// The raw input. The trace writer bounds it at the operation's end.
    args: Value,
    /// When the door opened the op, however late its record is sent.
    at: EpochMillis,
}

/// The tab an op's start record names.
#[derive(Clone, Copy, Debug)]
pub(super) enum StartTab {
    /// The tab the call named, or `None` for a method with no page: recorded at once.
    Named(Option<TabId>),
    /// The run's lazy page, which has no tab until the door opens it. The start is recorded
    /// once the door resolved the page ([`OperationGuard::started_on`]), so the op is on its
    /// tab's record before any rule runs: a lock's settle counts it as that tab's open write,
    /// and the trace's screenshot and `?tab=` listings find it.
    Lazy,
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
    /// too (skeleton D7 rule 3). A channel send; it never waits on disk. An op on the run's lazy
    /// page holds its start until [`OperationGuard::started_on`], still before the rules.
    pub(super) fn start(trace: &TraceSink, run: &Arc<RunCtx>, start: OpStart) -> Self {
        let OpStart {
            state,
            name,
            spec,
            args,
            tab,
            site,
            reply,
        } = start;
        let opened = Unsent {
            id: state.id(),
            parent: state.parent_id(),
            name,
            effect: spec.effect,
            args,
            at: EpochMillis::now(),
        };
        let unsent = match tab {
            StartTab::Named(tab) => {
                trace.op_started(opened.sent(tab));
                None
            }
            StartTab::Lazy => Some(opened),
        };
        Self {
            open: Some(Open {
                state,
                tab: match tab {
                    StartTab::Named(tab) => tab,
                    StartTab::Lazy => None,
                },
                unsent,
                spec,
                site,
                run: Arc::clone(run),
                trace: trace.clone(),
                reply,
            }),
        }
    }

    /// Records a held start ([`StartTab::Lazy`]) on the tab the door resolved. Called before the
    /// rules run; a no-op for a start already recorded.
    pub(super) fn started_on(&mut self, tab: Option<TabId>) {
        if let Some(open) = &mut self.open
            && let Some(unsent) = open.unsent.take()
        {
            open.tab = tab;
            open.trace.op_started(unsent.sent(tab));
        }
    }

    /// Settles the op and answers its caller.
    pub(super) fn finish(
        mut self,
        ending: Ending,
        pages: Vec<PageFact>,
        default_page: Option<PageRef>,
        observed: Option<Observation>,
    ) {
        if let Some(open) = self.open.take() {
            settle(open, ending, pages, default_page, observed);
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
            settle(open, ending, Vec::new(), None, None);
        }
    }
}

/// Shared by `finish` and `Drop`, so it is sync: channel sends and std locks only.
fn settle(
    open: Open,
    ending: Ending,
    pages: Vec<PageFact>,
    default_page: Option<PageRef>,
    observed: Option<Observation>,
) {
    let Open {
        state,
        tab,
        unsent,
        spec,
        site,
        run,
        trace,
        reply,
    } = open;
    // An op that ended before the door resolved its page (a gate, a stop, a page that would not
    // open) still has every op id on the record: its start goes first, with no tab.
    if let Some(unsent) = unsent {
        trace.op_started(unsent.sent(None));
    }
    let id = state.id();
    let sent_at = state.sent_at();
    let settlement = Settlement::decide(sent_at.is_some(), &spec, ending);
    for note in state.take_notes() {
        tracing::debug!(operation = %id, method = spec.name, %note, "op note");
    }
    let (result, error) = match &settlement.result {
        Settled::Value(value) => (Some(value.clone()), None),
        Settled::Failed(error) => (None, Some(error.record(settlement.retryable, Some(id)))),
        Settled::Retired(_) => (None, None),
    };
    let page_after = recorded_page_state(&pages, tab);
    trace.op_ended(OperationEnded {
        id,
        outcome: settlement.outcome,
        delivery: settlement.delivery,
        sent_at,
        result,
        error,
        page_after,
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
    // A departed caller is normal during teardown.
    let _ = reply.send(OpReply {
        outcome: outcome.map_err(|error| OpFailure {
            error,
            delivery,
            retryable,
            operation: id,
        }),
        pages,
        observed,
        default_page,
    });
}

/// A missing target fact must not turn a related page into the operation's trace target.
fn recorded_page_state(pages: &[PageFact], tab: Option<TabId>) -> Option<PageState> {
    let page = match tab {
        Some(tab) => pages.iter().find(|page| page.tab == tab),
        None => pages.first(),
    }?;
    Some(page.page_state())
}

impl Unsent {
    /// The record as it is sent now, on `tab`.
    fn sent(self, tab: Option<TabId>) -> OperationStarted {
        let Self {
            id,
            parent,
            name,
            effect,
            args,
            at,
        } = self;
        OperationStarted {
            id,
            parent,
            name,
            kind: OperationKind::Builtin,
            effect,
            tab,
            args,
            at,
        }
    }
}

#[cfg(test)]
mod tests {
    use neo_trace::{Delivery, Effect, ErrorOwner, Outcome};
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
