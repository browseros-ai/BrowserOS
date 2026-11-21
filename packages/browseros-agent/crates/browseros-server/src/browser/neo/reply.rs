//! An engine `run` reply, made into what a model reads (design 3.5, "Pictures").
//!
//! The engine answers with one text block (the returned value as JSON, then console lines and
//! notes) and the same value in `structuredContent.value`. Bytes a script returns come back as
//! `{"$neo":"bytes","base64":…}` inline, or `{"$neo":"blob","id":…,"size":…}` when the value
//! would pass the 256 KiB limit; a blob is read from the host API. Each picture becomes a
//! `Picture` and `[picture N]` in the text; bytes that are UTF-8 text become that text, because
//! the engine's own `result_too_large` message tells the script to return large text as
//! `Buffer.from(text)`. The text is then fenced as untrusted page content.

use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use serde_json::Value;

use super::super::{Picture, ToolOutput, fence};
use super::host::Host;
use super::wire::RUN_META;

/// The most blob bytes one call reads. A full-page screenshot was 1.2 MB in probe p1.
const BLOB_BUDGET: usize = 16 * 1024 * 1024;

pub(super) struct Reply {
    run_id: Option<String>,
    text: String,
    value: Option<Value>,
    is_error: bool,
}

impl Reply {
    pub(super) fn parse(result: Value) -> Result<Self, String> {
        let content = result["content"]
            .as_array()
            .ok_or("the engine's reply has no content")?;
        let text = content
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Self {
            run_id: result["_meta"][RUN_META]["id"].as_str().map(str::to_owned),
            text,
            value: result["structuredContent"].get("value").cloned(),
            is_error: result["isError"].as_bool().unwrap_or(false),
        })
    }

    /// The blobs the value names, by id, with the size each marker states. `None` when two
    /// markers give one id two sizes, which only a forged value would.
    fn blob_requests(&self) -> Option<BTreeMap<String, u64>> {
        let mut blobs = BTreeMap::new();
        let mut pending: Vec<&Value> = self.value.iter().collect();
        while let Some(node) = pending.pop() {
            if let Some((id, size)) = blob_marker(node) {
                if blobs
                    .insert(id.to_owned(), size)
                    .is_some_and(|known| known != size)
                {
                    return None;
                }
                continue;
            }
            match node {
                Value::Array(items) => pending.extend(items),
                Value::Object(fields) => pending.extend(fields.values()),
                _ => {}
            }
        }
        Some(blobs)
    }

    pub(super) fn present(self, blobs: &HashMap<String, Vec<u8>>) -> ToolOutput {
        let Reply {
            text,
            value,
            is_error,
            ..
        } = self;
        let mut pictures = Vec::new();
        let text = match value {
            None => text,
            Some(mut value) => {
                let original = value.to_string();
                if replace_markers(&mut value, blobs, &mut pictures) {
                    // The engine writes the value first and its console and notes after it.
                    // Keep those when the text starts with exactly the value as serialized;
                    // otherwise the value alone.
                    match text
                        .strip_prefix(&original)
                        .filter(|rest| rest.is_empty() || rest.starts_with('\n'))
                    {
                        Some(rest) => format!("{value}{rest}"),
                        None => value.to_string(),
                    }
                } else {
                    text
                }
            }
        };
        ToolOutput {
            text: fence(&text),
            pictures,
            is_error,
        }
    }
}

/// Reads every blob the reply names, within `BLOB_BUDGET`. A blob that cannot be read stays a
/// marker the text describes; the call itself still succeeds.
pub(super) async fn fetch_blobs(reply: &Reply, host: &Host) -> HashMap<String, Vec<u8>> {
    let mut fetched = HashMap::new();
    let (Some(run_id), Some(requests)) = (&reply.run_id, reply.blob_requests()) else {
        return fetched;
    };
    let mut budget = BLOB_BUDGET;
    for (id, size) in requests {
        let Ok(size) = usize::try_from(size) else {
            continue;
        };
        if size > budget {
            tracing::warn!(
                size,
                "a returned blob is over this call's budget; it is not read"
            );
            continue;
        }
        match host.blob(run_id, &id, size).await {
            Ok(bytes) => {
                budget -= bytes.len();
                fetched.insert(id, bytes);
            }
            Err(reason) => tracing::warn!(%reason, "a returned blob could not be read"),
        }
    }
    fetched
}

