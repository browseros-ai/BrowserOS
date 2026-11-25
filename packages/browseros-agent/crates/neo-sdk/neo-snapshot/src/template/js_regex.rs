use std::error::Error;
use std::fmt;

use regress::{Flags, Regex};

/// Failure compiling a template regular expression under JavaScript `RegExp` syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexError {
    message: String,
}

impl fmt::Display for RegexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for RegexError {}

/// A compiled ECMAScript regular expression (no flags, matching `new RegExp(pattern)` in
/// Playwright's `ariaSnapshot.ts`).
///
/// Wraps `regress::Regex` behind this crate's own seam so callers never depend on `regress`
/// directly. Both pattern compilation and matching operate on UTF-16 code units (UCS-2) because
/// unflagged JavaScript `RegExp` treats supplementary Unicode characters as surrogate pairs rather
/// than single characters.
#[derive(Clone, Debug)]
pub(crate) struct JsRegex {
    inner: Regex,
}

impl JsRegex {
    /// Compiles `pattern` with default unflagged ECMAScript UCS-2 semantics (`flags = ""`).
    pub(crate) fn compile(pattern: &str) -> Result<Self, RegexError> {
        Regex::from_unicode(pattern.encode_utf16().map(u32::from), Flags::default())
            .map(|inner| Self { inner })
            .map_err(|error| RegexError {
                message: error.to_string(),
            })
    }

    /// Reports whether `pattern` matches anywhere in `text`, matching `Boolean(text.match(re))`
    /// under unflagged JavaScript UTF-16 code-unit semantics.
    #[must_use]
    pub(crate) fn is_match(&self, text: &str) -> bool {
        let units: Vec<u16> = text.encode_utf16().collect();
        self.inner.find_from_ucs2(&units, 0).next().is_some()
    }
}
