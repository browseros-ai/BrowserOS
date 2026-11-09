//! Neo: the values a run's hooks swapped into its calls (hooks design D11), and their scrubbing
//! from what the agent's script reads. A hook that replaces `fill('{{password}}')`'s value types
//! the real password; the page may then show it in an element's attributes (a page that mirrors
//! an input's value into its `value` attribute, as React's controlled inputs do), and a later
//! call's log or error names that element by its preview, attributes included. So once a hook
//! swapped a string in, every agent call's call log and error text replies, and is recorded,
//! with that string replaced by [`REDACTED`].
//!
//! Results are not scrubbed: a read that returns the value is the hooks' to redact (afterAction).
//! Nor are a hook's own replies: a hook needs the real values.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

/// Neo: what replaces a swapped value in the agent's call logs and error texts.
pub(crate) const REDACTED: &str = "[redacted]";

/// Neo: the shortest string a swap remembers, in characters. Shorter ones would scrub ordinary
/// words and selectors from every log, and a value that short protects nothing.
const MIN_SWAPPED_CHARS: usize = 4;

/// Neo: the longest string a swap remembers, in bytes: file contents and other large values,
/// which no call log or error text echoes, would make every scrub slow.
const MAX_SWAPPED_BYTES: usize = 64 * 1024;

/// Neo: the longest cut-short prefix of a swapped value a scrub looks for, in characters: an
/// element preview cuts its attributes at 500 characters and its text at 50, each ending in an
/// ellipsis.
const MAX_CUT_CHARS: usize = 500;

/// The run's swapped values, shared by every call of the run; empty, and free to check, until a
/// hook swaps one in.
#[derive(Default)]
pub(crate) struct SwappedValues {
    /// Longest first, so a value that holds another is scrubbed whole.
    values: Mutex<Vec<String>>,
    any: AtomicBool,
}

impl SwappedValues {
    /// Remembers each string of a hook's `replacement` arguments that the script's `script`
    /// arguments do not hold anywhere, if it is at least [`MIN_SWAPPED_CHARS`] long.
    pub(crate) fn remember(&self, script: &Value, replacement: &Value) {
        let mut sent = Vec::new();
        strings(script, &mut sent);
        let sent: HashSet<&str> = sent.into_iter().collect();
        let mut swapped = Vec::new();
        strings(replacement, &mut swapped);
        let fresh: Vec<&str> = swapped
            .into_iter()
            .filter(|value| {
                value.chars().count() >= MIN_SWAPPED_CHARS
                    && value.len() <= MAX_SWAPPED_BYTES
                    && !sent.contains(value)
            })
            .collect();
        if fresh.is_empty() {
            return;
        }
        // A poisoned lock (a panic while scrubbing) keeps the values it has.
        if let Ok(mut values) = self.values.lock() {
            for value in fresh {
                if !values.iter().any(|known| known == value) {
                    values.push(value.to_string());
                }
            }
            values.sort_by_key(|value| Reverse(value.len()));
            self.any.store(true, Ordering::SeqCst);
        }
    }

    /// `text` with every swapped value replaced by [`REDACTED`], and every prefix of one that
    /// a preview cut short (ending in an ellipsis); `None` when it holds none.
    pub(crate) fn scrub(&self, text: &str) -> Option<String> {
        if !self.any.load(Ordering::SeqCst) {
            return None;
        }
        let Ok(values) = self.values.lock() else {
            // A poisoned lock scrubs everything rather than let a value through.
            return Some(REDACTED.to_string());
        };
        let mut scrubbed: Option<String> = None;
        for value in values.iter() {
            let current = scrubbed.as_deref().unwrap_or(text);
            let replaced = current
                .contains(value.as_str())
                .then(|| current.replace(value.as_str(), REDACTED));
            let current = replaced.as_deref().unwrap_or(current);
            let cut = scrub_cut(current, value);
            if let Some(next) = cut.or(replaced) {
                scrubbed = Some(next);
            }
        }
        scrubbed
    }