/// Replaces each bytes or blob marker in `value`, in document order. True when one was found.
fn replace_markers(
    value: &mut Value,
    blobs: &HashMap<String, Vec<u8>>,
    pictures: &mut Vec<Picture>,
) -> bool {
    let mut presenter = Presenter {
        pictures,
        blob_pictures: HashMap::new(),
        text_left: TEXT_BUDGET,
    };
    let mut replaced = false;
    // Iterative, like the engine's own result fitter: a deep value cannot overflow the stack.
    let mut pending = vec![value];
    while let Some(node) = pending.pop() {
        let replacement = if let Some(bytes) = bytes_marker(node) {
            Some(presenter.bytes(&bytes))
        } else if let Some((id, size)) = blob_marker(node) {
            Some(match blobs.get(id) {
                Some(bytes) => presenter.blob(id, bytes),
                None => format!("[{size} returned bytes that could not be read]"),
            })
        } else {
            None
        };
        if let Some(replacement) = replacement {
            *node = Value::String(replacement);
            replaced = true;
            continue;
        }
        match node {
            Value::Array(items) => pending.extend(items.iter_mut().rev()),
            Value::Object(fields) => pending.extend(fields.values_mut().rev()),
            _ => {}
        }
    }
    replaced
}

/// The most returned text one result inlines. A script can repeat one blob's marker many
/// times inside its 256 KiB value; without a bound, each copy would inline the blob again.
const TEXT_BUDGET: usize = 16 * 1024 * 1024;

/// Turns returned bytes into placeholders and text, within the result's budgets.
struct Presenter<'a> {
    pictures: &'a mut Vec<Picture>,
    /// A blob id met again names the same picture rather than a second copy of its bytes.
    blob_pictures: HashMap<String, usize>,
    text_left: usize,
}

impl Presenter<'_> {
    fn blob(&mut self, id: &str, bytes: &[u8]) -> String {
        if let Some(number) = self.blob_pictures.get(id) {
            return format!("[picture {number}]");
        }
        let presented = self.bytes(bytes);
        if picture_type(bytes).is_some() {
            self.blob_pictures
                .insert(id.to_owned(), self.pictures.len());
        }
        presented
    }

    fn bytes(&mut self, bytes: &[u8]) -> String {
        if let Some(media_type) = picture_type(bytes) {
            self.pictures.push(Picture {
                media_type: media_type.to_owned(),
                bytes: bytes.to_vec(),
            });
            return format!("[picture {}]", self.pictures.len());
        }
        match std::str::from_utf8(bytes) {
            Ok(text) if text.len() <= self.text_left => {
                self.text_left -= text.len();
                text.to_owned()
            }
            Ok(text) => format!(
                "[{} bytes of returned text not shown: this result already holds {} MiB of it]",
                text.len(),
                TEXT_BUDGET / (1024 * 1024)
            ),
            Err(_) => format!(
                "[{} returned bytes that are neither a picture nor text]",
                bytes.len()
            ),
        }
    }
}

