//! Neo: the values a run's hooks swapped into its calls (hooks design D11), and their scrubbing
//! from what the agent's script reads. A hook that replaces `fill('{{password}}')`'s value types
//! the real password; the page may then show it in an element's attributes (a page that mirrors
//! an input's value into its `value` attribute, as React's controlled inputs do), and a later
//! call's log or error names that element by its preview, attributes included. So once a hook
//! swapped a string in, typed one with its own call or handed one to its own page script, every
//! agent call's call log and error text replies, and is recorded, with that string replaced by
//! [`REDACTED`].
//!
//! The run keeps every such value: none is ever dropped, or an agent that makes the run remember
//! more could push the password out. Past the caps, the scrub fails closed instead: from then on
//! every agent call replies with no log and [`UNTRACKED`] for its error text.
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

/// Neo: the error text of every agent call once the run's hooks swapped in more values than the
/// scrub keeps ([`MAX_SWAPPED_VALUES`], [`MAX_SWAPPED_TOTAL_BYTES`]): the real text could name one
/// it did not keep.
pub(crate) const UNTRACKED: &str = "the run's hooks swapped in more values than Neo keeps out of \
                                    call logs; this call's message is withheld";

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

/// Neo: how many values a run remembers at most, so a run that swaps on every call cannot make
/// each scrub slower without bound; past it, the scrub fails closed ([`UNTRACKED`]).
const MAX_SWAPPED_VALUES: usize = 1024;

/// Neo: how many bytes of values a run remembers at most; past it, the scrub fails closed.
const MAX_SWAPPED_TOTAL_BYTES: usize = 1024 * 1024;

/// Neo: the markers Playwright's argument serializer writes for values JSON cannot hold
/// (`{v: 'undefined'}` and kin), which are no value a hook handed its page script.
const SERIALIZER_MARKERS: [&str; 6] = ["undefined", "null", "NaN", "Infinity", "-Infinity", "-0"];

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
    /// Set once a value did not fit the caps: the scrub fails closed for the rest of the run.
    overflowed: AtomicBool,
}

#[derive(Default)]
struct Remembered {
    /// In the order they were remembered.
    order: Vec<String>,
    /// The same values, longest first, so a value that holds another is scrubbed whole.
    by_length: Vec<String>,
    bytes: usize,
}

impl Remembered {
    /// Keeps `value`; `false` when it does not fit the caps, and is not kept: nothing kept is
    /// ever dropped.
    fn add(&mut self, value: &str) -> bool {
        if self.order.iter().any(|known| known == value) {
            return true;
        }
        if self.order.len() >= MAX_SWAPPED_VALUES
            || self.bytes + value.len() > MAX_SWAPPED_TOTAL_BYTES
        {
            return false;
        }
        self.order.push(value.to_string());
        self.bytes += value.len();
        self.by_length = self.order.clone();
        self.by_length.sort_by_key(|value| Reverse(value.len()));
        true
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

    /// Remembers the strings of what a hook hands its own page script (the serialized `arg` of an
    /// `evaluate`, `$eval` or a handle's `evaluate`), from [`MIN_OTHER_CHARS`] characters: a hook
    /// that sets a box's value by script leaves it in the page as a swap does. The serializer's
    /// markers and an object's keys are no value.
    pub(crate) fn remember_serialized(&self, value: &Value) {
        let mut found = Vec::new();
        serialized_strings(value, &mut found);
        self.add(
            found
                .into_iter()
                .filter(|text| worth_remembering(text, false)),
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
                if !values.add(value) {
                    self.overflowed.store(true, Ordering::SeqCst);
                }
            }
        }
        self.any.store(true, Ordering::SeqCst);
    }

    /// Whether a value did not fit the caps: every later agent call keeps no log and replies
    /// [`UNTRACKED`] for its error text.
    pub(crate) fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::SeqCst)
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

/// Every string leaf of a serialized page-script argument but the serializer's markers and an
/// object entry's key (`{k, v}`).
fn serialized_strings<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => {
            if !SERIALIZER_MARKERS.contains(&text.as_str()) {
                out.push(text);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| serialized_strings(item, out)),
        Value::Object(fields) => {
            let entry = fields.contains_key("k") && fields.contains_key("v");
            for (key, field) in fields {
                if !(entry && key == "k") {
                    serialized_strings(field, out);
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
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
    fn tests_that_nothing_kept_is_dropped_and_past_the_caps_the_scrub_fails_closed() {
        let swapped = SwappedValues::default();
        swapped.remember(
            &json!({ "value": "{{password}}" }),
            &json!({ "value": SECRET }),
        );
        let values: Vec<String> = (1..MAX_SWAPPED_VALUES)
            .map(|n| format!("value-{n:05}"))
            .collect();
        swapped.remember_typed(values.iter().map(String::as_str));
        assert!(!swapped.overflowed(), "exactly at the cap");
        assert!(
            swapped.scrub(SECRET).is_some(),
            "the first value is still scrubbed at the cap"
        );
        swapped.remember_typed(["one-too-many"]);
        assert!(swapped.overflowed(), "past the cap");
        assert!(swapped.scrub(SECRET).is_some(), "nothing kept was dropped");
        let large = SwappedValues::default();
        let big = "x".repeat(MAX_SWAPPED_BYTES - 2);
        let bigger: Vec<String> = (0..17).map(|n| format!("{n:02}{big}")).collect();
        large.remember_typed(bigger.iter().take(16).map(String::as_str));
        assert!(!large.overflowed(), "16 values of 64 KiB fit 1 MiB");
        large.remember_typed([bigger[16].as_str()]);
        assert!(large.overflowed(), "the 17th does not");
    }

    #[test]
    fn tests_that_a_page_scripts_argument_strings_are_remembered_but_not_its_markers_or_keys() {
        let swapped = SwappedValues::default();
        swapped.remember_serialized(&json!({
            "o": [
                { "k": "password", "v": SECRET },
                { "k": "missing", "v": { "v": "undefined" } },
                { "k": "short", "v": "abc" },
            ],
            "id": 1,
        }));
        assert_eq!(
            swapped.scrub(&format!("{SECRET} password undefined abc")),
            Some(format!("{REDACTED} password undefined abc"))
        );
    }
}
