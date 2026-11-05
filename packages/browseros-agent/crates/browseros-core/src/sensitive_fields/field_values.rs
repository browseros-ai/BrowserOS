//! Values read from sensitive fields, used to mask the same values wherever else they appear.
//!
//! Two callers:
//! - the snapshot renderer, for a value echoed elsewhere on the page ("We sent 482913");
//! - the main-world guard around `page.evaluate` (sdk-playwright), which sweeps the frame's
//!   sensitive fields before and after the script and masks their values in its result.
//!
//! Matching follows `browseros-trace`'s `SecretSet`: each value in its plain, JSON-escaped and
//! percent-encoded forms, leftmost-longest, replaced by the same `[private]` token. It adds the
//! two forms the HTML serializer writes (text and attribute value), because serialized markup
//! reaches the agent too: sdk-playwright's `page.content()`, and a main-world script that
//! returns `outerHTML`. Unlike the secret set these values are short-lived (one capture or one
//! op) and are never registered here; the caller decides whether to register them with
//! sdk-traces' `Privacy`.

use aho_corasick::{AhoCorasick, MatchKind};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;
use std::fmt;
use zeroize::Zeroizing;

/// What an echoed value becomes. The same token as `browseros-trace`'s known-secret mask, so a
/// reader sees one marker for "a secret was here", distinct from a field's own `••••`.
pub const ECHO_MASK: &str = "[private]";

/// Values at least this long are masked wherever they occur. Shorter ones would mask ordinary
/// text (a 3-digit CVC is also a price), so they are masked only where a whole JSON string equals
/// them. The same threshold as the secret set's default `min_length`.
pub const MIN_SUBSTRING_LEN: usize = 4;

/// The bytes `encodeURIComponent` leaves alone, so a value a page put in a URL is found.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// A set of raw field values. It has no `Serialize` or `Display`, prints as `[field values]`
/// under `Debug`, and zeroes its own copies when dropped (the matcher's copies are freed, not
/// zeroed, as in the secret set).
#[derive(Default)]
pub struct FieldValues {
    values: Vec<Zeroizing<String>>,
    /// For rendered text: plain forms holding a newline are left out (see [`build_matcher`]).
    text_matcher: Option<AhoCorasick>,
    /// For decoded JSON strings, where a multi-line value appears with its newlines intact.
    leaf_matcher: Option<AhoCorasick>,
}

impl fmt::Debug for FieldValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[field values]")
    }
}

impl FieldValues {
    /// The set of non-empty, distinct values.
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = Zeroizing<String>>) -> Self {
        let mut distinct: Vec<Zeroizing<String>> = Vec::new();
        for value in values {
            if !value.is_empty() && !distinct.iter().any(|known| **known == *value) {
                distinct.push(value);
            }
        }
        let text_matcher = build_matcher(&distinct, Newlines::Excluded);
        let leaf_matcher = build_matcher(&distinct, Newlines::Included);
        Self {
            values: distinct,
            text_matcher,
            leaf_matcher,
        }
    }

    /// The values `seal.js`'s `sensitiveValues()` returned (a JSON array of strings). Anything
    /// else yields an empty set.
    #[must_use]
    pub fn from_sweep(sweep: &Value) -> Self {
        let values = sweep
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|value| Zeroizing::new(value.to_string()));
        Self::new(values)
    }

    /// Both sets together.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self::new(self.values.into_iter().chain(other.values))
    }

    /// No values at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The values long enough to register with the secret set, for a caller that does.
    pub fn registrable(&self) -> impl Iterator<Item = &str> {
        self.values
            .iter()
            .map(|value| value.as_str())
            .filter(|value| value.chars().count() >= MIN_SUBSTRING_LEN)
    }

    /// Replaces each value of at least [`MIN_SUBSTRING_LEN`] characters in `text` with
    /// `[private]`. Returns whether anything changed.
    pub fn mask_str(&self, text: &mut String) -> bool {
        replace_all(self.text_matcher.as_ref(), text)
    }

    /// Masks every string in `value`, keys included: long values wherever they occur, short
    /// ones where a whole string equals them. Returns whether anything changed.
    pub fn mask_json(&self, value: &mut Value) -> bool {
        match value {
            Value::String(text) => self.mask_leaf(text),
            // Every item is visited: stopping at the first change would leak the rest.
            Value::Array(items) => items
                .iter_mut()
                .fold(false, |changed, item| self.mask_json(item) | changed),
            Value::Object(map) => {
                let mut changed = false;
                let keys = map.keys().cloned().collect::<Vec<_>>();
                for key in keys {
                    let mut masked_key = key.clone();
                    let key_changed = self.mask_leaf(&mut masked_key);
                    if let Some(mut child) = map.remove(&key) {
                        changed |= self.mask_json(&mut child);
                        map.insert(masked_key, child);
                    }
                    changed |= key_changed;
                }
                changed
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => false,
        }
    }

    fn mask_leaf(&self, text: &mut String) -> bool {
        let exact_short = self.values.iter().any(|value| {
            value.chars().count() < MIN_SUBSTRING_LEN && value.as_str() == text.trim()
        });
        if exact_short {
            *text = ECHO_MASK.to_string();
            return true;
        }
        replace_all(self.leaf_matcher.as_ref(), text)
    }
}

