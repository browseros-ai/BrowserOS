//! The navigation adapter and admission door share URL parsing, so normalization cannot turn a
//! refused scheme into an allowed browser navigation after the permission check. The scheme
//! reading itself, and the rule for pages Neo refuses, live in `neo_playwright::url_scheme`,
//! which the target watcher reads too.

use neo_playwright::url_scheme;

/// The navigation adapter's scheme admission, including its existing HTTP completion for raw
/// lowercase local addresses. The engine can normalize more inputs before completing them, but
/// normalizing here would widen ordinary admission (for example uppercase `LOCALHOST:3000`).
#[must_use]
pub(crate) fn navigation_scheme(url: &str) -> Option<String> {
    if url.starts_with("localhost") || url.starts_with("127.0.0.1") {
        Some("http".to_owned())
    } else {
        url_scheme::url_scheme(url)
    }
}
