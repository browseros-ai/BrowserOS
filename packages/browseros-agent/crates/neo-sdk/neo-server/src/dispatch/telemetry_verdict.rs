//! Telemetry's labels for what the door settled, read from the values the trace records.
//!
//! The one place where `neo_trace`'s vocabularies (run status, op outcome, delivery, error owner)
//! and Neo's error codes become `neo_telemetry::label`s. The run's guard reports its end through
//! [`telemetry_verdict`], the op's settle reports through the `op_*` mappings below, and both
//! take the very values their `/v1/runs` records carry, so a span or a metric cannot disagree with
//! the trace. Every match is exhaustive (STYLE.md P8): a new variant fails the build here instead
//! of reaching a metric under a guessed label.

use neo_telemetry::label::{self, ErrorLabel, OpOutcome};
use neo_telemetry::{OpCounting, OpSite, RunVerdict};
use neo_trace::{Delivery, ErrorOwner, ErrorRecord, OperationKind, Outcome, PageState, RunStatus};

use super::operation_guard::Settled;
use super::{CallSite, ErrorCode, OpMetrics, OperationError, RunEnd};

/// A run's end as its telemetry records it (`RunSpan::finish`): the status and the failure its
/// `run_ended` record carries. `RunGuard` reports both of its ends through it, and a call refused
/// before it ran is counted with the failure this reads from its record.
#[must_use]
pub(crate) fn telemetry_verdict(end: &RunEnd) -> RunVerdict {
    RunVerdict {
        status: run_status(end.status),
        error: end.error.as_ref().map(recorded_failure),
    }
}

/// How an op ended, as its record says it.
#[must_use]
pub(super) fn op_outcome(outcome: Outcome) -> OpOutcome {
    match outcome {
        Outcome::Ok => OpOutcome::Ok,
        Outcome::Error => OpOutcome::Error,
        Outcome::Cancelled => OpOutcome::Cancelled,
    }
}

/// Whether an op's side effect happened, as its record says it.
#[must_use]
pub(super) fn op_delivery(delivery: Delivery) -> label::Delivery {
    match delivery {
        Delivery::NotSent => label::Delivery::NotSent,
        Delivery::Done => label::Delivery::Done,
        Delivery::Unknown => label::Delivery::Unknown,
    }
}

/// An op's failure, exactly when its record carries one: a retired background pull ended
/// without failing.
#[must_use]
pub(super) fn op_failure(result: &Settled) -> Option<ErrorLabel> {
    match result {
        Settled::Failed(error) => Some(failure(error)),
        Settled::Value(_) | Settled::Retired(_) => None,
    }
}

/// Whether an op counts on the op metrics: always, unless its method is Neo's housekeeping and
/// the op carries no script call site (`site`, as its record names it), so Neo sent it on its own.
#[must_use]
pub(super) fn op_counting(metrics: OpMetrics, site: Option<CallSite>) -> OpCounting {
    match (metrics, site) {
        (OpMetrics::Counted, _) | (OpMetrics::Housekeeping, Some(_)) => OpCounting::Counted,
        (OpMetrics::Housekeeping, None) => OpCounting::Internal,
    }
}

/// Where the script called an op, as its record names it.
#[must_use]
pub(super) fn op_site(site: CallSite) -> OpSite {
    OpSite {
        line: site.line,
        column: site.column,
    }
}

/// The URL of an op's page after it, only as its record keeps it: read from the facts the door
/// already gathered, never with a browser call of its own, and never for the host's private
/// typing (protected-fill ruling R13), whose record keeps no address. The span's privacy dial
/// decides how much of it leaves the process.
#[must_use]
pub(super) fn op_url(kind: OperationKind, page_after: Option<&PageState>) -> Option<&str> {
    match kind {
        OperationKind::Protected => None,
        OperationKind::Builtin
        | OperationKind::Defined
        | OperationKind::Next { .. }
        | OperationKind::HostCall => page_after.and_then(|page| page.url.as_deref()),
    }
}

