//! What the run service reports to telemetry, read back from the recording double beside the
//! trace store: each run starts once with its caller's trace and ends once with the verdict
//! `/v1/runs` records (ok, error, cancelled, timeout, a panic), in-flight comes back to zero,
//! each op reports once under its run with the values of its record, and a refused call is
//! counted without becoming a run.

use std::sync::Arc;
use std::time::Duration;

use neo_telemetry::label::{self, ErrorLabel};
use neo_telemetry::{
    OpSite, Recording, RunFacts, RunVerdict, Telemetry, TelemetryCall, TraceContext, Vocabulary,
};
use neo_trace::{Delivery, Outcome, RunId, RunStatus, RunSummary};
use serde_json::json;
use tokio::time::Instant;

use super::{GRACE, ReplyStatus, RunMeta, RunTimeout, caller, meta, request, until_status};
use crate::browser::TabsScope;
use crate::dispatch::{
    Call, CallSite, Caller, ErrorCode, OperationError, RunCtx, RunGuard, RunStart,
};
use crate::test_rig::{TestError, TestRig, on_neo_runtime};

/// A W3C `traceparent` a caller could send.
const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

/// The recording handle a test's service reports to, labelled with the rig's own tables as
/// startup labels Neo's: its method registry and Neo's error codes.
fn telemetry(rig: &TestRig) -> (Telemetry, Recording) {
    let (telemetry, recording) = Telemetry::recording();
    telemetry.bind_vocabulary(Vocabulary::new(
        rig.dispatcher.registry().names(),
        ErrorCode::all().map(ErrorCode::as_str),
    ));
    (telemetry, recording)
}

/// How a run ended in the wire names `/v1/runs` serves and the run metrics label with. A plain
/// data bag.
#[derive(Debug, PartialEq, Eq)]
struct WireEnd {
    status: String,
    /// The failure's owner and code.
    error: Option<(String, String)>,
}

impl WireEnd {
    fn new(status: &str, error: Option<(&str, &str)>) -> Self {
        Self {
            status: status.to_owned(),
            error: error.map(|(owner, code)| (owner.to_owned(), code.to_owned())),
        }
    }

    fn reported(verdict: &RunVerdict) -> Self {
        Self::new(
            verdict.status.as_str(),
            verdict
                .error
                .map(|error| (error.owner.as_str(), error.code.as_str())),
        )
    }

    fn recorded(run: &RunSummary) -> Self {
        Self::new(
            run.status.as_str(),
            run.error
                .as_ref()
                .map(|error| (error.owner.as_str(), error.code.as_str())),
        )
    }
}

/// Run `id` as the trace store has it, once everything sent so far is committed.
async fn recorded_run(rig: &TestRig, id: RunId) -> Result<RunSummary, TestError> {
    rig.sink.flush().await?;
    Ok(rig
        .store
        .run_summary(id)
        .await?
        .ok_or("the run is on the record")?)
}

/// A run started at the door as the run service starts an agent's run, counted by `telemetry`.
fn counted_run(
    rig: &TestRig,
    telemetry: &Telemetry,
) -> Result<(RunId, Arc<RunCtx>, RunGuard), TestError> {
    let id = RunId::new();
    let (run, guard) = RunCtx::start(
        &rig.dispatcher,
        RunStart {
            id,
            code: "test".to_owned(),
            request_id: None,
            task_id: None,
            host_data: None,
            view: rig.link.view().ok_or("the link is up")?,
            deadline: Instant::now() + Duration::from_secs(30),
            default_page: None,
            tabs_scope: TabsScope::Browser,
            access: None,
            telemetry: telemetry.start_run(RunFacts {
                run_id: &id,
                task_id: None,
                parent: None,
            }),
        },
    );
    Ok((id, run, guard))
}

/// Asserts that `run` reported one start and then one end, the end `/v1/runs` records for it,
/// which is `expected`, and that no run is left in flight.
fn assert_reported_once(
    recording: &Recording,
    run: &RunSummary,
    expected: &WireEnd,
) -> Result<(), TestError> {
    let id = run.id.to_string();
    let calls: Vec<TelemetryCall> = recording
        .calls()
        .into_iter()
        .filter(|call| match call {
            TelemetryCall::RunStarted { run_id, .. }
            | TelemetryCall::RunFinished { run_id, .. }
            | TelemetryCall::RunDropped { run_id } => *run_id == id,
            _ => false,
        })
        .collect();
    let [
        TelemetryCall::RunStarted { .. },
        TelemetryCall::RunFinished { verdict, .. },
    ] = calls.as_slice()
    else {
        return Err(format!("run {id} reports one start, then one finish: {calls:?}").into());
    };
    assert_eq!(
        WireEnd::reported(verdict),
        WireEnd::recorded(run),
        "run {id}: telemetry's end is the one /v1/runs records"
    );
    assert_eq!(&WireEnd::recorded(run), expected, "run {id}: how it ended");
    assert_eq!(
        recording.runs_in_flight(),
        0,
        "run {id}: in-flight comes back to zero on every path"
    );
    Ok(())
}

