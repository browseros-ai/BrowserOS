//! The one process-wide set of secret values Neo masks wherever text leaves it: op results into
//! either JS realm, error messages, console lines, the model-facing run result (the skeleton's
//! `Masker` calls [`SecretSet::mask_str`] and [`SecretSet::mask_json`]), and every trace record.
//!
//! sdk-extensions feeds it: each protected-fill value before its first keystroke, and
//! `POST /host/v1/secrets`. Takeover and the control display feed it what the owner typed into
//! private fields. Values live in memory only, never on disk. Each [`SecretValue`] is zeroed
//! when dropped; the matcher keeps its own copies, which are freed but not zeroed. One set
//! masks every session, because all sessions share one browser and can read each other's tabs.
//! The scope tag only decides expiry.
//!
//! Each value is matched in its plain, JSON-escaped and percent-encoded forms, with one
//! Aho-Corasick automaton rebuilt on every change. Changes are rare (one per fill); masks run on
//! every result.

use crate::SessionId;
use aho_corasick::{AhoCorasick, MatchKind};
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

/// Which session a value came from. It decides expiry only; every value masks every session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretScope {
    /// Registered for one session: a fill, a host registration, a takeover.
    Session(SessionId),
    /// Collected from the whole browser: the control display.
    Browser,
}

/// `[secrets]` in `neo.toml`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecretSetConfig {
    /// Shorter values are refused: they would mask ordinary text (zuzu's rule).
    pub min_length: usize,
    /// How long a value is masked when its registration names no TTL.
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
    scope: SecretScope,
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

    /// Masks `value` from now on, until `ttl` (or the configured retention) passes.
    pub fn register(
        &self,
        value: SecretValue,
        scope: SecretScope,
        ttl: Option<Duration>,
    ) -> Registration {
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
        inner.entries.push(Entry {
            value,
            scope,
            expires_at,
        });
        rebuild(&mut inner);
        Registration::Registered
    }

    /// Drops values whose time has passed. The writer calls this on its retention tick.
    pub fn expire(&self, now: Instant) -> usize {
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        let before = inner.entries.len();
        inner.entries.retain(|entry| entry.expires_at > now);
        let dropped = before - inner.entries.len();
        if dropped > 0 {
            rebuild(&mut inner);
        }
        dropped
    }

    /// How many values are masked, and how many of them came from `scope`.
    #[must_use]
    pub fn count(&self, scope: Option<SecretScope>) -> usize {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        match scope {
            None => inner.entries.len(),
            Some(scope) => inner
                .entries
                .iter()
                .filter(|entry| entry.scope == scope)
                .count(),
        }
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
fn rebuild(inner: &mut Inner) {
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

#[cfg(test)]
mod tests {
    use super::{Registration, SecretScope, SecretSet, SecretSetConfig, SecretValue};
    use crate::SessionId;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn set_with(values: &[&str]) -> SecretSet {
        let set = SecretSet::new(SecretSetConfig::default());
        for value in values {
            set.register(
                SecretValue::new((*value).to_string()),
                SecretScope::Browser,
                None,
            );
        }
        set
    }

    fn masked(set: &SecretSet, text: &str) -> String {
        let mut text = text.to_string();
        set.mask_str(&mut text);
        text
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
        let session = SecretScope::Session(SessionId::new());
        assert_eq!(
            set.register(SecretValue::new("abc".into()), session, None),
            Registration::TooShort
        );
        assert_eq!(
            set.register(SecretValue::new("abcd".into()), session, None),
            Registration::Registered
        );
        assert_eq!(
            set.register(SecretValue::new("abcd".into()), SecretScope::Browser, None),
            Registration::Refreshed
        );
        assert_eq!(set.count(None), 1);
        assert_eq!(set.count(Some(session)), 1);
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
            SecretScope::Browser,
            Some(Duration::from_secs(60)),
        );
        assert_eq!(set.expire(Instant::now()), 0);
        assert_eq!(masked(&set, "code one-time-123"), "code [private]");
        assert_eq!(set.expire(Instant::now() + Duration::from_secs(61)), 1);
        assert_eq!(masked(&set, "code one-time-123"), "code one-time-123");
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
                SecretScope::Browser,
                Some(Duration::from_secs(ttl)),
            );
        }
        assert_eq!(set.count(None), 2);
        assert_eq!(masked(&set, "first-value"), "first-value");
        assert_eq!(masked(&set, "third-value"), "[private]");
    }
}