fn replace_all(matcher: Option<&AhoCorasick>, text: &mut String) -> bool {
    let Some(matcher) = matcher else {
        return false;
    };
    if !matcher.is_match(text.as_str()) {
        return false;
    }
    let mut masked = String::with_capacity(text.len());
    matcher.replace_all_with(text.as_str(), &mut masked, |_, _, out| {
        out.push_str(ECHO_MASK);
        true
    });
    *text = masked;
    true
}

/// Whether a value's plain form may hold a newline in the text being masked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Newlines {
    /// Rendered snapshot text: a value never carries a raw newline there, and matching one
    /// across lines could join two lines' text (and shift every frame span after it).
    Excluded,
    /// Decoded JSON strings: a textarea's value keeps its newlines.
    Included,
}

/// One automaton over every form of every long value.
fn build_matcher(values: &[Zeroizing<String>], newlines: Newlines) -> Option<AhoCorasick> {
    let mut patterns: Vec<Zeroizing<String>> = Vec::new();
    for value in values {
        let plain = value.as_str();
        if plain.chars().count() < MIN_SUBSTRING_LEN {
            continue;
        }
        if newlines == Newlines::Included || !plain.contains('\n') {
            patterns.push(Zeroizing::new(plain.to_string()));
        }
        if let Ok(quoted) = serde_json::to_string(plain) {
            let escaped = quoted
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .unwrap_or(&quoted);
            if escaped != plain {
                patterns.push(Zeroizing::new(escaped.to_string()));
            }
        }
        let encoded = utf8_percent_encode(plain, URI_COMPONENT).to_string();
        if encoded != plain {
            if encoded.contains("%20") {
                patterns.push(Zeroizing::new(encoded.replace("%20", "+")));
            }
            patterns.push(Zeroizing::new(encoded));
        }
        for markup in [Markup::Text, Markup::Attribute] {
            let escaped = html_escaped(plain, markup);
            // The serializer leaves newlines raw, so the plain form's newline rule applies.
            if *escaped != plain
                && (newlines == Newlines::Included || !escaped.contains('\n'))
                && !patterns.contains(&escaped)
            {
                patterns.push(escaped);
            }
        }
    }
    if patterns.is_empty() {
        return None;
    }
    AhoCorasick::builder()
        .match_kind(MatchKind::LeftmostLongest)
        .build(patterns.iter().map(|pattern| pattern.as_str()))
        .map_err(|error| tracing::error!(%error, "field-value matcher could not be built"))
        .ok()
}

/// Where the HTML serializer writes a string.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Markup {
    Text,
    Attribute,
}

