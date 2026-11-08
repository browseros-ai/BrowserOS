//! Bounds model-visible run reports; retained originals are shared across MCP sessions.
//! Each truncated field of a retained original is also saved as a file the LRU deletes.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::future;
use tokio::io::AsyncWriteExt;

use serde::{Serialize, Serializer};
use serde_json::{Map, Value, json};

pub(super) const BUDGET: usize = 24_000;
/// Largest continuation window (the med-fix-b decision asks for at least 100 000). Prose runs
/// about 4 bytes per token, so this is near Claude Code's default 25 000-token MCP output cap;
/// denser text (base64, minified code) costs more tokens per byte and may be cut by such a
/// client, which then wants a smaller window or the saved `file`.
pub(super) const LIMIT_MAX: usize = 100_000;
/// Smallest continuation window: the ancestor `context` (`CONTEXT_BUDGET`) and window metadata
/// must still leave room for text.
pub(super) const LIMIT_MIN: usize = 4_000;
/// Directory under the host's state dir that holds saved originals.
pub(super) const RESULTS_DIR: &str = "run-results";
/// A slow disk may drop a reply's `file` links, never delay the reply past this.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Saved files older than this are swept, whichever process wrote them. Eviction deletes a
/// process's own files; the sweep catches files of crashed processes and writes that
/// finished after their entry was evicted, without touching another live process's
/// recent results (stdio servers can share one state dir).
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const LOG_BUDGET: usize = 2_000;
const CALL_BUDGET: usize = 4_000;
const ERROR_BUDGET: usize = 4_000;
const CONTEXT_BUDGET: usize = 2_000;
const MAX_RESULTS: usize = 16;
const STORE_BUDGET: usize = 64 * 1024 * 1024;
/// A big array element or object entry kept beside cut siblings shows at least this many
/// encoded bytes, or a `MIN_SHARE_PARTS`th of a smaller container's budget, so a nested
/// record still splits its share among its fields.
const MIN_SHARE: usize = 2_000;
const MIN_SHARE_PARTS: usize = 8;
/// Listed cuts cost about 150 bytes each, so the list stays near 3 KB of the budget.
const MAX_CUTS: usize = 20;
/// Metadata grows with the cuts it lists; each attempt shrinks the value by the overshoot.
const FIT_ATTEMPTS: usize = 8;

struct Retained {
    id: String,
    original: Value,
    /// Serialized JSON plus the saved files' text: the store budget bounds memory and disk.
    bytes: usize,
    files: Vec<PathBuf>,
}

/// Process-local LRU. Its owner shares one mutex across transport-local service clones.
#[derive(Default)]
pub(super) struct Store {
    entries: VecDeque<Retained>,
    bytes: usize,
}

/// Disk work for one bounded reply. The store stays free of IO; its owner applies this after
/// releasing the store, so disk speed never stalls other sessions' replies or retrievals.
#[derive(Default)]
#[must_use]
pub(super) struct Disk {
    dir: Option<PathBuf>,
    removes: Vec<PathBuf>,
    writes: Vec<(&'static str, PathBuf, String)>,
}

impl Disk {
    /// Saves each truncated field's full text, deletes evicted results' files and sweeps
    /// stale ones, all within one `WRITE_TIMEOUT`. A failed or late write drops that field's
    /// `file` link and logs a warning; it never fails the reply. A write still running at the
    /// deadline may finish after its entry's eviction; the stale sweep removes that file.
    pub(super) async fn apply(self, mut result: Value) -> Value {
        let deadline = tokio::time::Instant::now() + WRITE_TIMEOUT;
        let cleanup = async {
            for path in &self.removes {
                if let Err(error) = tokio::fs::remove_file(path).await
                    && error.kind() != io::ErrorKind::NotFound
                {
                    tracing::warn!(path = %path.display(), %error, "could not delete an evicted run result");
                }
            }
            if let Some(dir) = &self.dir {
                sweep(dir).await;
            }
        };
        let saves = future::join_all(self.writes.into_iter().map(
            |(field, path, text)| async move {
                let saved = tokio::time::timeout_at(deadline, save(&path, text)).await;
                (field, path, saved)
            },
        ));
        let (cleaned, saved) = tokio::join!(tokio::time::timeout_at(deadline, cleanup), saves);
        if cleaned.is_err() {
            tracing::warn!("run result cleanup timed out after {WRITE_TIMEOUT:?}");
        }
        for (field, path, saved) in saved {
            let error = match saved {
                Ok(Ok(())) => continue,
                Ok(Err(error)) => error.to_string(),
                Err(_) => format!("timed out after {WRITE_TIMEOUT:?}"),
            };
            tracing::warn!(path = %path.display(), %error, "could not save a truncated run result");
            if let Some(meta) = result["truncated"][field].as_object_mut() {
                meta.remove("file");
            }
        }
        result
    }
}

/// Writes `text` readable only by this user: results can hold logged-in pages.
async fn save(path: &Path, text: String) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        let mut builder = tokio::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(dir).await?;
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    file.write_all(text.as_bytes()).await?;
    // Platform: tokio::fs::File finishes writes in the background unless flushed before drop.
    file.flush().await
}

/// Deletes saved files older than `STALE_AFTER`. Only files it judged stale, never the folder,
/// so a sweep that outlives its deadline cannot remove a fresh write.
async fn sweep(dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let stale = entry.metadata().await.is_ok_and(|meta| {
            meta.is_file()
                && meta
                    .modified()
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > STALE_AFTER)
        });
        if stale
            && let Err(error) = tokio::fs::remove_file(entry.path()).await
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(path = %entry.path().display(), %error, "could not sweep a stale run result");
        }
    }
}

/// What truncation metadata links to: the stored result and, when the host saves them, the
/// absolute directory of its files.
struct Link<'a> {
    id: &'a str,
    files: Option<&'a Path>,
}

impl Link<'_> {
    fn file(&self, field: &str, value: &Value) -> Option<PathBuf> {
        // The file holds exactly the text continuation offsets count into.
        let extension = if field == "value" && value.is_string() {
            "txt"
        } else {
            "json"
        };
        self.files
            .map(|dir| dir.join(format!("{}-{field}.{extension}", self.id)))
    }
}

