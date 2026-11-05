//! Turns what the dispatcher saw into what a record may keep. In order: redact the declared
//! input fields, mask every registered secret, then cut to size.
//!
//! Cuts are structural, so a stored value always parses: long strings end in `…[+N bytes]`, and
//! arrays and objects that run over budget end in a `{"$more": n}` marker (`"$more": n` for an
//! object). claw-server's audit cut stored JSON mid-text instead.

use crate::{RedactionPolicy, SecretSet, TargetSensitivity};
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// What a redacted field becomes.
pub const REDACTED: &str = "[redacted]";

/// Size limits, in bytes of JSON text.
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
    /// Messages, URLs, titles and host data.
    pub text: usize,
}

impl Default for RedactionLimits {
    fn default() -> Self {
        Self {
            string: 1024,
            args: 4096,
            result: 2048,
            code: 64 * 1024,
            text: 2048,
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
    limits: RedactionLimits,
}

impl Redactor {
    /// A redactor over the process-wide secret set.
    #[must_use]
    pub fn new(secrets: Arc<SecretSet>, limits: RedactionLimits) -> Self {
        Self { secrets, limits }
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
        self.secrets.mask_json(&mut value);
        self.bound(value, self.limits.args)
    }

    /// A result: masked and bounded.
    #[must_use]
    pub fn result(&self, result: &Value) -> Bounded<Value> {
        let mut value = result.clone();
        self.secrets.mask_json(&mut value);
        self.bound(value, self.limits.result)
    }

    /// Host data: masked and bounded like a message.
    #[must_use]
    pub fn host_data(&self, host: &Value) -> Bounded<Value> {
        let mut value = host.clone();
        self.secrets.mask_json(&mut value);
        self.bound(value, self.limits.text)
    }

    /// A message, URL or title: masked and clipped.
    #[must_use]
    pub fn text(&self, text: &str) -> String {
        let mut text = text.to_string();
        self.secrets.mask_str(&mut text);
        clip(&text, self.limits.text).0
    }

    /// A run's code: masked and clipped.
    #[must_use]
    pub fn code(&self, code: &str) -> Bounded<String> {
        let mut code = code.to_string();
        self.secrets.mask_str(&mut code);
        let (value, cut) = clip(&code, self.limits.code);
        Bounded {
            value,
            truncation: if cut {
                Truncation::Cut
            } else {
                Truncation::Whole
            },
        }
    }

    fn bound(&self, mut value: Value, budget: usize) -> Bounded<Value> {
        let mut remaining = i64::try_from(budget).unwrap_or(i64::MAX);
        let cut = shrink(&mut value, &mut remaining, self.limits.string);
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
        JsonPointer, RedactionPolicy, SecretScope, SecretSet, SecretSetConfig, SecretValue,
        TargetSensitivity,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn redactor(limits: RedactionLimits) -> Redactor {
        let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
        secrets.register(
            SecretValue::new("hunter22".into()),
            SecretScope::Browser,
            None,
        );
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
