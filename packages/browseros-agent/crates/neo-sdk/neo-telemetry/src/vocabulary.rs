//! Neo's own tables of op names and error codes: the only values the `neo.op.name`
//! and `error.type` metric attributes may take.

use std::collections::HashSet;
use std::sync::Arc;

/// The op names Neo's method registry serves (hidden ones included) and the error codes Neo
/// defines, bound to the handle once at startup ([`crate::Telemetry::bind_vocabulary`]) from the
/// server's own tables (`Registry::names`, `ErrorCode::all`): never a list kept here. A label is one of these or [`Vocabulary::UNKNOWN`], so neither a
/// script calling invented names nor a code built at run time can grow a series set. Spans keep
/// the raw name. Cheap to clone: two shared sets.
#[derive(Clone, Debug, Default)]
pub struct Vocabulary {
    ops: Arc<HashSet<&'static str>>,
    error_codes: Arc<HashSet<&'static str>>,
}

impl Vocabulary {
    /// The label of every name and code outside the tables.
    pub const UNKNOWN: &'static str = "unknown";

    /// The server's method registry names and its error codes. `'static` because both tables
    /// are compile-time constants there, so a label never allocates.
    #[must_use]
    pub fn new(
        ops: impl IntoIterator<Item = &'static str>,
        error_codes: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        Self {
            ops: Arc::new(ops.into_iter().collect()),
            error_codes: Arc::new(error_codes.into_iter().collect()),
        }
    }

    /// The `neo.op.name` value for `name`: the registered name itself, or `unknown` (always a label,
    /// listed or not: it is the door's own name for a call with no method).
    #[must_use]
    pub fn op_label(&self, name: &str) -> &'static str {
        self.ops.get(name).copied().unwrap_or(Self::UNKNOWN)
    }

    /// The `error.type` value for `code`: one of Neo's codes, or `unknown`.
    #[must_use]
    pub fn error_code_label(&self, code: &str) -> &'static str {
        self.error_codes.get(code).copied().unwrap_or(Self::UNKNOWN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listed_value_is_its_own_label_and_anything_else_is_unknown() {
        let vocabulary = Vocabulary::new(
            ["page.goto", "locator.click"],
            ["no_match", "internal_error"],
        );
        assert_eq!(
            vocabulary.op_label("page.goto"),
            "page.goto",
            "registered ops label as themselves"
        );
        assert_eq!(
            vocabulary.error_code_label("no_match"),
            "no_match",
            "Neo's codes label as themselves"
        );
        for raw in ["page.gotoo", "", "PAGE.GOTO", "page.goto ", "no_match"] {
            assert_eq!(
                vocabulary.op_label(raw),
                Vocabulary::UNKNOWN,
                "`{raw}` is not a registered op, so it must not become a series of its own"
            );
        }
        for raw in ["page.goto", "No_Match", "made_up_code"] {
            assert_eq!(
                vocabulary.error_code_label(raw),
                Vocabulary::UNKNOWN,
                "`{raw}` is not one of Neo's codes, so it must not become a series of its own"
            );
        }
        let empty = Vocabulary::default();
        assert_eq!(
            empty.op_label("page.goto"),
            Vocabulary::UNKNOWN,
            "before the tables arrive, everything is unknown"
        );
        assert_eq!(
            empty.error_code_label("no_match"),
            Vocabulary::UNKNOWN,
            "before the tables arrive, everything is unknown"
        );
    }
}