impl Store {
    /// Bounds `original` to the reply budget. With `files`, an absolute directory, each truncated
    /// field of a retained original also gets a `file` link that the returned `Disk` writes.
    pub(super) fn limit(&mut self, original: Value, files: Option<&Path>) -> (Value, Disk) {
        let bytes = original.to_string().len();
        if bytes <= BUDGET {
            return (original, Disk::default());
        }
        let id = loop {
            let id = format!("r{:012x}", rand::random::<u64>() & 0xffff_ffff_ffff);
            if !self.entries.iter().any(|entry| entry.id == id) {
                break id;
            }
        };
        let retained = bytes <= STORE_BUDGET;
        // Only an LRU entry can delete its files, so an unretained original gets none.
        let files = files.filter(|_| retained);
        let link = Link { id: &id, files };
        let mut bounded = bound(&original, &link, retained);
        let mut disk = Disk {
            dir: files.map(Path::to_path_buf),
            ..Disk::default()
        };
        for field in ["value", "logs", "calls", "error"] {
            if bounded["truncated"][field].get("file").is_some()
                && let Some(path) = link.file(field, &original[field])
            {
                disk.writes
                    .push((field, path, field_text(field, &original[field])));
            }
        }
        let saved = disk
            .writes
            .iter()
            .map(|(_, _, text)| text.len())
            .sum::<usize>();
        let bytes = if bytes + saved <= STORE_BUDGET {
            bytes + saved
        } else {
            // Files would push this result past the store budget; keep it retrievable only.
            for (field, _, _) in disk.writes.drain(..) {
                if let Some(meta) = bounded["truncated"][field].as_object_mut() {
                    meta.remove("file");
                }
            }
            bytes
        };
        if retained {
            while self.entries.len() >= MAX_RESULTS || self.bytes + bytes > STORE_BUDGET {
                if let Some(old) = self.entries.pop_front() {
                    self.bytes -= old.bytes;
                    disk.removes.extend(old.files);
                } else {
                    break;
                }
            }
            self.bytes += bytes;
            self.entries.push_back(Retained {
                id,
                original,
                bytes,
                files: disk
                    .writes
                    .iter()
                    .map(|(_, path, _)| path.clone())
                    .collect(),
            });
        }
        (bounded, disk)
    }

    /// Reads one window of at most `limit` bytes of a stored field, or of the node `path`
    /// (RFC 6901) names in it.
    pub(super) fn retrieve(
        &mut self,
        id: &str,
        field: &str,
        path: Option<&str>,
        offset: usize,
        limit: usize,
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
        let root = entry
            .original
            .get(field)
            .ok_or_else(|| format!("result has no {field} field"))?;
        let value = match path {
            None => root,
            Some(path) => root.pointer(path).ok_or_else(|| {
                format!("{field} has no node at path {path:?}; path is a JSON Pointer such as /snap or /items/0")
            })?,
        };
        // A string node reads as raw text, like a string value; other nodes as pretty JSON.
        let text = node_text(value, field == "value" || path.is_some());
        if offset > text.len() || !text.is_char_boundary(offset) || inside_ref(&text, offset) {
            return Err(
                "offset must be a UTF-8 byte boundary within the field, outside a [ref=…] token"
                    .into(),
            );
        }
        let mut out = json!({"status":"ok", "value":"", "logs":[], "calls":[], "pages":[],
            "truncated":{"result":id, "field":field, "offset":offset,"end":offset,"total":text.len(), "type":value_type(value)}});
        if let Some(path) = path {
            out["truncated"]["path"] = json!(path);
        }
        if field == "value" && value.is_string() {
            let (context, clipped) = ancestors(&text, offset);
            if !context.is_empty() {
                out["truncated"]["context"] = json!(context);
            }
            if clipped {
                out["truncated"]["contextTruncated"] = json!(true);
            }
        }
        let end = fit_window(&text[offset..], limit, |shown| {
            let end = offset + shown;
            out["value"] = json!(&text[offset..end]);
            out["truncated"]["end"] = json!(end);
            if end < text.len() {
                out["truncated"]["next"] = next(id, field, path, end);
                // Following `next` verbatim keeps the window size the client asked for.
                if limit != BUDGET {
                    out["truncated"]["next"]["limit"] = json!(limit);
                }
            } else if let Some(meta) = out["truncated"].as_object_mut() {
                meta.remove("next");
            }
            out.to_string().len() <= limit
        });
        if end == 0 && offset < text.len() {
            return Err("a single ref token cannot fit in a retrieval window".into());
        }
        Ok(out)
    }
}

fn field_text(field: &str, value: &Value) -> String {
    node_text(value, field == "value")
}

