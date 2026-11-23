//! What the run service reports to telemetry, read from the live pipeline
//! (`Telemetry::in_memory`) as a collector and the host API's scrape would read it, beside the
//! trace store: each run ends once as one `neo.run` span carrying the end `/v1/runs` records
//! (ok, error, cancelled, timeout, a panic), counted in the run metrics with in-flight back at
//! zero; each op is one span under its run, named after it, with the values of its record,
//! counted under its registered name or `unknown`; each failure counts once; a refused call is
//! counted without becoming a span; and the site-detail dial decides what of a page's address a
//! span keeps.

use std::sync::Arc;
use std::time::Duration;

use neo_telemetry::{
    CapturedSpan, Probe, RunFacts, SiteDetail, SpanStatus, SpanValue, Telemetry, TelemetryConfig,
    TraceContext, Vocabulary,
};
use neo_trace::{Delivery, Outcome, RunId, RunStatus, RunSummary, WriterConfig};
use serde_json::json;
use tokio::time::Instant;

use super::{GRACE, ReplyStatus, RunMeta, RunTimeout, caller, meta, request, until_status};
use crate::browser::TabsScope;
use crate::dispatch::{
    Call, CallSite, Caller, ErrorCode, OperationError, RunCtx, RunGuard, RunStart,
};
use crate::telemetry_rig::Scrape;
use crate::test_rig::{TestError, TestRig, on_neo_runtime};

/// A W3C `traceparent` a caller could send.
const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

/// The run metrics a test compares whole, as `name{labels} value` lines.
const RUN_METRICS: &[&str] = &[
    "neo_runs_total",
    "neo_run_duration_seconds_count",
    "neo_errors_total",
];

/// The live pipeline a test's service reports to, with no URL detail on spans, labelled with
/// the rig's own tables as startup labels Neo's: its method registry and Neo's error codes.
fn telemetry(rig: &TestRig) -> (Telemetry, Probe) {
    telemetry_with(rig, SiteDetail::None)
}

/// As [`telemetry`], with spans keeping `detail` of each page's address.
fn telemetry_with(rig: &TestRig, site_detail: SiteDetail) -> (Telemetry, Probe) {
    let (telemetry, probe) = Telemetry::in_memory(&TelemetryConfig {
        site_detail,
        ..TelemetryConfig::default()
    });
    telemetry.bind_vocabulary(Vocabulary::new(
        rig.dispatcher.registry().names(),
        ErrorCode::all().map(ErrorCode::as_str),
    ));
    (telemetry, probe)
}

/// The string attribute `key` of `span`.
fn text<'a>(span: &'a CapturedSpan, key: &str) -> Option<&'a str> {
    match span.attributes.get(key) {
        Some(SpanValue::Str(value)) => Some(value),
        _ => None,
    }
}

