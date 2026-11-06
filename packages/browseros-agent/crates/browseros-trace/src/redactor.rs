//! Turns what the dispatcher saw into what a record may keep. In order: redact the declared
//! input fields, mask every registered secret and every value of the record's own sets
//! ([`Redactor::also_masking`]), then cut to size. Every mask comes before the cut: a cut that
//! falls inside a value it has not masked yet keeps the value's head.
//!
//! Cuts are structural, so a stored value always parses: long strings end in `…[+N bytes]`, and
//! arrays and objects that run over budget end in a `{"$more": n}` marker (`"$more": n` for an
//! object). claw-server's audit cut stored JSON mid-text instead.

use crate::secret_set::{mask_json_with, mask_spans};
use crate::{RedactionPolicy, SecretSet, TargetSensitivity};
use serde_json::{Map, Value, json};
use std::ops::Range;
use std::sync::Arc;

/// What a redacted field becomes.
pub const REDACTED: &str = "[redacted]";

/// Size limits, in bytes of JSON text, except `note_lines`, which counts lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RedactionLimits {
    /// Any one string inside a value.
    pub string: usize,
    /// An operation's input.
    pub args: usize,
    /// An operation's result, a run's result.
    pub result: usize,
    /// A run's code.
    pub code: usize,
    /// Messages, URLs, titles and operation names.
    pub text: usize,
    /// A run's host data. D14 lets a host send up to 4 KiB, and it is kept whole up to this
    /// size: no string inside it is cut on its own, so the host reads back what it sent.
    pub host_data: usize,
    /// Any one line of an operation's notes.
    pub note: usize,
    /// The lines of an operation's notes that are kept. The rest are counted in one last
    /// `…[+N lines]` line.
    pub note_lines: usize,
}

impl Default for RedactionLimits {
    fn default() -> Self {
        Self {
            string: 1024,
            args: 4096,
            result: 2048,
            code: 64 * 1024,
            text: 2048,
            host_data: 4096,
            note: 1024,
            note_lines: 64,
        }
    }
}

/// Whether a value had to be cut to fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Truncation {
    /// Stored whole.
    Whole,
    /// Cut to fit; the flag is stored beside it.
    Cut,
}

/// A value after redaction, with whether it was cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Bounded<T> {
    /// The value to store.
    pub value: T,
    /// Whether it was cut.
    pub truncation: Truncation,
}

/// Redacts, masks and bounds values for records. Cheap to clone.
#[derive(Clone)]
pub struct Redactor {
    secrets: Arc<SecretSet>,
    /// A record's own sets, masked with together with `secrets`: what claw-sdk still holds
    /// unregistered when it sends the record (sdk-traces decision 32). Nothing in them is ever
    /// registered in `secrets`, so they mask only the records that carry them. Empty for the
    /// writer's own redactor.
    also: Vec<Arc<SecretSet>>,
    limits: RedactionLimits,
}

impl Redactor {
    /// A redactor over the process-wide secret set.
    #[must_use]
    pub fn new(secrets: Arc<SecretSet>, limits: RedactionLimits) -> Self {
        Self {
            secrets,
            also: Vec::new(),
            limits,
        }
    }