#[test]
fn a_run_starts_with_its_callers_trace_and_ends_ok_once() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, recording) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry);
        let parent =
            TraceContext::from_w3c(TRACEPARENT, Some("vendor=1")).ok_or("a valid traceparent")?;
        let reply = service
            .run(request(
                "return 1;",
                RunMeta {
                    trace: Some(parent.clone()),
                    ..meta(None, Some("task-traced"))?
                },
            )?)
            .await;
        assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
        let calls = recording.calls();
        assert_eq!(
            calls.first(),
            Some(&TelemetryCall::RunStarted {
                run_id: reply.id.to_string(),
                task_id: Some("task-traced".to_owned()),
                parent: Some(parent),
            }),
            "the run starts under the id /v1/runs knows it by, with its task and the caller's \
             trace for its span to join"
        );
        let run = recorded_run(&rig, reply.id).await?;
        assert_reported_once(&recording, &run, &WireEnd::new("ok", None))
    })
}

#[test]
fn a_run_that_throws_ends_with_the_scripts_error() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, recording) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry);
        let reply = service
            .run(request("throw new Error('boom');", meta(None, None)?)?)
            .await;
        assert_eq!(reply.status, ReplyStatus::Error, "{reply:?}");
        let run = recorded_run(&rig, reply.id).await?;
        assert_reported_once(
            &recording,
            &run,
            &WireEnd::new("error", Some(("script", "script_error"))),
        )
    })
}

#[test]
fn a_run_whose_caller_left_is_in_flight_until_it_ends_cancelled() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, recording) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry);
        let code = "await new Promise((resolve) => setTimeout(resolve, 600000));";
        let leaving = caller(&service, request(code, meta(None, Some("task-left"))?)?);
        until_status(&rig, "task-left", RunStatus::Running).await?;
        assert_eq!(
            recording.runs_in_flight(),
            1,
            "a running run is in flight from before its start record"
        );
        leaving.abort();
        let run = until_status(&rig, "task-left", RunStatus::Cancelled).await?;
        assert_reported_once(
            &recording,
            &run,
            &WireEnd::new("cancelled", Some(("host", "cancelled"))),
        )
    })
}

#[test]
fn a_run_past_its_deadline_ends_timed_out() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, recording) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry);
        let mut short = request("await new Promise(() => {});", meta(None, None)?)?;
        short.timeout = RunTimeout::from_millis(Some(300))?;
        let reply = service.run(short).await;
        assert_eq!(reply.status, ReplyStatus::Timeout, "{reply:?}");
        let run = recorded_run(&rig, reply.id).await?;
        assert_reported_once(
            &recording,
            &run,
            &WireEnd::new("timeout", Some(("neo", "run_timeout"))),
        )
    })
}

