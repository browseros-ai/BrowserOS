//! A caller's W3C trace context, which a run's `neo.run` span continues.

use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};

/// The trace a caller asked a run to join: a parsed W3C `traceparent` and its `tracestate`.
/// The MCP boundary builds it from the request's HTTP headers or from the run's `_meta`, and
/// the run's root span takes it as its parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceContext {
    span_context: SpanContext,
}

/// A `traceparent`'s three values, once its text passed every W3C rule.
struct Traceparent {
    trace_id: TraceId,
    parent_id: SpanId,
    flags: TraceFlags,
}

impl TraceContext {
    /// Parses a `traceparent` header value and an optional `tracestate`, strictly per W3C Trace
    /// Context: `version-traceid-parentid-flags` in lowercase hex, a 32-digit trace id and a
    /// 16-digit parent id that are not all zero, and 2-digit flags. Version `ff` is invalid;
    /// version `00` has exactly those four fields, while a later version may carry more after
    /// another `-`. Anything else, whitespace included, is `None` and never an error: a bad header
    /// must not fail the run, which then starts a trace of its own. Of the flags only `sampled`
    /// is kept, as the spec's "other flags" rule asks. A `tracestate` that does not parse is
    /// dropped and the `traceparent` still counts.
    #[must_use]
    pub fn from_w3c(traceparent: &str, tracestate: Option<&str>) -> Option<Self> {
        use std::str::FromStr;

        let Traceparent {
            trace_id,
            parent_id,
            flags,
        } = parse_traceparent(traceparent)?;
        let state = tracestate
            .and_then(|state| TraceState::from_str(state).ok())
            .unwrap_or_default();
        Some(Self {
            span_context: SpanContext::new(trace_id, parent_id, flags, true, state),
        })
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
    #[cfg_attr(
        not(feature = "otel"),
        expect(dead_code, reason = "without `otel` no span is started under it")
    )]
    pub(crate) fn span_context(&self) -> &SpanContext {
        &self.span_context
    }
}

/// The W3C `traceparent` grammar, field by field. Our own check rather than the SDK propagator's:
/// that one reads each id with `from_str_radix`, so a short id such as `abc` passes, and it trims
/// whitespace and forgives a trailing `-`.
fn parse_traceparent(value: &str) -> Option<Traceparent> {
    let mut fields = value.splitn(5, '-');
    let version = fields.next()?;
    let trace_id = fields.next()?;
    let parent_id = fields.next()?;
    let flags = fields.next()?;
    let more = fields.next();
    let version_ok = is_hex(version, 2) && version != "ff" && (version != "00" || more.is_none());
    let ids_ok =
        is_hex(trace_id, 32) && !is_zero(trace_id) && is_hex(parent_id, 16) && !is_zero(parent_id);
    if !(version_ok && ids_ok && is_hex(flags, 2)) {
        return None;
    }
    Some(Traceparent {
        trace_id: TraceId::from_hex(trace_id).ok()?,
        parent_id: SpanId::from_hex(parent_id).ok()?,
        flags: TraceFlags::new(u8::from_str_radix(flags, 16).ok()?) & TraceFlags::SAMPLED,
    })
}