    /// This redactor for one record, which also masks every value of `sets`. Each set counts
    /// once, however often it is passed.
    pub(crate) fn also_masking<'a>(
        &self,
        sets: impl IntoIterator<Item = &'a Arc<SecretSet>>,
    ) -> Self {
        let mut also: Vec<Arc<SecretSet>> = Vec::new();
        for set in sets {
            if !also.iter().any(|kept| Arc::ptr_eq(kept, set)) {
                also.push(Arc::clone(set));
            }
        }
        Self {
            secrets: Arc::clone(&self.secrets),
            also,
            limits: self.limits,
        }
    }

    /// An operation's input: `secret` fields always redacted, `sensitive` ones when the target
    /// was sensitive, then masked and bounded.
    #[must_use]
    pub fn args(
        &self,
        args: &Value,
        policy: &RedactionPolicy,
        target: TargetSensitivity,
    ) -> Bounded<Value> {
        let mut value = args.clone();
        let redacted = Value::String(REDACTED.to_string());
        for pointer in &policy.secret {
            pointer.replace_in(&mut value, &redacted);
        }
        if target == TargetSensitivity::Sensitive {
            for pointer in &policy.sensitive {
                pointer.replace_in(&mut value, &redacted);
            }
        }
        self.mask_json(&mut value);
        self.bound(value, self.limits.args, self.limits.string)
    }

    /// A result: masked and bounded.
    #[must_use]
    pub fn result(&self, result: &Value) -> Bounded<Value> {
        let mut value = result.clone();
        self.mask_json(&mut value);
        self.bound(value, self.limits.result, self.limits.string)
    }

    /// Host data: masked, then bounded by `host_data` alone. Neo never reads it, so no string
    /// inside it is clipped to `string` the way a result's are.
    #[must_use]
    pub fn host_data(&self, host: &Value) -> Bounded<Value> {
        let mut value = host.clone();
        self.mask_json(&mut value);
        self.bound(value, self.limits.host_data, self.limits.host_data)
    }

    /// A message, URL, title or operation name: masked and clipped.
    #[must_use]
    pub fn text(&self, text: &str) -> String {
        let mut text = text.to_string();
        self.mask_str(&mut text);
        clip(&text, self.limits.text).0
    }

    /// An operation's notes, oldest first: the first `note_lines` lines, each masked, then
    /// clipped to `note` bytes. Lines past them are dropped and counted in one last
    /// `…[+N lines]` line. Each line is masked before it is clipped, so a cut never leaves part
    /// of a secret behind.
    #[must_use]
    pub fn notes(&self, notes: Vec<String>) -> Vec<String> {
        let total = notes.len();
        let mut kept: Vec<String> = notes
            .into_iter()
            .take(self.limits.note_lines)
            .map(|mut line| {
                self.mask_str(&mut line);
                clip(&line, self.limits.note).0
            })
            .collect();
        if total > kept.len() {
            kept.push(format!("…[+{} lines]", total - kept.len()));
        }
        kept
    }

    /// A run's code: masked and clipped.
    #[must_use]
    pub fn code(&self, code: &str) -> Bounded<String> {
        self.code_with(code, Vec::new())
    }

    /// A run's code, masked over `spans` found earlier ([`Redactor::spans`]) and over what this
    /// redactor finds now, then clipped. The writer finds the start's spans as a run begins and
    /// adds the end's here, so a value held at either moment is masked whole whether it grew or
    /// shrank in between, and before the cut.
    #[must_use]
    pub(crate) fn code_with(&self, code: &str, mut spans: Vec<Range<usize>>) -> Bounded<String> {
        self.spans(code, &mut spans);
        let masked = mask_spans(code, spans);
        let (value, cut) = clip(masked.as_deref().unwrap_or(code), self.limits.code);
        Bounded {
            value,
            truncation: if cut {
                Truncation::Cut
            } else {
                Truncation::Whole
            },
        }
    }

    /// Adds where `text` holds a value of the process set or of the record's own sets.
    pub(crate) fn spans(&self, text: &str, spans: &mut Vec<Range<usize>>) {
        self.secrets.spans(text, spans);
        for set in &self.also {
            set.spans(text, spans);
        }
    }

    /// Masks the union of every set's matches on the text as given. One set after another would
    /// not do: a held value that grew or shrank since another set learned it, or one whose
    /// prefix the process set has registered, would lose its head to the first set and keep its
    /// tail from the second.
    fn mask_str(&self, text: &mut String) -> bool {
        if self.also.is_empty() {
            return self.secrets.mask_str(text);
        }
        let mut spans = Vec::new();
        self.spans(text, &mut spans);
        match mask_spans(text, spans) {
            Some(masked) => {
                *text = masked;
                true
            }
            None => false,
        }
    }

    fn mask_json(&self, value: &mut Value) {
        mask_json_with(value, &mut |text| self.mask_str(text));
    }

    /// Cuts `value` to `budget` bytes of JSON, and any one string in it to `max_string`.
    fn bound(&self, mut value: Value, budget: usize, max_string: usize) -> Bounded<Value> {
        let mut remaining = i64::try_from(budget).unwrap_or(i64::MAX);
        let cut = shrink(&mut value, &mut remaining, max_string);
        Bounded {
            value,
            truncation: if cut {
                Truncation::Cut
            } else {
                Truncation::Whole
            },
        }
    }
}

