//! The spans that leave Neo, with the attributes and status wire-contract.md pins ("Spans"): one
//! `neo.run` per counted run, and one span per op named after the op.
//!
//! Built only when a tracer provider exists (trace push on, or the test-support capture): with
//! push off no span is ever started. No bridge from the `tracing` crate: only what this module
//! starts is exported. Parents are passed explicitly and never read from a thread-local context,
//! so an op that settles on another task, or after its run ended, still lands under its own run:
//!
//! ```text
//!  caller's traceparent (or none: a new trace)
//!   └─ neo.run       SERVER: started at admission, ended with the run's verdict
//!       └─ page.goto   INTERNAL: built when the op settles: start = OpSettled::started, end = now
//! ```
//!
//! An op span's name is the op's metric label: a method Neo serves, or `unknown`, so the set of
//! span names stays as small as Neo's method table while `neo.op.name` keeps what the script
//! wrote. Both kinds carry `neo.span.type` (`run` or `op`), which is how a reader tells them
//! apart now that op names vary. A run is Neo serving a caller's call, which waits for its reply,
//! so `neo.run` is a `SERVER` span: a trace backend then draws the caller's edge to Neo, and the
//! caller's own client span pairs with it. An op is in-process work under it: `INTERNAL`. An internal run (the janitor's sweep, the host's private typing)
//! has no `neo.run`, so its ops' spans are roots. Attributes are set only on a span the sampler
//! kept, so an unsampled run formats and allocates nothing.

use std::time::{Duration, SystemTime};

use opentelemetry::trace::{
    Span as _, SpanBuilder, SpanContext, SpanKind, Status, TraceContextExt as _, Tracer as _,
};
use opentelemetry::{Context, Key, KeyValue};
use opentelemetry_sdk::trace::{SdkTracer, Span};

use crate::label::{ErrorLabel, OpOutcome, RunStatus};
use crate::privacy::Privacy;
use crate::{OpSettled, RunFacts, RunVerdict, SiteDetail};

const RUN: &str = "neo.run";

const SPAN_TYPE: Key = Key::from_static_str("neo.span.type");

const RUN_ID: Key = Key::from_static_str("neo.run.id");
const TASK_ID: Key = Key::from_static_str("neo.task.id");
const RUN_STATUS: Key = Key::from_static_str("neo.run.status");
const OP_NAME: Key = Key::from_static_str("neo.op.name");
const OP_ID: Key = Key::from_static_str("neo.op.id");
// The registry's keys for a source position: the op's call site in the script. The script is
// no file, so there is no `code.file.path` beside them.
const SITE_LINE: Key = Key::from_static_str("code.line.number");
const SITE_COLUMN: Key = Key::from_static_str("code.column.number");
const TAB_ID: Key = Key::from_static_str("neo.tab.id");
const OP_OUTCOME: Key = Key::from_static_str("neo.op.outcome");
const OP_DELIVERY: Key = Key::from_static_str("neo.op.delivery");
const ERROR_OWNER: Key = Key::from_static_str("neo.error.owner");
const ERROR_CODE: Key = Key::from_static_str("neo.error.code");
const ERROR_TYPE: Key = Key::from_static_str("error.type");
const URL_DOMAIN: Key = Key::from_static_str("url.domain");
const URL_FULL: Key = Key::from_static_str("url.full");

/// Starts and ends Neo's spans on one tracer, applying the privacy dial to op spans.
#[derive(Debug)]
pub(crate) struct Spans {
    tracer: SdkTracer,
    privacy: Privacy,
}

impl Spans {
    pub(crate) fn new(tracer: SdkTracer, detail: SiteDetail) -> Self {
        Self {
            tracer,
            privacy: Privacy::new(detail),
        }
    }

    /// Starts a counted run's `neo.run`: a child of the caller's span when it sent a trace
    /// context, else the root of a new trace. Its span context is its ops' parent.
    pub(crate) fn run_started(&self, facts: &RunFacts<'_>) -> Span {
        let parent = match facts.parent {
            Some(parent) => Context::new().with_remote_span_context(parent.span_context().clone()),
            None => Context::new(),
        };
        let mut span = self.tracer.build_with_context(
            SpanBuilder::from_name(RUN).with_kind(SpanKind::Server),
            &parent,
        );
        if span.is_recording() {
            span.set_attributes([
                KeyValue::new(SPAN_TYPE, "run"),
                KeyValue::new(RUN_ID, facts.run_id.to_string()),
            ]);
            if let Some(task_id) = facts.task_id {
                span.set_attribute(KeyValue::new(TASK_ID, task_id.to_owned()));
            }
        }
        span
    }

