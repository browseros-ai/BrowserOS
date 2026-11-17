//! The navigation adapter and admission door share URL parsing, so normalization cannot turn a
//! refused scheme into an allowed browser navigation after the permission check.

/// The navigation adapter's scheme admission, including its existing HTTP completion for raw
/// lowercase local addresses. The engine can normalize more inputs before completing them, but
/// normalizing here would widen ordinary admission (for example uppercase `LOCALHOST:3000`).
#[must_use]
pub(crate) fn navigation_scheme(url: &str) -> Option<String> {
    if url.starts_with("localhost") || url.starts_with("127.0.0.1") {
        Some("http".to_owned())
    } else {
        url_scheme(url)
    }
}

/// The scheme of `url` as the URL Standard reads it: leading and trailing C0 controls and spaces
/// dropped, tabs and newlines removed wherever they are, then an ASCII letter and letters,
/// digits, `+`, `-` or `.` up to the first `:`, lowercased. `None` when there is none. Chromium
/// trims, strips and lowercases the same way, so ` JavaScript:` and `java\tscript:` are
/// `javascript:` to it too; a scheme with any other character is no URL it navigates to.
#[must_use]
pub(crate) fn url_scheme(url: &str) -> Option<String> {
    let mut scheme = String::new();
    let chars = url
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'));
    for c in chars {
        match c {
            ':' if !scheme.is_empty() => return Some(scheme),
            'a'..='z' | 'A'..='Z' => scheme.push(c.to_ascii_lowercase()),
            '0'..='9' | '+' | '-' | '.' if !scheme.is_empty() => scheme.push(c),
            _ => return None,
        }
    }
    None
}
