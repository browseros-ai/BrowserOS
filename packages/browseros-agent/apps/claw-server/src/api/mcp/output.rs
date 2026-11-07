//! Bounds model-visible run reports; retained originals are shared across MCP sessions.

use std::collections::VecDeque;

use serde_json::{Value, json};

pub(super) const BUDGET: usize = 24_000;
const LOG_BUDGET: usize = 2_000;
const CALL_BUDGET: usize = 4_000;
const ERROR_BUDGET: usize = 4_000;
const CONTEXT_BUDGET: usize = 2_000;
const MAX_RESULTS: usize = 16;
const STORE_BUDGET: usize = 64 * 1024 * 1024;

struct Retained {
    id: String,
    original: Value,
    bytes: usize,
}

/// Process-local LRU. Its owner shares one mutex across transport-local service clones.
#[derive(Default)]
pub(super) struct Store {
    entries: VecDeque<Retained>,
    bytes: usize,
}

impl Store {
    pub(super) fn limit(&mut self, original: Value) -> Value {
        let bytes = original.to_string().len();
        if bytes <= BUDGET {
            return original;
        }
        let id = loop {
            let id = format!("r{:012x}", rand::random::<u64>() & 0xffff_ffff_ffff);
            if !self.entries.iter().any(|entry| entry.id == id) {
                break id;
            }
        };
        let retained = bytes <= STORE_BUDGET;
        let bounded = bound(&original, &id, retained);
        if retained {
            while self.entries.len() >= MAX_RESULTS || self.bytes + bytes > STORE_BUDGET {
                if let Some(old) = self.entries.pop_front() {
                    self.bytes -= old.bytes;
                } else {
                    break;
                }
            }
            self.bytes += bytes;
            self.entries.push_back(Retained {
                id,
                original,
                bytes,
            });
        }
        bounded
    }

    pub(super) fn retrieve(
        &mut self,
        id: &str,
        field: &str,
        offset: usize,
    ) -> Result<Value, String> {
        if !matches!(field, "value" | "logs" | "calls" | "error") {
            return Err("field must be value, logs, calls or error".into());
        }
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Ok(json!({"status":"error", "logs":[], "calls":[], "pages":[],
                "error":{"name":"Error", "message":"Stored result expired or is unknown. Results live in this server process: the 16 most recently used truncated results, up to 64 MiB of serialized JSON; restarting the server clears them."}}));
        };
        // Access promotes the original, including clients that create a new MCP session per call.
        if let Some(entry) = self.entries.remove(index) {
            self.entries.push_back(entry);
        }
        let entry = &self.entries[self.entries.len() - 1];
        let value = entry
            .original
            .get(field)
            .ok_or_else(|| format!("result has no {field} field"))?;
        let text = field_text(field, value);
        if offset > text.len() || !text.is_char_boundary(offset) || inside_ref(&text, offset) {
            return Err(
                "offset must be a UTF-8 byte boundary within the field, outside a [ref=…] token"
                    .into(),
            );
        }
        let mut out = json!({"status":"ok", "value":"", "logs":[], "calls":[], "pages":[],
            "truncated":{"result":id, "field":field, "offset":offset,"end":offset,"total":text.len(), "type":value_type(value)}});
        if field == "value" && value.is_string() {
            let (context, clipped) = ancestors(&text, offset);
            if !context.is_empty() {
                out["truncated"]["context"] = json!(context);
            }
            if clipped {
                out["truncated"]["contextTruncated"] = json!(true);
            }
        }
        let end = fit_prefix(&text[offset..], |shown| {
            let end = offset + shown;
            out["value"] = json!(&text[offset..end]);
            out["truncated"]["end"] = json!(end);
            if end < text.len() {
                out["truncated"]["next"] = next(id, field, end);
            } else if let Some(meta) = out["truncated"].as_object_mut() {
                meta.remove("next");
            }
            out.to_string().len() <= BUDGET
        });
        if end == 0 && offset < text.len() {
            return Err("a single ref token cannot fit in a retrieval window".into());
        }
        Ok(out)
    }
}