/// Cuts `value` to roughly `remaining` bytes of JSON, depth first. Returns whether it cut.
fn shrink(value: &mut Value, remaining: &mut i64, max_string: usize) -> bool {
    match value {
        Value::String(text) => {
            let room = usize::try_from(*remaining).unwrap_or(0).min(max_string + 2);
            let (clipped, cut) = clip_json(text, room);
            *remaining -= json_len(&clipped);
            if cut {
                *text = clipped;
            }
            cut
        }
        Value::Array(items) => {
            *remaining -= 2;
            let mut cut = false;
            for index in 0..items.len() {
                if *remaining <= 0 {
                    let more = items.len() - index;
                    items.truncate(index);
                    items.push(json!({ "$more": more }));
                    return true;
                }
                cut |= shrink(&mut items[index], remaining, max_string);
            }
            cut
        }
        Value::Object(map) => {
            *remaining -= 2;
            let mut cut = false;
            let mut kept = Map::with_capacity(map.len());
            let total = map.len();
            for (index, (key, mut child)) in std::mem::take(map).into_iter().enumerate() {
                // Keys count like strings: clipped to the string limit, escapes included. An
                // entry whose key alone uses up the budget is not kept.
                let (key, key_cut) = clip_json(&key, max_string + 2);
                let key_len = json_len(&key) + 1;
                if *remaining - key_len <= 0 {
                    kept.insert("$more".to_string(), json!(total - index));
                    cut = true;
                    break;
                }
                *remaining -= key_len;
                cut |= key_cut;
                cut |= shrink(&mut child, remaining, max_string);
                kept.insert(key, child);
            }
            *map = kept;
            cut
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {
            *remaining -= i64::try_from(value.to_string().len()).unwrap_or(i64::MAX);
            false
        }
    }
}

/// The bytes `text` takes as a JSON string: quotes and escapes included.
fn json_len(text: &str) -> i64 {
    serde_json::to_string(text).map_or(i64::MAX, |quoted| {
        i64::try_from(quoted.len()).unwrap_or(i64::MAX)
    })
}

/// Clips `text` so that it takes at most `room` bytes as a JSON string, escapes included.
/// Text full of quotes or control characters escapes to several bytes per char, so the cut
/// shrinks until the escaped form fits.
fn clip_json(text: &str, room: usize) -> (String, bool) {
    let room_bytes = i64::try_from(room).unwrap_or(i64::MAX);
    if json_len(text) <= room_bytes {
        return (text.to_string(), false);
    }
    let mut limit = room.saturating_sub(2);
    loop {
        let (clipped, _) = clip(text, limit);
        let over = json_len(&clipped) - room_bytes;
        if over <= 0 || limit == 0 {
            return (clipped, true);
        }
        limit = limit.saturating_sub(usize::try_from(over).unwrap_or(usize::MAX).max(1));
    }
}

/// Clips `text` to at most `limit` bytes on a char boundary, marking how much was dropped.
fn clip(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_string(), false);
    }
    let marker_room = 24;
    let mut end = limit.saturating_sub(marker_room);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (
        format!("{}…[+{} bytes]", &text[..end], text.len() - end),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::{RedactionLimits, Redactor, Truncation};
    use crate::{
        JsonPointer, RedactionPolicy, SecretSet, SecretSetConfig, SecretValue, TargetSensitivity,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn redactor(limits: RedactionLimits) -> Redactor {
        let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
        secrets.register(SecretValue::new("hunter22".into()), None);
        Redactor::new(secrets, limits)
    }

    fn pointers(paths: &[&str]) -> Vec<JsonPointer> {
        paths
            .iter()
            .filter_map(|path| JsonPointer::parse(path).ok())
            .collect()
    }

    fn fill_policy() -> RedactionPolicy {
        RedactionPolicy {
            secret: pointers(&["/token"]),
            sensitive: pointers(&["/value"]),
        }
    }

    #[test]
    fn secret_fields_are_always_redacted_and_sensitive_ones_only_for_sensitive_targets() {
        let redactor = redactor(RedactionLimits::default());
        let args = json!({"value": "typed", "token": "t0k3n", "locator": {"$neo": "locator", "page": 3, "selector": "getByLabel('Email')"}});
        let ordinary = redactor.args(&args, &fill_policy(), TargetSensitivity::Ordinary);
        assert_eq!(
            ordinary.value,
            json!({"value": "typed", "token": "[redacted]", "locator": {"$neo": "locator", "page": 3, "selector": "getByLabel('Email')"}})
        );
        let sensitive = redactor.args(&args, &fill_policy(), TargetSensitivity::Sensitive);
        assert_eq!(sensitive.value["value"], json!("[redacted]"));
        assert_eq!(
            sensitive.value["locator"]["selector"],
            json!("getByLabel('Email')"),
            "the selector stays for the skill builder"
        );
    }

    #[test]
    fn registered_secrets_are_masked_everywhere() {
        let redactor = redactor(RedactionLimits::default());
        let result = redactor.result(&json!({"text": "the password is hunter22"}));
        assert_eq!(result.value, json!({"text": "the password is [private]"}));
        assert_eq!(redactor.text("hunter22 in a title"), "[private] in a title");
        assert_eq!(redactor.code("fill('hunter22')").value, "fill('[private]')");
    }

    #[test]
    fn a_cut_value_still_parses_and_says_how_much_is_missing() {
        let limits = RedactionLimits {
            string: 40,
            result: 120,
            ..RedactionLimits::default()
        };
        let redactor = redactor(limits);
        let long = "x".repeat(500);
        let value =
            json!({"rows": (0..50).map(|i| json!({"i": i, "s": long})).collect::<Vec<_>>()});
        let bounded = redactor.result(&value);
        assert_eq!(bounded.truncation, Truncation::Cut);
        let text = bounded.value.to_string();
        assert!(text.len() < 400, "{} bytes: {text}", text.len());
        assert!(serde_json::from_str::<Value>(&text).is_ok());
        assert!(text.contains("$more"), "{text}");
        assert!(text.contains("…[+"), "{text}");
    }

    #[test]
    fn strings_are_clipped_on_a_char_boundary() {
        let limits = RedactionLimits {
            string: 30,
            ..RedactionLimits::default()
        };
        let redactor = redactor(limits);
        let bounded = redactor.result(&json!("éééééééééééééééééééééééééééééé"));
        let Value::String(text) = bounded.value else {
            panic!("expected a string");
        };
        assert!(text.contains("…[+"), "{text}");
    }

    #[test]
    fn huge_keys_and_escape_heavy_strings_stay_inside_the_budget() {
        let redactor = redactor(RedactionLimits::default());
        let key = "k".repeat(100_000);
        let mut object = serde_json::Map::new();
        object.insert(key, Value::Null);
        let bounded = redactor.result(&Value::Object(object));
        assert_eq!(bounded.truncation, Truncation::Cut);
        assert!(
            bounded.value.to_string().len() <= 2048 + 64,
            "{} bytes",
            bounded.value.to_string().len()
        );

        let quotes = "\"".repeat(5_000);
        let bounded = redactor.result(&json!({ "q": quotes }));
        assert_eq!(bounded.truncation, Truncation::Cut);
        let text = bounded.value.to_string();
        assert!(text.len() <= 2048 + 64, "{} bytes", text.len());
        assert!(serde_json::from_str::<Value>(&text).is_ok());
    }

    #[test]
    fn a_small_value_is_kept_whole() {
        let redactor = redactor(RedactionLimits::default());
        let bounded = redactor.result(&json!({"ok": true, "n": [1, 2, 3]}));
        assert_eq!(bounded.truncation, Truncation::Whole);
        assert_eq!(bounded.value, json!({"ok": true, "n": [1, 2, 3]}));
    }
}