/// The integer attribute `key` of `span`.
fn int(span: &CapturedSpan, key: &str) -> Option<i64> {
    match span.attributes.get(key) {
        Some(SpanValue::Int(value)) => Some(*value),
        _ => None,
    }
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

    /// The end a `neo.run` span carries.
    fn exported(span: &CapturedSpan) -> Self {
        let error = text(span, "neo.error.owner").zip(text(span, "neo.error.code"));
        Self::new(text(span, "neo.run.status").unwrap_or_default(), error)
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

/// The one `neo.run` span of run `id`.
fn run_span(probe: &Probe, id: RunId) -> Result<CapturedSpan, TestError> {
    let id = id.to_string();
    let spans: Vec<CapturedSpan> = probe
        .spans()
        .into_iter()
        .filter(|span| span.name == "neo.run" && text(span, "neo.run.id") == Some(id.as_str()))
        .collect();
    match <[CapturedSpan; 1]>::try_from(spans) {
        Ok([span]) => Ok(span),
        Err(spans) => Err(format!("run {id} ends as exactly one neo.run span: {spans:#?}").into()),
    }
}

/// The op spans whose parent is `run`, told from other spans by `neo.span.type`, as a reader
/// tells them.
fn op_spans(probe: &Probe, run: &CapturedSpan) -> Vec<CapturedSpan> {
    probe
        .spans()
        .into_iter()
        .filter(|span| {
            text(span, "neo.span.type") == Some("op")
                && span.parent_span_id.as_ref() == Some(&run.span_id)
        })
        .collect()
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

/// Asserts that `run` ended as one `neo.run` span carrying the end `/v1/runs` records for it,
/// which is `expected`, with an error status exactly when it did not end ok, and that no run is
/// left in flight. Returns the span.
fn assert_ended_once(
    telemetry: &Telemetry,
    probe: &Probe,
    run: &RunSummary,
    expected: &WireEnd,
) -> Result<CapturedSpan, TestError> {
    let span = run_span(probe, run.id)?;
    assert_eq!(
        WireEnd::exported(&span),
        WireEnd::recorded(run),
        "run {}: the span's end is the one /v1/runs records",
        run.id
    );
    assert_eq!(
        &WireEnd::recorded(run),
        expected,
        "run {}: how it ended",
        run.id
    );
    assert_eq!(
        matches!(span.status, SpanStatus::Error { .. }),
        expected.status != "ok",
        "run {}: the span's status is ERROR exactly when the run did not end ok: {:?}",
        run.id,
        span.status
    );
    assert_eq!(
        Scrape::of(telemetry)?.whole("neo_run_active", &[]),
        Some(0),
        "run {}: in-flight comes back to zero on every path",
        run.id
    );
    Ok(span)
}

#[test]
fn a_run_joins_its_callers_trace_and_ends_ok_once() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, probe) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry.clone());
        let parent =
            TraceContext::from_w3c(TRACEPARENT, Some("vendor=1")).ok_or("a valid traceparent")?;
        let reply = service
            .run(request(
                "await __neo.invoke('test.echo', {}); return 1;",
                RunMeta {
                    trace: Some(parent),
                    ..meta(None, Some("task-traced"))?
                },
            )?)
            .await;
        assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
        let run = recorded_run(&rig, reply.id).await?;
        let span = assert_ended_once(&telemetry, &probe, &run, &WireEnd::new("ok", None))?;
        assert_eq!(
            (
                span.trace_id.as_str(),
                span.parent_span_id.as_deref(),
                span.parent_is_remote
            ),
            (
                "4bf92f3577b34da6a3ce929d0e0e4736",
                Some("00f067aa0ba902b7"),
                true
            ),
            "the run's span joins the caller's trace under the caller's span"
        );
        assert_eq!(
            text(&span, "neo.task.id"),
            Some("task-traced"),
            "the span names its task"
        );
        let ops = op_spans(&probe, &span);
        assert!(
            ops.len() == 1 && ops.iter().all(|op| op.trace_id == span.trace_id),
            "its op is a child of the run in the caller's trace: {ops:#?}"
        );
        assert_eq!(
            Scrape::of(&telemetry)?.lines_of(RUN_METRICS),
            [
                r#"neo_run_duration_seconds_count{neo_run_status="ok"} 1"#,
                r#"neo_runs_total{neo_error_owner="none",neo_run_status="ok"} 1"#,
            ],
            "one ok run, one duration, no error"
        );
        Ok(())
    })
}

#[test]
fn a_run_that_throws_ends_with_the_scripts_error_counted_once() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, probe) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry.clone());
        let reply = service
            .run(request("throw new Error('boom');", meta(None, None)?)?)
            .await;
        assert_eq!(reply.status, ReplyStatus::Error, "{reply:?}");
        let run = recorded_run(&rig, reply.id).await?;
        let span = assert_ended_once(
            &telemetry,
            &probe,
            &run,
            &WireEnd::new("error", Some(("script", "script_error"))),
        )?;
        assert!(
            span.parent_span_id.is_none(),
            "with no caller trace the run starts a trace of its own"
        );
        assert_eq!(
            Scrape::of(&telemetry)?.lines_of(RUN_METRICS),
            [
                r#"neo_errors_total{error_type="script_error",neo_error_owner="script"} 1"#,
                r#"neo_run_duration_seconds_count{neo_run_status="error"} 1"#,
                r#"neo_runs_total{neo_error_owner="script",neo_run_status="error"} 1"#,
            ],
            "the failed run and its one error, which no op had reported"
        );
        Ok(())
    })
}

