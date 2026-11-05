//! What the host needs to register one op: plain data, read once at startup.

use browseros_trace::{Effect, JsonPointer, RedactionPolicy};

/// One op's registration metadata. claw-sdk copies it field by field into its `MethodSpec`;
/// the prelude's `ops.generated.ts` carries the same name, effect, target and lifetime.
#[derive(Clone, Copy, Debug)]
pub struct OpSpec {
    /// The wire name the prelude invokes, such as `locator.click`. Unique across all ops.
    /// Playwright aliases (`page.click`, `frame.click`, `elementHandle.click`) share one op and
    /// tell it their own name in the input's `call.api`, which error messages start with.
    pub name: &'static str,
    /// One line for traces and the summarized listing in the `run` tool's description.
    pub doc: &'static str,
    /// `Read` ops never mark themselves sent, so a failure is always `not_sent` (P6).
    pub effect: Effect,
    /// Which input field holds the page handle, for the dispatcher's tab lock and page facts.
    pub target: OpTarget,
    /// `Background` only for the prelude's event pump.
    pub lifetime: OpLifetime,
    /// JSON pointers into the input that the trace redacts once the op marks its target
    /// sensitive (a password, card or one-time-code field), such as `/value` for a fill. Each
    /// names a top-level field of the op's input and starts with `/`.
    pub sensitive_args: &'static [&'static str],
}

/// Where the page an op acts on sits in its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpTarget {
    /// The op acts on no page (`context.pages`, the event pump).
    None,
    /// This top-level input field holds a branded handle: `"page"` holds the page brand
    /// `{"$neo":"page","id":7,"tab":1234}`, and `"target"` holds a locator or element handle
    /// whose `page` field is that brand. The host resolves and checks that page, and hands the
    /// crate its resolved brand there.
    Field(&'static str),
}

/// How long an op may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpLifetime {
    /// Ends before the run does, like every Playwright call.
    Operation,
    /// The event pump: a long poll that the host cancels quietly when the run ends.
    Background,
}

/// The page field of ops that act on a page handle.
pub(crate) const PAGE: OpTarget = OpTarget::Field("page");
/// The target field of ops that act on a locator or an element handle.
pub(crate) const TARGET: OpTarget = OpTarget::Field("target");

impl OpSpec {
    /// The trace's redaction policy: the sensitive arguments, redacted only once the op marks
    /// its target sensitive. No argument of a Playwright op is always secret.
    #[must_use]
    pub fn redaction(&self) -> RedactionPolicy {
        RedactionPolicy {
            secret: Vec::new(),
            // A unit test parses every declared pointer, so none is dropped here.
            sensitive: self
                .sensitive_args
                .iter()
                .filter_map(|pointer| JsonPointer::parse(pointer).ok())
                .collect(),
        }
    }

    /// An op that only reads.
    pub(crate) const fn read(name: &'static str, target: OpTarget, doc: &'static str) -> Self {
        Self {
            name,
            doc,
            effect: Effect::Read,
            target,
            lifetime: OpLifetime::Operation,
            sensitive_args: &[],
        }
    }

    /// An op that may change the page (or, for `download.saveAs`, the host's files).
    pub(crate) const fn write(name: &'static str, target: OpTarget, doc: &'static str) -> Self {
        Self {
            effect: Effect::Write,
            ..Self::read(name, target, doc)
        }
    }

    /// The same op, with arguments the trace redacts for a sensitive target.
    pub(crate) const fn sensitive(self, sensitive_args: &'static [&'static str]) -> Self {
        Self {
            sensitive_args,
            ..self
        }
    }

    /// The same op, as the background event pump.
    pub(crate) const fn background(self) -> Self {
        Self {
            lifetime: OpLifetime::Background,
            ..self
        }
    }
}