/// The finished half of `/v1/runs`' status. `running` never ends a run, and `interrupted` is
/// written only by the next process for a run this one never ended: neither reaches a guard, and
/// either would be a failure if it did.
fn run_status(status: RunStatus) -> label::RunStatus {
    match status {
        RunStatus::Ok => label::RunStatus::Ok,
        RunStatus::Error | RunStatus::Running | RunStatus::Interrupted => label::RunStatus::Error,
        RunStatus::Cancelled => label::RunStatus::Cancelled,
        RunStatus::Timeout => label::RunStatus::Timeout,
    }
}

/// A failure the door built, as its labels.
fn failure(error: &OperationError) -> ErrorLabel {
    ErrorLabel {
        owner: owner(error.owner()),
        code: label::ErrorCode::new(error.code().as_str()),
    }
}

/// A recorded failure, as its labels. Its code is a string by then; every one a run records
/// came from Neo's own list, so the lookup finds it, and anything else would be Neo's fault.
fn recorded_failure(record: &ErrorRecord) -> ErrorLabel {
    let code = ErrorCode::known(&record.code).unwrap_or(ErrorCode::INTERNAL_ERROR);
    ErrorLabel {
        owner: owner(record.owner),
        code: label::ErrorCode::new(code.as_str()),
    }
}

fn owner(owner: ErrorOwner) -> label::ErrorOwner {
    match owner {
        ErrorOwner::Website => label::ErrorOwner::Website,
        ErrorOwner::Script => label::ErrorOwner::Script,
        ErrorOwner::Neo => label::ErrorOwner::Neo,
        ErrorOwner::Host => label::ErrorOwner::Host,
    }
}

#[cfg(test)]
mod tests {
    use neo_telemetry::label::{self, ErrorLabel};
    use neo_trace::{
        Delivery, ErrorOwner, ErrorRecord, OperationKind, Outcome, PageState, RunStatus,
    };
    use serde_json::json;

    use neo_telemetry::OpCounting;

    use super::{op_counting, op_delivery, op_failure, op_outcome, op_url, telemetry_verdict};
    use crate::dispatch::operation_guard::Settled;
    use crate::dispatch::{CallSite, ErrorCode, OpMetrics, OperationError, RunEnd, StopReason};

    fn run_end(status: RunStatus, error: Option<ErrorRecord>) -> RunEnd {
        RunEnd {
            status,
            error,
            result: None,
            replay: None,
        }
    }

    fn failure_label(owner: label::ErrorOwner, code: &'static str) -> ErrorLabel {
        ErrorLabel {
            owner,
            code: label::ErrorCode::new(code),
        }
    }

    #[test]
    fn a_runs_verdict_is_its_records_status_owner_and_code() {
        let failed =
            OperationError::script(ErrorCode::NO_MATCH, "nothing matched").record(false, None);
        let cases = [
            (run_end(RunStatus::Ok, None), label::RunStatus::Ok, None),
            (
                run_end(RunStatus::Error, Some(failed)),
                label::RunStatus::Error,
                Some(failure_label(label::ErrorOwner::Script, "no_match")),
            ),
            (
                run_end(
                    RunStatus::Cancelled,
                    Some(StopReason::CallerLeft.error().record(true, None)),
                ),
                label::RunStatus::Cancelled,
                Some(failure_label(label::ErrorOwner::Host, "cancelled")),
            ),
            (
                run_end(
                    RunStatus::Timeout,
                    Some(StopReason::Deadline.error().record(false, None)),
                ),
                label::RunStatus::Timeout,
                Some(failure_label(label::ErrorOwner::Neo, "run_timeout")),
            ),
        ];
        for (end, status, error) in cases {
            let verdict = telemetry_verdict(&end);
            assert_eq!(
                (verdict.status, verdict.error),
                (status, error),
                "{:?}: telemetry reads the run's end exactly as /v1/runs records it",
                end.status
            );
        }
    }

    #[test]
    fn a_status_no_finished_run_has_still_ends_as_a_failure() {
        for status in [RunStatus::Running, RunStatus::Interrupted] {
            assert_eq!(
                telemetry_verdict(&run_end(status, None)).status,
                label::RunStatus::Error,
                "{status:?} never ends a run, so a guard that saw one would be Neo's failure"
            );
        }
    }