#[test]
fn a_run_whose_caller_left_is_in_flight_until_it_ends_cancelled() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, probe) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry.clone());
        let code = "await new Promise((resolve) => setTimeout(resolve, 600000));";
        let leaving = caller(&service, request(code, meta(None, Some("task-left"))?)?);
        until_status(&rig, "task-left", RunStatus::Running).await?;
        assert_eq!(
            Scrape::of(&telemetry)?.whole("neo_run_active", &[]),
            Some(1),
            "a running run is in flight"
        );
        leaving.abort();
        let run = until_status(&rig, "task-left", RunStatus::Cancelled).await?;
        assert_ended_once(
            &telemetry,
            &probe,
            &run,
            &WireEnd::new("cancelled", Some(("host", "cancelled"))),
        )?;
        Ok(())
    })
}

#[test]
fn a_run_past_its_deadline_ends_timed_out() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, probe) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry.clone());
        let mut short = request("await new Promise(() => {});", meta(None, None)?)?;
        short.timeout = RunTimeout::from_millis(Some(300))?;
        let reply = service.run(short).await;
        assert_eq!(reply.status, ReplyStatus::Timeout, "{reply:?}");
        let run = recorded_run(&rig, reply.id).await?;
        assert_ended_once(
            &telemetry,
            &probe,
            &run,
            &WireEnd::new("timeout", Some(("neo", "run_timeout"))),
        )?;
        Ok(())
    })
}

#[test]
fn a_run_whose_task_panicked_ends_once_as_neos_failure() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, probe) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry.clone());
        service.crash_next_run();
        let crashed = service
            .run(request("return 1;", meta(Some("call-crash"), None)?)?)
            .await;
        assert_eq!(crashed.status, ReplyStatus::Error, "{crashed:?}");
        let run = recorded_run(&rig, crashed.id).await?;
        assert_ended_once(
            &telemetry,
            &probe,
            &run,
            &WireEnd::new("error", Some(("neo", "internal_error"))),
        )?;
        let next = service
            .run(request("return 2;", meta(Some("call-after"), None)?)?)
            .await;
        let run = recorded_run(&rig, next.id).await?;
        assert_ended_once(&telemetry, &probe, &run, &WireEnd::new("ok", None))?;
        assert_eq!(
            Scrape::of(&telemetry)?.lines_of(&["neo_runs_total"]),
            [
                r#"neo_runs_total{neo_error_owner="neo",neo_run_status="error"} 1"#,
                r#"neo_runs_total{neo_error_owner="none",neo_run_status="ok"} 1"#,
            ],
            "the panic is counted as Neo's failure, and the next run as itself"
        );
        Ok(())
    })
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one script's ops are checked field by field against their stored records"
)]
fn each_op_is_one_span_under_its_run_as_its_record_says() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, probe) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry.clone());
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
        let run = run_span(&probe, reply.id)?;
        let ops = op_spans(&probe, &run);
        let registered: Vec<&str> = rig.dispatcher.registry().names().collect();
        assert_eq!(
            ops.len(),
            detail.operations.len(),
            "one span per op on the record, each a child of its run: {ops:#?}"
        );
        assert!(
            ops.iter()
                .all(|op| op.end <= run.end && op.trace_id == run.trace_id),
            "every op settled before its run ended, in the run's trace"
        );
        for row in &detail.operations {
            let op = row.id.to_string();
            let spans: Vec<&CapturedSpan> = ops
                .iter()
                .filter(|span| text(span, "neo.op.id") == Some(op.as_str()))
                .collect();
            let [span] = spans.as_slice() else {
                return Err(format!("op {op} is exactly one span: {spans:#?}").into());
            };
            assert_eq!(
                text(span, "neo.op.name"),
                Some(row.name.as_str()),
                "{op}: named as the script wrote it, as its record names it"
            );
            let label = if registered.contains(&row.name.as_str()) {
                row.name.as_str()
            } else {
                "unknown"
            };
            assert_eq!(
                span.name, label,
                "{op}: the span name is the op's metric label, so a script's text never names one"
            );
            assert_eq!(
                (
                    int(span, "code.line.number"),
                    int(span, "code.column.number")
                ),
                (
                    row.site.map(|site| i64::from(site.line)),
                    row.site.map(|site| i64::from(site.column))
                ),
                "{op}: the record's call site"
            );
            assert_eq!(
                int(span, "neo.tab.id"),
                row.tab.map(|tab| tab.0),
                "{op}: the record's tab"
            );
            assert_eq!(
                text(span, "neo.op.outcome"),
                row.outcome.map(Outcome::as_str),
                "{op}: the record's outcome"
            );
            assert_eq!(
                text(span, "neo.op.delivery"),
                row.delivery.map(Delivery::as_str),
                "{op}: the record's delivery"
            );
            assert_eq!(
                text(span, "neo.error.owner").zip(text(span, "neo.error.code")),
                row.error
                    .as_ref()
                    .map(|error| (error.owner.as_str(), error.code.as_str())),
                "{op}: the record's error owner and code"
            );
            assert!(
                !span.attributes.contains_key("url.domain")
                    && !span.attributes.contains_key("url.full"),
                "{op}: with site detail off a span carries no address"
            );
        }
        let touched = ops
            .iter()
            .find(|span| text(span, "neo.op.name") == Some("test.touch"))
            .ok_or("the call on the lazy page is a span")?;
        assert!(
            int(touched, "neo.tab.id").is_some(),
            "an op on the run's lazy page carries the tab the door opened for it"
        );
        assert_eq!(
            Scrape::of(&telemetry)?.lines_of(&["neo_operations_total", "neo_errors_total"]),
            [
                r#"neo_errors_total{error_type="cdp_error",neo_error_owner="website"} 1"#,
                r#"neo_errors_total{error_type="method_unavailable",neo_error_owner="script"} 1"#,
                r#"neo_operations_total{neo_error_owner="none",neo_op_name="context.newPage",neo_op_outcome="ok"} 1"#,
                r#"neo_operations_total{neo_error_owner="none",neo_op_name="test.echo",neo_op_outcome="ok"} 1"#,
                r#"neo_operations_total{neo_error_owner="none",neo_op_name="test.touch",neo_op_outcome="ok"} 1"#,
                r#"neo_operations_total{neo_error_owner="script",neo_op_name="test.hang",neo_op_outcome="cancelled"} 1"#,
                r#"neo_operations_total{neo_error_owner="script",neo_op_name="unknown",neo_op_outcome="error"} 1"#,
                r#"neo_operations_total{neo_error_owner="website",neo_op_name="test.fail",neo_op_outcome="error"} 1"#,
            ],
            "registered ops count under their names (the lazy page's newPage among them), the \
             unregistered one as unknown (its span keeps the raw name), each failed op's error \
             counts once, and the write the run cut short (cancelled, not_awaited) counts as an \
             op but in no error total: the run that cut it succeeded"
        );
        Ok(())
    })
}

