//! Bounds trace values before they reach storage. Cuts are structural, so stored JSON always
//! parses: strings end in `…[+N bytes]`, and oversized arrays or objects carry a `$more` marker.
//! The writer chooses when inputs become visible; this module owns only their size limits.

use serde_json::{Map, Value, json};

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

impl RedactionLimits {
    /// The defaults, with an operation's arguments and result, and any one string inside them,
    /// raised to `bytes` where that is larger. A development setting (`CLAW_TRACE_VALUE_BYTES`
    /// on the server): a tool that shows whole values, such as the Neo viewer, asks for one;
    /// production leaves it unset, so its trace stays at the defaults. Nothing else changes:
    /// host data, text, notes and code keep their own limits.
    #[must_use]
    pub fn with_value_bytes(bytes: usize) -> Self {
        let base = Self::default();
        Self {
            string: base.string.max(bytes),
            args: base.args.max(bytes),
            result: base.result.max(bytes),
            ..base
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

/// A value after size bounding, with whether it was cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Bounded<T> {
    /// The value to store.
    pub value: T,
    /// Whether it was cut.
    pub truncation: Truncation,
}

/// Bounds values for trace records. Cheap to clone; it carries only the limits.
#[derive(Clone)]
pub struct Redactor {
    limits: RedactionLimits,
}

impl Redactor {
    /// The size limits applied when the writer materializes a record.
    #[must_use]
    pub fn new(limits: RedactionLimits) -> Self {
        Self { limits }
    }

    /// An operation's input, bounded independently of its target.
    #[must_use]
    pub fn args(&self, args: &Value) -> Bounded<Value> {
        let value = args.clone();
        self.bound(value, self.limits.args, self.limits.string)
    }

    /// A result, bounded for storage.
    #[must_use]
    pub fn result(&self, result: &Value) -> Bounded<Value> {
        let value = result.clone();
        self.bound(value, self.limits.result, self.limits.string)
    }

    /// Host data, bounded by `host_data` alone. Neo never reads it, so no string
    /// inside it is clipped to `string` the way a result's are.
    #[must_use]
    pub fn host_data(&self, host: &Value) -> Bounded<Value> {
        let value = host.clone();
        self.bound(value, self.limits.host_data, self.limits.host_data)
    }

    /// A message, URL, title or operation name, clipped to its text budget.
    #[must_use]
    pub fn text(&self, text: &str) -> String {
        clip(text, self.limits.text).0
    }

    /// An operation's notes, oldest first: keep `note_lines` lines, each clipped to `note`
    /// bytes. Count any remaining lines in one final `…[+N lines]` marker.
    #[must_use]
    pub fn notes(&self, notes: Vec<String>) -> Vec<String> {
        let total = notes.len();
        let mut kept: Vec<String> = notes
            .into_iter()
            .take(self.limits.note_lines)
            .map(|line| clip(&line, self.limits.note).0)
            .collect();
        if total > kept.len() {
            kept.push(format!("…[+{} lines]", total - kept.len()));
        }
        kept
    }

    /// A run's code, clipped to the code budget.
    #[must_use]
    pub fn code(&self, code: &str) -> Bounded<String> {
        let (value, cut) = clip(code, self.limits.code);
        Bounded {
            value,
            truncation: if cut {
                Truncation::Cut
            } else {
                Truncation::Whole
            },
        }
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
    use super::{Bounded, RedactionLimits, Redactor, Truncation};
    use serde_json::{Value, json};

    fn redactor(limits: RedactionLimits) -> Redactor {
        Redactor::new(limits)
    }

    #[test]
    fn inputs_preserve_field_values() {
        let redactor = redactor(RedactionLimits::default());
        let args = json!({"value": "typed", "token": "t0k3n", "selector": "input[type=password]"});
        assert_eq!(redactor.args(&args).value, args);
    }

    #[test]
    fn results_text_and_code_keep_the_original_values() {
        let redactor = redactor(RedactionLimits::default());
        let result = redactor.result(&json!({"text": "the password is hunter22"}));
        assert_eq!(result.value, json!({"text": "the password is hunter22"}));
        assert_eq!(redactor.text("hunter22 in a title"), "hunter22 in a title");
        assert_eq!(redactor.code("fill('hunter22')").value, "fill('hunter22')");
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
    fn a_value_setting_keeps_a_snapshot_whole_and_leaves_everything_else_alone() {
        // A whole-page snapshot's text, as `page.snapshotPending` records it: one long string.
        let text = "- link \"Story\" [ref=e1]\n".repeat(1_000);
        let args = json!({ "text": text, "form": "text" });
        let defaults = redactor(RedactionLimits::default());
        assert_eq!(defaults.args(&args).truncation, Truncation::Cut);

        let limits = RedactionLimits::with_value_bytes(256 * 1024);
        let raised = redactor(limits);
        assert_eq!(
            raised.args(&args),
            Bounded {
                value: args.clone(),
                truncation: Truncation::Whole,
            }
        );
        assert_eq!(raised.result(&json!(text)).truncation, Truncation::Whole);
        assert_eq!(
            raised.result(&json!("x".repeat(300 * 1024))).truncation,
            Truncation::Cut
        );
        assert_eq!(limits.text, RedactionLimits::default().text);
        assert_eq!(limits.host_data, RedactionLimits::default().host_data);
        // Never below the defaults.
        assert_eq!(
            RedactionLimits::with_value_bytes(16),
            RedactionLimits::default()
        );
    }

    #[test]
    fn a_small_value_is_kept_whole() {
        let redactor = redactor(RedactionLimits::default());
        let bounded = redactor.result(&json!({"ok": true, "n": [1, 2, 3]}));
        assert_eq!(bounded.truncation, Truncation::Whole);
        assert_eq!(bounded.value, json!({"ok": true, "n": [1, 2, 3]}));
    }
}
