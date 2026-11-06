//! Playwright's `urlMatches` (`packages/isomorphic/urlMatch.ts` at 26137face) for the `url` a script
//! passes to `waitForURL`. neo-sdk sends it as Playwright's own `SerializedURLMatch` without
//! `urlPattern` (the realm has no `URLPattern`): a glob, where `''` matches every URL, or a
//! `RegExp`'s `source` and `flags`. Neo has no `baseURL` option, so a glob resolves against none.
//!
//! This interface is the contract `waits.rs` codes against. The body is a placeholder until the
//! Playwright port lands: a glob matches only the identical URL (`''` every URL), and a `RegExp`
//! fails to match with an error.

use serde::Deserialize;

use crate::progress::PlaywrightError;

/// Playwright's `SerializedURLMatch` without `urlPattern`: exactly one of `glob` or `regexSource`
/// (with `regexFlags`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SerializedUrlMatch {
    pub(crate) glob: Option<String>,
    pub(crate) regex_source: Option<String>,
    pub(crate) regex_flags: Option<String>,
}

/// One compiled `waitForURL` match. `Send + Sync`: a wait holds it across awaits.
#[derive(Debug)]
pub(crate) struct UrlMatcher(Pattern);

#[derive(Debug)]
enum Pattern {
    Glob(String),
    Regex { source: String, flags: String },
}

impl UrlMatcher {
    /// Compiles the match the SDK sent.
    ///
    /// # Errors
    ///
    /// An invalid glob (Playwright's `Invalid glob pattern …`) or `RegExp`, or a match that is
    /// neither a glob nor a `RegExp`.
    pub(crate) fn new(wire: &SerializedUrlMatch) -> Result<Self, PlaywrightError> {
        match (&wire.glob, &wire.regex_source) {
            (Some(glob), None) => Ok(Self(Pattern::Glob(glob.clone()))),
            (None, Some(source)) => Ok(Self(Pattern::Regex {
                source: source.clone(),
                flags: wire.regex_flags.clone().unwrap_or_default(),
            })),
            _ => Err(PlaywrightError::error(
                "url: expected exactly one of glob or regexSource",
            )),
        }
    }

    /// Whether `url` matches, as Playwright's `urlMatches(undefined, url, match)`.
    ///
    /// # Errors
    ///
    /// The `RegExp` engine failed or overran its budget on `url`.
    pub(crate) fn matches(&self, url: &str) -> Result<bool, PlaywrightError> {
        match &self.0 {
            Pattern::Glob(glob) => Ok(glob.is_empty() || glob == url),
            Pattern::Regex { source, flags } => Err(PlaywrightError::error(format!(
                "neo-runtime: /{source}/{flags} cannot be matched yet"
            ))),
        }
    }

    /// The glob as the script wrote it, for the `waiting for navigation to "<glob>"` log line;
    /// `None` for a `RegExp`, which Playwright does not print.
    pub(crate) fn glob(&self) -> Option<&str> {
        match &self.0 {
            Pattern::Glob(glob) => Some(glob),
            Pattern::Regex { .. } => None,
        }
    }
}