/// Neo's housekeeping (here the rig's background pull, as the event pump and the frame-tree
/// reads are) keeps its span under the run, and leaves every op metric to the script's own ops.
#[test]
fn housekeeping_ops_keep_their_spans_and_leave_the_op_metrics() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, probe) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry.clone());
        let code = [
            "__neo.invoke('test.pull', {});",
            "await __neo.invoke('test.echo', { hello: 1 });",
            "return 1;",
        ]
        .join("\n");
        let reply = service.run(request(&code, meta(None, None)?)?).await;
        assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
        rig.sink.flush().await?;
        let run = run_span(&probe, reply.id)?;
        let mut names: Vec<Option<&str>> = Vec::new();
        let ops = op_spans(&probe, &run);
        for op in &ops {
            names.push(text(op, "neo.op.name"));
        }
        names.sort_unstable();
        assert_eq!(
            names,
            [Some("test.echo"), Some("test.pull")],
            "the pull is still a span under its run, so the trace shows Neo's own work"
        );
        assert_eq!(
            Scrape::of(&telemetry)?.lines_of(&[
                "neo_operations_total",
                "neo_operation_duration_seconds_count",
                "neo_errors_total",
            ]),
            [
                r#"neo_operation_duration_seconds_count{neo_op_name="test.echo",neo_op_outcome="ok"} 1"#,
                r#"neo_operations_total{neo_error_owner="none",neo_op_name="test.echo",neo_op_outcome="ok"} 1"#,
            ],
            "only the script's op counts: housekeeping grows with open tabs, not with its work"
        );
        Ok(())
    })
}

