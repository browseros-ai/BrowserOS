//! The one process-wide set of secret values Neo masks wherever text leaves it: op results into
//! either JS realm, error messages, console lines, the model-facing run result (the skeleton's
//! `Masker` calls [`SecretSet::mask_str`] and [`SecretSet::mask_json`]), and every trace record.
//!
//! sdk-extensions feeds it: each protected-fill value before its first keystroke, and
//! `POST /host/v1/secrets`. Takeover and the control display feed it what the owner typed into
//! private fields. Values live in memory only, never on disk. Each [`SecretValue`] is zeroed
//! when dropped; the matcher keeps its own copies, which are freed but not zeroed. One set
//! masks every run, because all runs share one browser and any run can read any tab. A value
//! expires by its TTL, never with a tab or a task.
//!
//! Each value is matched in its plain, JSON-escaped and percent-encoded forms, with one
//! Aho-Corasick automaton rebuilt on every change. Changes are rare (one per fill); masks run on
//! every result.

use aho_corasick::{AhoCorasick, MatchKind};
use browseros_core::sensitive_fields;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;
use std::fmt;
use std::sync::{PoisonError, RwLock};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// What a masked value becomes.
pub const MASK: &str = "[private]";

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

/// The bytes form encoding (`application/x-www-form-urlencoded`, `URLSearchParams`, a submitted
/// form) leaves alone. It escapes `!~'()` too, and writes a space as `+`.
const FORM: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'*')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_');

/// A secret value. It has no `Serialize` or `Display`, prints as `[secret]` under `Debug`, and
/// is zeroed when dropped.
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    /// Wraps a value. The caller's copy should be dropped right after.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[secret]")
    }
}

/// The secret set's limits. [`Default`] is the fixed default claw-sdk runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecretSetConfig {
    /// Shorter values are refused: they would mask ordinary text (zuzu's rule).
    pub min_length: usize,
    /// How long a value is masked when its registration names no TTL: 24 h by default.
    pub retention: Duration,
    /// The most values kept; registering past it drops the one that expires first.
    pub max_entries: usize,
}

impl Default for SecretSetConfig {
    fn default() -> Self {
        Self {
            min_length: 4,
            retention: Duration::from_secs(24 * 60 * 60),
            max_entries: 4096,
        }
    }
}

/// What a registration did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registration {
    /// The value is now masked.
    Registered,
    /// It was already masked; its expiry moved to the later of the two.
    Refreshed,
    /// Shorter than `min_length` characters: not kept.
    TooShort,
}

struct Entry {
    value: SecretValue,
    expires_at: Instant,
}

#[derive(Default)]
struct Inner {
    entries: Vec<Entry>,
    matcher: Option<AhoCorasick>,
}

/// The process-wide secret set.
pub struct SecretSet {
    config: SecretSetConfig,
    inner: RwLock<Inner>,
}

impl SecretSet {
    /// An empty set.
    #[must_use]
    pub fn new(config: SecretSetConfig) -> Self {
        Self {
            config,
            inner: RwLock::new(Inner::default()),
        }
    }

