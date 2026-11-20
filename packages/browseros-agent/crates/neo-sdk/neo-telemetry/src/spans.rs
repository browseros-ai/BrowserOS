//! The two spans that leave Neo, `neo.run` and `neo.op`, with the attributes and status
//! wire-contract.md pins ("Spans").
//!
//! Built only when a tracer provider exists (trace push on, or the test-support capture): with
//! push off no span is ever started. No bridge from the `tracing` crate: only what this module
//! starts is exported. Parents are passed explicitly and never read from a thread-local context,
//! so an op that settles on another task, or after its run ended, still lands under its own run:
//!
//! ```text
//!  caller's traceparent (or none: a new trace)
//!   └─ neo.run   started at admission, ended with the run's verdict
//!       └─ neo.op   built when the op settles: start = OpSettled::started, end = now
//! ```
//!
//! An internal run (the janitor's sweep, the host's private typing) has no `neo.run`, so its ops'
//! spans are roots. Attributes are set only on a span the sampler kept, so an unsampled run
//! formats and allocates nothing.

use std::time::{Duration, SystemTime};

use opentelemetry::trace::{
    Span as _, SpanBuilder, SpanContext, Status, TraceContextExt as _, Tracer as _,
};
use opentelemetry::{Context, Key, KeyValue};
use opentelemetry_sdk::trace::{SdkTracer, Span};

use crate::label::{ErrorLabel, OpOutcome, RunStatus};
use crate::privacy::Privacy;
use crate::{OpSettled, RunFacts, RunVerdict, SiteDetail};

const RUN: &str = "neo.run";
const OP: &str = "neo.op";

const RUN_ID: Key = Key::from_static_str("neo.run.id");
const TASK_ID: Key = Key::from_static_str("neo.task.id");
const RUN_STATUS: Key = Key::from_static_str("neo.run.status");
const OP_NAME: Key = Key::from_static_str("neo.op.name");
const OP_ID: Key = Key::from_static_str("neo.op.id");
const SITE_LINE: Key = Key::from_static_str("neo.op.site.line");
const SITE_COLUMN: Key = Key::from_static_str("neo.op.site.column");
const TAB_ID: Key = Key::from_static_str("neo.tab.id");
const OP_OUTCOME: Key = Key::from_static_str("neo.op.outcome");
const OP_DELIVERY: Key = Key::from_static_str("neo.op.delivery");
const ERROR_OWNER: Key = Key::from_static_str("neo.error.owner");
const ERROR_CODE: Key = Key::from_static_str("neo.error.code");
const URL_DOMAIN: Key = Key::from_static_str("url.domain");
const URL_FULL: Key = Key::from_static_str("url.full");

/// How much of a script-chosen op name a span keeps: as much as the door's own refusal keeps
/// (`NAME_CHARS` in the server's dispatcher), so an invented name cannot inflate every span.
const NAME_CHARS: usize = 128;

/// Starts and ends Neo's spans on one tracer, applying the privacy dial to `neo.op`.
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
        let mut span = self
            .tracer
            .build_with_context(SpanBuilder::from_name(RUN), &parent);
        if span.is_recording() {
            span.set_attribute(KeyValue::new(RUN_ID, facts.run_id.to_string()));
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
                span.set_status(Status::error(description(
                    verdict.error,
                    verdict.status.as_str(),
                )));
            }
        }
        span.end();
    }

    /// Records one settled op's `neo.op`, from `took` ago until now: a child of its run's span
    /// (`run`), or a root for an op of an internal run. Status `ERROR` unless it ended ok, as for
    /// the run.
    pub(crate) fn op_settled(&self, run: Option<&SpanContext>, op: &OpSettled<'_>, took: Duration) {
        let end = SystemTime::now();
        let start = end.checked_sub(took).unwrap_or(end);
        let parent = match run {
            Some(run) => Context::new().with_remote_span_context(run.clone()),
            None => Context::new(),
        };
        let mut span = self
            .tracer
            .build_with_context(SpanBuilder::from_name(OP).with_start_time(start), &parent);
        if span.is_recording() {
            span.set_attributes([
                KeyValue::new(OP_NAME, clip(op.raw_name.unwrap_or(op.op)).to_owned()),
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
            let failed = match op.outcome {
                OpOutcome::Ok => false,
                OpOutcome::Error | OpOutcome::Cancelled => true,
            };
            if failed {
                span.set_status(Status::error(description(op.error, op.outcome.as_str())));
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

/// A failed span's status text: its error code when it has one, else how it ended
/// (`cancelled`, `timeout`).
fn description(error: Option<ErrorLabel>, ended: &'static str) -> &'static str {
    error.map_or(ended, |error| error.code.as_str())
}

/// `name`, cut to [`NAME_CHARS`] characters.
fn clip(name: &str) -> &str {
    name.char_indices()
        .nth(NAME_CHARS)
        .and_then(|(end, _)| name.get(..end))
        .unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::{ErrorCode, ErrorOwner};

    #[test]
    fn a_long_invented_name_is_cut_on_a_character_boundary() {
        assert_eq!(clip("page.goto"), "page.goto", "a real name is kept whole");
        let long = "é".repeat(NAME_CHARS + 5);
        let kept = clip(&long);
        assert_eq!(
            kept.chars().count(),
            NAME_CHARS,
            "an invented name keeps as many characters as the door's refusal does"
        );
        assert!(
            long.starts_with(kept),
            "the cut keeps the start of the name"
        );
    }

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