/// `value` as Chromium's HTML serializer writes it (`outerHTML`, `innerHTML`). Measured on
/// BrowserOS (Chromium 155): `&`, `<`, `>` and U+00A0 become entities in text and in attribute
/// values, and `"` in attribute values only. Built in a zeroizing buffer, as it holds the value.
fn html_escaped(value: &str, markup: Markup) -> Zeroizing<String> {
    let mut escaped = Zeroizing::new(String::with_capacity(value.len()));
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\u{a0}' => escaped.push_str("&nbsp;"),
            '"' if markup == Markup::Attribute => escaped.push_str("&quot;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{ECHO_MASK, FieldValues};
    use serde_json::json;
    use zeroize::Zeroizing;

    fn values(list: &[&str]) -> FieldValues {
        FieldValues::new(
            list.iter()
                .map(|value| Zeroizing::new((*value).to_string())),
        )
    }

    #[test]
    fn long_values_are_masked_in_every_form() {
        let set = values(&["482913", "pa ss\"w0rd"]);
        let mut text = r#"We sent 482913. Typed "pa ss\"w0rd" or ?p=pa%20ss%22w0rd&q=pa+ss%22w0rd"#
            .to_string();
        assert!(set.mask_str(&mut text));
        assert!(!text.contains("482913"));
        assert!(!text.contains("w0rd"));
        assert_eq!(text.matches(ECHO_MASK).count(), 4);
    }

    #[test]
    fn html_escaped_echoes_are_masked_in_text_and_attributes() {
        let set = values(&["typed&\"secret<3>\u{a0}x"]);
        let markup = "<p data-echo=\"typed&amp;&quot;secret&lt;3&gt;&nbsp;x\">\
                      you typed typed&amp;\"secret&lt;3&gt;&nbsp;x</p>";
        let mut text = markup.to_string();
        assert!(set.mask_str(&mut text));
        assert_eq!(
            text,
            format!("<p data-echo=\"{ECHO_MASK}\">you typed {ECHO_MASK}</p>")
        );
        let mut leaf = json!({ "html": markup });
        assert!(set.mask_json(&mut leaf));
        assert_eq!(
            leaf,
            json!({ "html": format!("<p data-echo=\"{ECHO_MASK}\">you typed {ECHO_MASK}</p>") })
        );
    }

    #[test]
    fn short_values_are_masked_only_as_whole_strings() {
        let set = values(&["987", "7"]);
        let mut result = json!({
            "cvc": "987",
            "digits": ["7", " 7 ", "17"],
            "price": "$9870",
            "count": 987,
        });
        assert!(set.mask_json(&mut result));
        assert_eq!(
            result,
            json!({
                "cvc": ECHO_MASK,
                "digits": [ECHO_MASK, ECHO_MASK, "17"],
                "price": "$9870",
                "count": 987,
            })
        );
        let mut text = "987 and 7".to_string();
        assert!(!set.mask_str(&mut text));
    }

    #[test]
    fn multi_line_values_are_masked_in_json_but_never_across_rendered_lines() {
        let set = values(&["line one\nline two"]);
        let mut result = json!({"note": "saved: line one\nline two"});
        assert!(set.mask_json(&mut result));
        assert_eq!(result, json!({"note": format!("saved: {ECHO_MASK}")}));
        let mut rendered = "- textbox: \"line one\\nline two\"\nline one\nline two".to_string();
        assert!(set.mask_str(&mut rendered));
        assert_eq!(rendered.lines().count(), 3, "{rendered}");
        assert!(rendered.contains(ECHO_MASK));
    }

    #[test]
    fn keys_and_nested_values_are_masked_and_every_item_is_visited() {
        let set = values(&["PWTYPED_4f7h"]);
        let mut result = json!({"PWTYPED_4f7h": ["a PWTYPED_4f7h", {"x": "PWTYPED_4f7h"}]});
        assert!(set.mask_json(&mut result));
        assert!(!result.to_string().contains("PWTYPED_4f7h"));
    }

    #[test]
    fn sweeps_and_unions_drop_empties_and_duplicates() {
        let sweep = FieldValues::from_sweep(&json!(["", "abcd", "abcd", 5, "xy"]));
        let joined = sweep.union(values(&["abcd", "efgh"]));
        assert_eq!(joined.registrable().collect::<Vec<_>>(), ["abcd", "efgh"]);
        assert!(FieldValues::from_sweep(&json!({"not": "an array"})).is_empty());
        assert_eq!(format!("{joined:?}"), "[field values]");
    }
}