#[test]
fn a_run_whose_task_panicked_ends_once_as_neos_failure() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, recording) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry);
        service.crash_next_run();
        let crashed = service
            .run(request("return 1;", meta(Some("call-crash"), None)?)?)
            .await;
        assert_eq!(crashed.status, ReplyStatus::Error, "{crashed:?}");
        let run = recorded_run(&rig, crashed.id).await?;
        assert_reported_once(
            &recording,
            &run,
            &WireEnd::new("error", Some(("neo", "internal_error"))),
        )?;
        let next = service
            .run(request("return 2;", meta(Some("call-after"), None)?)?)
            .await;
        let run = recorded_run(&rig, next.id).await?;
        assert_reported_once(&recording, &run, &WireEnd::new("ok", None))
    })
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one script's ops are checked field by field against their stored records"
)]
fn each_op_reports_once_under_its_run_as_its_record_says() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, recording) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry);
        let code = [
            "await __neo.invoke('test.echo', { hello: 1 });",
            "try { await __neo.invoke('test.fail', {}); } catch (error) {}",
            "try { await __neo.invoke('no.such.method', {}); } catch (error) {}",
            "await __neo.invoke('test.touch', { page });",
            "__neo.invoke('test.hang', {});",
            "await new Promise((resolve) => setTimeout(resolve, 50));",
            "return 1;",
        ]
        .join("\n");
        let reply = service.run(request(&code, meta(None, None)?)?).await;
        assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
        rig.sink.flush().await?;
        let detail = rig
            .store
            .run(reply.id)
            .await?
            .ok_or("the run is on the record")?;
        let run_id = reply.id.to_string();
        let calls = recording.calls();
        let finished = calls
            .iter()
            .position(|call| {
                matches!(call, TelemetryCall::RunFinished { run_id: id, .. } if *id == run_id)
            })
            .ok_or("the run finished")?;
        let reports = calls
            .iter()
            .filter(|call| matches!(call, TelemetryCall::OpSettled { .. }))
            .count();
        assert_eq!(
            reports,
            detail.operations.len(),
            "one report per op on the record, and none beside them"
        );
        for row in &detail.operations {
            let op = row.id.to_string();
            let reported: Vec<(usize, &TelemetryCall)> = calls
                .iter()
                .enumerate()
                .filter(|(_, call)| {
                    matches!(call, TelemetryCall::OpSettled { op_id, .. } if *op_id == op)
                })
                .collect();
            let [
                (
                    at,
                    TelemetryCall::OpSettled {
                        run_id: under,
                        op: spec_name,
                        raw_name,
                        op_label,
                        site,
                        tab_id,
                        outcome,
                        delivery,
                        error,
                        url,
                        ..
                    },
                ),
            ] = reported.as_slice()
            else {
                return Err(format!("op {op} reports exactly once: {reported:?}").into());
            };
            assert_eq!(
                under.as_deref(),
                Some(run_id.as_str()),
                "{op}: reported under its own run"
            );
            assert!(*at < finished, "{op}: settled before its run ended");
            assert_eq!(
                raw_name.as_deref().unwrap_or(spec_name),
                row.name,
                "{op}: named as the script wrote it, as its record names it"
            );
            assert_eq!(
                *op_label,
                if row.name == "no.such.method" {
                    Vocabulary::UNKNOWN
                } else {
                    row.name.as_str()
                },
                "{op}: a registered op labels as itself, anything else as unknown"
            );
            assert_eq!(
                *site,
                row.site.map(|site| OpSite {
                    line: site.line,
                    column: site.column,
                }),
                "{op}: the record's call site"
            );
            assert_eq!(*tab_id, row.tab.map(|tab| tab.0), "{op}: the record's tab");
            assert_eq!(
                Some(outcome.as_str()),
                row.outcome.map(Outcome::as_str),
                "{op}: the record's outcome"
            );
            assert_eq!(
                Some(delivery.as_str()),
                row.delivery.map(Delivery::as_str),
                "{op}: the record's delivery"
            );
            assert_eq!(
                error.map(|error| (error.owner.as_str(), error.code.as_str())),
                row.error
                    .as_ref()
                    .map(|error| (error.owner.as_str(), error.code.as_str())),
                "{op}: the record's error owner and code"
            );
            assert_eq!(
                url.as_deref(),
                row.page.url.as_deref(),
                "{op}: the URL the record keeps, read by no call of telemetry's"
            );
        }
        let named = |name: &str| {
            calls.iter().find(|call| {
                matches!(call, TelemetryCall::OpSettled { op, raw_name, .. }
                    if raw_name.as_deref().unwrap_or(op) == name)
            })
        };
        let Some(TelemetryCall::OpSettled {
            op,
            raw_name,
            op_label,
            error,
            ..
        }) = named("no.such.method")
        else {
            return Err("the unknown call reports".into());
        };
        assert_eq!(
            (*op, raw_name.as_deref(), *op_label),
            ("unknown", Some("no.such.method"), "unknown"),
            "an unregistered name reaches telemetry raw, for the span only; its label is unknown"
        );
        assert_eq!(
            *error,
            Some(ErrorLabel {
                owner: label::ErrorOwner::Script,
                code: label::ErrorCode::new("method_unavailable"),
            }),
            "calling a method Neo does not have is the script's failure"
        );
        let Some(TelemetryCall::OpSettled { site, error, .. }) = named("test.fail") else {
            return Err("the failing call reports".into());
        };
        assert_eq!(
            (site.map(|site| site.line), *error),
            (
                Some(2),
                Some(ErrorLabel {
                    owner: label::ErrorOwner::Website,
                    code: label::ErrorCode::new("cdp_error"),
                })
            ),
            "the failure the page gave, at the line that called it"
        );
        let Some(TelemetryCall::OpSettled { tab_id, url, .. }) = named("test.touch") else {
            return Err("the call on the lazy page reports".into());
        };
        assert!(
            tab_id.is_some() && url.is_some(),
            "an op on the run's lazy page reports the tab the door opened for it, and the URL \
             its page facts already held: {tab_id:?} {url:?}"
        );
        let Some(TelemetryCall::OpSettled {
            outcome,
            delivery,
            error,
            ..
        }) = named("test.hang")
        else {
            return Err("the call the script left behind reports".into());
        };
        assert_eq!(
            (outcome.as_str(), delivery.as_str(), *error),
            (
                "cancelled",
                "unknown",
                Some(ErrorLabel {
                    owner: label::ErrorOwner::Script,
                    code: label::ErrorCode::new("not_awaited"),
                })
            ),
            "a sent write the script did not await is cut short, its effect unknown"
        );
        Ok(())
    })
}

