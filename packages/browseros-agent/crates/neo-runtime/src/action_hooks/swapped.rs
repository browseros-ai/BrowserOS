//! Neo: the values a run's hooks swapped into its calls (hooks design D11), and their scrubbing
//! from what the agent's script reads. A hook that replaces `fill('{{password}}')`'s value types
//! the real password; the page may then show it in an element's attributes (a page that mirrors
//! an input's value into its `value` attribute, as React's controlled inputs do), and a later
//! call's log or error names that element by its preview, attributes included. So once a hook
//! swapped a string in, or typed one with its own call, every agent call's call log and error
//! text replies, and is recorded, with that string replaced by [`REDACTED`].
//!
//! Results are not scrubbed: a read that returns the value is the hooks' to redact (afterAction).
//! Nor are a hook's own replies: a hook needs the real values.

use std::cmp::Reverse;
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

/// Neo: what replaces a swapped value in the agent's call logs and error texts.
pub(crate) const REDACTED: &str = "[redacted]";

/// Neo: the shortest value typed into the page that a swap remembers, in characters: a value or
/// text a hook swapped in or typed itself (a card's 3-digit code is one). Shorter ones would
/// scrub single letters from every log, and protect nothing.
const MIN_TYPED_CHARS: usize = 3;

/// Neo: the shortest other string a swap remembers, in characters: shorter ones would scrub
/// ordinary words and selectors from every log.
const MIN_OTHER_CHARS: usize = 4;

/// Neo: the object keys whose strings are typed into the page: a fill's `value`, a type's
/// `text`, a select option's `valueOrLabel`.
const TYPED_KEYS: [&str; 3] = ["value", "text", "valueOrLabel"];

/// Neo: the longest string a swap remembers, in bytes: file contents and other large values,
/// which no call log or error text echoes, would make every scrub slow.
const MAX_SWAPPED_BYTES: usize = 64 * 1024;

/// Neo: how many values a run remembers at most; past it, the oldest go first, so a run that
/// swaps on every call cannot make each scrub slower without bound.
const MAX_SWAPPED_VALUES: usize = 1024;

/// Neo: how many bytes of values a run remembers at most, the oldest going first past it.
const MAX_SWAPPED_TOTAL_BYTES: usize = 1024 * 1024;

/// Neo: the longest cut-short prefix of a swapped value a scrub looks for, in characters: an
/// element preview cuts its attributes at 500 characters and its text at 50, each ending in an
/// ellipsis.
const MAX_CUT_CHARS: usize = 500;

/// The run's swapped values, shared by every call of the run; empty, and free to check, until a
/// hook swaps one in or types one itself.
#[derive(Default)]
pub(crate) struct SwappedValues {
    values: Mutex<Remembered>,
    any: AtomicBool,
}

#[derive(Default)]
struct Remembered {
    /// In the order they were remembered, the oldest first: who goes past the caps.
    order: VecDeque<String>,
    /// The same values, longest first, so a value that holds another is scrubbed whole.
    by_length: Vec<String>,
    bytes: usize,
}

impl Remembered {
    fn add(&mut self, value: &str) {
        if self.order.iter().any(|known| known == value) {
            return;
        }
        self.order.push_back(value.to_string());
        self.bytes += value.len();
        while self.order.len() > MAX_SWAPPED_VALUES || self.bytes > MAX_SWAPPED_TOTAL_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.bytes -= oldest.len();
        }
        self.by_length = self.order.iter().cloned().collect();
        self.by_length.sort_by_key(|value| Reverse(value.len()));
    }
}

impl SwappedValues {
    /// Remembers each string of a hook's `replacement` arguments that the script's `script`
    /// arguments do not hold anywhere: a value or text typed into the page from
    /// [`MIN_TYPED_CHARS`] characters, any other string from [`MIN_OTHER_CHARS`].
    pub(crate) fn remember(&self, script: &Value, replacement: &Value) {
        let mut sent = Vec::new();
        strings(script, None, &mut sent);
        let sent: HashSet<&str> = sent.into_iter().map(|(text, _)| text).collect();
        let mut swapped = Vec::new();
        strings(replacement, None, &mut swapped);
        let fresh = swapped
            .into_iter()
            .filter(|(text, typed)| worth_remembering(text, *typed) && !sent.contains(text))
            .map(|(text, _)| text);
        self.add(fresh);
    }