    #[test]
    fn a_recorded_code_outside_neos_list_is_neos_internal_error() {
        let record = ErrorRecord {
            code: "made_up".to_owned(),
            ..OperationError::website(ErrorCode::CDP_ERROR, "the page said no").record(true, None)
        };
        assert_eq!(
            telemetry_verdict(&run_end(RunStatus::Error, Some(record))).error,
            Some(failure_label(label::ErrorOwner::Website, "internal_error")),
            "a code is a label only if it is Neo's own, so a stray string never becomes a series"
        );
    }

    #[test]
    fn an_ops_outcome_and_delivery_keep_their_record_names() {
        for outcome in [Outcome::Ok, Outcome::Error, Outcome::Cancelled] {
            assert_eq!(
                op_outcome(outcome).as_str(),
                outcome.as_str(),
                "one spelling on the metric, the span and /v1/runs"
            );
        }
        for delivery in [Delivery::NotSent, Delivery::Done, Delivery::Unknown] {
            assert_eq!(
                op_delivery(delivery).as_str(),
                delivery.as_str(),
                "one spelling on the span and /v1/runs"
            );
        }
        for owner in [
            ErrorOwner::Website,
            ErrorOwner::Script,
            ErrorOwner::Neo,
            ErrorOwner::Host,
        ] {
            let error = ErrorRecord {
                owner,
                ..OperationError::neo(ErrorCode::CDP_ERROR, "failed").record(true, None)
            };
            let verdict = telemetry_verdict(&run_end(RunStatus::Error, Some(error)));
            assert_eq!(
                verdict.error.map(|error| error.owner.as_str()),
                Some(owner.as_str()),
                "the owner keeps its record's spelling"
            );
        }
    }

    #[test]
    fn only_housekeeping_neo_sent_on_its_own_leaves_the_op_metrics() {
        let site = Some(CallSite { line: 3, column: 7 });
        let cases = [
            (OpMetrics::Counted, site, OpCounting::Counted),
            (OpMetrics::Counted, None, OpCounting::Counted),
            (OpMetrics::Housekeeping, site, OpCounting::Counted),
            (OpMetrics::Housekeeping, None, OpCounting::Internal),
        ];
        for (metrics, site, counting) in cases {
            assert_eq!(
                op_counting(metrics, site),
                counting,
                "{metrics:?} with site {site:?}: a script's own `page.on` listener counts, the \
                 prelude's unsited bookkeeping does not"
            );
        }
    }

    #[test]
    fn only_a_failed_op_carries_a_failure() {
        assert_eq!(
            op_failure(&Settled::Failed(OperationError::website(
                ErrorCode::CDP_ERROR,
                "the page said no"
            ))),
            Some(failure_label(label::ErrorOwner::Website, "cdp_error")),
            "a failed op's record carries its error"
        );
        assert_eq!(
            op_failure(&Settled::Value(json!(1))),
            None,
            "a value is no failure"
        );
        assert_eq!(
            op_failure(&Settled::Retired(StopReason::Deadline)),
            None,
            "the run's end is how a pull ends, and its record carries no error"
        );
    }

    #[test]
    fn private_typing_never_reports_a_url() {
        let page = PageState {
            tab_id: None,
            url: Some("https://example.com/form?typed=secret".to_owned()),
            title: None,
        };
        assert_eq!(
            op_url(OperationKind::Protected, Some(&page)),
            None,
            "a protected op's record keeps no address, so neither does its span"
        );
        for kind in [
            OperationKind::Builtin,
            OperationKind::Defined,
            OperationKind::Next { attempt: 1 },
            OperationKind::HostCall,
        ] {
            assert_eq!(
                op_url(kind, Some(&page)),
                page.url.as_deref(),
                "{kind:?}: the URL the door already read, as its record keeps it"
            );
            assert_eq!(
                op_url(kind, None),
                None,
                "no page fact, no URL: telemetry never reads the browser"
            );
        }
    }
}