/// A run stopped at its deadline with two ops in flight: teardown cuts both, and each settles
/// cancelled with the stop's own error before the run ends, yet the timeout is one failure, the
/// run's (wire-contract.md; review F1).
#[test]
fn a_timeout_with_ops_in_flight_counts_one_error() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, probe) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry.clone());
        let code =
            "await Promise.all([__neo.invoke('test.hang', {}), __neo.invoke('test.hang', {})]);";
        let mut short = request(code, meta(None, None)?)?;
        short.timeout = RunTimeout::from_millis(Some(300))?;
        let reply = service.run(short).await;
        assert_eq!(reply.status, ReplyStatus::Timeout, "{reply:?}");
        let run = recorded_run(&rig, reply.id).await?;
        let span = assert_ended_once(
            &telemetry,
            &probe,
            &run,
            &WireEnd::new("timeout", Some(("neo", "run_timeout"))),
        )?;
        let ops = op_spans(&probe, &span);
        let cut: Vec<Option<&str>> = ops.iter().map(|op| text(op, "neo.op.outcome")).collect();
        assert_eq!(cut, [Some("cancelled"); 2], "both ops were cut by the stop");
        assert_eq!(
            Scrape::of(&telemetry)?.lines_of(&["neo_errors_total", "neo_operations_total"]),
            [
                r#"neo_errors_total{error_type="run_timeout",neo_error_owner="neo"} 1"#,
                r#"neo_operations_total{neo_error_owner="neo",neo_op_name="test.hang",neo_op_outcome="cancelled"} 2"#,
            ],
            "two cut ops, one timeout"
        );
        Ok(())
    })
}

/// A run whose task died with an op still running: the guard ends the run once, as the trace
/// does, and the op it aborted settles on its own task, before or after that end, still a child
/// of the run, its failure (the run's own) counted once.
#[test]
fn an_op_cut_by_a_crash_still_reports_under_its_run() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, probe) = telemetry(&rig);
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
        assert_eq!(
            Scrape::of(&telemetry)?.whole("neo_run_active", &[]),
            Some(1),
            "the run is in flight"
        );
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
        let span = run_span(&probe, id)?;
        assert_eq!(
            WireEnd::exported(&span),
            WireEnd::new("error", Some(("neo", "internal_error"))),
            "the guard's drop ends the run once, with the crash the trace records"
        );
        let ops = op_spans(&probe, &span);
        let [op] = ops.as_slice() else {
            return Err(format!("the cut op is one span under the run: {ops:#?}").into());
        };
        assert_eq!(
            (
                text(op, "neo.op.name"),
                text(op, "neo.op.id"),
                int(op, "code.line.number"),
                text(op, "neo.op.outcome"),
                text(op, "neo.error.code"),
            ),
            (
                Some("test.hang"),
                Some(format!("{id}.1").as_str()),
                Some(3),
                Some("cancelled"),
                Some("internal_error"),
            ),
            "the aborted op reports under the run it belonged to, cut by the crash"
        );
        let scrape = Scrape::of(&telemetry)?;
        assert_eq!(
            scrape.lines_of(&["neo_errors_total"]),
            [r#"neo_errors_total{error_type="internal_error",neo_error_owner="neo"} 1"#],
            "the crash is one failure, whether the run or its cut op reported it first"
        );
        assert_eq!(
            scrape.whole("neo_run_active", &[]),
            Some(0),
            "the crash left no run in flight"
        );
        Ok(())
    })
}

#[test]
fn a_refused_call_is_counted_once_and_never_becomes_a_span() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (telemetry, probe) = telemetry(&rig);
        let service = rig.run_service_reporting(GRACE, telemetry.clone());
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
        assert_eq!(probe.spans(), [], "a refusal starts no span");
        let scrape = Scrape::of(&telemetry)?;
        assert_eq!(
            scrape.lines_of(RUN_METRICS),
            [
                r#"neo_errors_total{error_type="browser_unavailable",neo_error_owner="neo"} 1"#,
                r#"neo_errors_total{error_type="invalid_meta",neo_error_owner="host"} 1"#,
                r#"neo_runs_total{neo_error_owner="host",neo_run_status="error"} 1"#,
                r#"neo_runs_total{neo_error_owner="neo",neo_run_status="error"} 1"#,
            ],
            "each refusal is a failed run and one error, with the error its record carries, and \
             no duration"
        );
        assert_eq!(
            scrape.whole("neo_run_active", &[]),
            Some(0),
            "a refusal is never in flight"
        );
        Ok(())
    })
}

