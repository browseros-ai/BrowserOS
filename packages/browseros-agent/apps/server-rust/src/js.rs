//! The JavaScript string/number semantics the TS server bakes into stored and wire values
//! (`trim()`, `value?.trim() || null`, `length` and `slice()` on UTF-16 units, `Date.now()`),
//! so a value produced here is the one the TS server would have produced for the same input.
//! Use these rather than `str::trim` or char counts wherever TS parity is observable.

use std::time::{SystemTime, UNIX_EPOCH};

/// `Date.now()`: epoch milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// `String.prototype.trim`: JS whitespace and line terminators. Differs from `str::trim` on
/// U+FEFF (JS trims it) and U+0085 (JS keeps it).
pub fn trim(value: &str) -> &str {
    value.trim_matches(|ch: char| ch == '\u{feff}' || (ch.is_whitespace() && ch != '\u{85}'))
}

/// `value?.trim() || null`, the acp-agent store's normalization of optional text columns.
pub fn optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// `value.length`: UTF-16 code units, so an astral symbol counts twice.
pub fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

/// `value.slice(0, limit)` counted in UTF-16 units. A surrogate pair split by the cut becomes
/// U+FFFD, which is what a lone surrogate turns into once JS hands the string to SQLite (and
/// what a Rust `String` can hold at all). A wire value that must keep JS's lone surrogate
/// escaped (`\ud83d`) needs its own serialization; see `http::llm_tools`' preview.
pub fn utf16_prefix(value: &str, limit: usize) -> String {
    let units: Vec<u16> = value.encode_utf16().take(limit).collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::{optional_text, trim, utf16_len, utf16_prefix};

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
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(utf16_prefix("héllo", 2), "hé");
        assert_eq!(utf16_prefix("a😀b", 2), "a\u{fffd}");
        assert_eq!(utf16_prefix("a😀b", 3), "a😀");
    }
}