    /// Masks `value` from now on, until `ttl` passes, or the configured retention when `None`.
    pub fn register(&self, value: SecretValue, ttl: Option<Duration>) -> Registration {
        if value.as_str().chars().count() < self.config.min_length {
            return Registration::TooShort;
        }
        let expires_at = Instant::now() + ttl.unwrap_or(self.config.retention);
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = inner
            .entries
            .iter_mut()
            .find(|entry| entry.value.as_str() == value.as_str())
        {
            entry.expires_at = entry.expires_at.max(expires_at);
            return Registration::Refreshed;
        }
        if inner.entries.len() >= self.config.max_entries {
            let first_to_expire = inner
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(index, _)| index);
            if let Some(index) = first_to_expire {
                inner.entries.swap_remove(index);
            }
        }
        inner.entries.push(Entry { value, expires_at });
        rebuild(&mut inner, self.config.min_length);
        Registration::Registered
    }

    /// Drops values whose time has passed. The writer calls this on its retention tick.
    pub fn expire(&self, now: Instant) -> usize {
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        let before = inner.entries.len();
        inner.entries.retain(|entry| entry.expires_at > now);
        let dropped = before - inner.entries.len();
        if dropped > 0 {
            rebuild(&mut inner, self.config.min_length);
        }
        dropped
    }

    /// How many values are masked.
    #[must_use]
    pub fn count(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .len()
    }

    /// Replaces every secret in `text` with `[private]`. Returns whether anything changed.
    pub fn mask_str(&self, text: &mut String) -> bool {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        let Some(matcher) = inner.matcher.as_ref() else {
            return false;
        };
        if !matcher.is_match(text.as_str()) {
            return false;
        }
        let mut masked = String::with_capacity(text.len());
        matcher.replace_all_with(text.as_str(), &mut masked, |_, _, out| {
            out.push_str(MASK);
            true
        });
        *text = masked;
        true
    }

    /// Masks every string in `value`, keys included. Returns whether anything changed.
    pub fn mask_json(&self, value: &mut Value) -> bool {
        match value {
            Value::String(text) => self.mask_str(text),
            // Every item is masked: stopping at the first change would leak the rest.
            Value::Array(items) => {
                let mut changed = false;
                for item in items.iter_mut() {
                    changed |= self.mask_json(item);
                }
                changed
            }
            Value::Object(map) => {
                let mut changed = false;
                let keys = map.keys().cloned().collect::<Vec<_>>();
                for key in keys {
                    let mut masked_key = key.clone();
                    let key_changed = self.mask_str(&mut masked_key);
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
}

/// Rebuilds the automaton over every form of every value. Leftmost-longest, so a value that
/// contains another is masked whole.
fn rebuild(inner: &mut Inner, min_length: usize) {
    let mut patterns = Vec::with_capacity(inner.entries.len() * 3);
    for entry in &inner.entries {
        let plain = entry.value.as_str();
        patterns.push(Zeroizing::new(plain.to_string()));
        if let Ok(quoted) = serde_json::to_string(plain) {
            let escaped = quoted
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .unwrap_or(&quoted);
            if escaped != plain {
                patterns.push(Zeroizing::new(escaped.to_string()));
            }
        }
        let component = utf8_percent_encode(plain, URI_COMPONENT).to_string();
        let form = utf8_percent_encode(plain, FORM)
            .to_string()
            .replace("%20", "+");
        for encoded in [component, form] {
            if encoded != plain && !patterns.iter().any(|known| **known == encoded) {
                patterns.push(Zeroizing::new(encoded));
            }
        }
        // A value the page shows only as text or in an attribute reads back from the HTML
        // serializer (`page.content()`, `innerHTML`) with `&`, `<`, `>`, U+00A0 and, in
        // attributes, `"` as entities. L1 masks these spellings only for a value still in a
        // field, so for one only the host or an earlier field knows, L2 is the last layer.
        for escaped in sensitive_fields::html_spellings(plain) {
            if *escaped != plain && !patterns.iter().any(|known| **known == *escaped) {
                patterns.push(escaped);
            }
        }
        // A download's name (`suggestedFilename()`, the records and the trace) spells the value
        // as Chromium sanitizes a file name, so a value in a name would match nothing above.
        for spelled in file_name_spellings(plain, min_length) {
            if !patterns.iter().any(|known| **known == *spelled) {
                patterns.push(spelled);
            }
        }
    }
    inner.matcher = if patterns.is_empty() {
        None
    } else {
        AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(patterns.iter().map(|pattern| pattern.as_str()))
            .map_err(|error| tracing::error!(%error, "secret matcher could not be built"))
            .ok()
    };
}

/// `value` as Chromium writes it into a download's name, where that differs from `value`. On
/// Linux, `net::GetSuggestedFilename` first trims every `.` from both ends of the name
/// (`SanitizeGeneratedFileName`), then `base::i18n::ReplaceIllegalCharactersInPath` replaces each
/// character illegal anywhere with `_`, and the name's first and last character too when white
/// space or `~`. The value may start or end the name, or not, so each case is a spelling.
///
/// Each spelling needs the whole value, every other character as typed, so a page that shows one
/// by chance is all but impossible; masking a long prefix of every value instead would catch
/// unrelated text that shares a common head (`https://`, a key's `sk-` or a JWT's `eyJ`). A
/// trimmed spelling shorter than `min_length` is left out, as a value that short is not kept.
fn file_name_spellings(value: &str, min_length: usize) -> Vec<Zeroizing<String>> {
    let mut spellings: Vec<Zeroizing<String>> = Vec::new();
    for (starts_name, ends_name) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut kept = value;
        if starts_name {
            kept = kept.trim_start_matches('.');
        }
        if ends_name {
            kept = kept.trim_end_matches('.');
        }
        if kept.chars().count() < min_length {
            continue;
        }
        let mut spelled = Zeroizing::new(String::with_capacity(kept.len()));
        for (at, character) in kept.char_indices() {
            let at_end =
                (starts_name && at == 0) || (ends_name && at + character.len_utf8() == kept.len());
            let replaced =
                illegal_in_file_name(character) || (at_end && illegal_at_file_name_ends(character));
            spelled.push(if replaced { '_' } else { character });
        }
        if *spelled != value && !spellings.contains(&spelled) {
            spellings.push(spelled);
        }
    }
    spellings
}

/// Unicode's format characters (general category Cf, Unicode 15.1), which Chromium's ICU set
/// for illegal file name characters includes. A format character added by a later Unicode
/// version is kept as typed here, so a value holding one gets no file name spelling.
const FORMAT_CHARACTERS: &[(u32, u32)] = &[
    (0x00ad, 0x00ad),
    (0x0600, 0x0605),
    (0x061c, 0x061c),
    (0x06dd, 0x06dd),
    (0x070f, 0x070f),
    (0x0890, 0x0891),
    (0x08e2, 0x08e2),
    (0x180e, 0x180e),
    (0x200b, 0x200f),
    (0x202a, 0x202e),
    (0x2060, 0x2064),
    (0x2066, 0x206f),
    (0xfeff, 0xfeff),
    (0xfff9, 0xfffb),
    (0x110bd, 0x110bd),
    (0x110cd, 0x110cd),
    (0x13430, 0x1343f),
    (0x1bca0, 0x1bca3),
    (0x1d173, 0x1d17a),
    (0xe0001, 0xe0001),
    (0xe0020, 0xe007f),
];

/// Whether Chromium replaces `character` anywhere in a file name: `"*/:<>?\|`, controls (Cc),
/// format characters (Cf) and the noncharacters.
fn illegal_in_file_name(character: char) -> bool {
    let code = u32::from(character);
    matches!(
        character,
        '"' | '*' | '/' | ':' | '<' | '>' | '?' | '\\' | '|'
    ) || character.is_control()
        || FORMAT_CHARACTERS
            .iter()
            .any(|&(low, high)| (low..=high).contains(&code))
        || (0xfdd0..=0xfdef).contains(&code)
        || code & 0xfffe == 0xfffe
}

/// Whether Chromium replaces `character` as a file name's first or last: white space, `.`, `~`.
fn illegal_at_file_name_ends(character: char) -> bool {
    character.is_whitespace() || matches!(character, '.' | '~')
}

#[cfg(test)]
mod tests {
    use super::{Registration, SecretSet, SecretSetConfig, SecretValue};
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn set_with(values: &[&str]) -> SecretSet {
        let set = SecretSet::new(SecretSetConfig::default());
        for value in values {
            set.register(SecretValue::new((*value).to_string()), None);
        }
        set
    }

    fn masked(set: &SecretSet, text: &str) -> String {
        let mut text = text.to_string();
        set.mask_str(&mut text);
        text
    }

    /// sdk-privacy leak test finding 2: a host-registered value shown only as page text came back
    /// raw from `page.content()`, which spells `&`, `<`, `>` and U+00A0 as entities in text and
    /// in attributes, and `"` in attributes.
    #[test]
    fn a_value_is_masked_as_the_html_serializer_spells_it_in_text_and_attributes()
    -> Result<(), Box<dyn std::error::Error>> {
        let set = set_with(&["p&ss<w\"rd>\u{a0}x"]);
        assert_eq!(
            masked(&set, "<p>p&amp;ss&lt;w\"rd&gt;&nbsp;x</p>"),
            "<p>[private]</p>"
        );
        assert_eq!(
            masked(&set, "<i data-v=\"p&amp;ss&lt;w&quot;rd&gt;&nbsp;x\"></i>"),
            "<i data-v=\"[private]\"></i>"
        );
        Ok(())
    }

    /// sdk-privacy leak test finding 3: a registered value in a download's name came back from
    /// `suggestedFilename()` as Chromium sanitizes a file name (`&<\"` became `&__`), and the
    /// part before the first replaced character leaked.
    #[test]
    fn a_value_is_masked_as_chromium_sanitizes_it_into_a_file_name()
    -> Result<(), Box<dyn std::error::Error>> {
        let set = set_with(&["hunter2&<\"pw:*?|/\\x"]);
        assert_eq!(
            masked(&set, "report-hunter2&__pw______x.pdf"),
            "report-[private].pdf"
        );
        let ends = set_with(&["~pass.word."]);
        assert_eq!(
            masked(&set_with(&["zw\u{200b}sp\u{feff}x"]), "a zw_sp_x b"),
            "a [private] b",
            "format characters are replaced too"
        );
        assert_eq!(
            masked(&ends, "_pass.word"),
            "[private]",
            "a whole name: its trailing dot trimmed, and its leading tilde replaced"
        );
        assert_eq!(
            masked(&ends, "_pass.word..pdf"),
            "[private].pdf",
            "a name it starts"
        );
        assert_eq!(
            masked(&set_with(&[".hunter2"]), "hunter2.pdf"),
            "[private].pdf",
            "a leading dot trimmed from the name"
        );
        assert_eq!(
            masked(&set_with(&[".abc"]), "abc.pdf"),
            "abc.pdf",
            "a trimmed spelling shorter than a kept value is not matched"
        );
        assert_eq!(
            masked(&ends, "x ~pass.word. y"),
            "x [private] y",
            "the plain spelling still masks"
        );
        Ok(())
    }

    #[test]
    fn a_value_is_masked_in_plain_json_escaped_and_percent_encoded_forms() {
        let set = set_with(&["p@ss w\"rd"]);
        assert_eq!(
            masked(&set, "typed p@ss w\"rd here"),
            "typed [private] here"
        );
        assert_eq!(
            masked(&set, r#"{"v":"p@ss w\"rd"}"#),
            r#"{"v":"[private]"}"#
        );
        assert_eq!(
            masked(&set, "https://x.example/?q=p%40ss%20w%22rd&x=1"),
            "https://x.example/?q=[private]&x=1"
        );
        assert_eq!(
            masked(&set, "q=p%40ss+w%22rd"),
            "q=[private]",
            "form encoding writes spaces as +"
        );
    }

    #[test]
    fn form_encoding_escapes_punctuation_url_components_keep() {
        let set = set_with(&["p!ss~word"]);
        assert_eq!(masked(&set, "a?pw=p!ss~word"), "a?pw=[private]");
        assert_eq!(
            masked(&set, "pw=p%21ss%7Eword&next=1"),
            "pw=[private]&next=1",
            "URLSearchParams escapes ! and ~"
        );
    }

    #[test]
    fn short_values_are_refused_and_repeats_refresh() {
        let set = SecretSet::new(SecretSetConfig::default());
        assert_eq!(
            set.register(SecretValue::new("abc".into()), None),
            Registration::TooShort
        );
        assert_eq!(
            set.register(SecretValue::new("abcd".into()), None),
            Registration::Registered
        );
        assert_eq!(
            set.register(SecretValue::new("abcd".into()), None),
            Registration::Refreshed
        );
        assert_eq!(set.count(), 1);
    }

    #[test]
    fn the_longest_overlapping_value_wins() {
        let set = set_with(&["4242", "4242424242424242"]);
        assert_eq!(masked(&set, "card 4242424242424242"), "card [private]");
    }

    #[test]
    fn values_expire_after_their_ttl() {
        let set = SecretSet::new(SecretSetConfig::default());
        set.register(
            SecretValue::new("one-time-123".into()),
            Some(Duration::from_secs(60)),
        );
        assert_eq!(set.expire(Instant::now()), 0);
        assert_eq!(masked(&set, "code one-time-123"), "code [private]");
        assert_eq!(set.expire(Instant::now() + Duration::from_secs(61)), 1);
        assert_eq!(masked(&set, "code one-time-123"), "code one-time-123");
    }

    #[test]
    fn a_refresh_keeps_the_later_expiry() {
        let set = SecretSet::new(SecretSetConfig::default());
        let minute = Some(Duration::from_secs(60));
        let hour = Duration::from_secs(60 * 60);
        set.register(SecretValue::new("day-then-minute".into()), None);
        set.register(SecretValue::new("day-then-minute".into()), minute);
        set.register(SecretValue::new("minute-then-day".into()), minute);
        set.register(SecretValue::new("minute-then-day".into()), None);
        assert_eq!(
            set.expire(Instant::now() + hour),
            0,
            "a shorter TTL did not cut the day short, and the day outlasted the minute"
        );
        assert_eq!(set.expire(Instant::now() + 25 * hour), 2);
    }

    #[test]
    fn a_value_without_a_ttl_is_masked_for_a_day() {
        let set = set_with(&["owner-typed-1"]);
        let hour = Duration::from_secs(60 * 60);
        assert_eq!(set.expire(Instant::now() + 23 * hour), 0);
        assert_eq!(masked(&set, "owner-typed-1"), "[private]");
        assert_eq!(set.expire(Instant::now() + 25 * hour), 1);
        assert_eq!(set.count(), 0);
    }

    #[test]
    fn json_strings_and_keys_are_masked_deeply() {
        let set = set_with(&["hunter22"]);
        let mut value = json!({"a": ["x hunter22", {"hunter22": 1}], "n": 5});
        assert!(set.mask_json(&mut value));
        assert_eq!(
            value,
            json!({"a": ["x [private]", {"[private]": 1}], "n": 5})
        );
    }

    #[test]
    fn an_empty_set_changes_nothing() {
        let set = SecretSet::new(SecretSetConfig::default());
        let mut text = "anything".to_string();
        assert!(!set.mask_str(&mut text));
        assert_eq!(format!("{:?}", SecretValue::new("x".into())), "[secret]");
    }

    #[test]
    fn the_oldest_value_goes_when_the_set_is_full() {
        let set = SecretSet::new(SecretSetConfig {
            max_entries: 2,
            ..SecretSetConfig::default()
        });
        for (value, ttl) in [
            ("first-value", 10),
            ("second-value", 100),
            ("third-value", 100),
        ] {
            set.register(
                SecretValue::new(value.into()),
                Some(Duration::from_secs(ttl)),
            );
        }
        assert_eq!(set.count(), 2);
        assert_eq!(masked(&set, "first-value"), "first-value");
        assert_eq!(masked(&set, "third-value"), "[private]");
    }
}