fn node_text(value: &Value, raw_strings: bool) -> String {
    if raw_strings && let Some(text) = value.as_str() {
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

fn next(id: &str, field: &str, path: Option<&str>, offset: usize) -> Value {
    let mut next = json!({"result":id, "field":field, "offset":offset});
    if let Some(path) = path {
        next["path"] = json!(path);
    }
    next
}

fn metadata(link: &Link, field: &str, shown: usize, total: usize, value: &Value) -> Value {
    let mut meta = json!({"shown":shown,"total":total,"type":value_type(value)});
    if shown < total {
        meta["next"] = next(link.id, field, None, shown);
        if let Some(file) = link.file(field, value) {
            meta["file"] = json!(file.to_string_lossy());
        }
    }
    meta
}

fn bound(original: &Value, link: &Link, retained: bool) -> Value {
    let id = link.id;
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
        mark_changed(&mut out, original, link, "logs");
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
        mark_calls(&mut out, original, link);
    }
    if let Some(message) = original["error"]["message"].as_str()
        && message.len() > ERROR_BUDGET
    {
        let end = fit_prefix(message, BUDGET, |shown| {
            serde_json::to_string(&message[..shown])
                .unwrap_or_default()
                .len()
                <= ERROR_BUDGET
        });
        out["error"]["message"] = json!(&message[..end]);
        mark_changed(&mut out, original, link, "error");
    }
    if original.get("value").is_some() {
        out["value"] = json!("");
    }
    // Recovery identities and errors take priority over optional log/trace excerpts.
    if out.to_string().len() > BUDGET {
        out["logs"] = json!([]);
        out["calls"] = json!([]);
        mark_changed(&mut out, original, link, "logs");
        mark_calls(&mut out, original, link);
    }
    match original.get("value") {
        Some(value @ Value::String(text)) => {
            fit_prefix(text, BUDGET, |shown| {
                out["value"] = json!(&text[..shown]);
                out["truncated"]["value"] = metadata(link, "value", shown, text.len(), value);
                out.to_string().len() <= BUDGET
            });
        }
        Some(value) => fit_value(&mut out, value, link),
        None => {}
    }
    if !retained {
        for field in ["value", "logs", "calls", "error"] {
            if let Some(meta) = out["truncated"][field].as_object_mut() {
                meta.remove("next");
            }
        }
        if let Some(cuts) = out["truncated"]["value"]["cut"].as_array_mut() {
            for cut in cuts.iter_mut().filter_map(Value::as_object_mut) {
                cut.remove("next");
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

fn mark_changed(out: &mut Value, original: &Value, link: &Link, field: &str) {
    if out[field] != original[field] {
        let text = field_text(field, &original[field]);
        // Array excerpts and clipped errors are not byte prefixes of their original JSON.
        out["truncated"][field] = metadata(link, field, 0, text.len(), &original[field]);
    }
}

fn mark_calls(out: &mut Value, original: &Value, link: &Link) {
    mark_changed(out, original, link, "calls");
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

/// Bounds a non-string value without changing its JSON type. Small leaves stay whole; the
/// room goes to oversized strings, arrays and objects, each listed in `cut` with its own
/// `next`. Nothing shown is a byte prefix of the field's pretty JSON, so the whole-field
/// `next` reads that text from offset 0, as for clipped logs and calls.
fn fit_value(out: &mut Value, value: &Value, link: &Link) {
    out["value"] = Value::Null;
    let mut room = (BUDGET + "null".len()).saturating_sub(out.to_string().len());
    let size = encoded_len(value);
    if size <= room {
        out["value"] = value.clone();
        return;
    }
    let total = pretty_len(value);
    for attempt in 1..=FIT_ATTEMPTS {
        // A value that keeps missing gives up its content, never the recovery metadata.
        if attempt == FIT_ATTEMPTS {
            room = 0;
        }
        let mut cuts = Vec::new();
        out["value"] = shrink(value, size, room, &mut String::new(), &mut cuts);
        let mut meta = metadata(link, "value", 0, total, value);
        meta["cut"] = cuts
            .iter()
            .take(MAX_CUTS)
            .map(|cut| cut.metadata(link.id))
            .collect();
        if cuts.len() > MAX_CUTS {
            meta["cutOmitted"] = json!(cuts.len() - MAX_CUTS);
        }
        out["truncated"]["value"] = meta;
        let len = out.to_string().len();
        if len <= BUDGET {
            return;
        }
        // Shrink from what was shown, not the room: a value can leave slack under its room.
        room = encoded_len(&out["value"]).saturating_sub(len - BUDGET);
    }
}

/// One oversized node of a bounded value, addressed by its RFC 6901 JSON Pointer.
struct Cut {
    path: String,
    kind: Kind,
}

enum Kind {
    /// The value shows the first `shown` of `total` UTF-8 bytes.
    Text { shown: usize, total: usize },
    /// The value keeps the first `kept` of `length` elements or entries; the omitted ones
    /// start at byte `offset` of the node's pretty JSON.
    Entries {
        array: bool,
        kept: usize,
        length: usize,
        offset: usize,
    },
}

impl Cut {
    fn metadata(&self, id: &str) -> Value {
        let path = Some(self.path.as_str());
        match self.kind {
            Kind::Text { shown, total } => json!({"path":self.path, "type":"string",
                "shown":shown, "total":total, "next":next(id, "value", path, shown)}),
            Kind::Entries {
                array,
                kept,
                length,
                offset,
            } => json!({"path":self.path, "type":if array {"array"} else {"object"},
                "kept":kept, "length":length, "next":next(id, "value", path, offset)}),
        }
    }
}

/// Fits `value` (`size` encoded bytes) into `budget` encoded bytes, recording each cut node in
/// document order. Only a budget below a node's smallest form (`""`, `[]`, `{}` or a scalar)
/// can overshoot, and `fit_value` reports that as metadata over budget.
fn shrink(
    value: &Value,
    size: usize,
    budget: usize,
    path: &mut String,
    cuts: &mut Vec<Cut>,
) -> Value {
    if size <= budget {
        return value.clone();
    }
    let entries: Vec<(Option<&String>, &Value)> = match value {
        Value::String(text) => {
            let shown = leaf_end(text, budget.saturating_sub(2));
            cuts.push(Cut {
                path: path.clone(),
                kind: Kind::Text {
                    shown,
                    total: text.len(),
                },
            });
            return json!(&text[..shown]);
        }
        Value::Array(items) => items.iter().map(|item| (None, item)).collect(),
        Value::Object(map) => map.iter().map(|(key, item)| (Some(key), item)).collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) => return value.clone(),
    };
    let array = value.is_array();
    let sizes: Vec<usize> = entries.iter().map(|(_, item)| encoded_len(item)).collect();
    let least: Vec<usize> = entries
        .iter()
        .zip(&sizes)
        .map(|((_, item), &size)| match item {
            Value::String(_) | Value::Array(_) | Value::Object(_) => size.min(2),
            Value::Null | Value::Bool(_) | Value::Number(_) => size,
        })
        .collect();
    // `"key":` for an object entry; a separator precedes every kept entry but the first.
    let costs: Vec<usize> = entries
        .iter()
        .map(|(key, _)| key.map_or(0, |key| quoted_len(key) + 1))
        .collect();
    // A big entry kept beside cut siblings shows at least `floor` bytes, so a container drops
    // entries rather than showing many unreadable fragments.
    let floor = MIN_SHARE.min(budget / MIN_SHARE_PARTS);
    let order: Vec<usize> = if array {
        // A tail, never a middle element: kept indices must still match the original.
        (0..entries.len()).collect()
    } else {
        // Small sibling keys identify a record, so they go before any big entry.
        let (small, big): (Vec<usize>, Vec<usize>) =
            (0..entries.len()).partition(|&index| sizes[index] <= floor);
        small.into_iter().chain(big).collect()
    };
    let mut keep = vec![false; entries.len()];
    let mut used = 2;
    let mut kept: usize = 0;
    for index in order {
        let add = usize::from(kept > 0) + costs[index] + floor.max(least[index]).min(sizes[index]);
        if used + add > budget {
            break;
        }
        used += add;
        keep[index] = true;
        kept += 1;
    }
    let chosen: Vec<usize> = (0..entries.len()).filter(|&index| keep[index]).collect();
    let fixed =
        2 + kept.saturating_sub(1) + chosen.iter().map(|&index| costs[index]).sum::<usize>();
    let level = water_level(
        &chosen.iter().map(|&index| sizes[index]).collect::<Vec<_>>(),
        &chosen.iter().map(|&index| least[index]).collect::<Vec<_>>(),
        budget.saturating_sub(fixed),
    );
    if kept < entries.len() {
        // Only a kept prefix leaves the omitted entries contiguous in the pretty JSON.
        let prefix = chosen.last().is_none_or(|&last| last + 1 == kept);
        cuts.push(Cut {
            path: path.clone(),
            kind: Kind::Entries {
                array,
                kept,
                length: entries.len(),
                offset: if prefix {
                    omitted_offset(value, kept)
                } else {
                    0
                },
            },
        });
    }
    let depth = path.len();
    let mut shown = Vec::with_capacity(kept);
    for index in chosen {
        let (key, item) = entries[index];
        match key {
            Some(key) => push_token(path, key),
            None => push_token(path, &index.to_string()),
        }
        let share = level.max(least[index]).min(sizes[index]);
        shown.push((key, shrink(item, sizes[index], share, path, cuts)));
        path.truncate(depth);
    }
    if array {
        Value::Array(shown.into_iter().map(|(_, item)| item).collect())
    } else {
        Value::Object(
            shown
                .into_iter()
                .filter_map(|(key, item)| key.map(|key| (key.clone(), item)))
                .collect(),
        )
    }
}

/// The highest per-entry share whose total fits `room`: entries smaller than it stay whole,
/// larger ones get exactly it, and none goes below its smallest form.
fn water_level(sizes: &[usize], least: &[usize], room: usize) -> usize {
    let used = |level: usize| {
        sizes
            .iter()
            .zip(least)
            .map(|(&size, &least)| level.max(least).min(size))
            .sum::<usize>()
    };
    let (mut low, mut high) = (0, sizes.iter().copied().max().unwrap_or(0));
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if used(middle) <= room {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    low
}

/// Where the first omitted entry starts in the node's pretty JSON. The first `kept` entries
/// print the same on their own, except that their closing `\n]` is `,\n` in the original.
/// Falls back to 0 when retrieval would read the offset as inside a ref token.
fn omitted_offset(value: &Value, kept: usize) -> usize {
    if kept == 0 {
        // After the opening `[\n` or `{\n`.
        return 2;
    }
    let head = match value {
        Value::Array(items) => serde_json::to_string_pretty(&items[..kept]),
        Value::Object(map) => serde_json::to_string_pretty(&Entries(map, kept)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => return 0,
    }
    .unwrap_or_default();
    let shared = head.get(..head.len().saturating_sub(2)).unwrap_or_default();
    let open_ref = shared.rfind('[').is_some_and(|start| {
        shared[start..].starts_with("[ref=") && !shared[start..].contains(']')
    });
    if open_ref { 0 } else { head.len() }
}

/// The first entries of an object, serialized as an object of their own.
struct Entries<'a>(&'a Map<String, Value>, usize);

impl Serialize for Entries<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().take(self.1))
    }
}

/// The longest prefix whose JSON escaping fits `room` bytes, ending outside a ref token and
/// after a complete line when that keeps at least half of it. Unlike a top-level string, a
/// leaf never shrinks to its first line: one early newline must not hide the rest.
fn leaf_end(text: &str, room: usize) -> usize {
    let mut used = 0;
    let mut end = text.len();
    for (index, ch) in text.char_indices() {
        used += escaped_len(ch);
        if used > room {
            end = index;
            break;
        }
    }
    prefix_end(text, end, end / 2)
}

fn quoted_len(text: &str) -> usize {
    2 + text.chars().map(escaped_len).sum::<usize>()
}

/// Bytes `ch` takes inside a JSON string as serde_json writes it.
fn escaped_len(ch: char) -> usize {
    match ch {
        '"' | '\\' | '\u{8}' | '\u{c}' | '\n' | '\r' | '\t' => 2,
        '\0'..='\u{1f}' => 6,
        _ => ch.len_utf8(),
    }
}

fn push_token(path: &mut String, token: &str) {
    path.push('/');
    for ch in token.chars() {
        match ch {
            '~' => path.push_str("~0"),
            '/' => path.push_str("~1"),
            _ => path.push(ch),
        }
    }
}

/// Counts serialized bytes without building the text.
#[derive(Default)]
struct Count(usize);

impl io::Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encoded_len(value: &Value) -> usize {
    let mut count = Count::default();
    // A counter never fails a write, and a Value always serializes.
    let _ = serde_json::to_writer(&mut count, value);
    count.0
}

fn pretty_len(value: &Value) -> usize {
    let mut count = Count::default();
    let _ = serde_json::to_writer_pretty(&mut count, value);
    count.0
}

/// Maximize the encoded envelope, then prefer a complete line and keep ref tokens whole.
fn fit_prefix(text: &str, cap: usize, fits: impl FnMut(usize) -> bool) -> usize {
    fit(text, cap, false, fits)
}

/// Like `fit_prefix`, but ends after a line only when that keeps half the window, so one
/// early newline (pretty JSON's `{\n`) cannot shrink a retrieval window to a few bytes.
fn fit_window(text: &str, cap: usize, fits: impl FnMut(usize) -> bool) -> usize {
    fit(text, cap, true, fits)
}

fn fit(text: &str, cap: usize, half_lines: bool, mut fits: impl FnMut(usize) -> bool) -> usize {
    if text.len() <= cap && fits(text.len()) {
        return text.len();
    }
    let (mut low, mut high) = (0, text.len().min(cap));
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if fits(char_boundary(text, middle)) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let end = prefix_end(text, low, if half_lines { low / 2 } else { 0 });
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
    text[..offset].rfind('[').filter(|&start| {
        text[start..].starts_with("[ref=")
            && text[start..]
                .find(']')
                .is_some_and(|end| offset <= start + end)
    })
}

/// Cuts at `limit`, or after the last line ending at or past `min_line`, then before any ref
/// token the cut would split.
fn prefix_end(text: &str, limit: usize, min_line: usize) -> usize {
    let mut end = char_boundary(text, limit.min(text.len()));
    if end == text.len() {
        return end;
    }
    if let Some(newline) = text[..end].rfind('\n')
        && newline + 1 >= min_line
    {
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
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use serde_json::{Value, json};

    use super::{BUDGET, Store};

    fn report(value: Value) -> Value {
        json!({"status":"ok", "value":value, "logs":[], "calls":[],
            "pages":[{"id":"p1", "created":true, "closed":false,"url":"https://example.com"}]})
    }

    fn remainder(store: &mut Store, mut next: Value, mut text: String) -> String {
        while next.is_object() {
            let limit = next["limit"]
                .as_u64()
                .map_or(BUDGET, |limit| limit as usize);
            let window = store
                .retrieve(
                    next["result"].as_str().expect("expected test value"),
                    next["field"].as_str().expect("expected test value"),
                    next["path"].as_str(),
                    next["offset"].as_u64().expect("expected test value") as usize,
                    limit,
                )
                .expect("expected test value");
            assert!(window.to_string().len() <= limit);
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
        assert_eq!(Store::default().limit(original.clone(), None).0, original);
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
        let bounded = store.limit(report(json!(text)), None).0;
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
                None,
                prefix.len(),
                BUDGET,
            )
            .expect("expected test value");
        assert_eq!(
            window["truncated"]["context"],
            "- main [ref=e1]:\n  - list [ref=e2]:\n"
        );
        assert_eq!(remainder(&mut store, next, prefix.into()), text);

        let object = json!({"items":["quoted \" \\ \n 🦀".repeat(12_000)]});
        let bounded = store.limit(report(object.clone()), None).0;
        assert!(bounded.to_string().len() <= BUDGET);
        assert_eq!(bounded["truncated"]["value"]["type"], "object");
        let shown = bounded["value"]["items"][0]
            .as_str()
            .expect("escaped leaf keeps its type");
        assert!(
            shown.len() > 10_000,
            "escaped leaf shows {} bytes",
            shown.len()
        );
        assert_eq!(
            remainder(
                &mut store,
                bounded["truncated"]["value"]["cut"][0]["next"].clone(),
                shown.into()
            ),
            object["items"][0]
        );
        assert_eq!(
            remainder(
                &mut store,
                bounded["truncated"]["value"]["next"].clone(),
                String::new()
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
        let bounded = store.limit(original.clone(), None).0;
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
                store.limit(report(json!("🦀".repeat(10_000))), None).0["truncated"]["result"]
                    .as_str()
                    .expect("expected test value")
                    .to_string(),
            );
        }
        assert!(store.retrieve(&ids[0], "value", None, 1, BUDGET).is_err());
        assert!(
            store
                .retrieve(&ids[0], "value", None, 40_001, BUDGET)
                .is_err()
        );
        assert_eq!(
            store
                .retrieve(&ids[0], "value", None, 0, BUDGET)
                .expect("expected test value")["status"],
            "ok"
        );
        let _ = store.limit(report(json!("x".repeat(40_000))), None);
        assert_eq!(
            store
                .retrieve(&ids[1], "value", None, 0, BUDGET)
                .expect("expected test value")["status"],
            "error"
        );
        assert_eq!(
            store
                .retrieve(&ids[0], "value", None, 0, BUDGET)
                .expect("expected test value")["status"],
            "ok"
        );
        assert_eq!(
            store
                .retrieve("missing", "value", None, 0, BUDGET)
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
            let bounded = store.limit(report(json!(text)), None).0;
            let id = bounded["truncated"]["result"]
                .as_str()
                .expect("expected test value");
            for offset in padding + 1..padding + 10 {
                assert!(store.retrieve(id, "value", None, offset, BUDGET).is_err());
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
        let text = format!("ordinary text [ref={}", "x".repeat(40_000));
        let bounded = store.limit(report(json!(text)), None).0;
        assert_eq!(
            remainder(
                &mut store,
                bounded["truncated"]["value"]["next"].clone(),
                bounded["value"].as_str().expect("string prefix").into()
            ),
            text
        );
        let mut failed = report(Value::Null);
        failed["status"] = json!("error");
        failed["error"] = json!({"name":"Error", "message":"\u{1}".repeat(40_000)});
        assert!(store.limit(failed, None).0.to_string().len() <= BUDGET);
    }

    fn aria(items: usize) -> String {
        format!(
            "- main [ref=e1]:\n  - list [ref=e2]:\n{}",
            (0..items)
                .map(|i| format!("    - button \"商品 {i} 🦀\" [ref=e{}]\n", i + 3))
                .collect::<String>()
        )
    }

    fn pretty(value: &Value) -> String {
        serde_json::to_string_pretty(value).expect("expected test value")
    }

    #[test]
    fn tests_that_an_object_keeps_small_keys_and_gives_the_budget_to_its_big_leaf() {
        let snap = aria(3000);
        assert!(snap.len() > 100_000);
        let original = json!({"snap":snap, "n":snap.len(), "url":"https://example.com/a"});
        let mut store = Store::default();
        let bounded = store.limit(report(original.clone()), None).0;
        assert!(bounded.to_string().len() <= BUDGET);
        assert_eq!(bounded["pages"][0]["id"], "p1");
        assert_eq!(bounded["value"]["n"], original["n"]);
        assert_eq!(bounded["value"]["url"], original["url"]);
        let prefix = bounded["value"]["snap"].as_str().expect("string leaf");
        assert!(prefix.len() > 20_000, "leaf shows {} bytes", prefix.len());
        assert!(snap.starts_with(prefix) && prefix.ends_with('\n'));
        let meta = &bounded["truncated"]["value"];
        let id = bounded["truncated"]["result"].clone();
        assert_eq!(meta["type"], "object");
        assert_eq!(meta["shown"], 0);
        assert_eq!(meta["total"], pretty(&original).len());
        assert_eq!(
            meta["next"],
            json!({"result":id, "field":"value", "offset":0})
        );
        assert_eq!(
            meta["cut"],
            json!([{"path":"/snap", "type":"string", "shown":prefix.len(), "total":snap.len(),
                "next":{"result":id, "field":"value", "path":"/snap", "offset":prefix.len()}}])
        );
        let window = store
            .retrieve(
                id.as_str().expect("expected test value"),
                "value",
                Some("/snap"),
                prefix.len(),
                BUDGET,
            )
            .expect("expected test value");
        assert_eq!(window["truncated"]["path"], "/snap");
        assert_eq!(
            window["truncated"]["context"],
            "- main [ref=e1]:\n  - list [ref=e2]:\n"
        );
        assert_eq!(
            remainder(&mut store, meta["cut"][0]["next"].clone(), prefix.into()),
            snap
        );
        // The whole-field continuation reads the same pretty JSON as before.
        assert_eq!(
            remainder(&mut store, meta["next"].clone(), String::new()),
            pretty(&original)
        );
    }

    #[test]
    fn tests_that_a_long_array_keeps_whole_leading_elements_and_its_tail_stays_retrievable() {
        let items = (0..5000)
            .map(|i| format!("Buy product {i:04} 漢字 🦀"))
            .collect::<Vec<_>>();
        let mut store = Store::default();
        let bounded = store.limit(report(json!(items)), None).0;
        assert!(bounded.to_string().len() <= BUDGET);
        let shown = bounded["value"].as_array().expect("array keeps its type");
        assert!(shown.len() > 500, "kept {} elements", shown.len());
        assert_eq!(
            *shown,
            json!(items[..shown.len()])
                .as_array()
                .expect("array")
                .clone()
        );
        let meta = &bounded["truncated"]["value"];
        assert_eq!(meta["type"], "array");
        let cut = &meta["cut"][0];
        assert_eq!(meta["cut"].as_array().expect("cut list").len(), 1);
        assert_eq!(cut["path"], "");
        assert_eq!(cut["type"], "array");
        assert_eq!(cut["kept"], shown.len());
        assert_eq!(cut["length"], 5000);
        assert_eq!(cut["next"]["path"], "");
        let tail = remainder(&mut store, cut["next"].clone(), String::new());
        assert!(tail.starts_with(&format!("  \"Buy product {:04}", shown.len())));
        let rest: Vec<String> =
            serde_json::from_str(&format!("[\n{tail}")).expect("tail is the omitted elements");
        assert_eq!(
            [
                json!(shown).as_array().expect("array").clone(),
                json!(rest).as_array().expect("array").clone()
            ]
            .concat(),
            json!(items).as_array().expect("array").clone()
        );
        assert_eq!(
            remainder(&mut store, meta["next"].clone(), String::new()),
            pretty(&json!(items))
        );
    }

    #[test]
    fn tests_that_nested_big_leaves_share_the_budget_under_escaped_paths() {
        let original = json!({"a/b":{"m~n":["x".repeat(60_000), "y".repeat(60_000), "short"]},
            "count":3, "ok":true, "none":null});
        let mut store = Store::default();
        let bounded = store.limit(report(original.clone()), None).0;
        assert!(bounded.to_string().len() <= BUDGET);
        let value = &bounded["value"];
        assert_eq!(
            (&value["count"], &value["ok"], &value["none"]),
            (&json!(3), &json!(true), &Value::Null)
        );
        let list = &value["a/b"]["m~n"];
        assert_eq!(list[2], "short");
        let cuts = bounded["truncated"]["value"]["cut"]
            .as_array()
            .expect("cut list");
        assert_eq!(
            cuts.iter()
                .map(|cut| cut["path"].clone())
                .collect::<Vec<_>>(),
            [json!("/a~1b/m~0n/0"), json!("/a~1b/m~0n/1")]
        );
        for (index, cut) in cuts.iter().enumerate() {
            let shown = list[index].as_str().expect("string leaf");
            assert!(
                shown.len() > 9_000,
                "leaf {index} shows {} bytes",
                shown.len()
            );
            assert_eq!(
                remainder(&mut store, cut["next"].clone(), shown.into()),
                original["a/b"]["m~n"][index]
            );
        }
        let id = bounded["truncated"]["result"]
            .as_str()
            .expect("expected test value");
        let small = store
            .retrieve(id, "value", Some("/a~1b/m~0n/2"), 0, BUDGET)
            .expect("expected test value");
        assert_eq!(small["value"], "short");
        assert!(small["truncated"].get("next").is_none());
        for path in ["snap", "/missing", "/a~1b/m~0n/9"] {
            assert!(store.retrieve(id, "value", Some(path), 0, BUDGET).is_err());
        }
    }

    #[test]
    fn tests_that_records_keep_small_fields_and_drop_whole_entries_instead_of_fragments() {
        let records = (0..400)
            .map(|i| json!({"id":i, "description":format!("{i} ").repeat(900)}))
            .collect::<Vec<_>>();
        let mut wide = (0..30)
            .map(|i| (format!("a{i:02}"), json!("w".repeat(10_000))))
            .collect::<serde_json::Map<_, _>>();
        wide.insert("z".into(), json!(1));
        let links = (0..3000)
            .map(|i| {
                (
                    format!("link {i:04}"),
                    json!(format!("https://example.com/{i}")),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let mut store = Store::default();
        for original in [json!(records), Value::Object(wide), Value::Object(links)] {
            let bounded = store.limit(report(original.clone()), None).0;
            assert!(bounded.to_string().len() <= BUDGET);
            let meta = &bounded["truncated"]["value"];
            let cuts = meta["cut"].as_array().expect("cut list");
            assert!(cuts.len() <= 20);
            for cut in cuts {
                if cut["type"] == "string" {
                    assert!(
                        cut["shown"].as_u64().expect("shown") >= 1_000,
                        "fragment: {cut}"
                    );
                }
                let path = cut["path"].as_str().expect("path");
                let text = remainder(&mut store, cut["next"].clone(), String::new());
                let node = original.pointer(path).expect("cut path names a node");
                let full = if cut["type"] == "string" {
                    node.as_str().expect("string").to_string()
                } else {
                    pretty(node)
                };
                assert!(
                    full.ends_with(&text),
                    "{path} continuation is a suffix of the node"
                );
            }
        }
        let wide = store
            .limit(
                report(json!({"z":1, "a":"w".repeat(30_000), "b":"w".repeat(30_000)})),
                None,
            )
            .0;
        assert_eq!(wide["value"]["z"], 1);
    }

    #[test]
    fn tests_that_leaf_cuts_stay_outside_ref_tokens_and_survive_an_early_newline() {
        let mut store = Store::default();
        let probe = store
            .limit(report(json!({"n":1, "snap":"x".repeat(40_000)})), None)
            .0;
        let room = probe["value"]["snap"].as_str().expect("string leaf").len();
        for padding in room - 12..room + 3 {
            let text = format!("{}[ref=e999]{}", "x".repeat(padding), "z".repeat(20_000));
            let bounded = store.limit(report(json!({"n":1, "snap":text})), None).0;
            assert!(bounded.to_string().len() <= BUDGET);
            let shown = bounded["value"]["snap"].as_str().expect("string leaf");
            assert!(!(padding + 1..padding + 10).contains(&shown.len()));
            assert_eq!(
                remainder(
                    &mut store,
                    bounded["truncated"]["value"]["cut"][0]["next"].clone(),
                    shown.into()
                ),
                text
            );
        }
        let text = format!("\n{}", "a".repeat(50_000));
        let bounded = store.limit(report(json!({"n":1, "snap":text})), None).0;
        assert!(
            bounded["value"]["snap"]
                .as_str()
                .expect("string leaf")
                .len()
                > 20_000
        );
    }

    #[test]
    fn tests_that_values_beside_large_traces_keep_their_type_and_bare_strings_their_shape() {
        let calls = (0..200)
            .map(|i| {
                json!({"callId":i, "method":"click", "title":"Click",
                "params":{"text":"x".repeat(1000)}, "result":{"kind":"ok"}, "delivery":"done"})
            })
            .collect::<Vec<_>>();
        let mut store = Store::default();
        for value in [
            json!({"guests":"32 Guests", "count":32}),
            json!([1, 2]),
            Value::Null,
            json!(7),
        ] {
            let mut original = report(value.clone());
            original["calls"] = json!(calls);
            let bounded = store.limit(original, None).0;
            assert!(bounded.to_string().len() <= BUDGET);
            assert_eq!(bounded["value"], value);
            assert!(bounded["truncated"].get("value").is_none());
        }
        // A bare string keeps today's shape: a prefix and byte-offset metadata, no cut list.
        let bounded = store.limit(report(json!("abc".repeat(30_000))), None).0;
        let shown = bounded["value"].as_str().expect("string prefix").len();
        let id = bounded["truncated"]["result"].clone();
        assert_eq!(
            bounded["truncated"]["value"],
            json!({"shown":shown, "total":90_000, "type":"string",
                "next":{"result":id, "field":"value", "offset":shown}})
        );
    }

    #[test]
    fn tests_that_escape_costs_match_serde() {
        let text = format!(
            "{}\u{7f}é🦀\\\"",
            (0..0x20_u8).map(char::from).collect::<String>()
        );
        assert_eq!(super::quoted_len(&text), json!(text).to_string().len());
        let value = json!({"a":[text, 1.5, null], "b/c":{"d":true}});
        assert_eq!(super::encoded_len(&value), value.to_string().len());
        assert_eq!(super::pretty_len(&value), pretty(&value).len());
    }

    /// Deterministic generator, so a failure names a reproducible case.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, bound: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) % bound as u64) as usize
        }

        /// A random value of about `room` bytes at most, so cases stay retained and quick.
        fn tree(&mut self, depth: usize, room: &mut usize) -> Value {
            let pieces = [
                "x",
                "\n",
                "  - link \"商品 🦀\" [ref=e12]\n",
                "\"",
                "\\",
                "\u{1}",
                "é",
            ];
            let leaf = depth == 0 || *room < 100;
            match self.below(if leaf { 3 } else { 6 }) {
                0 => {
                    let len = [3, 40, 1_500, 9_000, 70_000][self.below(5)].min(*room);
                    *room = room.saturating_sub(len);
                    let mut text = String::new();
                    while text.len() < len {
                        text.push_str(pieces[self.below(pieces.len())]);
                    }
                    json!(text)
                }
                1 => json!(self.below(1_000_000)),
                2 => [Value::Null, json!(true), json!(-1.5)][self.below(3)].clone(),
                3 | 4 => {
                    let len = [2, 5, 40, 2_000][self.below(4)];
                    Value::Array((0..len).map(|_| self.tree(depth - 1, room)).collect())
                }
                _ => {
                    let len = [1, 3, 8, 300][self.below(4)];
                    Value::Object(
                        (0..len)
                            .map(|i| {
                                (
                                    format!("k{i}{}", ["", "/", "~"][self.below(3)]),
                                    self.tree(depth - 1, room),
                                )
                            })
                            .collect(),
                    )
                }
            }
        }
    }

    /// Checks that `shown` is `original` with only listed nodes cut, each the way its kind cuts.
    fn same_but_cut(original: &Value, shown: &Value, path: &str, cuts: Option<&[Value]>) {
        let listed = |kind: &str| {
            cuts.is_none_or(|cuts| {
                cuts.iter()
                    .any(|cut| cut["path"] == path && cut["type"] == kind)
            })
        };
        match (original, shown) {
            (Value::String(original), Value::String(shown)) => {
                assert!(
                    original.starts_with(shown.as_str()),
                    "{path} is not a prefix"
                );
                assert!(
                    shown == original || listed("string"),
                    "{path} cut but not listed"
                );
            }
            (Value::Array(original), Value::Array(shown)) => {
                assert!(
                    shown.len() == original.len() || listed("array"),
                    "{path} shortened but not listed"
                );
                for (index, item) in shown.iter().enumerate() {
                    same_but_cut(&original[index], item, &format!("{path}/{index}"), cuts);
                }
            }
            (Value::Object(original), Value::Object(shown)) => {
                assert!(
                    shown.len() == original.len() || listed("object"),
                    "{path} lost keys but not listed"
                );
                for (key, item) in shown {
                    let token = key.replace('~', "~0").replace('/', "~1");
                    same_but_cut(&original[key], item, &format!("{path}/{token}"), cuts);
                }
            }
            _ => assert_eq!(original, shown, "{path} changed"),
        }
    }

    #[test]
    fn tests_that_random_values_stay_in_budget_keep_their_types_and_reassemble() {
        let mut random = Lcg(7);
        let mut store = Store::default();
        let mut bounded_count = 0;
        for case in 0..80 {
            let original = random.tree(3, &mut 150_000);
            let bounded = store.limit(report(original.clone()), None).0;
            assert!(bounded.to_string().len() <= BUDGET, "case {case}");
            assert_eq!(
                super::value_type(&bounded["value"]),
                super::value_type(&original)
            );
            let Some(meta) = bounded["truncated"].get("value") else {
                assert_eq!(bounded["value"], original, "case {case}");
                continue;
            };
            bounded_count += 1;
            if original.is_string() {
                continue;
            }
            let cuts = meta["cut"].as_array().expect("cut list");
            let listed = meta.get("cutOmitted").is_none().then_some(cuts.as_slice());
            same_but_cut(&original, &bounded["value"], "", listed);
            for cut in cuts {
                let node = original
                    .pointer(cut["path"].as_str().expect("path"))
                    .expect("cut path names a node");
                if let Some(text) = node.as_str() {
                    let shown = bounded["value"]
                        .pointer(cut["path"].as_str().expect("path"))
                        .and_then(Value::as_str)
                        .expect("cut string is shown");
                    assert_eq!(
                        remainder(&mut store, cut["next"].clone(), shown.into()),
                        text,
                        "case {case}"
                    );
                } else {
                    let tail = remainder(&mut store, cut["next"].clone(), String::new());
                    assert!(pretty(node).ends_with(&tail), "case {case}");
                }
            }
            assert_eq!(
                remainder(&mut store, meta["next"].clone(), String::new()),
                pretty(&original),
                "case {case}"
            );
        }
        assert!(
            bounded_count > 20,
            "only {bounded_count} cases exceeded the budget"
        );
    }

    #[test]
    fn tests_that_an_unretained_value_lists_its_cuts_without_continuations() {
        let original = report(json!({"n":1, "snap":"x".repeat(40_000), "rows":vec!["row"; 9_000]}));
        let bounded = super::bound(
            &original,
            &super::Link {
                id: "r000000000000",
                files: None,
            },
            false,
        );
        assert!(bounded.to_string().len() <= BUDGET);
        assert!(bounded["truncated"]["unavailable"].is_string());
        let meta = &bounded["truncated"]["value"];
        assert!(meta.get("next").is_none());
        let cuts = meta["cut"].as_array().expect("cut list");
        assert_eq!(cuts.len(), 2);
        assert!(cuts.iter().all(|cut| cut.get("next").is_none()));
    }

    async fn bounded_with_files(store: &mut Store, original: Value, dir: &Path) -> Value {
        let (bounded, disk) = store.limit(original, Some(dir));
        let bounded = disk.apply(bounded).await;
        assert!(bounded.to_string().len() <= BUDGET);
        bounded
    }

    fn file(bounded: &Value, field: &str) -> PathBuf {
        PathBuf::from(
            bounded["truncated"][field]["file"]
                .as_str()
                .expect("truncated field links its file"),
        )
    }

    #[tokio::test]
    async fn tests_that_truncated_fields_are_saved_as_files_that_leave_with_their_results() {
        let root = tempfile::TempDir::new().expect("expected test value");
        let dir = root.path().join("run-results");
        fs::create_dir_all(&dir).expect("expected test value");
        let stale = dir.join("r000000000000-value.txt");
        fs::write(&stale, "left by a crashed process").expect("expected test value");
        fs::File::options()
            .write(true)
            .open(&stale)
            .and_then(|file| {
                file.set_modified(SystemTime::now() - Duration::from_secs(2 * 24 * 60 * 60))
            })
            .expect("expected test value");
        // Another live server sharing the state dir keeps its recent file.
        let foreign = dir.join("r111111111111-value.txt");
        fs::write(&foreign, "another live process").expect("expected test value");
        let mut store = Store::default();
        let text = aria(3000);
        let mut original = report(json!(text));
        original["logs"] = json!(["log line\n".repeat(3000)]);
        let first = bounded_with_files(&mut store, original.clone(), &dir).await;
        assert!(!stale.exists() && foreign.exists());
        let (value, logs) = (file(&first, "value"), file(&first, "logs"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&value)
                .expect("expected test value")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(value.is_absolute() && value.starts_with(&dir));
        assert_eq!(value.extension().and_then(|ext| ext.to_str()), Some("txt"));
        assert_eq!(fs::read_to_string(&value).expect("saved value"), text);
        assert_eq!(
            fs::read_to_string(&logs).expect("saved logs"),
            pretty(&original["logs"])
        );
        // An object's file is the pretty JSON its whole-field continuation reads.
        let object = json!({"snap":text, "n":1});
        let second = bounded_with_files(&mut store, report(object.clone()), &dir).await;
        assert_eq!(
            fs::read_to_string(file(&second, "value")).expect("saved object"),
            pretty(&object)
        );
        // A whole small string still links no file.
        let mut traced = report(json!("small"));
        traced["logs"] = json!(["log line\n".repeat(3000)]);
        let third = bounded_with_files(&mut store, traced, &dir).await;
        assert!(third["truncated"]["value"].get("file").is_none());
        // 16 results fit; the 17th evicts only the first.
        for _ in 0..14 {
            bounded_with_files(&mut store, report(json!("x".repeat(30_000))), &dir).await;
        }
        assert!(!value.exists() && !logs.exists());
        assert!(file(&second, "value").exists());
        assert_eq!(
            fs::read_dir(&dir).expect("expected test value").count(),
            16 + 1,
            "one file per retained result (the third keeps only its logs), plus the foreign one"
        );
    }

    #[tokio::test]
    async fn tests_that_a_failed_write_drops_only_its_file_link() {
        let root = tempfile::TempDir::new().expect("expected test value");
        let blocker = root.path().join("not-a-directory");
        fs::write(&blocker, "").expect("expected test value");
        let mut store = Store::default();
        let (bounded, disk) = store.limit(
            report(json!("x".repeat(30_000))),
            Some(&blocker.join("run-results")),
        );
        assert!(bounded["truncated"]["value"]["file"].is_string());
        let bounded = disk.apply(bounded).await;
        assert!(bounded["truncated"]["value"].get("file").is_none());
        let next = bounded["truncated"]["value"]["next"].clone();
        let shown = bounded["value"]
            .as_str()
            .expect("string prefix")
            .to_string();
        assert_eq!(remainder(&mut store, next, shown), "x".repeat(30_000));
    }

    #[test]
    fn tests_that_a_larger_limit_returns_a_bigger_window_and_keeps_it_in_next() {
        let text = format!("{}{}", aria(3000), "abc".repeat(100_000));
        let mut store = Store::default();
        let (bounded, _) = store.limit(report(json!(text)), None);
        let prefix = bounded["value"].as_str().expect("string prefix");
        let id = bounded["truncated"]["result"]
            .as_str()
            .expect("expected test value");
        let window = store
            .retrieve(id, "value", None, prefix.len(), super::LIMIT_MAX)
            .expect("expected test value");
        let size = window.to_string().len();
        assert!(
            (90_000..=super::LIMIT_MAX).contains(&size),
            "window {size} bytes"
        );
        assert_eq!(window["truncated"]["next"]["limit"], super::LIMIT_MAX);
        assert_eq!(
            remainder(
                &mut store,
                window["truncated"]["next"].clone(),
                format!("{prefix}{}", window["value"].as_str().expect("window text"))
            ),
            text
        );
        let small = store
            .retrieve(id, "value", None, 0, super::LIMIT_MIN)
            .expect("expected test value");
        assert!(small.to_string().len() <= super::LIMIT_MIN);
        assert!(small["truncated"]["end"].as_u64() > Some(0));
    }

    #[tokio::test]
    async fn tests_that_a_new_folder_is_private_and_files_count_toward_the_store_budget() {
        let root = tempfile::TempDir::new().expect("expected test value");
        let dir = root.path().join("run-results");
        let mut store = Store::default();
        let small = bounded_with_files(&mut store, report(json!("x".repeat(30_000))), &dir).await;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dir)
                .expect("expected test value")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        // A 33 MiB string fits the 64 MiB store, but not together with its 33 MiB file.
        let big = bounded_with_files(&mut store, report(json!("y".repeat(33 << 20))), &dir).await;
        assert!(big["truncated"]["value"].get("file").is_none());
        assert!(big["truncated"]["value"]["next"].is_object());
        assert!(file(&small, "value").exists());
        assert_eq!(fs::read_dir(&dir).expect("expected test value").count(), 1);
    }

    #[test]
    fn tests_that_an_early_newline_does_not_shrink_a_retrieval_window() {
        let mut store = Store::default();
        let (bounded, _) = store.limit(report(json!({"n":1, "snap":"z".repeat(90_000)})), None);
        let next = &bounded["truncated"]["value"]["next"];
        let window = store
            .retrieve(
                next["result"].as_str().expect("expected test value"),
                "value",
                None,
                0,
                BUDGET,
            )
            .expect("expected test value");
        let shown = window["value"].as_str().expect("window text").len();
        assert!(
            shown > 20_000,
            "first pretty-JSON window shows {shown} bytes"
        );
    }
}