/// A repeat of a `requestId` whose run ended before the replay window is refused as a
/// duplicate: counted like every refusal, as `/v1/runs` lists it, and never a span.
#[test]
fn a_refused_duplicate_is_counted_as_a_failed_run_with_no_span() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::with_writer(WriterConfig {
            replay_window: Duration::ZERO,
            ..WriterConfig::default()
        })
        .await?;
        let first = rig
            .run_service(GRACE)
            .run(request("return 1;", meta(Some("call-dup"), None)?)?)
            .await;
        assert_eq!(first.status, ReplyStatus::Ok, "{first:?}");
        rig.sink.flush().await?;
        // A service of its own: an empty in-memory ledger, so the repeat is decided by the store.
        let (telemetry, probe) = telemetry(&rig);
        let repeat = rig
            .run_service_reporting(GRACE, telemetry.clone())
            .run(request("return 1;", meta(Some("call-dup"), None)?)?)
            .await;
        assert_eq!(
            WireEnd::recorded(&recorded_run(&rig, repeat.id).await?),
            WireEnd::new("error", Some(("host", "duplicate_request"))),
            "the duplicate is on the record as a failed run"
        );
        assert_eq!(probe.spans(), [], "a duplicate starts no span");
        let scrape = Scrape::of(&telemetry)?;
        assert_eq!(
            scrape.lines_of(RUN_METRICS),
            [
                r#"neo_errors_total{error_type="duplicate_request",neo_error_owner="host"} 1"#,
                r#"neo_runs_total{neo_error_owner="host",neo_run_status="error"} 1"#,
            ],
            "the duplicate is counted once, as the host's failure, with no duration"
        );
        assert_eq!(scrape.whole("neo_run_active", &[]), Some(0));
        Ok(())
    })
}

/// `CLAW_TELEMETRY_SITE_DETAIL`, end to end through a real op on an owner's tab: `none` keeps
/// no address, `host` the domain, `url` the address without its userinfo, query or fragment;
/// no mode lets a credential or a query value into a span.
#[test]
fn the_site_detail_dial_decides_what_of_the_page_address_an_op_span_keeps() -> Result<(), TestError>
{
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let page = rig
            .owner_tab("https://user:pw@shop.example/cart?token=s3cr3t#frag")
            .await?;
        let code = format!(
            "await __neo.invoke('test.touch', {{ page: {} }}); return 1;",
            json!({ "$neo": "page", "tab": page.tab.0 })
        );
        let modes = [
            (SiteDetail::None, None, None),
            (SiteDetail::Host, Some("shop.example"), None),
            (
                SiteDetail::Url,
                Some("shop.example"),
                Some("https://shop.example/cart"),
            ),
        ];
        for (detail, domain, full) in modes {
            let (telemetry, probe) = telemetry_with(&rig, detail);
            let reply = rig
                .run_service_reporting(GRACE, telemetry)
                .run(request(&code, meta(None, None)?)?)
                .await;
            assert_eq!(reply.status, ReplyStatus::Ok, "{detail:?}: {reply:?}");
            let run = run_span(&probe, reply.id)?;
            let ops = op_spans(&probe, &run);
            let op = ops
                .iter()
                .find(|span| text(span, "neo.op.name") == Some("test.touch"))
                .ok_or_else(|| format!("{detail:?}: the touch is an op span: {ops:#?}"))?;
            assert_eq!(
                (text(op, "url.domain"), text(op, "url.full")),
                (domain, full),
                "{detail:?}: what of the address the span keeps"
            );
            let leaked: Vec<String> = probe
                .spans()
                .iter()
                .flat_map(|span| span.attributes.values())
                .map(|value| format!("{value:?}"))
                .filter(|value| value.contains("s3cr3t") || value.contains("pw@"))
                .collect();
            assert_eq!(
                leaked,
                Vec::<String>::new(),
                "{detail:?}: nothing secret leaves"
            );
        }
        Ok(())
    })
}