    /// Remembers what a hook's own call types into the page (`values`: a fill's value, a type's
    /// text, a select option's value), from [`MIN_TYPED_CHARS`] characters: a hook that types a
    /// secret itself leaves it in the page as a swap does.
    pub(crate) fn remember_typed<'a>(&self, values: impl IntoIterator<Item = &'a str>) {
        self.add(
            values
                .into_iter()
                .filter(|value| worth_remembering(value, true)),
        );
    }

    fn add<'a>(&self, fresh: impl Iterator<Item = &'a str>) {
        let mut fresh = fresh.peekable();
        if fresh.peek().is_none() {
            return;
        }
        // A poisoned lock (a panic while scrubbing) makes every scrub redact everything.
        if let Ok(mut values) = self.values.lock() {
            for value in fresh {
                values.add(value);
            }
        }
        self.any.store(true, Ordering::SeqCst);
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
        for value in &values.by_length {
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

/// Whether `text` is long enough to remember, and short enough: `typed` into the page from
/// [`MIN_TYPED_CHARS`] characters, otherwise from [`MIN_OTHER_CHARS`].
fn worth_remembering(text: &str, typed: bool) -> bool {
    let floor = if typed {
        MIN_TYPED_CHARS
    } else {
        MIN_OTHER_CHARS
    };
    text.chars().count() >= floor && text.len() <= MAX_SWAPPED_BYTES
}

/// Every string leaf of `value`, with whether its object key ([`TYPED_KEYS`]) types it into the
/// page; an array's items take the key of the array.
fn strings<'a>(value: &'a Value, key: Option<&str>, out: &mut Vec<(&'a str, bool)>) {
    match value {
        Value::String(text) => out.push((text, key.is_some_and(|key| TYPED_KEYS.contains(&key)))),
        Value::Array(items) => items.iter().for_each(|item| strings(item, key, out)),
        Value::Object(fields) => fields
            .iter()
            .for_each(|(key, field)| strings(field, Some(key), out)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// `text` with each prefix of `value` that runs into an ellipsis replaced, at least
/// [`MIN_OTHER_CHARS`] long: an element preview that cut the value short. `None` when there
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
/// [`MIN_OTHER_CHARS`] and at most [`MAX_CUT_CHARS`] long.
fn cut_prefix(before: &str, value: &str) -> Option<usize> {
    let ends: Vec<usize> = value
        .char_indices()
        .map(|(at, _)| at)
        .skip(MIN_OTHER_CHARS)
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
            "the selector the script sent stays; a 3-character string that is no value is ignored"
        );
        assert_eq!(swapped.scrub("abc and {{password}}"), None);
    }

    #[test]
    fn tests_that_a_short_value_typed_into_the_page_is_remembered() {
        let swapped = SwappedValues::default();
        swapped.remember(&json!({ "value": "{{cvv}}" }), &json!({ "value": "739" }));
        swapped.remember(&json!({ "text": "{{pin}}" }), &json!({ "text": "40" }));
        assert_eq!(
            swapped.scrub("value=\"739\" and 40"),
            Some(format!("value=\"{REDACTED}\" and 40")),
            "a 3-character value is remembered, a 2-character one is not"
        );
        let typed = SwappedValues::default();
        typed.remember_typed(["s3c", "no"]);
        assert_eq!(typed.scrub("s3c no"), Some(format!("{REDACTED} no")));
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

    #[test]
    fn tests_that_the_oldest_values_go_past_the_caps() {
        let swapped = SwappedValues::default();
        let values: Vec<String> = (0..=MAX_SWAPPED_VALUES)
            .map(|n| format!("value-{n:05}"))
            .collect();
        swapped.remember_typed(values.iter().map(String::as_str));
        assert_eq!(swapped.scrub("value-00000"), None, "the oldest went first");
        assert!(swapped.scrub("value-00001").is_some());
        let big = "x".repeat(MAX_SWAPPED_BYTES);
        let bigger: Vec<String> = (0..17).map(|n| format!("{n:02}{big}")).collect();
        let large = SwappedValues::default();
        large.remember_typed(bigger.iter().map(|value| &value[..MAX_SWAPPED_BYTES]));
        let remembered = large.values.lock().map_or(0, |values| values.bytes);
        assert!(remembered <= MAX_SWAPPED_TOTAL_BYTES, "{remembered}");
    }
}