    /// `text`, scrubbed ([`SwappedValues::scrub`]).
    pub(crate) fn scrubbed(&self, text: String) -> String {
        self.scrub(&text).unwrap_or(text)
    }
}

/// Every string leaf of `value`.
fn strings<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => out.push(text),
        Value::Array(items) => items.iter().for_each(|item| strings(item, out)),
        Value::Object(fields) => fields.values().for_each(|field| strings(field, out)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// `text` with each prefix of `value` that runs into an ellipsis replaced, at least
/// [`MIN_SWAPPED_CHARS`] long: an element preview that cut the value short. `None` when there
/// is none.
fn scrub_cut(text: &str, value: &str) -> Option<String> {
    // The ellipsis a preview ends a cut text with.
    let ellipsis = '\u{2026}';
    if !text.contains(ellipsis) {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let mut rest = text;
    while let Some(at) = rest.find(ellipsis) {
        let before = &rest[..at];
        match cut_prefix(before, value) {
            Some(len) => {
                out.push_str(&before[..before.len() - len]);
                out.push_str(REDACTED);
                changed = true;
            }
            None => out.push_str(before),
        }
        out.push(ellipsis);
        rest = &rest[at + ellipsis.len_utf8()..];
    }
    out.push_str(rest);
    changed.then_some(out)
}

/// The byte length of the longest prefix of `value` that `before` ends with, if it is at least
/// [`MIN_SWAPPED_CHARS`] and at most [`MAX_CUT_CHARS`] long.
fn cut_prefix(before: &str, value: &str) -> Option<usize> {
    let ends: Vec<usize> = value
        .char_indices()
        .map(|(at, _)| at)
        .skip(MIN_SWAPPED_CHARS)
        .take(MAX_CUT_CHARS)
        .chain((value.chars().count() <= MAX_CUT_CHARS).then_some(value.len()))
        .collect();
    ends.into_iter()
        .rev()
        .find(|end| before.ends_with(&value[..*end]))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SECRET: &str = "Zq9vX7-real-password";

    #[test]
    fn tests_that_only_strings_the_script_never_sent_are_remembered() {
        let swapped = SwappedValues::default();
        assert_eq!(
            swapped.scrub(SECRET),
            None,
            "nothing swapped: nothing scrubbed"
        );
        swapped.remember(
            &json!({ "selector": "#pw", "value": "{{password}}", "strict": true }),
            &json!({ "selector": "#pw", "value": SECRET, "strict": true, "extra": ["abc", "#pw"] }),
        );
        assert_eq!(
            swapped.scrub(&format!("value=\"{SECRET}\" at #pw")),
            Some(format!("value=\"{REDACTED}\" at #pw")),
            "the selector the script sent stays; a 3-character string is ignored"
        );
        assert_eq!(swapped.scrub("abc and {{password}}"), None);
    }

    #[test]
    fn tests_that_a_value_that_holds_another_is_scrubbed_whole() {
        let swapped = SwappedValues::default();
        swapped.remember(&json!({}), &json!({ "a": "real", "b": "real-password-2" }));
        assert_eq!(
            swapped.scrub("x real-password-2 y real z"),
            Some(format!("x {REDACTED} y {REDACTED} z"))
        );
    }

    #[test]
    fn tests_that_a_preview_that_cut_the_value_short_is_scrubbed() {
        let swapped = SwappedValues::default();
        swapped.remember(&json!({}), &json!({ "value": SECRET }));
        assert_eq!(
            swapped.scrub("<div>hello Zq9vX7-real-pa\u{2026}</div>"),
            Some(format!("<div>hello {REDACTED}\u{2026}</div>"))
        );
        assert_eq!(
            swapped.scrub("<div>Zq9\u{2026}</div>"),
            None,
            "a prefix under four characters is no value"
        );
    }
}