/// `{"$neo":"bytes","base64":…}` with exactly those keys, decoded.
fn bytes_marker(value: &Value) -> Option<Vec<u8>> {
    let object = value.as_object()?;
    if object.len() != 2 || object.get("$neo")?.as_str()? != "bytes" {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(object.get("base64")?.as_str()?)
        .ok()
}

/// `{"$neo":"blob","id":"b<n>","size":<n>}` with exactly those keys and the engine's canonical
/// id spelling: any other object with the same names is the script's own data.
fn blob_marker(value: &Value) -> Option<(&str, u64)> {
    let object = value.as_object()?;
    if object.len() != 3 || object.get("$neo")?.as_str()? != "blob" {
        return None;
    }
    let id = object.get("id")?.as_str()?;
    let index: u32 = id.strip_prefix('b')?.parse().ok()?;
    if id != format!("b{index}") {
        return None;
    }
    Some((id, object.get("size")?.as_u64()?))
}

/// The picture formats every model provider here accepts, by their magic bytes.
fn picture_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    fn engine_reply(value: Value, suffix: &str, is_error: bool) -> Reply {
        let text = format!("{value}{suffix}");
        Reply::parse(json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": { "value": value },
            "isError": is_error,
            "_meta": { RUN_META: { "id": "run-1", "status": "ok" } }
        }))
        .unwrap_or_else(|_| Reply {
            run_id: None,
            text: String::new(),
            value: None,
            is_error,
        })
    }

    fn inner(output: &ToolOutput) -> &str {
        let after_open = output.text.split_once('\n').map_or("", |(_, rest)| rest);
        after_open.rsplit_once('\n').map_or("", |(body, _)| body)
    }

    #[test]
    fn a_plain_value_keeps_the_engines_text_and_is_fenced() {
        let output = engine_reply(json!(2), "", false).present(&HashMap::new());
        assert_eq!(inner(&output), "2");
        assert!(output.text.starts_with("[UNTRUSTED_PAGE_CONTENT nonce="));
        assert!(output.pictures.is_empty());
        assert!(!output.is_error);
    }

    #[test]
    fn inline_bytes_and_a_fetched_blob_become_numbered_pictures_and_keep_the_console() {
        let base64 = base64::engine::general_purpose::STANDARD.encode(PNG);
        let value = json!({
            "shot": { "$neo": "bytes", "base64": base64 },
            "full": { "$neo": "blob", "id": "b0", "size": PNG.len() },
            "label": "two"
        });
        let reply = engine_reply(value, "\nconsole:\nkept", false);
        assert_eq!(
            reply.blob_requests(),
            Some(BTreeMap::from([("b0".to_owned(), PNG.len() as u64)]))
        );
        let blobs = HashMap::from([("b0".to_owned(), PNG.to_vec())]);
        let output = reply.present(&blobs);
        // serde_json orders keys, so "full" is met before "shot".
        assert_eq!(
            inner(&output),
            r#"{"full":"[picture 1]","label":"two","shot":"[picture 2]"}
console:
kept"#
        );
        assert_eq!(output.pictures.len(), 2);
        assert_eq!(output.pictures[0].media_type, "image/png");
        assert_eq!(output.pictures[0].bytes, PNG);
    }

    #[test]
    fn text_bytes_are_inlined_and_other_bytes_are_described() {
        let text = base64::engine::general_purpose::STANDARD.encode("long text");
        let pdf = base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.4\xff\xfe");
        let value = json!([
            { "$neo": "bytes", "base64": text },
            { "$neo": "bytes", "base64": pdf },
            { "$neo": "blob", "id": "b3", "size": 9 }
        ]);
        let output = engine_reply(value, "", false).present(&HashMap::new());
        assert_eq!(
            inner(&output),
            r#"["long text","[10 returned bytes that are neither a picture nor text]","[9 returned bytes that could not be read]"]"#
        );
    }

    #[test]
    fn a_repeated_blob_is_one_picture_and_repeated_text_stops_at_its_budget() {
        let text = "t".repeat(TEXT_BUDGET / 2 + 1);
        let blobs = HashMap::from([
            ("b0".to_owned(), PNG.to_vec()),
            ("b1".to_owned(), text.clone().into_bytes()),
        ]);
        let picture = json!({ "$neo": "blob", "id": "b0", "size": PNG.len() });
        let words = json!({ "$neo": "blob", "id": "b1", "size": text.len() });
        let value = json!([picture.clone(), picture, words.clone(), words]);
        let output = engine_reply(value, "", false).present(&blobs);
        assert_eq!(output.pictures.len(), 1, "one copy of the picture's bytes");
        let shown: Value = serde_json::from_str(inner(&output)).unwrap_or(Value::Null);
        assert_eq!(shown[0], "[picture 1]");
        assert_eq!(shown[1], "[picture 1]");
        assert_eq!(shown[2], Value::String(text.clone()));
        assert_eq!(
            shown[3],
            format!(
                "[{} bytes of returned text not shown: this result already holds 16 MiB of it]",
                text.len()
            )
        );
    }

    #[test]
    fn an_object_that_only_looks_like_a_marker_is_the_scripts_data() {
        let value = json!([
            { "$neo": "blob", "id": "b01", "size": 3 },
            { "$neo": "blob", "id": "b1", "size": 3, "extra": true },
            { "$neo": "bytes", "base64": "not base64!" }
        ]);
        let reply = engine_reply(value.clone(), "", false);
        assert_eq!(reply.blob_requests(), Some(BTreeMap::new()));
        let output = reply.present(&HashMap::new());
        assert_eq!(inner(&output), value.to_string());
    }

    #[test]
    fn a_failed_script_is_an_error_output_with_the_engines_message() -> Result<(), String> {
        let message = "script / result_too_large: the script returned 400002 bytes of JSON";
        let reply = Reply::parse(json!({
            "content": [{ "type": "text", "text": message }],
            "structuredContent": { "error": { "code": "result_too_large" } },
            "isError": true
        }))?;
        let output = reply.present(&HashMap::new());
        assert!(output.is_error);
        assert_eq!(inner(&output), message);
        Ok(())
    }
}
