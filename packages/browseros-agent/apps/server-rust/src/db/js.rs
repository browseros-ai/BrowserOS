//! The few JavaScript string/number semantics the TS stores bake into stored values (`trim()`,
//! `value?.trim() || null`, `slice()` on UTF-16 units, `Date.now()`), so a row written here is
//! the row the TS server would have written for the same input.

use std::time::{SystemTime, UNIX_EPOCH};

/// `Date.now()`: epoch milliseconds.
pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// `String.prototype.trim`: JS whitespace and line terminators. Differs from `str::trim` on
/// U+FEFF (JS trims it) and U+0085 (JS keeps it).
pub(super) fn trim(value: &str) -> &str {
    value.trim_matches(|ch: char| ch == '\u{feff}' || (ch.is_whitespace() && ch != '\u{85}'))
}

/// `value?.trim() || null`, the acp-agent store's normalization of optional text columns.
pub(super) fn optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// `value.slice(0, limit)` counted in UTF-16 units. A surrogate pair split by the cut becomes
/// U+FFFD, which is what a lone surrogate turns into once JS hands the string to SQLite.
pub(super) fn utf16_prefix(value: &str, limit: usize) -> String {
    let units: Vec<u16> = value.encode_utf16().take(limit).collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::{optional_text, trim, utf16_prefix};

    #[test]
    fn trim_matches_javascript() {
        assert_eq!(trim("\u{feff} a \u{3000}\n"), "a");
        assert_eq!(trim("\u{85}a"), "\u{85}a");
    }

    #[test]
    fn optional_text_maps_blank_to_none() {
        assert_eq!(optional_text(Some("  ")), None);
        assert_eq!(optional_text(Some(" x ")), Some("x".to_owned()));
        assert_eq!(optional_text(None), None);
    }

    #[test]
    fn utf16_prefix_counts_code_units() {
        assert_eq!(utf16_prefix("héllo", 2), "hé");
        assert_eq!(utf16_prefix("a😀b", 2), "a\u{fffd}");
        assert_eq!(utf16_prefix("a😀b", 3), "a😀");
    }
}