/// `field` is exactly `digits` lowercase hex digits.
fn is_hex(field: &str, digits: usize) -> bool {
    field.len() == digits
        && field
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_zero(field: &str) -> bool {
    field.bytes().all(|byte| byte == b'0')
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const SPAN: &str = "00f067aa0ba902b7";
    const PARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

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
    fn each_break_of_the_w3c_grammar_is_none_never_an_error() {
        let cases = [
            (String::new(), "an empty value"),
            ("garbage".to_owned(), "no fields at all"),
            (format!("00-{TRACE}-{SPAN}"), "three fields"),
            (
                format!("00-{TRACE}-{SPAN}-01-extra"),
                "version 00 with a fifth field",
            ),
            (
                format!("00-{TRACE}-{SPAN}-01-"),
                "version 00 with a trailing dash",
            ),
            (
                format!("ff-{TRACE}-{SPAN}-01"),
                "version ff, which the spec forbids",
            ),
            (format!("0-{TRACE}-{SPAN}-01"), "a one-digit version"),
            (format!("000-{TRACE}-{SPAN}-01"), "a three-digit version"),
            (format!("0g-{TRACE}-{SPAN}-01"), "a version that is not hex"),
            (format!("0A-{TRACE}-{SPAN}-01"), "an uppercase version"),
            (format!("00-abc-{SPAN}-01"), "a short trace id"),
            (
                format!("00-4bf92f3577b34da6a3ce929d0e0e473-{SPAN}-01"),
                "a 31-digit trace id",
            ),
            (format!("00-{TRACE}0-{SPAN}-01"), "a 33-digit trace id"),
            (
                format!("00-{}-{SPAN}-01", "0".repeat(32)),
                "an all-zero trace id",
            ),
            (
                format!("00-{}-{SPAN}-01", TRACE.to_uppercase()),
                "an uppercase trace id",
            ),
            (format!("00-{TRACE}-7-01"), "a short parent id"),
            (
                format!("00-{TRACE}-00f067aa0ba902b-01"),
                "a 15-digit parent id",
            ),
            (format!("00-{TRACE}-{SPAN}7-01"), "a 17-digit parent id"),
            (
                format!("00-{TRACE}-{}-01", "0".repeat(16)),
                "an all-zero parent id",
            ),
            (
                format!("00-{TRACE}-{}-01", SPAN.to_uppercase()),
                "an uppercase parent id",
            ),
            (format!("00-{TRACE}-{SPAN}-1"), "one-digit flags"),
            (format!("00-{TRACE}-{SPAN}-001"), "three-digit flags"),
            (format!("00-{TRACE}-{SPAN}-0x"), "flags that are not hex"),
            (
                format!("00-{TRACE}-{SPAN}-+1"),
                "a sign, which is not a hex digit",
            ),
            (format!("00-{TRACE}-{SPAN}-0A"), "uppercase flags"),
            (format!("00-{TRACE}-{SPAN}-01x"), "text glued to the flags"),
            (format!("00-{TRACE}-{SPAN}-0\u{ff11}"), "a non-ASCII digit"),
            (format!(" 00-{TRACE}-{SPAN}-01"), "leading whitespace"),
            (format!("00-{TRACE}-{SPAN}-01\n"), "trailing whitespace"),
            (format!("00_{TRACE}_{SPAN}_01"), "the wrong separator"),
        ];
        for (traceparent, why) in cases {
            assert_eq!(
                TraceContext::from_w3c(&traceparent, Some("vendor=value")),
                None,
                "`{traceparent}` has {why}, so the run starts its own trace"
            );
        }
    }

    #[test]
    fn a_valid_traceparent_parses_with_or_without_a_tracestate() -> Result<(), String> {
        let bare = TraceContext::from_w3c(PARENT, None).ok_or("a valid traceparent alone")?;
        assert_eq!(
            (
                bare.span_context.trace_id().to_string(),
                bare.span_context.span_id().to_string()
            ),
            (TRACE.to_owned(), SPAN.to_owned()),
            "the trace and the parent span are the caller's"
        );
        assert!(
            bare.span_context.is_remote(),
            "the parent is the caller's, not ours"
        );
        assert!(bare.span_context.is_sampled(), "flags 01: sampled");
        assert_eq!(
            bare.span_context.trace_state().header(),
            "",
            "no tracestate given, none kept"
        );
        let with_state = TraceContext::from_w3c(PARENT, Some("vendor=value,other=1"))
            .ok_or("a valid traceparent with its tracestate")?;
        assert_eq!(
            with_state.span_context.trace_state().header(),
            "vendor=value,other=1",
            "the tracestate rides along"
        );
        let bad_state = TraceContext::from_w3c(PARENT, Some("no list member here"))
            .ok_or("a bad tracestate does not spoil a good traceparent")?;
        assert_eq!(
            (
                bad_state.span_context.trace_id(),
                bad_state.span_context.trace_state().header()
            ),
            (bare.span_context.trace_id(), String::new()),
            "the traceparent still counts; the tracestate that does not parse is dropped"
        );
        Ok(())
    }

    #[test]
    fn flags_keep_only_sampled_and_a_later_version_may_add_fields() -> Result<(), String> {
        let unsampled = TraceContext::from_w3c(&format!("00-{TRACE}-{SPAN}-00"), None)
            .ok_or("flags 00 are valid")?;
        assert!(
            !unsampled.span_context.is_sampled(),
            "flags 00: not sampled"
        );
        let unknown = TraceContext::from_w3c(&format!("00-{TRACE}-{SPAN}-03"), None)
            .ok_or("an unknown flag is accepted")?;
        assert_eq!(
            unknown.span_context.trace_flags(),
            TraceFlags::SAMPLED,
            "flags the spec does not define are dropped, sampled is kept"
        );
        for later in [
            format!("01-{TRACE}-{SPAN}-01"),
            format!("01-{TRACE}-{SPAN}-01-what-the-next-version-adds"),
            format!("fe-{TRACE}-{SPAN}-01-"),
        ] {
            assert!(
                TraceContext::from_w3c(&later, None).is_some(),
                "`{later}`: a later version is read as far as version 00 goes"
            );
        }
        Ok(())
    }
}