/// A run whose task died with an op still running: the guard ends the run once, as the trace
/// does, and the op it aborted settles on its own task, before or after that end, still under
/// the run.
#[test]
fn an_op_cut_by_a_crash_still_reports_under_its_run() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, recording) = telemetry(&rig);
        let (id, run, guard) = counted_run(&rig, &telemetry)?;
        let pending = rig.dispatcher.submit(
            &run,
            Call {
                name: "test.hang".to_owned(),
                input: json!({}),
                caller: Caller::Script,
                site: Some(CallSite { line: 3, column: 7 }),
                abort: None,
            },
        );
        assert_eq!(recording.runs_in_flight(), 1, "the run is in flight");
        drop(guard);
        let reply = pending.wait().await;
        let failure = reply.outcome.err().ok_or("the cut op failed")?;
        assert_eq!(
            failure.error,
            OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                "the run stopped on an internal error"
            ),
            "the crash owns the op it cut"
        );
        let crashed = ErrorLabel {
            owner: label::ErrorOwner::Neo,
            code: label::ErrorCode::new("internal_error"),
        };
        let calls = recording.calls();
        let finished: Vec<&RunVerdict> = calls
            .iter()
            .filter_map(|call| match call {
                TelemetryCall::RunFinished {
                    run_id, verdict, ..
                } if *run_id == id.to_string() => Some(verdict),
                _ => None,
            })
            .collect();
        assert_eq!(
            finished,
            vec![&RunVerdict {
                status: label::RunStatus::Error,
                error: Some(crashed),
            }],
            "the guard's drop ends the run once, with the crash the trace records"
        );
        let reported: Vec<&TelemetryCall> = calls
            .iter()
            .filter(|call| matches!(call, TelemetryCall::OpSettled { .. }))
            .collect();
        let [
            TelemetryCall::OpSettled {
                run_id,
                op,
                op_id,
                site,
                outcome,
                error,
                ..
            },
        ] = reported.as_slice()
        else {
            return Err(format!("the cut op reports once: {reported:?}").into());
        };
        assert_eq!(
            (run_id.clone(), *op, op_id.clone(), *site, *outcome, *error),
            (
                Some(id.to_string()),
                "test.hang",
                format!("{id}.1"),
                Some(OpSite { line: 3, column: 7 }),
                label::OpOutcome::Cancelled,
                Some(crashed),
            ),
            "the aborted op reports under the run it belonged to, cut by the crash"
        );
        assert!(
            !calls
                .iter()
                .any(|call| matches!(call, TelemetryCall::RunDropped { .. })),
            "the guard always finishes the span; its drop backstop never fires"
        );
        assert_eq!(
            recording.runs_in_flight(),
            0,
            "the crash left no run in flight"
        );
        Ok(())
    })
}

#[test]
fn a_refused_call_is_counted_once_and_never_becomes_a_run() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, recording) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry);
        let invalid =
            OperationError::host(ErrorCode::INVALID_META, "taskId must be 1 to 128 bytes");
        let bad_meta = service.refuse("return 1;".to_owned(), meta(None, None)?, &invalid);
        rig.fake.drop_connection();
        let unavailable = service
            .run(request("return 1;", meta(Some("call-down"), None)?)?)
            .await;
        rig.fake.reconnect();
        let expected = [
            (&bad_meta, "host", "invalid_meta"),
            (&unavailable, "neo", "browser_unavailable"),
        ];
        for (reply, owner, code) in expected {
            assert_eq!(
                WireEnd::recorded(&recorded_run(&rig, reply.id).await?),
                WireEnd::new("error", Some((owner, code))),
                "the refusal is on the record as a failed run"
            );
        }
        let counted: Vec<(&str, &str)> = recording
            .calls()
            .iter()
            .map(|call| match call {
                TelemetryCall::RunRefused { error } => {
                    Ok((error.owner.as_str(), error.code.as_str()))
                }
                other => Err(format!("a refusal starts no run and no op: {other:?}")),
            })
            .collect::<Result<_, _>>()?;
        assert_eq!(
            counted,
            vec![("host", "invalid_meta"), ("neo", "browser_unavailable")],
            "each refusal is counted once, with the error its record carries"
        );
        assert_eq!(
            recording.runs_in_flight(),
            0,
            "a refusal is never in flight"
        );
        Ok(())
    })
}
