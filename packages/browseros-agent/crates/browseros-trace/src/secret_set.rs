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
//! The set holds `max_entries` values, and each keeps its [`SecretSource`], which decides what a
//! full set drops for it (sdk-traces decision 53). A page decides what its fields hold, so its
//! sweeps can report any number of values, and an agent can be talked into filling any number.
//! So a page value pushes out only page values, oldest first, and an agent value only page
//! values; with none to push out, either is refused ([`Registration::Full`]) and its caller must
//! withhold what could hold it. Host and owner values come at a rate people bound: they push out
//! a page value when there is one, and are otherwise kept past the cap, never refused.
//!
//! Each value is matched in its plain, JSON-escaped and percent-encoded forms, with one
//! Aho-Corasick automaton rebuilt on every change. Changes are rare (one per fill); masks run on
//! every result. A set a record carries (sdk-traces decision 43) is made whole for each record,
//! so it is built at once with [`SecretSet::with_values`], in one build over all its values
//! rather than one per value.

use aho_corasick::{AhoCorasick, MatchKind};
use browseros_core::sensitive_fields;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;
use std::ops::Range;
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
    /// The most values kept, but for host and owner values, which may pass it. What a full set
    /// drops, or refuses, is [`SecretSet::register_from`]'s.
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

/// Who registered a value. It decides what a full set drops for the value, and whether the set
/// may refuse it ([`SecretSet::register_from`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretSource {
    /// The host, through `POST /v1/secrets`.
    Host,
    /// The owner, typing into a sensitive field while they control a tab: the control journal.
    Owner,
    /// The agent, typing or filling.
    Agent,
    /// A page, as a sweep read its fields.
    Page,
}

