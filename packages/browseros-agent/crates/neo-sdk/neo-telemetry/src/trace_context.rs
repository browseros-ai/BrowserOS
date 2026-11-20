//! A caller's W3C trace context, which a run's `neo.run` span continues.

use std::collections::HashMap;

use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::{SpanContext, TraceContextExt};
use opentelemetry_sdk::propagation::TraceContextPropagator;

/// The trace a caller asked a run to join: a parsed W3C `traceparent` and its `tracestate`.
/// The MCP boundary builds it from the request's HTTP headers or from the run's `_meta`, and
/// the run's root span takes it as its parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceContext {
    span_context: SpanContext,
}

impl TraceContext {
    /// Parses a `traceparent` header value (`00-<32 hex>-<16 hex>-<2 hex>`) and an optional
    /// `tracestate`. Anything malformed, including an all-zero id, is `None` and never an
    /// error: a bad header must not fail the run, which then starts a trace of its own.
    #[must_use]
    pub fn from_w3c(traceparent: &str, tracestate: Option<&str>) -> Option<Self> {
        let mut carrier = HashMap::with_capacity(2);
        carrier.insert("traceparent".to_owned(), traceparent.to_owned());
        if let Some(tracestate) = tracestate {
            carrier.insert("tracestate".to_owned(), tracestate.to_owned());
        }
        let context = TraceContextPropagator::new().extract(&carrier);
        let span_context = context.span().span_context().clone();
        span_context.is_valid().then_some(Self { span_context })
    }

    /// THE precedence rule for a run's parent, in one place: a header pair whose `traceparent`
    /// parses wins; else the `_meta` pair; else `None`. A pair is `(traceparent, tracestate)`,
    /// and a `tracestate` is only ever taken with its own `traceparent`. Pure: a malformed value
    /// is skipped, never an error.
    #[must_use]
    pub fn resolve(
        header: Option<(&str, Option<&str>)>,
        meta: Option<(&str, Option<&str>)>,
    ) -> Option<Self> {
        let parse = |(traceparent, tracestate)| Self::from_w3c(traceparent, tracestate);
        header.and_then(parse).or_else(|| meta.and_then(parse))
    }

    /// The caller's span, as the root span's parent. Crate-private: the facade never hands out
    /// OpenTelemetry types.
    #[must_use]
    pub(crate) fn span_context(&self) -> &SpanContext {
        &self.span_context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn a_valid_traceparent_parses_with_its_tracestate() {
        let context = TraceContext::from_w3c(PARENT, Some("vendor=value"));
        let Some(context) = context else {
            panic!("a well-formed traceparent must parse");
        };
        assert_eq!(
            context.span_context.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736",
            "the trace id is the caller's"
        );
        assert_eq!(
            context.span_context.trace_state().header(),
            "vendor=value",
            "the tracestate rides along"
        );
        assert!(
            context.span_context.is_remote(),
            "the parent is the caller's, not ours"
        );
    }

    #[test]
    fn a_valid_header_wins_then_meta_then_none() {
        const META: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        let header = TraceContext::from_w3c(PARENT, Some("h=1"));
        let meta = TraceContext::from_w3c(META, Some("m=1"));
        assert_eq!(
            TraceContext::resolve(Some((PARENT, Some("h=1"))), Some((META, Some("m=1")))),
            header,
            "the HTTP header wins over run metadata"
        );
        assert_eq!(
            TraceContext::resolve(Some(("garbage", Some("h=1"))), Some((META, Some("m=1")))),
            meta,
            "a malformed header falls back to the metadata pair, with the metadata's own tracestate"
        );
        assert_eq!(
            TraceContext::resolve(None, Some((META, Some("m=1")))),
            meta,
            "no header: the metadata pair"
        );
        assert_eq!(
            TraceContext::resolve(Some(("garbage", None)), Some(("also garbage", None))),
            None,
            "nothing usable: the run starts its own trace"
        );
        assert_eq!(
            TraceContext::resolve(None, None),
            None,
            "nothing given: a new trace"
        );
    }

    #[test]
    fn malformed_input_is_none_never_an_error() {
        for bad in [
            "",
            "garbage",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BA902B7-01x",
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        ] {
            assert_eq!(
                TraceContext::from_w3c(bad, None),
                None,
                "`{bad}` is not a usable W3C traceparent, so the run starts its own trace"
            );
        }
    }
}