    /// Ends a run's `neo.run` with the verdict its record got: `neo.run.status`, the error's
    /// owner and code when it failed, and status `ERROR` unless it ended ok.
    pub(crate) fn run_ended(mut span: Span, verdict: &RunVerdict) {
        if span.is_recording() {
            span.set_attribute(KeyValue::new(RUN_STATUS, verdict.status.as_str()));
            set_error(&mut span, verdict.error);
            let failed = match verdict.status {
                RunStatus::Ok => false,
                RunStatus::Error | RunStatus::Cancelled | RunStatus::Timeout => true,
            };
            if failed {
                set_failed(
                    &mut span,
                    description(verdict.error, verdict.status.as_str()),
                );
            }
        }
        span.end();
    }

    /// Records one settled op's span, named `name` (its op label), from `took` ago until now: a
    /// child of its run's span (`run`), or a root for an op of an internal run. Status `ERROR`
    /// only when the op failed on its own (outcome `error`), not when its run's end cut it short.
    pub(crate) fn op_settled(
        &self,
        run: Option<&SpanContext>,
        name: &'static str,
        op: &OpSettled<'_>,
        took: Duration,
    ) {
        let end = SystemTime::now();
        let start = end.checked_sub(took).unwrap_or(end);
        let parent = match run {
            Some(run) => Context::new().with_remote_span_context(run.clone()),
            None => Context::new(),
        };
        let mut span = self.tracer.build_with_context(
            SpanBuilder::from_name(name)
                .with_kind(SpanKind::Internal)
                .with_start_time(start),
            &parent,
        );
        if span.is_recording() {
            span.set_attributes([
                KeyValue::new(SPAN_TYPE, "op"),
                KeyValue::new(OP_NAME, op.raw_name.unwrap_or(op.op).to_owned()),
                KeyValue::new(OP_ID, op.op_id.to_string()),
                KeyValue::new(OP_OUTCOME, op.outcome.as_str()),
                KeyValue::new(OP_DELIVERY, op.delivery.as_str()),
            ]);
            if let Some(site) = op.site {
                span.set_attributes([
                    KeyValue::new(SITE_LINE, i64::from(site.line)),
                    KeyValue::new(SITE_COLUMN, i64::from(site.column)),
                ]);
            }
            if let Some(tab_id) = op.tab_id {
                span.set_attribute(KeyValue::new(TAB_ID, tab_id));
            }
            set_error(&mut span, op.error);
            let site = self.privacy.site(op.url);
            if let Some(domain) = site.domain {
                span.set_attribute(KeyValue::new(URL_DOMAIN, domain));
            }
            if let Some(full) = site.full {
                span.set_attribute(KeyValue::new(URL_FULL, full));
            }
            // A cancelled op leaves the status unset: it was cut short by what ended its run, and
            // Neo's own long-poll (`events.next`) ends that way in every run, so an ERROR here
            // would paint every trace red. Its outcome and error attributes still say why.
            let failed = match op.outcome {
                OpOutcome::Error => true,
                OpOutcome::Ok | OpOutcome::Cancelled => false,
            };
            if failed {
                set_failed(&mut span, description(op.error, op.outcome.as_str()));
            }
        }
        span.end_with_timestamp(end);
    }
}

/// `neo.error.owner` and `neo.error.code`, only for a failure.
fn set_error(span: &mut Span, error: Option<ErrorLabel>) {
    if let Some(error) = error {
        span.set_attributes([
            KeyValue::new(ERROR_OWNER, error.owner.as_str()),
            KeyValue::new(ERROR_CODE, error.code.as_str()),
        ]);
    }
}

/// Status `ERROR` with `what` as its text, and the registry's `error.type` with the same value,
/// which semantic conventions ask for on every failed span so a backend can group failures
/// without knowing Neo's own `neo.error.code`.
fn set_failed(span: &mut Span, what: &'static str) {
    span.set_attribute(KeyValue::new(ERROR_TYPE, what));
    span.set_status(Status::error(what));
}

/// A failed span's status text and `error.type`: its error code when it has one, else how it
/// ended (a run's `cancelled` or `timeout`, an op's `error`).
fn description(error: Option<ErrorLabel>, ended: &'static str) -> &'static str {
    error.map_or(ended, |error| error.code.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::{ErrorCode, ErrorOwner};

    #[test]
    fn a_failed_span_says_its_code_or_else_how_it_ended() {
        let failure = ErrorLabel {
            owner: ErrorOwner::Script,
            code: ErrorCode::new("run_timeout"),
        };
        assert_eq!(
            description(Some(failure), "timeout"),
            "run_timeout",
            "the code names the failure"
        );
        assert_eq!(
            description(None, "cancelled"),
            "cancelled",
            "with no error recorded, the status says how it ended"
        );
    }
}