impl SecretSource {
    /// Host and owner values outrank agent values, which outrank page values. A value
    /// registered again keeps the higher of its sources.
    fn rank(self) -> u8 {
        match self {
            Self::Page => 0,
            Self::Agent => 1,
            Self::Host | Self::Owner => 2,
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
    /// Not kept: the set is full and holds nothing a value from this source may push out. Only
    /// agent and page values are refused. The value is masked nowhere, so its caller must
    /// withhold whatever could hold it.
    Full,
}

struct Entry {
    value: SecretValue,
    expires_at: Instant,
    /// The highest-ranked source that registered the value.
    source: SecretSource,
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

/// How many values it holds, never the values: a trace event that carries a set derives
/// `Debug`, and a log of one must not print a secret.
impl fmt::Debug for SecretSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretSet")
            .field("values", &self.count())
            .finish_non_exhaustive()
    }
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

    /// A set masking `values` from now on, built at once, for a record's or a run's own set such
    /// as the values claw-sdk still holds as it sends a record. It equals [`SecretSet::new`] then
    /// registering each value in order as a page value with no TTL ([`SecretSet::register_from`]):
    /// too short ones refused, a repeat kept once, the oldest dropped past `max_entries`. But it
    /// builds the matcher once, where each registration rebuilds it over every value it holds.
    #[must_use]
    pub fn with_values(
        config: SecretSetConfig,
        values: impl IntoIterator<Item = SecretValue>,
    ) -> Self {
        let values: Vec<SecretValue> = values
            .into_iter()
            .filter(|value| value.as_str().chars().count() >= config.min_length)
            .collect();
        let kept = kept_as_registered(&values, config.max_entries);
        let mut inner = Inner {
            entries: in_registration_order(values, &kept, Instant::now() + config.retention),
            matcher: None,
        };
        rebuild(&mut inner, config.min_length);
        Self {
            config,
            inner: RwLock::new(inner),
        }
    }

    /// Masks `value` from now on, until `ttl` passes, or the configured retention when `None`.
    ///
    /// In a full set, a value whose time has passed goes first, whatever its source, then the
    /// page value that expires first. With neither, a host or owner value is kept past
    /// `max_entries`, and an agent or page value is refused with [`Registration::Full`]. A
    /// refusal changes nothing, so a flood of refused values costs no matcher build. A value
    /// registered again keeps the later expiry and the higher-ranked source.
    #[must_use = "a refused value is masked nowhere, so its caller must withhold what could hold it"]
    pub fn register_from(
        &self,
        value: SecretValue,
        ttl: Option<Duration>,
        source: SecretSource,
    ) -> Registration {
        if value.as_str().chars().count() < self.config.min_length {
            return Registration::TooShort;
        }
        let now = Instant::now();
        let expires_at = now + ttl.unwrap_or(self.config.retention);
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = inner
            .entries
            .iter_mut()
            .find(|entry| entry.value.as_str() == value.as_str())
        {
            entry.expires_at = entry.expires_at.max(expires_at);
            if source.rank() > entry.source.rank() {
                entry.source = source;
            }
            return Registration::Refreshed;
        }
        match room(&inner.entries, source, self.config.max_entries, now) {
            Room::Add => {}
            Room::Replace(index) => {
                inner.entries.swap_remove(index);
            }
            Room::Refuse => return Registration::Full,
        }
        inner.entries.push(Entry {
            value,
            expires_at,
            source,
        });
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

    /// How many values are masked from `source`, for reports and checks, such as which tier a
    /// flood grew. Each value counts once, under the source the set keeps for it: the
    /// highest-ranked that registered it ([`SecretSet::register_from`]).
    #[must_use]
    pub fn count_from(&self, source: SecretSource) -> usize {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .iter()
            .filter(|entry| entry.source == source)
            .count()
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
        mask_json_with(value, &mut |text| self.mask_str(text))
    }

    /// Adds to `spans` where `text` holds a value of this set: the byte ranges
    /// [`SecretSet::mask_str`] would replace. Masking with several sets one after another
    /// destroys what a later set must match: a value one set masks leaves only the tail of a
    /// longer value another set holds. So a record masked with several sets takes every set's
    /// spans on the same text and masks their union ([`mask_spans`]).
    pub(crate) fn spans(&self, text: &str, spans: &mut Vec<Range<usize>>) {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(matcher) = inner.matcher.as_ref() {
            spans.extend(matcher.find_iter(text).map(|found| found.range()));
        }
    }
}

/// `text` with each run of overlapping `spans` replaced by one [`MASK`], or `None` when there
/// are none. Spans may come from several sets, in any order. Spans that only touch stay
/// separate, as one set's adjacent matches do.
pub(crate) fn mask_spans(text: &str, mut spans: Vec<Range<usize>>) -> Option<String> {
    if spans.is_empty() {
        return None;
    }
    spans.sort_unstable_by_key(|span| span.start);
    let mut masked = String::with_capacity(text.len());
    let mut copied = 0;
    let mut current: Option<Range<usize>> = None;
    for span in spans {
        match &mut current {
            Some(open) if span.start < open.end => open.end = open.end.max(span.end),
            _ => {
                if let Some(done) = current.replace(span) {
                    masked.push_str(&text[copied..done.start]);
                    masked.push_str(MASK);
                    copied = done.end;
                }
            }
        }
    }
    if let Some(done) = current {
        masked.push_str(&text[copied..done.start]);
        masked.push_str(MASK);
        copied = done.end;
    }
    masked.push_str(&text[copied..]);
    Some(masked)
}

/// Masks every string in `value`, keys included, with `mask`, which returns whether it changed
/// its text. Returns whether anything changed.
pub(crate) fn mask_json_with(
    value: &mut Value,
    mask: &mut impl FnMut(&mut String) -> bool,
) -> bool {
    match value {
        Value::String(text) => mask(text),
        // Every item is masked: stopping at the first change would leak the rest.
        Value::Array(items) => {
            let mut changed = false;
            for item in items.iter_mut() {
                changed |= mask_json_with(item, mask);
            }
            changed
        }
        Value::Object(map) => {
            let mut changed = false;
            let keys = map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                let mut masked_key = key.clone();
                let key_changed = mask(&mut masked_key);
                if let Some(mut child) = map.remove(&key) {
                    changed |= mask_json_with(&mut child, mask);
                    map.insert(masked_key, child);
                }
                changed |= key_changed;
            }
            changed
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

/// Where a new value goes in a set: what [`room`] decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Room {
    /// Beside the others: the set is under its cap, or nothing may go and the value is a
    /// host's or the owner's, which passes the cap.
    Add,
    /// In place of the entry at this index.
    Replace(usize),
    /// Nowhere: nothing may go, and the value is an agent's or a page's.
    Refuse,
}

/// Where a value from `source` goes in a set holding `entries`, the one place a full set
/// chooses what to drop (sdk-traces decision 53). A value whose time has passed may go for any
/// value, and so may a page value; the one that expires first goes, which is an expired value
/// when there is one, as nothing live expires before it. No live host, owner or agent value is
/// ever dropped for another value, so a flood can push out only page values.
fn room(entries: &[Entry], source: SecretSource, max_entries: usize, now: Instant) -> Room {
    if entries.len() < max_entries {
        return Room::Add;
    }
    let goes_first = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.expires_at <= now || entry.source == SecretSource::Page)
        .min_by_key(|(_, entry)| entry.expires_at)
        .map(|(index, _)| index);
    match (goes_first, source) {
        (Some(index), _) => Room::Replace(index),
        (None, SecretSource::Host | SecretSource::Owner) => Room::Add,
        (None, SecretSource::Agent | SecretSource::Page) => Room::Refuse,
    }
}

/// Which of `values`, in registration order, registering them one by one as page values with
/// no TTL keeps. A repeat moves its expiry to the later one, and past the cap the page value
/// that expires first goes, so walking from the newest, each value is kept where it was last
/// registered until the cap is full. A cap of 0 keeps none: a page value finds nothing to push
/// out, and is refused.
fn kept_as_registered(values: &[SecretValue], max_entries: usize) -> Vec<bool> {
    let mut kept = vec![false; values.len()];
    let mut seen = HashSet::new();
    let mut room = max_entries;
    for (index, value) in values.iter().enumerate().rev() {
        if room == 0 {
            break;
        }
        if seen.insert(value.as_str()) {
            kept[index] = true;
            room -= 1;
        }
    }
    kept
}

/// The `kept` values as page entries, each expiring a nanosecond after the one registered
/// before it (the first at `first_expiry`), as values registered one by one do. A later
/// registration past the cap drops the page value that expires first, so the set then loses
/// its values oldest first, however many evictions follow; with one expiry for all, its
/// `swap_remove` would scramble that order.
fn in_registration_order(
    values: Vec<SecretValue>,
    kept: &[bool],
    first_expiry: Instant,
) -> Vec<Entry> {
    let mut expires_at = first_expiry;
    let mut entries = Vec::new();
    for (value, _) in values.into_iter().zip(kept).filter(|(_, kept)| **kept) {
        entries.push(Entry {
            value,
            expires_at,
            source: SecretSource::Page,
        });
        expires_at += Duration::from_nanos(1);
    }
    entries
}

/// Rebuilds the automaton over every form of every value. Leftmost-longest, so a value that
/// contains another is masked whole.
fn rebuild(inner: &mut Inner, min_length: usize) {
    let mut patterns = Vec::with_capacity(inner.entries.len() * 3);
    for entry in &inner.entries {
        let plain = entry.value.as_str();
        // A value's spellings are compared with its own only. Two values may share one, which
        // the matcher takes twice, and comparing with every value's would make a build quadratic
        // in the values.
        let first = patterns.len();
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
            if encoded != plain && !patterns[first..].iter().any(|known| **known == encoded) {
                patterns.push(Zeroizing::new(encoded));
            }
        }
        // A value the page shows only as text or in an attribute reads back from the HTML
        // serializer (`page.content()`, `innerHTML`) with `&`, `<`, `>`, U+00A0 and, in
        // attributes, `"` as entities. L1 masks these spellings only for a value still in a
        // field, so for one only the host or an earlier field knows, L2 is the last layer.
        for escaped in sensitive_fields::html_spellings(plain) {
            if *escaped != plain && !patterns[first..].iter().any(|known| **known == *escaped) {
                patterns.push(escaped);
            }
        }
        // A download's name (`suggestedFilename()`, the records and the trace) spells the value
        // as Chromium sanitizes a file name, so a value in a name would match nothing above.
        for spelled in file_name_spellings(plain, min_length) {
            if !patterns[first..].iter().any(|known| **known == *spelled) {
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
    use super::{
        MASK, Registration, SecretSet, SecretSetConfig, SecretSource, SecretValue, mask_spans,
    };
    use aho_corasick::AhoCorasick;
    use serde_json::json;
    use std::sync::PoisonError;
    use std::time::{Duration, Instant};

    fn set_with(values: &[&str]) -> SecretSet {
        let set = SecretSet::new(SecretSetConfig::default());
        for value in values {
            let _registration = set.register_from(
                SecretValue::new((*value).to_string()),
                None,
                SecretSource::Page,
            );
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
            set.register_from(SecretValue::new("abc".into()), None, SecretSource::Page),
            Registration::TooShort
        );
        assert_eq!(
            set.register_from(SecretValue::new("abcd".into()), None, SecretSource::Page),
            Registration::Registered
        );
        assert_eq!(
            set.register_from(SecretValue::new("abcd".into()), None, SecretSource::Page),
            Registration::Refreshed
        );
        assert_eq!(set.count(), 1);
    }

    /// The form spelling of `a b c d` is the other value as typed, so the matcher holds that
    /// pattern twice, whichever value comes first.
    #[test]
    fn values_that_share_a_spelling_are_each_masked() {
        for values in [["a b c d", "a+b+c+d"], ["a+b+c+d", "a b c d"]] {
            assert_eq!(
                masked(&set_with(&values), "q=a+b+c+d&r=a b c d"),
                "q=[private]&r=[private]",
                "{values:?}"
            );
        }
    }

    #[test]
    fn the_longest_overlapping_value_wins() {
        let set = set_with(&["4242", "4242424242424242"]);
        assert_eq!(masked(&set, "card 4242424242424242"), "card [private]");
    }

    #[test]
    fn values_expire_after_their_ttl() {
        let set = SecretSet::new(SecretSetConfig::default());
        let _registration = set.register_from(
            SecretValue::new("one-time-123".into()),
            Some(Duration::from_secs(60)),
            SecretSource::Page,
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
        let _registration = set.register_from(
            SecretValue::new("day-then-minute".into()),
            None,
            SecretSource::Page,
        );
        let _registration = set.register_from(
            SecretValue::new("day-then-minute".into()),
            minute,
            SecretSource::Page,
        );
        let _registration = set.register_from(
            SecretValue::new("minute-then-day".into()),
            minute,
            SecretSource::Page,
        );
        let _registration = set.register_from(
            SecretValue::new("minute-then-day".into()),
            None,
            SecretSource::Page,
        );
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
            let _registration = set.register_from(
                SecretValue::new(value.into()),
                Some(Duration::from_secs(ttl)),
                SecretSource::Page,
            );
        }
        assert_eq!(set.count(), 2);
        assert_eq!(masked(&set, "first-value"), "first-value");
        assert_eq!(masked(&set, "third-value"), "[private]");
    }

    /// sdk-traces decision 53: a page reports as many values as its DOM holds, so a flood of
    /// page values past the cap must not unmask what the host registered, the owner typed or
    /// the agent filled. The page values go oldest first.
    #[test]
    fn host_owner_and_agent_values_outlast_a_flood_of_page_values() {
        let set = capped(16);
        hold(&set, &PROTECTED, "the protected values");
        let flood = flood(&set, 20);
        for (value, ..) in PROTECTED {
            assert_masked(&set, value, true, "the flood pushed it out");
        }
        assert_eq!(set.count(), 16);
        let first_kept = flood.len() - (16 - PROTECTED.len());
        for (index, value) in flood.iter().enumerate() {
            assert_masked(
                &set,
                value,
                index >= first_kept,
                "the oldest page values go first",
            );
        }
    }

    /// One value each from the host, the owner and the agent, with no TTL.
    const PROTECTED: [(&str, SecretSource, Option<u64>); 3] = [
        ("host-secret-1", SecretSource::Host, None),
        ("owner-typed-2", SecretSource::Owner, None),
        ("agent-filled-3", SecretSource::Agent, None),
    ];

    /// Registers `count` page values, oldest first, and returns them.
    fn flood(set: &SecretSet, count: usize) -> Vec<String> {
        let values: Vec<String> = (0..count).map(|i| format!("page-flood-{i:02}")).collect();
        for value in &values {
            let registration =
                set.register_from(SecretValue::new(value.clone()), None, SecretSource::Page);
            assert_eq!(registration, Registration::Registered, "{value}");
        }
        values
    }

    /// A TTL already over, so the value's time has passed by the next registration.
    const PASSED: Option<u64> = Some(0);
    const MINUTE: Option<u64> = Some(60);
    /// The set's retention, a day.
    const DAY: Option<u64> = None;

    /// One registration into a full set.
    struct FullSet {
        what: &'static str,
        max_entries: usize,
        /// Registered first, in order: each value, its source and its TTL in seconds.
        held: &'static [(&'static str, SecretSource, Option<u64>)],
        new: (&'static str, SecretSource),
        registration: Registration,
        /// What is masked afterwards, of the values held and the new one. No other is.
        masked: &'static [&'static str],
    }

    const FULL_SETS: &[FullSet] = &[
        FullSet {
            what: "a page value pushes out the page value that expires first, not a host value",
            max_entries: 3,
            held: &[
                ("page-older-1", SecretSource::Page, DAY),
                ("host-minute-2", SecretSource::Host, MINUTE),
                ("page-newer-3", SecretSource::Page, DAY),
            ],
            new: ("page-new-4", SecretSource::Page),
            registration: Registration::Registered,
            masked: &["host-minute-2", "page-newer-3", "page-new-4"],
        },
        FullSet {
            what: "a page value is refused when every value is protected",
            max_entries: 3,
            held: &[
                ("host-held-1", SecretSource::Host, MINUTE),
                ("owner-held-2", SecretSource::Owner, MINUTE),
                ("agent-held-3", SecretSource::Agent, MINUTE),
            ],
            new: ("page-new-4", SecretSource::Page),
            registration: Registration::Full,
            masked: &["host-held-1", "owner-held-2", "agent-held-3"],
        },
        FullSet {
            what: "an agent value pushes out a page value, not an agent value",
            max_entries: 2,
            held: &[
                ("agent-minute-1", SecretSource::Agent, MINUTE),
                ("page-held-2", SecretSource::Page, DAY),
            ],
            new: ("agent-new-3", SecretSource::Agent),
            registration: Registration::Registered,
            masked: &["agent-minute-1", "agent-new-3"],
        },
        FullSet {
            what: "an agent value is refused when every value is protected",
            max_entries: 2,
            held: &[
                ("agent-held-1", SecretSource::Agent, DAY),
                ("owner-held-2", SecretSource::Owner, DAY),
            ],
            new: ("agent-new-3", SecretSource::Agent),
            registration: Registration::Full,
            masked: &["agent-held-1", "owner-held-2"],
        },
        FullSet {
            what: "a host value pushes out a page value when there is one",
            max_entries: 2,
            held: &[
                ("agent-minute-1", SecretSource::Agent, MINUTE),
                ("page-held-2", SecretSource::Page, DAY),
            ],
            new: ("host-new-3", SecretSource::Host),
            registration: Registration::Registered,
            masked: &["agent-minute-1", "host-new-3"],
        },
        FullSet {
            what: "a host value is kept past the cap when no value may go",
            max_entries: 2,
            held: &[
                ("agent-held-1", SecretSource::Agent, DAY),
                ("owner-held-2", SecretSource::Owner, DAY),
            ],
            new: ("host-new-3", SecretSource::Host),
            registration: Registration::Registered,
            masked: &["agent-held-1", "owner-held-2", "host-new-3"],
        },
        FullSet {
            what: "an owner value is kept past the cap when no value may go",
            max_entries: 2,
            held: &[
                ("host-held-1", SecretSource::Host, DAY),
                ("agent-held-2", SecretSource::Agent, DAY),
            ],
            new: ("owner-new-3", SecretSource::Owner),
            registration: Registration::Registered,
            masked: &["host-held-1", "agent-held-2", "owner-new-3"],
        },
        FullSet {
            what: "a page value is refused from a set host and owner values took past the cap",
            max_entries: 1,
            held: &[
                ("host-held-1", SecretSource::Host, DAY),
                ("owner-held-2", SecretSource::Owner, DAY),
            ],
            new: ("page-new-3", SecretSource::Page),
            registration: Registration::Full,
            masked: &["host-held-1", "owner-held-2"],
        },
        FullSet {
            what: "a value whose time has passed goes before a page value, whatever its source",
            max_entries: 2,
            held: &[
                ("agent-passed-1", SecretSource::Agent, PASSED),
                ("page-held-2", SecretSource::Page, DAY),
            ],
            new: ("page-new-3", SecretSource::Page),
            registration: Registration::Registered,
            masked: &["page-held-2", "page-new-3"],
        },
        FullSet {
            what: "a host value whose time has passed makes room for an agent value",
            max_entries: 2,
            held: &[
                ("host-passed-1", SecretSource::Host, PASSED),
                ("owner-held-2", SecretSource::Owner, DAY),
            ],
            new: ("agent-new-3", SecretSource::Agent),
            registration: Registration::Registered,
            masked: &["owner-held-2", "agent-new-3"],
        },
    ];

    #[test]
    fn a_full_set_drops_and_refuses_values_by_their_source() {
        for case in FULL_SETS {
            let set = capped(case.max_entries);
            hold(&set, case.held, case.what);
            let (new, source) = case.new;
            let registration = set.register_from(SecretValue::new(new.into()), None, source);
            assert_eq!(registration, case.registration, "{}", case.what);
            for value in case.held.iter().map(|(value, ..)| *value).chain([new]) {
                assert_masked(&set, value, case.masked.contains(&value), case.what);
            }
            assert_eq!(set.count(), case.masked.len(), "{}", case.what);
        }
    }

    /// An empty set that keeps at most `max_entries` values.
    fn capped(max_entries: usize) -> SecretSet {
        SecretSet::new(SecretSetConfig {
            max_entries,
            ..SecretSetConfig::default()
        })
    }

    /// Registers each value from its source, with its TTL in seconds, and checks it was added.
    fn hold(set: &SecretSet, values: &[(&str, SecretSource, Option<u64>)], what: &str) {
        for &(value, source, ttl) in values {
            let ttl = ttl.map(Duration::from_secs);
            let registration = set.register_from(SecretValue::new(value.into()), ttl, source);
            assert_eq!(registration, Registration::Registered, "{what}: {value}");
        }
    }

    /// Checks that `value` reads as `[private]` in `set` when `is_masked`, and as itself when not.
    fn assert_masked(set: &SecretSet, value: &str, is_masked: bool, what: &str) {
        let expected = if is_masked { MASK } else { value };
        assert_eq!(masked(set, value), expected, "{what}: {value}");
    }

    /// A value keeps the highest-ranked source that registered it: a page value the host
    /// registers again is the host's, and a host value a sweep reads again stays the host's.
    #[test]
    fn a_value_registered_again_keeps_its_highest_source_through_a_flood() {
        let set = SecretSet::new(SecretSetConfig {
            max_entries: 4,
            ..SecretSetConfig::default()
        });
        let values = [
            ("page-then-host", [SecretSource::Page, SecretSource::Host]),
            ("host-then-page", [SecretSource::Host, SecretSource::Page]),
        ];
        for (value, [first, again]) in values {
            assert_eq!(
                set.register_from(SecretValue::new(value.into()), None, first),
                Registration::Registered
            );
            assert_eq!(
                set.register_from(SecretValue::new(value.into()), None, again),
                Registration::Refreshed
            );
        }
        for i in 0..8 {
            assert_eq!(
                set.register_from(
                    SecretValue::new(format!("page-flood-{i}")),
                    None,
                    SecretSource::Page
                ),
                Registration::Registered
            );
        }
        for (value, _) in values {
            assert_eq!(masked(&set, value), MASK, "the flood pushed {value} out");
        }
        assert_eq!(set.count(), 4);
    }

    #[test]
    fn count_from_counts_each_value_once_under_its_highest_source() {
        let set = capped(16);
        hold(
            &set,
            &PROTECTED,
            "one value from the host, the owner and the agent",
        );
        let values = [
            ("page-then-owner", [SecretSource::Page, SecretSource::Owner]),
            ("owner-then-page", [SecretSource::Owner, SecretSource::Page]),
        ];
        for (value, [first, again]) in values {
            assert_eq!(
                set.register_from(SecretValue::new(value.into()), None, first),
                Registration::Registered
            );
            assert_eq!(
                set.register_from(SecretValue::new(value.into()), None, again),
                Registration::Refreshed
            );
        }
        flood(&set, 2);
        let sources = [
            SecretSource::Host,
            SecretSource::Owner,
            SecretSource::Agent,
            SecretSource::Page,
        ];
        assert_eq!(
            sources.map(|source| set.count_from(source)),
            [1, 3, 1, 2],
            "host, owner, agent and page values"
        );
        assert_eq!(set.count(), 7, "a value registered twice counts once");
    }

    /// Values in the order they are registered: a repeat of an early value, one too short to
    /// keep, one inside another, two whose JSON, percent, HTML and file-name spellings differ,
    /// and two more than a set of 5 keeps.
    const IN_ORDER: [&str; 9] = [
        "dropped-first-1",
        "hunter22",
        "abc",
        "dropped-second-2",
        "4242",
        "p@ss w\"rd",
        "hunter22",
        "4242424242424242",
        "a&b<c>d\u{a0}x",
    ];

    /// Texts holding each value of [`IN_ORDER`], as typed and in each spelling a mask looks for.
    const HOLDING: [&str; 16] = [
        "dropped-first-1",
        "dropped-second-2",
        "abc",
        "pin 4242 ok",
        "card 4242424242424242",
        "typed hunter22 here",
        "typed p@ss w\"rd here",
        r#"{"v":"p@ss w\"rd"}"#,
        "https://x.example/?q=p%40ss%20w%22rd&x=1",
        "q=p%40ss+w%22rd",
        "<i data-v=\"p@ss w&quot;rd\"></i>",
        "report-p@ss w_rd.pdf",
        "a&b<c>d\u{a0}x",
        "<p>a&amp;b&lt;c&gt;d&nbsp;x</p>",
        "u=a%26b%3Cc%3Ed%C2%A0x",
        "a&b_c_d\u{a0}x.pdf",
    ];

    fn registered_in_order(config: SecretSetConfig) -> SecretSet {
        let set = SecretSet::new(config);
        for value in IN_ORDER {
            let _registration = set.register_from(
                SecretValue::new(value.to_string()),
                None,
                SecretSource::Page,
            );
        }
        set
    }

    fn built_at_once(config: SecretSetConfig) -> SecretSet {
        SecretSet::with_values(
            config,
            IN_ORDER.map(|value| SecretValue::new(value.to_string())),
        )
    }

    /// Both sets have the same values, and mask every text that holds one, and a JSON document,
    /// alike.
    fn assert_masks_alike(at_once: &SecretSet, one_by_one: &SecretSet, max_entries: usize) {
        assert_eq!(at_once.count(), one_by_one.count(), "max {max_entries}");
        let everything = HOLDING.join(" | ");
        for text in HOLDING.into_iter().chain([everything.as_str()]) {
            let (by_once, by_one) = (masked(at_once, text), masked(one_by_one, text));
            assert_eq!(by_once, by_one, "max {max_entries}: {text}");
        }
        let document = json!({
            "p@ss w\"rd": ["hunter22", "pin 4242 ok", "dropped-first-1"],
            "a&amp;b&lt;c&gt;d&nbsp;x": {"dropped-second-2": "card 4242424242424242"},
        });
        let (mut by_one, mut by_once) = (document.clone(), document);
        let changed = (
            at_once.mask_json(&mut by_once),
            one_by_one.mask_json(&mut by_one),
        );
        assert_eq!(changed.0, changed.1, "max {max_entries}");
        assert_eq!(by_once, by_one, "max {max_entries}");
    }

    #[test]
    fn a_set_built_at_once_masks_what_registering_its_values_in_order_masks() {
        for max_entries in [0, 1, 5, SecretSetConfig::default().max_entries] {
            let config = SecretSetConfig {
                max_entries,
                ..SecretSetConfig::default()
            };
            let one_by_one = registered_in_order(config);
            assert_masks_alike(&built_at_once(config), &one_by_one, max_entries);
        }
    }

    /// Past a cap of 5, the values registered last are kept, a repeat counting from its last
    /// registration, and a value too short is never kept.
    #[test]
    fn a_set_built_at_once_keeps_the_values_registered_last() {
        let five = built_at_once(SecretSetConfig {
            max_entries: 5,
            ..SecretSetConfig::default()
        });
        assert_eq!(five.count(), 5);
        assert_eq!(
            masked(&five, "dropped-first-1 dropped-second-2"),
            "dropped-first-1 dropped-second-2",
            "the two registered first went"
        );
        assert_eq!(
            masked(&five, "typed hunter22 here"),
            "typed [private] here",
            "a repeat counts from its last registration, so it outlived both"
        );
        assert_eq!(masked(&five, "abc"), "abc", "too short to keep");
        assert_eq!(
            masked(&five, "card 4242424242424242 pin 4242"),
            "card [private] pin [private]"
        );
    }

    /// A set built at once drops its oldest value first when later registrations pass the cap,
    /// as a set registered one by one does, however many evictions follow.
    #[test]
    fn a_set_built_at_once_drops_its_oldest_first_as_more_values_come() {
        let config = SecretSetConfig {
            max_entries: 3,
            ..SecretSetConfig::default()
        };
        let first = ["alpha-1", "bravo-2", "charlie-3"].map(|v| SecretValue::new(v.into()));
        let at_once = SecretSet::with_values(config, first);
        for later in ["delta-4", "echo-5"] {
            let _registration =
                at_once.register_from(SecretValue::new(later.into()), None, SecretSource::Page);
        }
        assert_eq!(
            masked(&at_once, "alpha-1 bravo-2 charlie-3 delta-4 echo-5"),
            "alpha-1 bravo-2 [private] [private] [private]",
            "the two oldest went, in the order they came"
        );
    }

    /// `count` distinct values of 16 to 32 characters, shaped like passwords and tokens. They are
    /// letters and digits, with about one character in ten a punctuation mark that a URL, JSON,
    /// HTML or a file name spells otherwise. The seed is fixed, so every run builds the same
    /// values.
    fn token_like(count: usize) -> Vec<String> {
        const PLAIN: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        const PUNCTUATION: &[u8] = b"!@#$%^&*-_.~+=?/";
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        // xorshift64*
        let mut below = move |bound: usize| {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            usize::try_from(state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 32).unwrap_or(0) % bound
        };
        (0..count)
            .map(|_| {
                (0..16 + below(17))
                    .map(|_| {
                        let pool = if below(10) == 0 { PUNCTUATION } else { PLAIN };
                        char::from(pool[below(pool.len())])
                    })
                    .collect()
            })
            .collect()
    }

    /// Median and range of `times`.
    fn spread(times: &mut [Duration]) -> String {
        times.sort_unstable();
        let (Some(fastest), Some(slowest)) = (times.first(), times.last()) else {
            return "no runs".to_string();
        };
        format!(
            "median {:?}, range {fastest:?} to {slowest:?}",
            times[times.len() / 2]
        )
    }

    /// What a set of `count` values costs to build, registered one by one and built at once,
    /// alternating so warmup and drift favour neither. Each `register_from` rebuilds the matcher over
    /// every value the set holds, so one by one is `count` builds. Outside the timed part, each
    /// run's set must hold every value and mask each one whole. The times are wall time, so on a
    /// busy machine run it under `/usr/bin/time -l` and compare user time with real. In release:
    /// `cargo test -p browseros-trace --release --lib build_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check: it prints build times and takes minutes"]
    #[expect(clippy::print_stdout, reason = "the measurement is what it prints")]
    fn build_cost() {
        const RUNS: usize = 5;
        let config = SecretSetConfig::default();
        for count in [256, 4096] {
            let values = token_like(count);
            let text = values.join(" ");
            let all_masked = vec![MASK; count].join(" ");
            let fresh = || -> Vec<SecretValue> {
                values
                    .iter()
                    .map(|value| SecretValue::new(value.clone()))
                    .collect()
            };
            let (mut one_by_one, mut at_once) = (Vec::new(), Vec::new());
            let mut patterns = 0;
            for _ in 0..RUNS {
                let values = fresh();
                let began = Instant::now();
                let set = SecretSet::new(config);
                for value in values {
                    let _registration = set.register_from(value, None, SecretSource::Page);
                }
                one_by_one.push(began.elapsed());
                assert_eq!(set.count(), count, "every value is distinct and kept");
                assert_eq!(masked(&set, &text), all_masked);

                let values = fresh();
                let began = Instant::now();
                let set = SecretSet::with_values(config, values);
                at_once.push(began.elapsed());
                assert_eq!(set.count(), count, "every value is distinct and kept");
                assert_eq!(masked(&set, &text), all_masked);
                patterns = set
                    .inner
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .matcher
                    .as_ref()
                    .map_or(0, AhoCorasick::patterns_len);
            }
            println!(
                "{count} values ({patterns} patterns), {RUNS} runs each:\n  one by one: {}\n  at once:    {}",
                spread(&mut one_by_one),
                spread(&mut at_once)
            );
        }
    }

    /// Each set's spans on one text, masked together.
    fn masked_by(sets: &[&SecretSet], text: &str) -> String {
        let mut spans = Vec::new();
        for set in sets {
            set.spans(text, &mut spans);
        }
        mask_spans(text, spans).unwrap_or_else(|| text.to_string())
    }

    #[test]
    fn spans_of_several_sets_are_masked_as_one_run_whatever_their_order() {
        let short = set_with(&["held-7Qx"]);
        let long = set_with(&["held-7Qx-swordfish"]);
        let tail = set_with(&["swordfish-tail"]);
        let text = "pw held-7Qx-swordfish-tail ok";
        for sets in [[&short, &long, &tail], [&tail, &long, &short]] {
            assert_eq!(masked_by(&sets, text), "pw [private] ok");
        }
        assert_eq!(
            masked_by(&[&short], "held-7Qxheld-7Qx"),
            "[private][private]",
            "spans that only touch stay two, as one set's matches do"
        );
        assert_eq!(
            masked_by(
                &[&set_with(&["pässwörd"]), &set_with(&["wörd-zwei"])],
                "é pässwörd-zwei é"
            ),
            "é [private] é"
        );
        assert_eq!(mask_spans("nothing held", Vec::new()), None);
    }
}