fn field_text(field: &str, value: &Value) -> String {
    if field == "value"
        && let Some(text) = value.as_str()
    {
        return text.to_owned();
    }
    // Values already crossed serde successfully; pretty JSON is a reversible text view.
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn next(id: &str, field: &str, offset: usize) -> Value {
    json!({"result":id, "field":field, "offset":offset})
}

fn metadata(id: &str, field: &str, shown: usize, text: &str, value: &Value) -> Value {
    let mut meta = json!({"shown":shown,"total":text.len(),"type":value_type(value)});
    if shown < text.len() {
        meta["next"] = next(id, field, shown);
    }
    meta
}

fn bound(original: &Value, id: &str, retained: bool) -> Value {
    let mut out = original.clone();
    out["truncated"] = json!({"result":id});
    if !retained {
        out["truncated"]["unavailable"] = json!("Original exceeds the 64 MiB retention limit");
    }
    if let Some(logs) = original["logs"].as_array() {
        let mut kept = Vec::new();
        for line in logs {
            kept.push(line.clone());
            if serde_json::to_string(&kept).unwrap_or_default().len() > LOG_BUDGET {
                kept.pop();
                break;
            }
        }
        out["logs"] = json!(kept);
        mark_changed(&mut out, original, id, "logs");
    }
    if let Some(calls) = original["calls"].as_array() {
        let mut kept = Vec::new();
        for call in calls.iter().rev() {
            let compact = ["callId", "method", "title", "result", "delivery"]
                .into_iter()
                .filter_map(|key| call.get(key).map(|value| (key.to_string(), value.clone())))
                .collect::<serde_json::Map<_, _>>();
            kept.push(Value::Object(compact));
            if serde_json::to_string(&kept).unwrap_or_default().len() > CALL_BUDGET {
                kept.pop();
                break;
            }
        }
        kept.reverse();
        out["calls"] = json!(kept);
        mark_calls(&mut out, original, id);
    }
    if let Some(message) = original["error"]["message"].as_str()
        && message.len() > ERROR_BUDGET
    {
        let end = fit_prefix(message, |shown| {
            serde_json::to_string(&message[..shown])
                .unwrap_or_default()
                .len()
                <= ERROR_BUDGET
        });
        out["error"]["message"] = json!(&message[..end]);
        mark_changed(&mut out, original, id, "error");
    }
    if original.get("value").is_some() {
        out["value"] = json!("");
    }
    // Recovery identities and errors take priority over optional log/trace excerpts.
    if out.to_string().len() > BUDGET {
        out["logs"] = json!([]);
        out["calls"] = json!([]);
        mark_changed(&mut out, original, id, "logs");
        mark_calls(&mut out, original, id);
    }
    if let Some(value) = original.get("value") {
        let text = field_text("value", value);
        fit_prefix(&text, |shown| {
            out["value"] = json!(&text[..shown]);
            out["truncated"]["value"] = metadata(id, "value", shown, &text, value);
            out.to_string().len() <= BUDGET
        });
    }
    if !retained {
        for field in ["value", "logs", "calls", "error"] {
            if let Some(meta) = out["truncated"][field].as_object_mut() {
                meta.remove("next");
            }
        }
    }
    // All page identities and error names are recovery data. If those alone exceed the
    // budget, say so instead of silently losing a tab or replacing the original failure.
    if out.to_string().len() > BUDGET {
        out["truncated"]["metadataOverBudget"] = json!(true);
    }
    out
}

fn mark_changed(out: &mut Value, original: &Value, id: &str, field: &str) {
    if out[field] != original[field] {
        let text = field_text(field, &original[field]);
        // Array excerpts and clipped errors are not byte prefixes of their original JSON.
        out["truncated"][field] = metadata(id, field, 0, &text, &original[field]);
    }
}

fn mark_calls(out: &mut Value, original: &Value, id: &str) {
    mark_changed(out, original, id, "calls");
    let Some(calls) = original["calls"].as_array() else {
        return;
    };
    let kept = out["calls"].as_array().map_or(0, Vec::len);
    let omitted = &calls[..calls.len() - kept];
    if omitted.is_empty() {
        return;
    }
    let delivery = omitted
        .iter()
        .filter_map(|call| call["delivery"].as_str())
        .max_by_key(|delivery| match *delivery {
            "unknown" => 2,
            "done" => 1,
            _ => 0,
        });
    let mut dropped = json!({"count":omitted.len()});
    if let Some(delivery) = delivery {
        dropped["delivery"] = json!(delivery);
    }
    out["truncated"]["calls"]["dropped"] = dropped;
}

/// Maximize the encoded envelope, then prefer a complete line and keep ref tokens whole.
fn fit_prefix(text: &str, mut fits: impl FnMut(usize) -> bool) -> usize {
    if text.len() <= BUDGET && fits(text.len()) {
        return text.len();
    }
    let (mut low, mut high) = (0, text.len().min(BUDGET));
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if fits(char_boundary(text, middle)) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let end = prefix_end(text, low);
    fits(end);
    end
}

fn char_boundary(text: &str, mut offset: usize) -> usize {
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn inside_ref(text: &str, offset: usize) -> bool {
    ref_start(text, offset).is_some()
}

fn ref_start(text: &str, offset: usize) -> Option<usize> {
    text[..offset]
        .rfind('[')
        .filter(|&start| text[start..].starts_with("[ref=") && !text[start..offset].contains(']'))
}

fn prefix_end(text: &str, limit: usize) -> usize {
    let mut end = char_boundary(text, limit.min(text.len()));
    if end == text.len() {
        return end;
    }
    if let Some(newline) = text[..end].rfind('\n') {
        end = newline + 1;
    }
    if let Some(start) = ref_start(text, end) {
        end = start;
    }
    end
}

/// Real aria-YAML ancestor lines, in document order; omit distant context before content.
fn ancestors(text: &str, offset: usize) -> (String, bool) {
    let start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let first = text[start..].lines().next().unwrap_or_default();
    let mut indent = first.len() - first.trim_start().len();
    if indent == 0 || !first.trim_start().starts_with("- ") {
        return (String::new(), false);
    }
    let mut lines = Vec::new();
    let mut bytes = 0;
    let mut clipped = false;
    for line in text[..start].split_inclusive('\n').rev() {
        let depth = line.len() - line.trim_start().len();
        if depth < indent && line.trim_start().starts_with("- ") {
            indent = depth;
            if bytes + line.len() <= CONTEXT_BUDGET {
                lines.push(line);
                bytes += line.len();
            } else {
                clipped = true;
            }
        }
    }
    lines.reverse();
    (lines.concat(), clipped)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "Assertions fail the test on malformed results"
)]
mod tests {
    use serde_json::{Value, json};

    use super::{BUDGET, Store};

    fn report(value: Value) -> Value {
        json!({"status":"ok", "value":value, "logs":[], "calls":[],
            "pages":[{"id":"p1", "created":true, "closed":false,"url":"https://example.com"}]})
    }

    fn remainder(store: &mut Store, mut next: Value, mut text: String) -> String {
        while next.is_object() {
            let window = store
                .retrieve(
                    next["result"].as_str().expect("expected test value"),
                    next["field"].as_str().expect("expected test value"),
                    next["offset"].as_u64().expect("expected test value") as usize,
                )
                .expect("expected test value");
            assert!(window.to_string().len() <= BUDGET);
            assert!(
                window["pages"]
                    .as_array()
                    .expect("expected test value")
                    .is_empty()
            );
            assert!(
                window["truncated"]["end"]
                    .as_u64()
                    .expect("expected test value")
                    > next["offset"].as_u64().expect("expected test value")
            );
            text.push_str(window["value"].as_str().expect("expected test value"));
            next = window["truncated"]["next"].clone();
        }
        text
    }

    #[test]
    fn tests_that_small_results_are_unchanged() {
        let original = report(json!({"count":3,"active":true}));
        assert_eq!(Store::default().limit(original.clone()), original);
    }

    #[test]
    fn tests_that_large_values_are_bounded_and_retrieved_without_losing_text_or_context() {
        let text = format!(
            "- main [ref=e1]:\n  - list [ref=e2]:\n{}",
            (0..2500)
                .map(|i| format!("    - button \"商品 {i} 🦀\" [ref=e{}]\n", i + 3))
                .collect::<String>()
        );
        let mut store = Store::default();
        let bounded = store.limit(report(json!(text)));
        assert!(bounded.to_string().len() <= BUDGET);
        assert_eq!(bounded["pages"][0]["id"], "p1");
        assert_eq!(bounded["pages"][0]["url"], "https://example.com");
        let prefix = bounded["value"].as_str().expect("expected test value");
        assert!(prefix.ends_with('\n'));
        let next = bounded["truncated"]["value"]["next"].clone();
        let window = store
            .retrieve(
                next["result"].as_str().expect("expected test value"),
                "value",
                prefix.len(),
            )
            .expect("expected test value");
        assert_eq!(
            window["truncated"]["context"],
            "- main [ref=e1]:\n  - list [ref=e2]:\n"
        );
        assert_eq!(remainder(&mut store, next, prefix.into()), text);

        let object = json!({"items":["quoted \" \\ \n 🦀".repeat(12_000)]});
        let bounded = store.limit(report(object.clone()));
        assert_eq!(bounded["truncated"]["value"]["type"], "object");
        assert_eq!(
            remainder(
                &mut store,
                bounded["truncated"]["value"]["next"].clone(),
                bounded["value"]
                    .as_str()
                    .expect("expected test value")
                    .into()
            ),
            serde_json::to_string_pretty(&object).expect("expected test value")
        );
    }

    #[test]
    fn tests_that_failed_runs_keep_pages_errors_and_uncertain_delivery() {
        let mut store = Store::default();
        let mut original = report(Value::Null);
        original["status"] = json!("error");
        original["error"] =
            json!({"name":"TimeoutError", "message":"navigation failed\n".repeat(2000)});
        original["logs"] = json!(["a large log\n".repeat(20_000)]);
        original["calls"] =
            json!((0..200).map(|i| json!({
            "callId":i,"method":"click","title":"Click","params":{"text":"x".repeat(1000)},
            "result":{"kind":"ok"},"delivery":if i == 0 {"unknown"} else {"done"}
        })).collect::<Vec<_>>());
        original["callsDropped"] = json!({"count":7,"delivery":"notSent"});
        let bounded = store.limit(original.clone());
        assert!(bounded.to_string().len() <= BUDGET);
        assert_eq!(bounded["status"], "error");
        assert_eq!(bounded["pages"], original["pages"]);
        assert_eq!(bounded["error"]["name"], "TimeoutError");
        assert!(
            !bounded["error"]["message"]
                .as_str()
                .expect("expected test value")
                .is_empty()
        );
        assert_eq!(bounded["callsDropped"], original["callsDropped"]);
        assert_eq!(
            bounded["truncated"]["calls"]["dropped"]["delivery"],
            "unknown"
        );
        for field in ["logs", "calls", "error"] {
            let text = remainder(
                &mut store,
                bounded["truncated"][field]["next"].clone(),
                String::new(),
            );
            assert_eq!(
                serde_json::from_str::<Value>(&text).expect("expected test value"),
                original[field]
            );
        }
    }

    #[test]
    fn tests_that_retention_is_lru_and_invalid_offsets_are_rejected() {
        let mut store = Store::default();
        let mut ids = Vec::new();
        for _ in 0..16 {
            ids.push(
                store.limit(report(json!("🦀".repeat(10_000))))["truncated"]["result"]
                    .as_str()
                    .expect("expected test value")
                    .to_string(),
            );
        }
        assert!(store.retrieve(&ids[0], "value", 1).is_err());
        assert!(store.retrieve(&ids[0], "value", 40_001).is_err());
        assert_eq!(
            store
                .retrieve(&ids[0], "value", 0)
                .expect("expected test value")["status"],
            "ok"
        );
        store.limit(report(json!("x".repeat(40_000))));
        assert_eq!(
            store
                .retrieve(&ids[1], "value", 0)
                .expect("expected test value")["status"],
            "error"
        );
        assert_eq!(
            store
                .retrieve(&ids[0], "value", 0)
                .expect("expected test value")["status"],
            "ok"
        );
        assert_eq!(
            store
                .retrieve("missing", "value", 0)
                .expect("expected test value")["status"],
            "error"
        );
    }

    #[test]
    fn tests_that_long_lines_keep_refs_whole_and_escaped_errors_stay_bounded() {
        let mut store = Store::default();
        // Every possible cut near a token, including its opening characters, stays outside it.
        for padding in 23_000..23_030 {
            let text = format!("{}[ref=e999]{}", "x".repeat(padding), "z".repeat(2000));
            let bounded = store.limit(report(json!(text)));
            let id = bounded["truncated"]["result"]
                .as_str()
                .expect("expected test value");
            for offset in padding + 1..padding + 10 {
                assert!(store.retrieve(id, "value", offset).is_err());
            }
            let prefix = bounded["value"].as_str().expect("expected test value");
            assert!(!((padding + 1..padding + 10).contains(&prefix.len())));
            assert_eq!(
                remainder(
                    &mut store,
                    bounded["truncated"]["value"]["next"].clone(),
                    prefix.into()
                ),
                text
            );
        }
        let mut failed = report(Value::Null);
        failed["status"] = json!("error");
        failed["error"] = json!({"name":"Error", "message":"\u{1}".repeat(40_000)});
        assert!(store.limit(failed).to_string().len() <= BUDGET);
    }
}
