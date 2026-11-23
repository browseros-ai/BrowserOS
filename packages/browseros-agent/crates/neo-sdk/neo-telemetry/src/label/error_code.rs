//! A failure's stable code, as a label.

/// A failure's stable `snake_case` code: the `error.type` of `neo_errors_total` and the
/// spans' `neo.error.code`.
///
/// It holds a `&'static str` on purpose: the server's codes are compile-time constants (its
/// `ErrorCode` list in `dispatch/operation_error.rs`), so page or script text cannot be passed
/// here. The pipeline also interns it against that list ([`crate::Vocabulary`]), so even a
/// leaked string labels as `unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ErrorCode(&'static str);

impl ErrorCode {
    /// The code Neo already serves on `/v1/runs` for this failure, e.g. `strict_mode_violation`.
    #[must_use]
    pub const fn new(code: &'static str) -> Self {
        Self(code)
    }

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}
